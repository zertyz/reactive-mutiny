//! Allows creating `multi`s, which represent pairs of (`producer`, `event pipeline`) that may be used to
//! `produce()` asynchronous payloads to be processed by a multiple `event pipeline` Streams -- and executed
//! by async tasks.
//!
//! Usage:
//! ```nocompile
//!    fn local_on_event(stream: impl Stream<Item=String>) -> impl Stream<Item=Arc<String>> {
//!        stream.inspect(|message| println!("To Zeta: '{}'", message))
//!    }
//!    fn zeta_on_event(stream: impl Stream<Item=String>) -> impl Stream<Item=Arc<String>> {
//!        stream.inspect(|message| println!("ZETA: Received a message: '{}'", message))
//!    }
//!    fn earth_on_event(stream: impl Stream<Item=String>) -> impl Stream<Item=Arc<String>> {
//!        stream.inspect(|sneak_peeked_message| println!("EARTH: Sneak peeked a message to Zeta Reticuli: '{}'", sneak_peeked_message))
//!    }
//!    let multi = MultiBuilder::new()
//!        .on_stream_close(|_| async {})
//!        .into_executable()
//!        .spawn_non_futures_non_fallible_executor("doc_test() local onEvent()", local_on_event).await
//!        .spawn_non_futures_non_fallible_executor("doc_test() zeta  onEvent()", zeta_on_event).await
//!        .spawn_non_futures_non_fallible_executor("doc_test() earth onEvent()", earth_on_event).await
//!        .handle();
//!    let producer = multi.producer_closure();
//!    producer("I've just arrived!".to_string()).await;
//!    producer("Nothing really interesting here... heading back home!".to_string()).await;
//!    multi.close().await;
//! ```

mod multi;
pub use multi::*;

pub mod channels;


/// Tests & enforces the requisites & expose good practices & exercises the API of of the [multi](self) module
#[cfg(any(test,doc))]
mod tests {
    use super::*;
    use super::super::{
        instruments::Instruments,
        mutiny_stream::MutinyStream,
        types::FullDuplexMultiChannel,
    };
    use std::{
        future::Future,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed},
        },
        time::{
            Duration,
            SystemTime,
        },
        io::Write,
        pin::Pin,
    };
    use futures::{stream::{self, Stream, StreamExt}};
    use minstant::Instant;
    use tokio::sync::Mutex;


    type MultiChannelType<ItemType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = channels::arc::crossbeam::Crossbeam<'static, ItemType, BUFFER_SIZE, MAX_STREAMS>;


    #[ctor::ctor]
    fn suite_setup() {
        simple_logger::SimpleLogger::new().with_utc_timestamps().init().unwrap_or_else(|_| eprintln!("--> LOGGER WAS ALREADY STARTED"));
    }

    /// exercises the code present on the documentation
    #[cfg_attr(not(doc),tokio::test)]
    async fn doc_tests() -> Result<(), Box<dyn std::error::Error>> {
        fn local_on_event(stream: impl Stream<Item=Arc<String>>) -> impl Stream<Item=Arc<String>> {
            stream.inspect(|message| println!("To Zeta: '{}'", message))
        }
        fn zeta_on_event(stream: impl Stream<Item=Arc<String>>) -> impl Stream<Item=Arc<String>> {
            stream.inspect(|message| println!("ZETA: Received a message: '{}'", message))
        }
        fn earth_on_event(stream: impl Stream<Item=Arc<String>>) -> impl Stream<Item=Arc<String>> {
            stream.inspect(|sneak_peeked_message| println!("EARTH: Sneak peeked a message to Zeta Reticuli: '{}'", sneak_peeked_message))
        }
        let multi: Multi<String, MultiChannelType<String, 1024, 4>, {Instruments::LogsWithMetrics.into()}, Arc<String>> = Multi::new("doc_test() event");
        multi.spawn_non_futures_non_fallible_executor(1, "local screen", local_on_event, |_| async {}).await?;
        multi.spawn_non_futures_non_fallible_executor(1, "zeta receiver", zeta_on_event, |_| async {}).await?;
        multi.spawn_non_futures_non_fallible_executor(1, "earth snapper", earth_on_event, |_| async {}).await?;
        let producer = |item: &str| multi.try_send_movable(item.to_string());
        producer("I've just arrived!");
        producer("Nothing really interesting here... heading back home!");
        multi.close(Duration::ZERO).await;
        Ok(())
    }

    /// guarantees that one of the simplest possible testable 'multi' pipelines will get executed all the way through
    #[cfg_attr(not(doc),tokio::test)]
    async fn simple_pipelines() -> Result<(), Box<dyn std::error::Error>> {
        const EXPECTED_SUM: u32 = 17;
        const PARTS: &[u32] = &[9, 8];

        // two pipelines are set for this test -- these are the variables they change
        let observed_sum_1 = Arc::new(AtomicU32::new(0));
        let observed_sum_2 = Arc::new(AtomicU32::new(0));

        let multi: Multi<u32, MultiChannelType<u32, 1024, 2>, {Instruments::LogsWithMetrics.into()}, Arc<u32>> = Multi::new("Simple Event");
            // #1 -- event pipeline
        multi.spawn_non_futures_non_fallible_executor(1, "Pipeline #1",
                                                      |stream| {
                                                          let observed_sum = Arc::clone(&observed_sum_1);
                                                          stream.map(move |number: Arc<u32>| observed_sum.fetch_add(*number, Relaxed))
                                                      },
                                                      |_| async {}).await?;
            // #2 -- event pipeline
        multi.spawn_non_futures_non_fallible_executor(1, "Pipeline #2",
                                                      |stream| {
                                                           let observed_sum = Arc::clone(&observed_sum_2);
                                                           stream.map(move |number| observed_sum.fetch_add(*number, Relaxed))
                                                       },
                                                      |_| async {}).await?;
        let producer = |item| multi.send(|slot| *slot = item);

        // produces some events concurrently -- they will be shared with all executable pipelines
        let shared_producer = &producer;
        stream::iter(PARTS)
            .for_each_concurrent(1, |number| async move {
                shared_producer(*number)
            }).await;

        multi.close(Duration::ZERO).await;
        assert_eq!(observed_sum_1.load(Relaxed), EXPECTED_SUM, "not all events passed through our pipeline #1");
        assert_eq!(observed_sum_2.load(Relaxed), EXPECTED_SUM, "not all events passed through our pipeline #2");
        Ok(())
    }

    /// shows how pipelines / executors may be cancelled / deleted / unsubscribed
    /// from the main event producer
    #[cfg_attr(not(doc),tokio::test)]
    async fn delete_pipelines() {
        const PIPELINE_1: &str     = "Pipeline #1";
        const PIPELINE_2: &str     = "Pipeline #2";
        const TIMEOUT:    Duration = Duration::ZERO;

        let multi: Multi<u32, MultiChannelType<u32, 1024, 2>, {Instruments::LogsWithMetrics.into()}, Arc<u32>> = Multi::new("Event with come and go pipelines");

        // correct creating & cancelling executors
        multi.spawn_non_futures_non_fallible_executor(1, PIPELINE_1, |s| s, |_| async {}).await
            .expect("Single instance of PIPELINE_1 should have been created");
        multi.spawn_non_futures_non_fallible_executor(1, PIPELINE_2, |s| s, |_| async {}).await
            .expect("Single instance of PIPELINE_2 should have been created");
        assert!(multi.flush_and_cancel_executor(PIPELINE_1, TIMEOUT).await, "'{}' was spawned, therefore it should have been cancelled", PIPELINE_1);
        assert!(multi.flush_and_cancel_executor(PIPELINE_2, TIMEOUT).await, "'{}' was spawned, therefore it should have been cancelled", PIPELINE_2);

        // attempt to double create -- which is an error condition
        multi.spawn_non_futures_non_fallible_executor(1, PIPELINE_1, |s| s, |_| async {}).await
            .expect("Single instance of PIPELINE_1 should have been created");
        let result = multi.spawn_non_futures_non_fallible_executor(1, PIPELINE_1, |s| s, |_| async {}).await;
        assert!(result.is_err(), "Second attempt to insert PIPELINE_1 should have failed");

            // attempt to double cancel
        assert!(multi.flush_and_cancel_executor(PIPELINE_1, TIMEOUT).await, "'{}' was spawned once, therefore it should have been cancelled", PIPELINE_1);
        assert!(!multi.flush_and_cancel_executor(PIPELINE_1, TIMEOUT).await, "A pipeline cannot be cancelled twice");

        // attempt to cancel non-existing
        assert!(!multi.flush_and_cancel_executor(PIPELINE_2, TIMEOUT).await, "An unexisting pipeline cannot be reported as having been cancelled");

        // stress test
        // (maybe this may be improved by detecting any memory leaks)
        for _ in 0..128 {
            multi.spawn_non_futures_non_fallible_executor(1, PIPELINE_1, |s| s, |_| async {}).await
                .expect("Single instance of PIPELINE_1 should have been created");
            multi.spawn_non_futures_non_fallible_executor(1, PIPELINE_2, |s| s, |_| async {}).await
                .expect("Single instance of PIPELINE_2 should have been created");
            assert!(multi.flush_and_cancel_executor(PIPELINE_1, TIMEOUT).await, "'{}' was spawned, therefore it should have been cancelled", PIPELINE_1);
            assert!(multi.flush_and_cancel_executor(PIPELINE_2, TIMEOUT).await, "'{}' was spawned, therefore it should have been cancelled", PIPELINE_2);
        }

        // finally, produce an event to check that all is fine
        let last_message = Arc::new(Mutex::new(0u32));
        let last_message_ref = Arc::clone(&last_message);
        multi.spawn_non_futures_non_fallible_executor(1, PIPELINE_2,
                                                      |s| s.inspect(move |n| *last_message_ref.try_lock().unwrap() = *(n as &u32)),
                                                      |_| async {} ).await
            .expect("Single instance of PIPELINE_2 should have been created");

        multi.send(|slot| *slot = 97);
        multi.close(Duration::ZERO).await;

        assert_eq!(*last_message.try_lock().unwrap(), 97, "event didn't complete");

        // executor will be a tokio task -- join handle -- which we may simply cancel
        // multi_builder might have a copy of it? or we should let the caller to hold it & pass it on? I should write a realistic test to check on that
        // depending on the decision above, the key would be the pipeline name... and I must assure no two pipelines are added with the same name, I should assert they won't be deleted twice, etc...
        // maybe we do have to key the tokio join handles anyway... because deleting an executor / pipeline means it should be taken out of the pipelines vector -- or, now, the pipelines hashmap
    }

    /// shows how we may call async functions inside `multi` pipelines
    /// and work with "future" elements
    #[cfg_attr(not(doc),tokio::test)]
    async fn async_elements() -> Result<(), Box<dyn std::error::Error>> {
        const EXPECTED_SUM: u32 = 30;
        const PARTS: &[u32] = &[9, 8, 7, 6];

        // each pipeline will report their execution to the following variables
        let observed_sum_1 = Arc::new(AtomicU32::new(0));
        let observed_sum_2 = Arc::new(AtomicU32::new(0));

        // now goes the stream builder functions -- or pipeline builders.
        // notice how to transform a regular event into a future event &
        // how to pass it down the pipeline. Also notice the required (as of Rust 1.63)
        // moving of Arc local variables so they will be accessible

        let pipeline1_builder = |stream: MutinyStream<'static, u32, _, Arc<u32>>| {
            let observed_sum = Arc::clone(&observed_sum_1);
            stream
                .map(|number| async move {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    number
                })
                .map(move |number| {
                    let observed_sum = Arc::clone(&observed_sum);
                    async move {
                        let number = number.await;
                        observed_sum.fetch_add(*number, Relaxed);
                        number
                    }
                })
                .map(|number| async move {
                    let number = number.await;
                    println!("PIPELINE 1: Just added # {}", number);
                    Ok(number)
                })
        };
        let pipeline2_builder = |stream: MutinyStream<'static, u32, _, Arc<u32>>| {
            let observed_sum = Arc::clone(&observed_sum_2);
            stream
                .map(|number| async move {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    number
                })
                .map(move |number| {
                    let observed_sum = Arc::clone(&observed_sum);
                    async move {
                        let number = number.await;
                        observed_sum.fetch_add(*number, Relaxed);
                        number
                    }
                })
                .map(|number| async move {
                    let number = number.await;
                    println!("PIPELINE 2: Just added # {}", number);
                    Ok(number)
                })
        };

        let multi: Multi<u32, MultiChannelType<u32, 1024, 2>, {Instruments::LogsWithMetrics.into()}, Arc<u32>> = Multi::new("async event");
        multi.spawn_executor(PARTS.len() as u32, Duration::from_secs(2), "Stream Pipeline #1", pipeline1_builder, |_| async {}, |_| async {}).await?;
        multi.spawn_executor(PARTS.len() as u32, Duration::from_secs(2), "Stream Pipeline #2", pipeline2_builder, |_| async {}, |_| async {}).await?;

        let producer = |item| multi.send(|slot| *slot = item);

        let shared_producer = &producer;
        stream::iter(PARTS)
            .for_each_concurrent(1, |number| async move {
                shared_producer(*number)
            }).await;

        multi.close(Duration::ZERO).await;
        assert_eq!(observed_sum_1.load(Relaxed), EXPECTED_SUM, "not all events passed through our async pipeline #1");
        assert_eq!(observed_sum_2.load(Relaxed), EXPECTED_SUM, "not all events passed through our async pipeline #2");
        Ok(())
    }

    /// assures stats are computed appropriately for every executor,
    /// according to the right instrumentation specifications
    #[cfg_attr(not(doc),tokio::test(flavor="multi_thread", worker_threads=2))]
    #[ignore]   // flaky if ran in multi-thread: timeout measurements go south
    async fn stats() -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(not(debug_assertions))]
        const N_PIPELINES: usize = 256;
        #[cfg(debug_assertions)]
        const N_PIPELINES: usize = 128;

        // asserts spawn_non_futures_non_fallible_executor() register statistics appropriately:
        // with counters, but without average futures resolution time measurements
        let event_name = "non_future/non_fallible event";
        let multi: Multi<String, MultiChannelType<String, 256, N_PIPELINES>, {Instruments::LogsWithMetrics.into()}, Arc<String>> = Multi::new(event_name);
        for i in 0..N_PIPELINES {
            multi.spawn_non_futures_non_fallible_executor(1, format!("Pipeline #{} for {}", i, event_name), |stream| stream, |_| async {}).await?;
        }
        let producer = |item: &str| multi.try_send_movable(item.to_string());
        producer("'only count successes' payload");
        multi.close(Duration::from_secs(5)).await;
        assert_eq!(N_PIPELINES, multi.executor_infos.read().await.len(), "Number of created pipelines doesn't match");
        for (i, executor_info) in multi.executor_infos.read().await.values().enumerate() {
            let (ok_counter, ok_avg_futures_resolution_duration) = executor_info.stream_executor.ok_events_avg_future_duration.lightweight_probe();
            assert_eq!(ok_counter,                               1,    "counter of successful '{event_name}' events is wrong for pipeline #{i}");
            assert_eq!(ok_avg_futures_resolution_duration,       -1.0, "avg futures resolution time of successful '{event_name}' events is wrong  for pipeline #{i} -- since it is a non-future, avg times should be always -1.0");
            let (failures_counter, failures_avg_futures_resolution_duration) = executor_info.stream_executor.failed_events_avg_future_duration.lightweight_probe();
            assert_eq!(failures_counter,                         0,    "counter of unsuccessful '{event_name}' events is wrong  for pipeline #{i} -- since it is a non-fallible event, failures should always be 0");
            assert_eq!(failures_avg_futures_resolution_duration, 0.0,  "avg futures resolution time of unsuccessful '{event_name}' events is wrong  for pipeline #{i} -- since it is a non-fallible event,, avg times should be always 0.0");
            let (timeouts_counter, timeouts_avg_futures_resolution_duration) = executor_info.stream_executor.timed_out_events_avg_future_duration.lightweight_probe();
            assert_eq!(timeouts_counter,                         0,    "counter of timed out '{event_name}' events is wrong  for pipeline #{i} -- since it is a non-future event, timeouts should always be 0");
            assert_eq!(timeouts_avg_futures_resolution_duration, 0.0,  "avg futures resolution time of timed out '{event_name}' events is wrong  for pipeline #{i} -- since it is a non-future event,, avg timeouts should be always 0.0");
        }

        // asserts spawn_executor() register statistics appropriately:
        // with counters & with average futures resolution time measurements
        let event_name = "future & fallible event";
        let multi: Multi<String, MultiChannelType<String, 256, N_PIPELINES>, {Instruments::LogsWithMetrics.into()}, Arc<String>> = Multi::new(event_name);
        for i in 0..N_PIPELINES {
            multi.spawn_executor(1, Duration::from_millis(150), format!("Pipeline #{} for {}", i, event_name),
                                 |stream| {
                                         stream.map(|payload| async move {
                                             if payload.contains("unsuccessful") {
                                                 tokio::time::sleep(Duration::from_millis(50)).await;
                                                 Err(Box::from(format!("failing the pipeline, as requested")))
                                             } else if payload.contains("timeout") {
                                                 tokio::time::sleep(Duration::from_millis(200)).await;
                                                 Ok(Arc::new(String::from("this answer will never make it -- stream executor times out after 100ms")))
                                             } else {
                                                 tokio::time::sleep(Duration::from_millis(100)).await;
                                                 Ok(payload)
                                             }
                                         })
                                     },
                                 |_| async {},
                                 |_| async {}
            ).await?;
        }
        let producer = |item: &str| multi.try_send_movable(item.to_string());
        // for this test, produce each event twice
        for _ in 0..2 {
            producer("'successful' payload");
            producer("'unsuccessful' payload");
            producer("'timeout' payload");
        }
        multi.close(Duration::from_secs(5)).await;
        assert_eq!(N_PIPELINES, multi.executor_infos.read().await.len(), "Number of created pipelines doesn't match");
        for (i, executor_info) in multi.executor_infos.read().await.values().enumerate() {
            let (ok_counter, ok_avg_futures_resolution_duration) = executor_info.stream_executor.ok_events_avg_future_duration.lightweight_probe();
            assert_eq!(ok_counter,                                             2,    "counter of successful '{event_name}' events is wrong for pipeline #{i}");
            assert!((ok_avg_futures_resolution_duration-0.100).abs()        < 15e-2, "avg futures resolution time of successful '{event_name}' events is wrong for pipeline #{i} -- it should be 0.1s, but was {ok_avg_futures_resolution_duration}s");
            let (failures_counter, failures_avg_futures_resolution_duration) = executor_info.stream_executor.failed_events_avg_future_duration.lightweight_probe();
            assert_eq!(failures_counter,                                      2,    "counter of unsuccessful '{event_name}' events is wrong for pipeline #{i}");
            assert!((failures_avg_futures_resolution_duration-0.050).abs() < 15e-2, "avg futures resolution time of unsuccessful '{event_name}' events is wrong for pipeline #{i} -- it should be 0.05s, but was {failures_avg_futures_resolution_duration}s");
            let (timeouts_counter, timeouts_avg_futures_resolution_duration) = executor_info.stream_executor.timed_out_events_avg_future_duration.lightweight_probe();
            assert_eq!(timeouts_counter,                                      2,    "counter of timed out '{event_name}' events is wrong for pipeline #{i}");
            assert!((timeouts_avg_futures_resolution_duration-0.150).abs() < 15e-2, "avg futures resolution time of timed out '{event_name}' events is wrong for pipeline #{i} -- it should be 0.150s, but was {timeouts_avg_futures_resolution_duration}s");
        }
        Ok(())
    }


    /// shows how to fuse multiple `multi`s, triggering payloads for another `multi` when certain conditions are met:
    /// events TWO and FOUR will set a shared state between them, firing SIX.
    /// NOTE: every 'on_X_event()' function will be called twice, since we're setting 2 pipelines for each `multi`
    #[cfg_attr(not(doc),tokio::test)]
    async fn demux() -> Result<(), Box<dyn std::error::Error>> {
        let shared_state = Arc::new(AtomicU32::new(0));
        let two_fire_count = Arc::new(AtomicU32::new(0));
        let four_fire_count = Arc::new(AtomicU32::new(0));
        let six_fire_count = Arc::new(AtomicU32::new(0));

        // SIX event
        let on_six_event = |stream: MutinyStream<'static, bool, _, Arc<bool>>| {
            let six_fire_count = Arc::clone(&six_fire_count);
            stream.inspect(move |_| {
                six_fire_count.fetch_add(1, Relaxed);
            })
        };
        let six_multi = Multi::<bool, MultiChannelType<bool, 1024, 2>, {Instruments::LogsWithMetrics.into()}, Arc<bool>>::new("SIX");
        six_multi.spawn_non_futures_non_fallible_executor(1, "Pipeline #1", on_six_event, |_| async {}).await?;
        six_multi.spawn_non_futures_non_fallible_executor(1, "Pipeline #2", on_six_event, |_| async {}).await?;
        let six_multi = Arc::new(six_multi);
        // assures we'll close SIX only once
        let can_six_be_closed = Arc::new(AtomicBool::new(true));
        let six_multi_ref = Arc::clone(&six_multi);
        let six_closer = Arc::new(move || {
            let can_six_be_closed = Arc::clone(&can_six_be_closed);
            let six_multi = Arc::clone(&six_multi_ref);
            async move {
                if can_six_be_closed.swap(false, Relaxed) {
                    six_multi.close(Duration::ZERO).await;
                }
            }
        });

        // TWO event
        let on_two_event = |stream: MutinyStream<'static, u32, _, Arc<u32>>| {
            let two_fire_count = Arc::clone(&two_fire_count);
            let shared_state = Arc::clone(&shared_state);
            let six_multi = Arc::clone(&six_multi);
            stream
                .map(move |payload| {
                    let two_fire_count = Arc::clone(&two_fire_count);
                    let shared_state = Arc::clone(&shared_state);
                    let six_multi = Arc::clone(&six_multi);
                    async move {
                        two_fire_count.fetch_add(1, Relaxed);
                        if *payload & 2 == 2 {
                            let previous_state = shared_state.fetch_or(2, Relaxed);
                            if previous_state & 6 == 6 {
                                shared_state.store(0, Relaxed); // reset the triggering state
                                six_multi.send(|slot| *slot = true);
                            }
                        } else if *payload == 97 {
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                        payload
                    }
            })
            .buffer_unordered(1)
        };
        let six_closer_for_two = Arc::clone(&six_closer);
        let on_two_close_builder = || {
            let six_closer_for_two = Arc::clone(&six_closer_for_two);
            move |_| {
                let six_closer_for_two = Arc::clone(&six_closer_for_two);
                async move {
                    six_closer_for_two().await;
                }
            }
        };
        let two_multi: Multi<u32, MultiChannelType<u32, 1024, 2>, {Instruments::LogsWithMetrics.into()}, Arc<u32>> = Multi::new("TWO");
        two_multi.spawn_non_futures_non_fallible_executor(1, "Pipeline #1", on_two_event, on_two_close_builder()).await?;
        two_multi.spawn_non_futures_non_fallible_executor(1, "Pipeline #2", on_two_event, on_two_close_builder()).await?;
        let two_multi = Arc::new(two_multi);
        let two_producer = |item| two_multi.send(|slot| *slot = item);

        // FOUR event
        let on_four_event = |stream: MutinyStream<'static, u32, _, Arc<u32>>| {
            let four_fire_count = Arc::clone(&four_fire_count);
            let shared_state = Arc::clone(&shared_state);
            let six_multi = Arc::clone(&six_multi);
            stream
                .map(move |payload| {
                    let four_fire_count = Arc::clone(&four_fire_count);
                    let shared_state = Arc::clone(&shared_state);
                    let six_multi = Arc::clone(&six_multi);
                    async move {
                        four_fire_count.fetch_add(1, Relaxed);
                        if *payload & 4 == 4 {
                            let previous_state = shared_state.fetch_or(4, Relaxed);
                            if previous_state & 6 == 6 {
                                shared_state.store(0, Relaxed); // reset the triggering state
                                six_multi.send(|slot| *slot = true);
                            }
                        } else if *payload == 97 {
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                        payload
                    }
                })
                .buffer_unordered(1)
        };
        let six_closer_for_four = Arc::clone(&six_closer);
        let on_four_close_builder = || {
            let six_closer_for_four = Arc::clone(&six_closer_for_four);
            move |_| {
                let six_closer_for_four = Arc::clone(&six_closer_for_four);
                async move {
                    six_closer_for_four().await;
                }
            }
        };
        let four_multi: Multi<u32, MultiChannelType<u32, 1024, 2>, {Instruments::LogsWithMetrics.into()}, Arc<u32>> = Multi::new("FOUR");
        four_multi.spawn_non_futures_non_fallible_executor(1, "Pipeline #1", on_four_event, on_four_close_builder()).await?;
        four_multi.spawn_non_futures_non_fallible_executor(1, "Pipeline #2", on_four_event, on_four_close_builder()).await?;
        let four_multi = Arc::new(four_multi);
        let four_producer = |item| four_multi.send(|slot| *slot = item);

        // NOTE: the special value of 97 causes a sleep on both TWO and FOUR pipelines
        //       so we can test race conditions for the 'close producer' functions
        two_producer(1);
        two_producer(2);
        four_producer(97);    // sleeps, forcing any bugs regarding racing conditions to blow up
        four_producer(1);
        four_producer(2);
        four_producer(3);
        four_producer(4);
        two_producer(3);
        two_producer(4);
        four_producer(5);
        // closing TWO (and, therefore, SIX) before all elements of FOUR are processed would cause the later consumer to try to publish to SIX (when it is already closed) --
        // this is why both events should be closed atomically in this case -- both share the closeable resource SIX -- which happens to be another multi, but could be any other resource
        multis_close_async!(Duration::ZERO, two_multi, four_multi);  // notice SIX is closed here as well -- when either TWO or FOUR are closed

        assert_eq!(two_fire_count.load(Relaxed),  4 * 2, "Wrong number of events processed for TWO");
        assert_eq!(four_fire_count.load(Relaxed), 6 * 2, "Wrong number of events processed for FOUR");
        assert_eq!(six_fire_count.load(Relaxed),  1 * 2, "Wrong number of events processed for SIX");
        Ok(())
    }

    /// shows how to handle errors when they happen anywhere down the pipeline
    /// -- and what happens when they are not handled.
    /// + tests meaningful messages are produced
    #[cfg_attr(not(doc),tokio::test)]
    async fn error_handling() -> Result<(), Box<dyn std::error::Error>> {

        let on_err_count = Arc::new(AtomicU32::new(0));

        fn on_fail_when_odd_event(stream: impl Stream<Item=Arc<u32>>) -> impl Stream<Item = impl Future<Output = Result<u32, Box<dyn std::error::Error + Send + Sync>> > + Send> {
            stream
                .map(|payload| async move {
                    if *payload % 2 == 0 {
                        Ok(*payload)
                    } else if *payload % 79 == 0 {
                        Err(format!("BLOW CODE received: {}", payload))
                    } else {
                        Err(format!("ODD payload received: {}", payload))
                    }
                })
                // treat known errors
                .filter_map(|payload| async {
                    let payload = payload.await;
                    match &payload {
                        Ok(ok_payload ) => {
                            println!("Payload {} ACCURATELY PROCESSED!", ok_payload);
                            Some(payload)
                        },
                        Err(ref err) => {
                            if err.contains("ODD") {
                                println!("Payload {} ERROR LOG -- this error is tolerable and this event will be skipped for the rest of the pipeline", err);
                                None
                            } else {
                                // other errors are "unknown" -- therefore, not tolerable nor treated nor recovered from... and will explode down the pipeline, causing the stream to close
                                Some(payload)
                            }
                        }
                        //unknown_error => Some(unknown_error),
                    }
                })
                .map(|payload| async {
                    let payload = payload?;
                    // if this is executed, the payload had no errors OR the error was handled and the failed event was filtered out
                    println!("Payload {} continued down the pipe ", payload);
                    Ok(payload)
                })
        }
        let on_err_count_clone_1 = Arc::clone(&on_err_count);
        let on_err_count_clone_2 = Arc::clone(&on_err_count);
        let multi: Multi<u32, MultiChannelType<u32, 1024, 2>, {Instruments::LogsWithMetrics.into()}, Arc<u32>> = Multi::new("Event with error handling");
        multi.spawn_executor(1,
                             Duration::from_millis(100),
                             "Pipeline #1",
                             on_fail_when_odd_event,
                             move |err| {
                                 let on_err_count_clone = Arc::clone(&on_err_count_clone_1);
                                 async move {
                                     on_err_count_clone.fetch_add(1, Relaxed);
                                     println!("Pipeline #1: ERROR CALLBACK WAS CALLED: '{:?}'", err);
                                 }
                             },
                             |_| async {}
            ).await?;
        multi.spawn_executor(1,
                             Duration::from_millis(100),
                             "Pipeline #2",
                             on_fail_when_odd_event,
                             move |err| {
                                 let on_err_count_clone = Arc::clone(&on_err_count_clone_2);
                                 async move {
                                     on_err_count_clone.fetch_add(1, Relaxed);
                                     println!("Pipeline #2: ERROR CALLBACK WAS CALLED: '{:?}'", err);
                                 }
                             },
                             |_| async {}
            ).await?;
        let producer = |item| multi.send(|slot| *slot = item);
        producer(0);
        producer(1);
        producer(2);
        producer(79);
        producer(80);
        multi.close(Duration::ZERO).await;

        assert_eq!(on_err_count.load(Relaxed), 1 * 2, "'on_err()' callback contract broken: events with handled errors should not call on_err(), the ones not 'caught', should -- twice, since we have 2 pipelines");
        Ok(())
    }

    /// verifies that our executors (each one on their own Tokio task) don't blow the latencies up -- when we have many of them in the waiting state.
    /// Two [multi]s are set: SIMPLE (with a single pipeline) and BLOATED (with several thousands)
    /// 1) Time is measured to produce & consume a SIMPLE event
    /// 2) BLOATED is created and a single payload is produced to get all of them activated -- a check is done that all got processed
    /// 3) (1) is repeated -- the production-to-consumption time should be (nearly?) unaffected
    #[cfg_attr(not(doc),tokio::test)]
    async fn undegradable_latencies() -> Result<(), Box<dyn std::error::Error>> {
        const BLOATED_PIPELINES_COUNT: usize = 256;

        let simple_count = Arc::new(AtomicU32::new(0));
        let simple_last_elapsed_nanos = Arc::new(AtomicU64::new(0));
        let bloated_count = Arc::new(AtomicU32::new(0));
        let bloated_last_elapsed_nanos = Arc::new(AtomicU64::new(0));

        let simple_multi: Multi<SystemTime, MultiChannelType<SystemTime, 1024, 1>, {Instruments::LogsWithMetrics.into()}, Arc<SystemTime>> = Multi::new("SIMPLE");
        simple_multi.spawn_non_futures_non_fallible_executor(1, "solo pipeline",
                                                             |stream| {
                                                                 let simple_count = Arc::clone(&simple_count);
                                                                 let simple_last_elapsed_nanos = Arc::clone(&simple_last_elapsed_nanos);
                                                                 stream.map(move |start: Arc<SystemTime>| {
                                                                     simple_last_elapsed_nanos.store(start.elapsed().unwrap().as_nanos() as u64, Relaxed);
                                                                     simple_count.fetch_add(1, Relaxed)
                                                                 })
                                                             },
                                                             |_| async {}).await?;
        let simple_producer = |item| simple_multi.send(|slot| *slot = item);

        // 1) Measure the time to produce & consume a SIMPLE event -- No other multi tokio tasks are available
        tokio::time::sleep(Duration::from_millis(10)).await;
        simple_producer(SystemTime::now());
        // waits for the event to be processed
        while simple_count.load(Relaxed) != 1 {
            tokio::task::yield_now().await;
        }
        let first_simple_duration = Duration::from_nanos(simple_last_elapsed_nanos.load(Relaxed));
        println!("1. Time to produce & consume a SIMPLE event (no other Multi Tokio tasks): {:?}", first_simple_duration);

        let bloated_multi: Multi<SystemTime, MultiChannelType<SystemTime, 16, BLOATED_PIPELINES_COUNT>, {Instruments::LogsWithMetrics.into()}, Arc<SystemTime>> = Multi::new("BLOATED");
        for i in 0..BLOATED_PIPELINES_COUNT {
            bloated_multi.spawn_non_futures_non_fallible_executor(1, format!("#{i})"),
                                                                  |stream| {
                                                                          let bloated_count = Arc::clone(&bloated_count);
                                                                          let bloated_last_elapsed_nanos = Arc::clone(&bloated_last_elapsed_nanos);
                                                                          stream.map(move |start| {
                                                                              bloated_last_elapsed_nanos.store(start.elapsed().unwrap().as_nanos() as u64, Relaxed);
                                                                              bloated_count.fetch_add(1, Relaxed)
                                                                          })
                                                                      },
                                                                  |_| async {}).await?;
        }
        let bloated_producer = |item| bloated_multi.send(|slot| *slot = item);

        // 2) Bloat the Tokio Runtime with a lot of tasks -- multi executors in our case -- verifying all of them will run once
        tokio::time::sleep(Duration::from_millis(10)).await;
        bloated_producer(SystemTime::now());
        // waits for the event to be processed
        while bloated_count.load(Relaxed) != BLOATED_PIPELINES_COUNT as u32 {
            tokio::task::yield_now().await;
        }
        let bloated_duration = Duration::from_nanos(bloated_last_elapsed_nanos.load(Relaxed));
        println!("2. Tokio Runtime is now BLOATED with {BLOATED_PIPELINES_COUNT} tasks -- all of them are multi executors. Time to produce + time for all pipelines to consume it: {:?}", bloated_duration);

        // 3) Measure (1) again. All BLOATED tasks are sleeping... so there should be no latency
        tokio::time::sleep(Duration::from_millis(10)).await;    // give a little time for all Streams to settle
        simple_producer(SystemTime::now());
        // waits for the event to be processed
        while simple_count.load(Relaxed) != 2 {
            tokio::task::yield_now().await;
        }
        let second_simple_duration = Duration::from_nanos(simple_last_elapsed_nanos.load(Relaxed));
        println!("3. Time to produce & consume another SIMPLE event (with lots of -- {BLOATED_PIPELINES_COUNT} -- sleeping Multi Tokio tasks): {:?}", second_simple_duration);

        const TOLERANCE_MICROS: u128 = 10;
        assert!(second_simple_duration < first_simple_duration || second_simple_duration.as_micros() - first_simple_duration.as_micros() < TOLERANCE_MICROS,
                "Tokio tasks' latency must be unaffected by whatever number of sleeping tasks there are (tasks executing our multi stream pipelines) -- task execution latencies exceeded the {TOLERANCE_MICROS}µs tolerance: with 0 sleeping: {:?}; with {BLOATED_PIPELINES_COUNT} sleeping: {:?}",
                first_simple_duration,
                second_simple_duration);

        simple_multi.close(Duration::ZERO).await;
        bloated_multi.close(Duration::ZERO).await;
        Ok(())
    }

    /// assures we're able to chain multiple multis while reusing the `Arc<T>` without any overhead
    #[cfg_attr(not(doc),tokio::test)]
    async fn chained_multis() -> Result<(), Box<dyn std::error::Error>> {
        let expected_msgs = vec![
            "Hello, beautiful world!",
            "Oh me, oh my, this will never do... Goodbye, cruel world!"
        ];
        let first_multi_msgs = Arc::new(std::sync::Mutex::new(vec![]));
        let first_multi_msgs_ref = Arc::clone(&first_multi_msgs);
        let second_multi_msgs = Arc::new(std::sync::Mutex::new(vec![]));
        let second_multi_msgs_ref = Arc::clone(&second_multi_msgs);

        let second_multi: Multi<String, MultiChannelType<String, 1024, 4>, {Instruments::LogsWithMetrics.into()}, Arc<String>> = Multi::new("second chained multi, receiving the Arc-wrapped event -- with no copying (and no additional Arc cloning)");
        second_multi.spawn_non_futures_non_fallible_executor(1, "second executor", move |stream| {
                stream.map(move |message| {
                    println!("`second_multi` received '{:?}'", message);
                    second_multi_msgs_ref
                        .lock().unwrap()
                        .push(message);
                })
            }, |_| async {}).await?;
        let second_multi = Arc::new(second_multi);
        let second_multi_ref = Arc::clone(&second_multi);
        let first_multi: Multi<String, MultiChannelType<String, 1024, 4>, {Instruments::LogsWithMetrics.into()}, Arc<String>> = Multi::new("first chained multi, receiving the original events");
        first_multi.spawn_non_futures_non_fallible_executor(1, "first executor", move |stream| {
                stream.map(move |message: Arc<String>| {
                    println!("`first_multi` received '{:?}'", message);
                    second_multi_ref.send_derived(&message);
                    first_multi_msgs_ref
                        .lock().unwrap()
                        .push(message);
                })
            }, |_| async {}).await?;
        let producer = |item: &str| first_multi.try_send_movable(item.to_string());
        expected_msgs.iter().for_each(|&msg| { producer(msg); });
        multis_close_async!(Duration::ZERO, first_multi, second_multi);
        let expected_msgs: Vec<Arc<String>> = expected_msgs.into_iter()
            .map(|msg| Arc::new(msg.to_string()))
            .collect();
        assert_eq!(*first_multi_msgs.lock().unwrap(), expected_msgs, "First multi didn't receive the expected messages");
        assert_eq!(*second_multi_msgs.lock().unwrap(), expected_msgs, "Second multi didn't receive the expected messages");
        Ok(())

    }

        /// assures performance won't be degraded when we make changes
    #[cfg_attr(not(doc),tokio::test(flavor="multi_thread", worker_threads=2))]
    #[ignore]   // must run in a single thread for accurate measurements
    async fn performance_measurements() -> Result<(), Box<dyn std::error::Error>> {

        #[cfg(not(debug_assertions))]
        const FACTOR: u32 = 4096;
        #[cfg(debug_assertions)]
        const FACTOR: u32 = 32;

        /// measure how long it takes to stream a certain number of elements through the given `multi`
        async fn profile_multi<MultiChannelType:  FullDuplexMultiChannel<'static, u32, Arc<u32>> + Sync + Send + 'static,
                               const INSTRUMENTS: usize>
                              (multi:          &Multi<u32, MultiChannelType, INSTRUMENTS, Arc<u32>>,
                               profiling_name: &str,
                               count:          u32) {
            print!("{profiling_name} "); std::io::stdout().flush().unwrap();
            let start = Instant::now();
            let mut e = 0;
            while e < count {
                let buffer_entries_left = multi.buffer_size() - multi.pending_items_count();
                for _ in 0..buffer_entries_left {
                    multi.send(|slot| *slot = e);
                    e += 1;
                }
                std::hint::spin_loop();
            }
            multi.close(Duration::from_secs(5)).await;
            let elapsed = start.elapsed();
            println!("{:10.2}/s -- {} items processed in {:?}",
                     count as f64 / elapsed.as_secs_f64(),
                     count,
                     elapsed);
        }

        println!();

        type MultiChannelType = channels::arc::crossbeam::Crossbeam<'static, u32, 8192, 1>;
        type DerivedType      = Arc<u32>;

        let profiling_name = "metricfull_non_futures_non_fallible_multi:    ";
        let multi: Multi<u32, MultiChannelType, {Instruments::MetricsWithoutLogs.into()}, DerivedType> = Multi::new(profiling_name.trim());
        multi.spawn_non_futures_non_fallible_executor(1, "", |stream| stream, |_| async {}).await?;
        profile_multi(&multi, profiling_name, 1024*FACTOR).await;

        let profiling_name = "metricless_non_futures_non_fallible_multi:    ";
        let multi: Multi<u32, MultiChannelType, {Instruments::NoInstruments.into()}, Arc<u32>> = Multi::new(profiling_name.trim());
        multi.spawn_non_futures_non_fallible_executor(1, "", |stream| stream, |_| async {}).await?;
        profile_multi(&multi, profiling_name, 1024*FACTOR).await;

        let profiling_name = "par_metricless_non_futures_non_fallible_multi:";
        let multi: Multi<u32, MultiChannelType, {Instruments::NoInstruments.into()}, Arc<u32>> = Multi::new(profiling_name.trim());
        multi.spawn_non_futures_non_fallible_executor(12, "", |stream| stream, |_| async {}).await?;
        profile_multi(&multi, profiling_name, 1024*FACTOR).await;

        let profiling_name = "metricfull_futures_fallible_multi:            ";
        let multi: Multi<u32, MultiChannelType, {Instruments::MetricsWithoutLogs.into()}, Arc<u32>> = Multi::new(profiling_name.trim());
        multi.spawn_executor(1, Duration::ZERO, "",
                             |stream| {
                                 stream.map(|number| async move {
                                     Ok(number)
                                 })
                             },
                             |_err| async {},
                             |_| async {}).await?;
        profile_multi(&multi, profiling_name, 1024*FACTOR).await;

        let profiling_name = "metricless_futures_fallible_multi:            ";
        let multi: Multi<u32, MultiChannelType, {Instruments::NoInstruments.into()}, Arc<u32>> = Multi::new(profiling_name.trim());
        multi.spawn_executor(1, Duration::ZERO, "",
                             |stream| {
                                 stream.map(|number| async move {
                                     Ok(number)
                                 })
                             },
                             |_err| async {},
                             |_| async {}).await?;
        profile_multi(&multi, profiling_name, 1024*FACTOR).await;

        let profiling_name = "timeoutable_metricfull_futures_fallible_multi:";
        let multi: Multi<u32, MultiChannelType, {Instruments::MetricsWithoutLogs.into()}, Arc<u32>> = Multi::new(profiling_name.trim());
        multi.spawn_executor(1, Duration::from_millis(100), "",
                             |stream| {
                                  stream.map(|number| async move {
                                      Ok(number)
                                  })
                              },
                             |_err| async {},
                             |_| async {}).await?;
        profile_multi(&multi, profiling_name, 768*FACTOR).await;

        let profiling_name = "timeoutable_metricless_futures_fallible_multi:";
        let multi: Multi<u32, MultiChannelType, {Instruments::NoInstruments.into()}, Arc<u32>> = Multi::new(profiling_name.trim());
        multi.spawn_executor(1, Duration::from_millis(100), "",
                             |stream| {
                                 stream.map(|number| async move {
                                     Ok(number)
                                 })
                             },
                             |_err| async {},
                             |_| async {}).await?;
        profile_multi(&multi, profiling_name, 768*FACTOR).await;

        Ok(())
        /*

        As of Sept, 22th (after using multi-threaded tokio tests):

        RUSTFLAGS="-C target-cpu=native" cargo test --release performance_measurements -- --test-threads 1 --nocapture

        test mutiny::multi::tests::performance_measurements ...
        metricfull_non_futures_non_fallible_multi:     1062998.22/s -- 1048576 items processed in 986.432507ms
        metricless_non_futures_non_fallible_multi:     1122625.49/s -- 1048576 items processed in 934.039005ms
        par_metricless_non_futures_non_fallible_multi: 1018904.36/s -- 1048576 items processed in 1.029121125s
        metricfull_futures_fallible_multi:              918609.20/s -- 1048576 items processed in 1.141482139s
        metricless_futures_fallible_multi:              934254.75/s -- 1048576 items processed in 1.122366245s
        timeoutable_metricfull_futures_fallible_multi:  739489.91/s -- 786432 items processed in 1.063479014s
        timeoutable_metricless_futures_fallible_multi:  786373.07/s -- 786432 items processed in 1.000074935s

        */
    }

}
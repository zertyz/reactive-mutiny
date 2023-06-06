//! Allows creating `uni`s, which represent pairs of (`producer`, `event pipeline`) that may be used to
//! `produce()` asynchronous payloads to be processed by a single `event pipeline` Stream -- and executed
//! by one or more async tasks.
//!
//! Usage:
//! ```nocompile
//!    fn on_event(stream: impl Stream<Item=String>) -> impl Stream<Item=String> {
//!        stream
//!            .inspect(|message| println!("To Zeta: '{}'", message))
//!            .inspect(|sneak_peeked_message| println!("EARTH: Sneak peeked a message to Zeta Reticuli: '{}'", sneak_peeked_message))
//!            .inspect(|message| println!("ZETA: Received a message: '{}'", message))
//!    }
//!    let uni = UniBuilder::new()
//!        .on_stream_close(|_| async {})
//!        .spawn_non_futures_non_fallible_executor("doc_test() Event", on_event);
//!    let producer = uni.producer_closure();
//!    producer("I've just arrived!".to_string()).await;
//!    producer("Nothing really interesting here... heading back home!".to_string()).await;
//!    uni.close().await;
//! ```

mod uni_builder;
pub use uni_builder::*;

mod uni;
pub use uni::*;

pub mod channels;


/// Tests & enforces the requisites & expose good practices & exercises the API of of the [uni](self) module
#[cfg(any(test,doc))]
mod tests {
    use super::*;
    use crate::{
        prelude::MutinyStream,
        instruments::Instruments,
        types::{ChannelCommon, FullDuplexUniChannel},
    };
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU32, Ordering::Relaxed},
        },
        time::Duration,
        future::Future,
        io::Write,
    };
    use futures::{
        stream::{self, Stream, StreamExt}
    };
    use minstant::Instant;


    /// The `UniBuilder` specialization used for the tests to follow
    type UniBuilder<InType,
                    const BUFFER_SIZE: usize,
                    const MAX_STREAMS: usize,
                    const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
        = crate::uni::UniBuilder<InType,
                                 channels::movable::full_sync::FullSync<'static, InType, BUFFER_SIZE, MAX_STREAMS>,
                                 INSTRUMENTS,
                                 InType>;



    #[ctor::ctor]
    fn suite_setup() {
        simple_logger::SimpleLogger::new().with_utc_timestamps().init().unwrap_or_else(|_| eprintln!("--> LOGGER WAS ALREADY STARTED"));
    }

    /// exercises the code present on the documentation
    #[cfg_attr(not(doc),tokio::test)]
    async fn doc_tests() {
        fn on_event<'r>(stream: impl Stream<Item=&'r str>) -> impl Stream<Item=&'r str> {
            stream
                .inspect(|message| println!("To Zeta: '{}'", message))
                .inspect(|sneak_peeked_message| println!("EARTH: Sneak peeked a message to Zeta Reticuli: '{}'", sneak_peeked_message))
                .inspect(|message| println!("ZETA: Received a message: '{}'", message))
        }
        let uni = UniBuilder::<&str, 1024, 1>::new()
            .spawn_non_futures_non_fallibles_executor("doc_test() Event", on_event, |_| async {});
        let producer = |item| uni.try_send(|slot| *slot = item);
        producer("I've just arrived!");
        producer("Nothing really interesting here... heading back home!");
        assert!(uni.close(Duration::from_secs(10)).await, "Uni wasn't properly closed");
    }

    /// guarantees that one of the simplest possible testable 'uni' pipelines will get executed all the way through
    #[cfg_attr(not(doc),tokio::test)]
    async fn simple_pipeline() {
        const EXPECTED_SUM: u32 = 17;
        const PARTS: &[u32] = &[9, 8];

        // consumers may run at any time, so they should have a `static lifetime. Arc help us here.
        let observed_sum = Arc::new(AtomicU32::new(0));

        // this is the uni to work with our local variable
        let uni = UniBuilder::<u32, 1024, 1>::new()
            .spawn_non_futures_non_fallibles_executor("simple_pipeline() Event",
                                                      |stream| {
                                                          let observed_sum = Arc::clone(&observed_sum);
                                                          stream
                                                              .map(move |number| observed_sum.fetch_add(number, Relaxed))
                                                      },
                                                      |_| async {});
        let producer = |item| uni.try_send(|slot| *slot = item);

        // now the consumer: lets suppose we share it among several different tasks -- sharing a reference is one way to do it
        // (in this case, wrapping it in an Arc is not needed)
        let shared_producer = &producer;
        stream::iter(PARTS)
            .for_each_concurrent(1, |number| async move {
                shared_producer(*number);
            }).await;

        assert!(uni.close(Duration::ZERO).await, "Uni wasn't properly closed");
        assert_eq!(observed_sum.load(Relaxed), EXPECTED_SUM, "not all events passed through our pipeline");
    }

    /// shows how we may call async functions inside a `Uni` pipeline
    /// and work with "future" elements
    #[cfg_attr(not(doc),tokio::test)]
    async fn async_elements() {
        const EXPECTED_SUM: u32 = 30;
        const PARTS: &[u32] = &[9, 8, 7, 6];
        let observed_sum = Arc::new(AtomicU32::new(0));

        // notice how to transform a regular event into a future event &
        // how to pass it down the pipeline. Also notice the required (as of Rust 1.63)
        // moving of Arc local variables so they will be accessible
        let on_event = |stream: MutinyStream<'static, u32, _, u32>| {
            let observed_sum = Arc::clone(&observed_sum);
            stream
                .map(|number| async move {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    number
                })
                .map(move |number| {
                    let observed_sum = Arc::clone(&observed_sum);
                    async move {
                        let number = number.await;
                        observed_sum.fetch_add(number, Relaxed);
                        number
                    }
                })
                .map(|number| async move {
                    let number = number.await;
                    println!("Just added # {}", number);
                    Ok(number)
                })
                // the line bellow is commented out, since the default executor, `spawn_executor()`, expects Results of Futures
                // -- the code bellow would remove the Future, making the Stream yield Results of numbers, which, then, could be executed
                // by the executors from the test cases above.
                // .buffer_unordered(4)
        };

        let uni = UniBuilder::<u32, 1024, 1>::new()
            .concurrency_limit(PARTS.len() as u32)
            .futures_timeout(Duration::from_secs(2))
            .spawn_executor("async_elements() Event", on_event, |_| async {}, |_| async {});

        let producer = |item| uni.try_send(|slot| *slot = item);

        let shared_producer = &producer;
        stream::iter(PARTS)
            .for_each_concurrent(1, |number| async move {
                shared_producer(*number);
            }).await;

        assert!(uni.close(Duration::ZERO).await, "Uni wasn't properly closed");
        assert_eq!(observed_sum.load(Relaxed), EXPECTED_SUM, "not all events passed through our async pipeline");
    }

    /// assures stats are computed appropriately for every executor,
    /// according to the right instrumentation specifications
    #[cfg_attr(not(doc),tokio::test)]
    #[ignore]   // flaky if ran in multi-thread: timeout measurements go south
    async fn stats() {

        // asserts spawn_non_futures_non_fallible_executor() register statistics appropriately:
        // with counters, but without average futures resolution time measurements
        let event_name = "non_future/non_fallible event";
        let uni = UniBuilder::<String, 1024, 1>::new()
            .spawn_non_futures_non_fallibles_executor(event_name, |stream| stream, |_| async {});
        let producer = |item| uni.try_send(|slot| *slot = item);
        producer("'only count successes' payload".to_string());
        assert!(uni.close(Duration::ZERO).await, "Uni wasn't properly closed");
        let (ok_counter, ok_avg_futures_resolution_duration) = uni.stream_executor.ok_events_avg_future_duration.lightweight_probe();
        assert_eq!(ok_counter,                               1,    "counter of successful '{}' events is wrong", event_name);
        assert_eq!(ok_avg_futures_resolution_duration,       -1.0, "avg futures resolution time of successful '{}' events is wrong -- since it is a non-future, avg times should be always -1.0", event_name);
        let (failures_counter, failures_avg_futures_resolution_duration) = uni.stream_executor.failed_events_avg_future_duration.lightweight_probe();
        assert_eq!(failures_counter,                         0,    "counter of unsuccessful '{}' events is wrong -- since it is a non-fallible event, failures should always be 0", event_name);
        assert_eq!(failures_avg_futures_resolution_duration, 0.0,  "avg futures resolution time of unsuccessful '{}' events is wrong -- since it is a non-fallible event,, avg times should be always 0.0", event_name);
        let (timeouts_counter, timeouts_avg_futures_resolution_duration) = uni.stream_executor.timed_out_events_avg_future_duration.lightweight_probe();
        assert_eq!(timeouts_counter,                         0,    "counter of timed out '{}' events is wrong -- since it is a non-future event, timeouts should always be 0", event_name);
        assert_eq!(timeouts_avg_futures_resolution_duration, 0.0,  "avg futures resolution time of timed out '{}' events is wrong -- since it is a non-future event,, avg timeouts should be always 0.0", event_name);

        // asserts spawn_executor() register statistics appropriately:
        // with counters & with average futures resolution time measurements
        let event_name = "future & fallible event";
        let uni = UniBuilder::<String, 1024, 1>::new()
            .futures_timeout(Duration::from_millis(150))
            .spawn_executor(event_name,
                            |stream| {
                                stream.map(|payload: String| async move {
                                    if payload.contains("unsuccessful") {
                                        tokio::time::sleep(Duration::from_millis(50)).await;
                                        Err(Box::from(format!("failing the pipeline, as requested")))
                                    } else if payload.contains("timeout") {
                                        tokio::time::sleep(Duration::from_millis(200)).await;
                                        Ok("this answer will never make it -- stream executor times out after 100ms".to_string())
                                    } else {
                                        tokio::time::sleep(Duration::from_millis(100)).await;
                                        Ok(payload)
                                    }
                                })
                            },
                            |_| async {},
                            |_| async {}
            );
        let producer = |item| uni.try_send(|slot| *slot = item);
        // for this test, produce each event twice
        for _i in 0..2 {
            producer("'successful' payload".to_string());
            producer("'unsuccessful' payload".to_string());
            producer("'timeout' payload".to_string());
        }
        assert!(uni.close(Duration::ZERO).await, "Uni wasn't properly closed");
        let (ok_counter, ok_avg_futures_resolution_duration) = uni.stream_executor.ok_events_avg_future_duration.lightweight_probe();
        assert_eq!(ok_counter,                                              2,   "counter of successful '{}' events is wrong", event_name);
        assert!((ok_avg_futures_resolution_duration-0.100).abs()        < 15e-2, "avg futures resolution time of successful '{}' events is wrong -- it should be 0.1s", event_name);
        let (failures_counter, failures_avg_futures_resolution_duration) = uni.stream_executor.failed_events_avg_future_duration.lightweight_probe();
        assert_eq!(failures_counter,                                       2,   "counter of unsuccessful '{}' events is wrong", event_name);
        assert!((failures_avg_futures_resolution_duration-0.050).abs() < 15e-2, "avg futures resolution time of unsuccessful '{}' events is wrong -- it should be 0.05s, but was {}", event_name, failures_avg_futures_resolution_duration);
        let (timeouts_counter, timeouts_avg_futures_resolution_duration) = uni.stream_executor.timed_out_events_avg_future_duration.lightweight_probe();
        assert_eq!(timeouts_counter,                                       2,   "counter of timed out '{}' events is wrong", event_name);
        assert!((timeouts_avg_futures_resolution_duration-0.150).abs() < 15e-2, "avg futures resolution time of timed out '{}' events is wrong -- it should be 0.150s", event_name);

    }


    /// shows how to fuse multiple `uni`s, triggering payloads for another uni when certain conditions are met:
    /// events TWO and FOUR will set a shared state between them, firing SIX.
    #[cfg_attr(not(doc),tokio::test)]
    async fn demux() {
        let shared_state = Arc::new(AtomicU32::new(0));
        let two_fire_count = Arc::new(AtomicU32::new(0));
        let four_fire_count = Arc::new(AtomicU32::new(0));
        let six_fire_count = Arc::new(AtomicU32::new(0));

        // SIX event
        let six_fire_count_ref = Arc::clone(&six_fire_count);
        let on_six_event = move |stream: MutinyStream<'static, (), _, ()>| {
            stream.inspect(move |_| {
                six_fire_count_ref.fetch_add(1, Relaxed);
            })
        };
        let six_uni = UniBuilder::<(), 1024, 1>::new()
            .spawn_non_futures_non_fallibles_executor("SIX event", on_six_event, |_| async {});
        // assures we'll close SIX only once
        let can_six_be_closed = Arc::new(AtomicBool::new(true));
        let six_uni_ref = Arc::clone(&six_uni);
        let six_closer = Arc::new(move || {
            let can_six_be_closed = Arc::clone(&can_six_be_closed);
            let six_uni = Arc::clone(&six_uni_ref);
            async move {
                if can_six_be_closed.swap(false, Relaxed) {
                    assert!(six_uni.close(Duration::ZERO).await, "`six_uni` wasn't properly closed");
                }
            }
        });

        // TWO event
        let on_two_event = |stream: MutinyStream<'static, u32, _, u32>| {
            let two_fire_count = Arc::clone(&two_fire_count);
            let shared_state = Arc::clone(&shared_state);
            let six_uni = Arc::clone(&six_uni);
            stream
                .map(move |event| {
                    let two_fire_count = Arc::clone(&two_fire_count);
                    let shared_state = Arc::clone(&shared_state);
                    let six_uni = Arc::clone(&six_uni);
                    async move {
                        two_fire_count.fetch_add(1, Relaxed);
                        if event & 2 == 2 {
                            let previous_state = shared_state.fetch_or(2, Relaxed);
                            if previous_state & 6 == 6 {
                                shared_state.store(0, Relaxed); // reset the triggering state
                                six_uni.try_send(|slot| *slot = ());
                            }
                        } else if event == 97 {
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                        event
                    }
            })
            .buffer_unordered(1)
        };
        let six_closer_for_two = Arc::clone(&six_closer);
        let on_two_close = move |_| {
            let six_closer_for_two = Arc::clone(&six_closer_for_two);
            async move {
                six_closer_for_two().await;
            }
        };
        let two_uni = UniBuilder::<u32, 1024, 1>::new()
            .spawn_non_futures_non_fallibles_executor("TWO event", on_two_event, on_two_close);
        let two_producer = |item| two_uni.try_send(|slot| *slot = item);

        // FOUR event
        let on_four_event = |stream: MutinyStream<'static, u32, _, u32>| {
            let four_fire_count = Arc::clone(&four_fire_count);
            let shared_state = Arc::clone(&shared_state);
            let six_uni = Arc::clone(&six_uni);
            stream
                .map(move |event| {
                    let four_fire_count = Arc::clone(&four_fire_count);
                    let shared_state = Arc::clone(&shared_state);
                    let six_uni = Arc::clone(&six_uni);
                    async move {
                        four_fire_count.fetch_add(1, Relaxed);
                        if event & 4 == 4 {
                            let previous_state = shared_state.fetch_or(4, Relaxed);
                            if previous_state & 6 == 6 {
                                shared_state.store(0, Relaxed); // reset the triggering state
                                six_uni.try_send(|slot| *slot = ());
                            }
                        } else if event == 97 {
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                        event
                    }
                })
                .buffer_unordered(1)
        };
        let six_closer_for_four = Arc::clone(&six_closer);
        let on_four_close = move |_| {
            let six_closer_for_four = Arc::clone(&six_closer_for_four);
            async move {
                six_closer_for_four().await;
            }
        };
        let four_uni = UniBuilder::<u32, 1024, 1>::new()
            .spawn_non_futures_non_fallibles_executor("FOUR event", on_four_event, on_four_close);
        let four_producer = |item| four_uni.try_send(|slot| *slot = item);

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
        tokio::time::sleep(Duration::from_millis(100)).await;     // flakiness protection: wait a tad before atomically closing `two` and `four` -- if not, `six` might be closed before the `six` event is sent, causing this test to fail.
        unis_close_async!(two_uni, four_uni);  // notice SIX is closed here as well
                                               // closing TWO (and, therefore, SIX) before all elements of FOUR are processed would cause the later consumer to try to publish to SIX (when it is already closed) --
                                               // this is why both events should be closed atomically in this case -- both share the closeable resource SIX -- which happens to be another uni, but could be any other resource

        assert_eq!(two_fire_count.load(Relaxed),  4, "Wrong number of events processed for TWO");
        assert_eq!(four_fire_count.load(Relaxed), 6, "Wrong number of events processed for FOUR");
        assert_eq!(six_fire_count.load(Relaxed),  1, "Wrong number of events processed for SIX");

    }

    /// shows how to handle errors when they happen anywhere down the pipeline
    /// -- and what happens when they are not handled.
    /// + tests meaningful messages are produced
    #[cfg_attr(not(doc),tokio::test)]
    async fn error_handling() {

        let on_err_count = Arc::new(AtomicU32::new(0));

        fn on_fail_when_odd_event(stream: impl Stream<Item=u32>) -> impl Stream<Item = impl Future<Output = Result<u32, Box<dyn std::error::Error + Send + Sync>> > + Send> {
            stream
                .map(|payload| async move {
                    if payload % 2 == 0 {
                        Ok(payload)
                    } else if payload % 79 == 0 {
                        Err(format!("BLOW CODE received: {}", payload))
                    } else {
                        Err(format!("ODD payload received: {}", payload))
                    }
                })
                // treat known errors
                .filter_map(|payload| async {
                    let payload = payload.await;
                    match payload {
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
        let on_err_count_clone = Arc::clone(&on_err_count);
        let uni = UniBuilder::<u32, 1024, 1>::new()
            .futures_timeout(Duration::from_millis(100))
            .spawn_executor("fallible event",
                            on_fail_when_odd_event,
                            move |err| {
                                let on_err_count_clone = Arc::clone(&on_err_count_clone);
                                async move {
                                    on_err_count_clone.fetch_add(1, Relaxed);
                                    println!("ERROR CALLBACK WAS CALLED: '{:?}'", err);
                                }
                            },
                            |_| async {}
            );
        let producer = |item| uni.try_send(|slot| *slot = item);
        producer(0);
        producer(1);
        producer(2);
        producer(79);
        producer(80);
        assert!(uni.close(Duration::ZERO).await, "Uni wasn't properly closed");

        assert_eq!(on_err_count.load(Relaxed), 1, "'on_err()' callback contract broken: events with handled errors should not call on_err(), the ones not 'caught', should")
    }

    /// assures performance won't be degraded when we make changes
    #[cfg_attr(not(doc),tokio::test(flavor="multi_thread", worker_threads=2))]
    #[ignore]   // must run in a single thread for accurate measurements
    async fn performance_measurements() {

        #[cfg(not(debug_assertions))]
        const FACTOR: u32 = 8192;
        #[cfg(debug_assertions)]
        const FACTOR: u32 = 40;

        /// measure how long it takes to stream a certain number of elements through the given `uni`
        async fn profile_uni<'a, UniChannelType:    FullDuplexUniChannel<'a, u32, u32> + Sync + Send + 'a,
                                 const INSTRUMENTS: usize>
                            (uni:            Arc<Uni<'a, u32, UniChannelType, INSTRUMENTS>>,
                             profiling_name: &str,
                             count:          u32) {
            print!("{profiling_name} "); std::io::stdout().flush().unwrap();
            let mut full_count = 0u32;
            let start = Instant::now();
            for e in 0..count {
                while !uni.try_send(|slot| *slot = e) {
                    std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop();
                    full_count += 1;
                    if full_count % (1<<28) == 0 {
                        let flushed = uni.flush(Duration::from_secs(5)).await;
                        if flushed > 0 {
                            let msg = format!("awakening the Stream via `flush()` -- consumed {flushed} elements. Chase the BUG preventing the Stream from being awaken");
                            println!("({msg})");
                            panic!("Hanging was recovered after {msg}");
                        } else {
                            print!("!");
                        }
                        std::io::stdout().flush().unwrap();
                    } else if full_count % (1<<27) == 0 {
                        print!("(still stuck at e #{e}? reverting to tokio yield...)"); std::io::stdout().flush().unwrap();
                        // if this fixes the hanging, means that tokio started the executor at the same thread the producer is executing
                        for _i in 0..e {
                            tokio::task::yield_now().await;
                        }
                    } else if full_count % (1<<24) == 0 {
                        print!("(stuck at e #{e}?)"); std::io::stdout().flush().unwrap();
                    } else if full_count % (1<<20) == 0 {
                        print!("."); std::io::stdout().flush().unwrap();
                    }
                };
            }
            assert!(uni.close(Duration::from_secs(5)).await, "Uni wasn't properly closed");
            let elapsed = start.elapsed();
            println!("{:10.2}/s -- {} items processed in {:?}",
                     count as f64 / elapsed.as_secs_f64(),
                     count,
                     elapsed);
        }

        println!();

        let profiling_name = "metricfull_non_futures_non_fallible_uni:    ";
        let uni = UniBuilder::<u32, 8192, 1, {Instruments::MetricsWithoutLogs.into()}>::new()
            .spawn_non_futures_non_fallibles_executor(profiling_name, |stream| stream, |_| async {});
        profile_uni(uni, profiling_name, 1024*FACTOR).await;

        let profiling_name = "metricless_non_futures_non_fallible_uni:    ";
        let uni = UniBuilder::<u32, 8192, 1, {Instruments::NoInstruments.into()}>::new()
            .spawn_non_futures_non_fallibles_executor(profiling_name, |stream| stream, |_| async {});
        profile_uni(uni, profiling_name, 1024*FACTOR).await;

        let profiling_name = "par_metricless_non_futures_non_fallible_uni:";
        let uni = UniBuilder::<u32, 8192, 1, {Instruments::NoInstruments.into()}>::new()
            .concurrency_limit(12)
            .spawn_non_futures_non_fallibles_executor(profiling_name, |stream| stream, |_| async {});
        profile_uni(uni, profiling_name, 1024*FACTOR).await;

        let profiling_name = "metricfull_futures_fallible_uni:            ";
        let uni = UniBuilder::<u32, 8192, 1, {Instruments::MetricsWithoutLogs.into()}>::new()
            .spawn_executor(profiling_name,
                            |stream| {
                                stream.map(|number| async move {
                                        Ok(number)
                                    })
                            },
                            |_err| async {},
                            |_| async {});
        profile_uni(uni, profiling_name, 1024*FACTOR).await;

        let profiling_name = "metricless_futures_fallible_uni:            ";
        let uni = UniBuilder::<u32, 8192, 1, {Instruments::NoInstruments.into()}>::new()
            .spawn_executor(profiling_name,
                            |stream| {
                                stream.map(|number| async move {
                                        Ok(number)
                                    })
                            },
                            |_err| async {},
                            |_| async {});
        profile_uni(uni, profiling_name, 1024*FACTOR).await;

        let profiling_name = "timeoutable_metricfull_futures_fallible_uni:";
        let uni = UniBuilder::<u32, 8192, 1, {Instruments::MetricsWithoutLogs.into()}>::new()
            .futures_timeout(Duration::from_millis(100))
            .spawn_executor(profiling_name,
                            |stream| {
                                stream.map(|number| async move {
                                        Ok(number)
                                    })
                            },
                            |_err| async {},
                            |_| async {});
        profile_uni(uni, profiling_name, 768*FACTOR).await;

        let profiling_name = "timeoutable_metricless_futures_fallible_uni:";
        let uni = UniBuilder::<u32, 8192, 1, {Instruments::NoInstruments.into()}>::new()
            .futures_timeout(Duration::from_millis(100))
            .spawn_executor(profiling_name,
                            |stream| {
                                stream.map(|number| async move {
                                        Ok(number)
                                    })
                            },
                            |_err| async {},
                            |_| async {});
        profile_uni(uni, profiling_name, 768*FACTOR).await;

        /*

        As of Sept, 22th (after using multi-threaded tokio tests):

        RUSTFLAGS="-C target-cpu=native" cargo test --release performance_measurements -- --test-threads 1 --nocapture

        test mutiny::uni::tests::performance_measurements ...
        metricfull_non_futures_non_fallible_uni:      511739.18/s -- 1048576 items processed in 2.049043793s
        metricless_non_futures_non_fallible_uni:      570036.96/s -- 1048576 items processed in 1.839487733s
        par_metricless_non_futures_non_fallible_uni:  479614.17/s -- 1048576 items processed in 2.18629069s
        metricfull_futures_fallible_uni:              428879.60/s -- 1048576 items processed in 2.444919271s
        metricless_futures_fallible_uni:              659091.97/s -- 1048576 items processed in 1.590940328s
        timeoutable_metricfull_futures_fallible_uni:  469629.46/s -- 786432 items processed in 1.674579774s
        timeoutable_metricless_futures_fallible_uni:  949109.14/s -- 786432 items processed in 828.600172ms

        */
    }

}
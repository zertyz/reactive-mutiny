//! Tests some conventional and not-so-conventional use cases for the [reactive_mutiny] library that
//! we care about, but are rather extensive, complicated or simply don't have enough didactics to be
//! part of the "examples" set.

use std::{
    future,
    time::Duration,
    sync::{
        Arc,
        atomic::{
            AtomicU32,
            Ordering::Relaxed,
        },
    },
    marker::PhantomData,
};
use reactive_mutiny::prelude::advanced::{
    GenericUni,
    UniMoveAtomic,
    UniMoveFullSync,
    GenericMulti,
    MultiAtomicOgreArc,
    MultiMmapLog,
};
use futures::stream::StreamExt;
use tokio::sync::Mutex;


#[ctor::ctor]
fn suite_setup() {
    simple_logger::SimpleLogger::new().with_utc_timestamps().init().unwrap_or_else(|_| eprintln!("--> LOGGER WAS ALREADY STARTED"));
}

/// Ensures wrapping the Event into `Arc`s is allowed for [Uni]s using the [reactive_mutiny::uni::channels::movable] channels:\
/// For payloads that are `Arc`s (or any other types that are `std::mem::needs_drop()`), one must use [Uni::try_send_movable()]
#[cfg_attr(not(doc),tokio::test)]
async fn unis_of_arcs() {
    let sum = Arc::new(AtomicU32::new(0));
    let executor_status = Arc::new(Mutex::new(String::new()));
    let sum_ref = Arc::clone(&sum);
    let executor_status_ref = Arc::clone(&executor_status);
    let uni = UniMoveFullSync::<Arc<u32>, 1024>::new("uni of arcs")
        .spawn_non_futures_non_fallibles_executors(1,
                                                   move |payloads| {
                                                      let sum_ref = Arc::clone(&sum_ref);
                                                      payloads.map(move | payload| sum_ref.fetch_add(*payload, Relaxed))
                                                  },
                                                   move |executor| async move {
                                                     executor_status_ref.lock().await.push_str(&format!("count: {}", executor.ok_events_avg_future_duration().lightweight_probe().0));
                                                 });
    let _ = uni.send(Arc::new(123));
    let _ = uni.send(Arc::new(321));
    let _ = uni.send(Arc::new(444));
    assert!(uni.close(Duration::from_secs(5)).await, "couldn't close");
    assert_eq!(sum.load(Relaxed), 888, "Wrong payloads received");
    assert_eq!("count: 3", executor_status.lock().await.as_str(), "Wrong execution report");
}

/// Ensures we are able to replay past events using the [reactive_mutiny::multi::channels::reference::mmap_log::MmapLog] channel
#[cfg_attr(not(doc),tokio::test)]
async fn replayable_events() -> Result<(), Box<dyn std::error::Error>> {
    let multi = Arc::new(MultiMmapLog::<u32, 16>::new("replayable_events_integration_test"));
    let _ = multi.send(123);
    let _ = multi.send(321);
    let _ = multi.send(444);

    // first listener -- will receive all elements. No big deal: `Uni`s does that.
    let first_sum = Arc::new(AtomicU32::new(0));
    let first_sum_ref1 = Arc::clone(&first_sum);
    let first_sum_ref2 = Arc::clone(&first_sum);
    multi.spawn_non_futures_non_fallible_oldies_executor(1, false,
                                                         "first listener (oldies)",
                                                         move |payloads| payloads.map(move | payload: &u32| first_sum_ref1.fetch_add(*payload, Relaxed)),
                                                         move |_| future::ready(()),
                                                         "first listener (newies)",
                                                         move |payloads| payloads.map(move | payload: &u32| first_sum_ref2.fetch_add(*payload, Relaxed)),
                                                         move |_| future::ready(())).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(first_sum.load(Relaxed), 888, "Wrong payloads received by the first listener");

    // second listener -- will receive all elements as well. Now, this, only this channel may do!
    // ... and, by doing it twice, it is proven it may do it as many times as requested
    // (notice the special executor requests only old events)
    let second_sum = Arc::new(AtomicU32::new(0));
    let second_sum_ref1 = Arc::clone(&second_sum);
    let second_sum_ref2 = Arc::clone(&second_sum);
    multi.spawn_oldies_executor(1, true, Duration::from_secs(5),
                                "second listener (oldies)",
                                move |payloads| payloads.map(move | payload: &u32| future::ready(Ok(second_sum_ref1.fetch_add(*payload, Relaxed)))),
                                move |_| future::ready(()),
                                "second listener (newies)",
                                move |payloads| payloads.map(move | payload: &u32| future::ready(Ok(second_sum_ref2.fetch_add(*payload, Relaxed)))),
                                move |_| future::ready(()),
                                |_| async {}).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(second_sum.load(Relaxed), 888, "Wrong payloads received by the second listener");

    // a new event, to shake the sums up to 999
    let _ = multi.send(111);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(first_sum.load(Relaxed), 999, "First listener failed to process a new event");
    assert_eq!(second_sum.load(Relaxed), 999, "Second listener failed to process a new event");

    Ok(())
}

/// Ensures we're able to create [Uni]s with all combinations of processed results -- fallible/non-fallible & futures/non-futures
#[cfg_attr(not(doc),tokio::test)]
async fn unis_with_any_resulting_type() {
    const EXPECTED_RAW_VALUE: u32 = 9758;

    // raw types
    let observed_raw_type_value = Arc::new(AtomicU32::new(0));
    let observed_raw_type_value_ref = Arc::clone(&observed_raw_type_value);
    let raw_type_uni = UniMoveAtomic::<u32, 1024>::new("Raw types results")
        .spawn_non_futures_non_fallibles_executors(1,
                                                   move |payloads| {
                                                      let observed_raw_type_value_ref = Arc::clone(&observed_raw_type_value_ref);
                                                      payloads.map(move | payload| observed_raw_type_value_ref.store(payload, Relaxed))
                                                  },
                                                   |_| async {});
    assert!(raw_type_uni.send(EXPECTED_RAW_VALUE).is_ok(), "couldn't send a raw value that would remain raw");
    assert!(raw_type_uni.close(Duration::from_millis(100)).await, "couldn't close the raw Uni in time");
    assert_eq!(observed_raw_type_value.load(Relaxed), EXPECTED_RAW_VALUE, "Values don't match for `raw_type_uni`");

    // future types
    let observed_future_type_value = Arc::new(AtomicU32::new(0));
    let observed_future_type_value_ref = Arc::clone(&observed_future_type_value);
    let future_type_uni = UniMoveAtomic::<u32, 1024>::new("Future types results")
        .spawn_futures_executors(1,
                                 Duration::ZERO,
                                 move |payloads| {
                                    let observed_future_type_value_ref = Arc::clone(&observed_future_type_value_ref);
                                    payloads.map(move | payload| {
                                        let observed_future_type_value_ref = Arc::clone(&observed_future_type_value_ref);
                                        async move {
                                            observed_future_type_value_ref.store(payload, Relaxed);
                                        }
                                    })
                                },
                                 |_| async {});
    assert!(future_type_uni.send(EXPECTED_RAW_VALUE).is_ok(), "couldn't send a value that would become a future");
    assert!(future_type_uni.close(Duration::from_millis(100)).await, "couldn't close the future Uni in time");
    assert_eq!(observed_future_type_value.load(Relaxed), EXPECTED_RAW_VALUE, "Values don't match for `future_type_uni`");

    // fallible types
    let observed_fallible_type_value = Arc::new(AtomicU32::new(0));
    let observed_fallible_type_value_ref = Arc::clone(&observed_fallible_type_value);
    let fallible_type_uni = UniMoveAtomic::<u32, 1024>::new("Fallible types results")
        .spawn_fallibles_executors(1,
                                   move |payloads| {
                                      let observed_fallible_type_value_ref = Arc::clone(&observed_fallible_type_value_ref);
                                      payloads.map(move | payload| Ok(observed_fallible_type_value_ref.store(payload, Relaxed)) )
                                  },
                                   |_| {},
                                   |_| async {});
    assert!(fallible_type_uni.send(EXPECTED_RAW_VALUE).is_ok(), "couldn't send a value that would become fallible");
    assert!(fallible_type_uni.close(Duration::from_millis(100)).await, "couldn't close the fallible Uni in time");
    assert_eq!(observed_fallible_type_value.load(Relaxed), EXPECTED_RAW_VALUE, "Values don't match for `fallible_type_uni`");

    // fallible future types
    let observed_fallible_future_type_value = Arc::new(AtomicU32::new(0));
    let observed_fallible_future_type_value_ref = Arc::clone(&observed_fallible_future_type_value);
    let fallible_future_type_uni = UniMoveAtomic::<u32, 1024>::new("Fallible future types results")
        .spawn_executors(1,
                         Duration::ZERO,
                         move |payloads| {
                            let observed_fallible_future_type_value_ref = Arc::clone(&observed_fallible_future_type_value_ref);
                            payloads.map(move | payload| {
                                let observed_fallible_future_type_value_ref = Arc::clone(&observed_fallible_future_type_value_ref);
                                async move {
                                    observed_fallible_future_type_value_ref.store(payload, Relaxed);
                                    Ok(())
                                }
                            })
                        },
                         |_| async {},
                         |_| async {});
    assert!(fallible_future_type_uni.send(EXPECTED_RAW_VALUE).is_ok(), "couldn't send a value that would become a fallible future");
    assert!(fallible_future_type_uni.close(Duration::from_millis(100)).await, "couldn't close the fallible future Uni in time");
    assert_eq!(observed_fallible_future_type_value.load(Relaxed), EXPECTED_RAW_VALUE, "Values don't match for `fallible_future_type_uni`");

}

/// Ensures we're able to create [Multi]s with all combinations of processed results -- fallible/non-fallible & futures/non-futures -- subscribing to new events only
#[cfg_attr(not(doc),tokio::test)]
async fn multi_with_any_resulting_type_for_newies() -> Result<(), Box<dyn std::error::Error>> {
    const EXPECTED_RAW_VALUE: u32 = 9758;

    // raw types
    let observed_raw_type_value = Arc::new(AtomicU32::new(0));
    let observed_raw_type_value_ref = Arc::clone(&observed_raw_type_value);
    let raw_type_multi = MultiAtomicOgreArc::<u32, 1024, 1>::new("Multi resulting in raw types");
    raw_type_multi.spawn_non_futures_non_fallible_executor(1,
                                                           "Processor resulting in raw types",
                                                           move |payloads| payloads.map(move | payload| observed_raw_type_value_ref.store(*payload, Relaxed)),
                                                           |_| async {}).await?;
    assert!(raw_type_multi.send(EXPECTED_RAW_VALUE).is_ok(), "couldn't send a raw value that would remain raw");
    assert!(raw_type_multi.close(Duration::from_millis(100)).await, "couldn't close the raw Multi in time");
    assert_eq!(observed_raw_type_value.load(Relaxed), EXPECTED_RAW_VALUE, "Values don't match for `raw_type_multi`");

    // future types
    let observed_future_type_value = Arc::new(AtomicU32::new(0));
    let observed_future_type_value_ref = Arc::clone(&observed_future_type_value);
    let future_type_multi = MultiAtomicOgreArc::<u32, 1024, 1>::new("Multi resulting in future types");
    future_type_multi.spawn_futures_executor(1,
                                             Duration::from_millis(100),
                                             "Processor resulting in future types",
                                             move |payloads| {
                                                 payloads.map(move | payload| {
                                                     let observed_future_type_value_ref = Arc::clone(&observed_future_type_value_ref);
                                                     async move {
                                                         observed_future_type_value_ref.store(*payload, Relaxed);
                                                     }
                                                 })
                                             },
                                             |_| async {}).await?;
    assert!(future_type_multi.send(EXPECTED_RAW_VALUE).is_ok(), "couldn't send a value that would become a future");
    assert!(future_type_multi.close(Duration::from_millis(100)).await, "couldn't close the future Multi in time");
    assert_eq!(observed_future_type_value.load(Relaxed), EXPECTED_RAW_VALUE, "Values don't match for `future_type_multi`");

    // fallible types
    let observed_fallible_type_value = Arc::new(AtomicU32::new(0));
    let observed_fallible_type_value_ref = Arc::clone(&observed_fallible_type_value);
    let fallible_type_multi = MultiAtomicOgreArc::<u32, 1024, 1>::new("Multi resulting in fallible types");
    fallible_type_multi.spawn_fallibles_executor(1,
                                                 "Processor resulting in fallible types",
                                                 move |payloads| payloads.map(move | payload| Ok(observed_fallible_type_value_ref.store(*payload, Relaxed)) ),
                                                 |_| {},
                                                 |_| async {}).await?;
    assert!(fallible_type_multi.send(EXPECTED_RAW_VALUE).is_ok(), "couldn't send a value that would become fallible");
    assert!(fallible_type_multi.close(Duration::from_millis(100)).await, "couldn't close the fallible Multi in time");
    assert_eq!(observed_fallible_type_value.load(Relaxed), EXPECTED_RAW_VALUE, "Values don't match for `fallible_type_multi`");

    // fallible future types
    let observed_fallible_future_type_value = Arc::new(AtomicU32::new(0));
    let observed_fallible_future_type_value_ref = Arc::clone(&observed_fallible_future_type_value);
    let fallible_future_type_multi = MultiAtomicOgreArc::<u32, 1024, 1>::new("Multi resulting in fallible future types");
    fallible_future_type_multi.spawn_executor(1,
                                              Duration::from_millis(100),
                                              "Processor resulting in fallible future types",
                                              move |payloads| {
                                                  let observed_fallible_future_type_value_ref = Arc::clone(&observed_fallible_future_type_value_ref);
                                                  payloads.map(move | payload| {
                                                      let observed_fallible_future_type_value_ref = Arc::clone(&observed_fallible_future_type_value_ref);
                                                      async move {
                                                          observed_fallible_future_type_value_ref.store(*payload, Relaxed);
                                                          Ok(())
                                                      }
                                                  })
                                              },
                                              |_| async {},
                                              |_| async {}).await?;
    assert!(fallible_future_type_multi.send(EXPECTED_RAW_VALUE).is_ok(), "couldn't send a value that would become a fallible future");
    assert!(fallible_future_type_multi.close(Duration::from_millis(100)).await, "couldn't close the fallible future Multi in time");
    assert_eq!(observed_fallible_future_type_value.load(Relaxed), EXPECTED_RAW_VALUE, "Values don't match for `fallible_future_type_multi`");

    Ok(())
}

/// Ensures we're able to create [Multi]s with all combinations of processed results -- fallible/non-fallible & futures/non-futures -- subscribing to both old and new events
#[cfg_attr(not(doc),tokio::test)]
async fn multi_with_any_resulting_type_for_oldies() -> Result<(), Box<dyn std::error::Error>> {
    const EXPECTED_RAW_VALUE: u32 = 9758;

    // raw types
    let observed_raw_type_value = Arc::new(AtomicU32::new(0));
    let observed_raw_type_value_ref1 = Arc::clone(&observed_raw_type_value);
    let observed_raw_type_value_ref2 = Arc::clone(&observed_raw_type_value);
    let raw_type_multi = Arc::new(MultiMmapLog::<u32, 1024, 1>::new("Multi resulting in raw types"));
    raw_type_multi.spawn_non_futures_non_fallible_oldies_executor(1,
                                                                  true,
                                                                  "Oldies processor resulting in raw types",
                                                                  move |payloads| payloads.map(move | payload| observed_raw_type_value_ref1.store(*payload, Relaxed)),
                                                                  |_| async {},
                                                                  "Newies processor resulting in raw types",
                                                                  move |payloads| payloads.map(move | payload| observed_raw_type_value_ref2.store(*payload, Relaxed)),
                                                                  |_| async {}).await?;
    assert!(raw_type_multi.send(EXPECTED_RAW_VALUE).is_ok(), "couldn't send a raw value that would remain raw");
    assert!(raw_type_multi.close(Duration::from_millis(100)).await, "couldn't close the raw Multi in time");
    assert_eq!(observed_raw_type_value.load(Relaxed), EXPECTED_RAW_VALUE, "Values don't match for `raw_type_multi`");

    // future types
    let observed_future_type_value = Arc::new(AtomicU32::new(0));
    let observed_future_type_value_ref1 = Arc::clone(&observed_future_type_value);
    let observed_future_type_value_ref2 = Arc::clone(&observed_future_type_value);
    let future_type_multi = Arc::new(MultiMmapLog::<u32, 1024, 1>::new("Multi resulting in future types"));
    future_type_multi.spawn_futures_oldies_executor(1,
                                                    true,
                                                    Duration::from_millis(100),
                                                    "Oldies processor resulting in future types",
                                                    move |payloads| {
                                                        payloads.map(move | payload| {
                                                            let observed_future_type_value_ref = Arc::clone(&observed_future_type_value_ref1);
                                                            async move {
                                                                observed_future_type_value_ref.store(*payload, Relaxed);
                                                            }
                                                        })
                                                    },
                                                    |_| async {},
                                                    "Newies processor resulting in future types",
                                                    move |payloads| {
                                                        payloads.map(move | payload| {
                                                            let observed_future_type_value_ref = Arc::clone(&observed_future_type_value_ref2);
                                                            async move {
                                                                observed_future_type_value_ref.store(*payload, Relaxed);
                                                            }
                                                        })
                                                    },
                                                    |_| async {}).await?;
    assert!(future_type_multi.send(EXPECTED_RAW_VALUE).is_ok(), "couldn't send a value that would become a future");
    assert!(future_type_multi.close(Duration::from_millis(100)).await, "couldn't close the future Multi in time");
    assert_eq!(observed_future_type_value.load(Relaxed), EXPECTED_RAW_VALUE, "Values don't match for `future_type_multi`");

    // fallible types
    let observed_fallible_type_value = Arc::new(AtomicU32::new(0));
    let observed_fallible_type_value_ref1 = Arc::clone(&observed_fallible_type_value);
    let observed_fallible_type_value_ref2 = Arc::clone(&observed_fallible_type_value);
    let fallible_type_multi = Arc::new(MultiMmapLog::<u32, 1024, 1>::new("Multi resulting in fallible types"));
    fallible_type_multi.spawn_fallibles_oldies_executor(1,
                                                        true,
                                                        "Oldies processor resulting in fallible types",
                                                        move |payloads| payloads.map(move | payload| Ok(observed_fallible_type_value_ref1.store(*payload, Relaxed)) ),
                                                        |_| async {},
                                                        "Newies processor resulting in fallible types",
                                                        move |payloads| payloads.map(move | payload| Ok(observed_fallible_type_value_ref2.store(*payload, Relaxed)) ),
                                                        |_| async {},
                                                        |_| {}).await?;
    assert!(fallible_type_multi.send(EXPECTED_RAW_VALUE).is_ok(), "couldn't send a value that would become fallible");
    assert!(fallible_type_multi.close(Duration::from_millis(100)).await, "couldn't close the fallible Multi in time");
    assert_eq!(observed_fallible_type_value.load(Relaxed), EXPECTED_RAW_VALUE, "Values don't match for `fallible_type_multi`");

    // fallible future types
    let observed_fallible_future_type_value = Arc::new(AtomicU32::new(0));
    let observed_fallible_future_type_value_ref1 = Arc::clone(&observed_fallible_future_type_value);
    let observed_fallible_future_type_value_ref2 = Arc::clone(&observed_fallible_future_type_value);
    let fallible_future_type_multi = Arc::new(MultiMmapLog::<u32, 1024, 1>::new("Multi resulting in fallible future types"));
    fallible_future_type_multi.spawn_oldies_executor(1,
                                                     true,
                                                     Duration::from_millis(100),
                                                     "Oldies processor resulting in fallible future types",
                                                     move |payloads| {
                                                         payloads.map(move | payload| {
                                                             let observed_fallible_future_type_value_ref = Arc::clone(&observed_fallible_future_type_value_ref1);
                                                             async move {
                                                                 observed_fallible_future_type_value_ref.store(*payload, Relaxed);
                                                                 Ok(())
                                                             }
                                                         })
                                                     },
                                                     |_| async {},
                                                     "Newies processor resulting in fallible future types",
                                                     move |payloads| {
                                                         payloads.map(move | payload| {
                                                             let observed_fallible_future_type_value_ref = Arc::clone(&observed_fallible_future_type_value_ref2);
                                                             async move {
                                                                 observed_fallible_future_type_value_ref.store(*payload, Relaxed);
                                                                 Ok(())
                                                             }
                                                         })
                                                     },
                                                     |_| async {},
                                                     |_| async {}).await?;
    assert!(fallible_future_type_multi.send(EXPECTED_RAW_VALUE).is_ok(), "couldn't send a value that would become a fallible future");
    assert!(fallible_future_type_multi.close(Duration::from_millis(100)).await, "couldn't close the fallible future Multi in time");
    assert_eq!(observed_fallible_future_type_value.load(Relaxed), EXPECTED_RAW_VALUE, "Values don't match for `fallible_future_type_multi`");

    Ok(())
}

/// Ensures [Uni] & [Multi] may be easily used in generic structs
#[cfg_attr(not(doc),test)]
fn generics() {

    // ensure `Uni`s can be represented by a simple generic type that may be "reconstructed" back into the concrete type when needed
    struct GenericUniUser<UniType: GenericUni> {
        the_uni: UniType,
        _phantom: PhantomData<UniType>,
    }
    impl<UniType: GenericUni> GenericUniUser<UniType> {
        // type TheUniType = Uni<UniType::ItemType, UniType::UniChannelType, { UniType::INSTRUMENTS }, UniType::DerivedItemType>;   // NOT YET ALLOWED :( when it is, `UNI_INSTRUMENTS` may be dropped
        /// Demonstrates the caller is able to see `UniType` as a `Uni`
        fn to_uni(self) -> UniType {
            self.the_uni
        }
        /// Demonstrates we can access the type
        fn new_uni() -> UniType {
            UniType::new("Another Uni")
        }
        /// Demonstrates `UniType` can be converted to a `Uni`
        async fn close(self) {
            self.the_uni.close(Duration::ZERO).await;
        }
    }
    type MyUniType = UniMoveAtomic::<u32, 1024>;
    let the_uni = GenericUniUser {the_uni: MyUniType::new("The Uni"), _phantom: PhantomData}.to_uni();
    let _future = the_uni.close(Duration::ZERO);
    let the_uni = GenericUniUser::<MyUniType>::new_uni();
    let _future = the_uni.close(Duration::ZERO);
    let _future = GenericUniUser {the_uni: MyUniType::new("The Uni"), _phantom: PhantomData}.close();
    

    // ensure `Multi`s can be represented by a simple generic type that may be "reconstructed" back into the concrete type when needed
    struct GenericMultiUser<const MULTI_INSTRUMENTS: usize, MultiType: GenericMulti<MULTI_INSTRUMENTS>> {
        the_multi: MultiType,
        _phantom: PhantomData<MultiType>,
    }
    impl<const MULTI_INSTRUMENTS: usize, MultiType: GenericMulti<MULTI_INSTRUMENTS> + 'static> GenericMultiUser<MULTI_INSTRUMENTS, MultiType> {
        // type TheMultiType = Multi<MultiType::ItemType, MultiType::MultiChannelType, { MultiType::INSTRUMENTS }, MultiType::DerivedItemType>;   // NOT YET ALLOWED :( when it is, `UNI_INSTRUMENTS` may be dropped
        /// Demonstrates the caller is able to see `MultiType` as a `Multi`
        fn to_multi(self) -> MultiType {
            self.the_multi
        }
        /// Demonstrates we can access the type
        fn new_multi() -> MultiType {
            MultiType::new("Another Multi")
        }
        /// Demonstrates `MultiType` can be converted to a `Multi`
        async fn close(self) {
            self.the_multi.to_multi().close(Duration::ZERO).await;
        }
    }
    type MyMultiType = MultiAtomicOgreArc::<u32, 1024, 1>;
    let the_multi = GenericMultiUser::<{MyMultiType::INSTRUMENTS}, _> {the_multi: MyMultiType::new("The Multi"), _phantom: PhantomData}.to_multi();
    let _future = the_multi.close(Duration::ZERO);
    let the_multi = GenericMultiUser::<{MyMultiType::INSTRUMENTS}, MultiAtomicOgreArc::<u32, 1024, 1>>::new_multi();
    let _future = the_multi.close(Duration::ZERO);
    let _future = GenericMultiUser::<{MyMultiType::INSTRUMENTS}, _> {the_multi: MyMultiType::new("The Multi"), _phantom: PhantomData}.close();

}
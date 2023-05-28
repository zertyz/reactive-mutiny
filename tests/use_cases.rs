//! Tests some conventional and not-so-conventional use cases for the [reactive_mutiny] library that
//! we care about, but are rather extensive, complicated or simply don't have enough didactics to be
//! part of the "examples" set.

use std::future;
use std::sync::{
    Arc,
    atomic::{
        AtomicU32,
        Ordering::Relaxed,
    },
};
use std::time::Duration;
use reactive_mutiny::{self, MultiMmapLog, UniMoveFullSync};
use futures::stream::StreamExt;
use tokio::sync::Mutex;
use reactive_mutiny::uni::Uni;


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
    let uni = UniMoveFullSync::<Arc<u32>, 1024>::new()
        .spawn_non_futures_non_fallible_executor("Payload processor",
                                                 move |payloads| payloads.map(move | payload| sum_ref.fetch_add(*payload, Relaxed)),
                                                 move |executor| async move {
                                                     executor_status_ref.lock().await.push_str(&format!("count: {}", executor.ok_events_avg_future_duration.lightweight_probe().0));
                                                 });
    let _ = uni.try_send_movable(Arc::new(123));
    let _ = uni.try_send_movable(Arc::new(321));
    let _ = uni.try_send_movable(Arc::new(444));
    // TODO: 2023-05-23: for an improved API, uni.try_send() & other functions must be denied (compilation error) if the type is `std::mem::needs_drop()`
    assert!(uni.close(Duration::from_secs(5)).await, "couldn't close");
    assert_eq!(sum.load(Relaxed), 888, "Wrong payloads received");
    assert_eq!("count: 3", executor_status.lock().await.as_str(), "Wrong execution report");
}

/// Ensures we are able to replay past events using the [reactive_mutiny::multi::channels::reference::mmap_log::MmapLog] channel
#[cfg_attr(not(doc),tokio::test)]
async fn replayable_events() -> Result<(), Box<dyn std::error::Error>> {
    let multi = Arc::new(MultiMmapLog::<u32, 16>::new("replayable_events_integration_test"));
    let _ = multi.try_send_movable(123);
    let _ = multi.try_send_movable(321);
    let _ = multi.try_send_movable(444);

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
    let _ = multi.try_send_movable(111);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(first_sum.load(Relaxed), 999, "First listener failed to process a new event");
    assert_eq!(second_sum.load(Relaxed), 999, "Second listener failed to process a new event");

    Ok(())
}
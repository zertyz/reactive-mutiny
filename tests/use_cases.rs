//! Tests some conventional and not-so-conventional use cases for the [reactive_mutiny] library that
//! we care about, but are rather extensive, complicated or simply don't have enough didactics to be
//! part of the "examples" set.

use std::sync::{
    Arc,
    atomic::{
        AtomicU32,
        Ordering::Relaxed,
    },
};
use std::time::Duration;
use reactive_mutiny::{
    self,
    UniMoveFullSync,
};
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
    // TODO: 2023-05-23: uni.try_send() & other functions must be denied (compilation error) if the type is `std::mem::needs_drop()`
    assert!(uni.close(Duration::from_secs(5)).await, "couldn't close");
    assert_eq!(sum.load(Relaxed), 888, "Wrong payloads received");
    assert_eq!("count: 3", executor_status.lock().await.as_str(), "Wrong execution report");
}
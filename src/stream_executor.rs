//! Contains the logic for executing [Stream] pipelines on their own Tokio tasks, gathering stats,
//! logging and so on.\
//! Four executors are provided to attend to different Stream output item types:
//!   1. Items that are non-futures & non-fallible. For instance, `Stream::Item = String`
//!   2. Items that are futures, but are non-fallible: `Stream::Item = Future<Output=DataType>` -- DataType may be, for instance, String
//!   3. Items that are non-futures, but are fallible: `Stream::Item = Result<DataType, Box<dyn std::error::Error>>`
//!   4. Items that are fallible futures: `Stream::Item = Future<Output=Result<DataType, Box<dyn std::error::Error>>>` -- allowing futures to time out
//!
//! Apart from handling the specifig return types, logging & filling in the available metrics, all executors does the same things:
//!   1. Register start & finish metrics
//!   2. Log start (info), items (trace) and finish (warn)
//!
//! Specific executors does additional work:
//!   1. Streams that returns a `Result` computes how many errors occurred and calls the provided `on_err_callback()`, as well as
//!      logging errors that made it all through the pipeline and showed up on the executor (untreated errors)
//!   2. Streams that returns a `Future` will, optionally, compute the time for each future to complete and, also optionally,
//!      register a time out for each future to be completed


use super::{
    incremental_averages::AtomicIncrementalAverage64,
};
use std::{
    sync::{
        Arc,
        atomic::{
            AtomicU64,
            Ordering::{Relaxed},
        },
    },
    future::Future,
    fmt::Debug,
    time::{Duration},
    error::Error,
};
use atomic_enum::atomic_enum;
use futures::{
    stream::{Stream,StreamExt}
};
use tokio::{
    time::{
        timeout,
    },
};
use log::{trace,info,warn,error};
// using this instead of Tokio's Instant -- or even std's Instant -- saves a system call (and a context switch), avoiding a huge performance prejudice
use minstant::Instant;


pub struct StreamExecutor<const LOG: bool, const METRICS: bool> {
    executor_name:                 String,
    futures_timeout:               Duration,
    creation_time:                 Instant,
    executor_status:               AtomicExecutorStatus,
    execution_start_delta_nanos:   AtomicU64,
    execution_finish_delta_nanos:  AtomicU64,

    // data to the fields bellow will depend on computations being enabled when instantiating this struct

    /// computes counter of successful events & average times (seconds) for the future resolution
    pub ok_events_avg_future_duration: AtomicIncrementalAverage64,

    /// computes counter of timed out events & average times (seconds) for the failed future resolution (even if it should be ~ constant)
    /// -- events computed here are NOT computed at [failed_events_avg_future_duration]
    pub timed_out_events_avg_future_duration: AtomicIncrementalAverage64,

    /// computes counter of all *other* failed events & average times (seconds) for the failed future resolution
    /// (timed out events, computed in [timed_out_events_avg_future_duration], are NOT computed here)
    pub failed_events_avg_future_duration: AtomicIncrementalAverage64,

    // currently, we're only able to measure how long it took to execute the future returned by the stream.
    // to automatically measure the time it took for the pipeline to process the event (time taken to build & return the future),
    // a new type has to be introduced -- which would wrap around the payload before passing to the pipeline... and it should
    // be available at the end, even if the actual "return" type changes along the way...
    // this may be done transparently if I have my own Stream implementation doing this... but it seems too much...
    // if needed, applications should do it by their own for now.
}


/// registers metrics & logs (if opted in) that an executor with the given capabilities has started.\
///   - `self`: this executor's instance. Example: `cloned_self`
///   - `future`: true if items yielded by this stream are of type `Future<Output=ItemType>`
///   - `fallible`: true if items yielded by this stream are of type `Result<DataType, ErrType>`
///   - `futures_timeout`: if different than `Duration::ZERO` means we enforce a timeout for every item (`Future<Outut=ItemType>`) when resolving them
macro_rules! on_executor_start {
    ($self: expr, $future: expr, $fallible: expr, $futures_timeout: expr) => {
        $self.register_execution_start();
        if LOG {
            info!("✓✓✓✓ Stream Executor '{}' started: {}Futures{} / {}Fallible Items & {}Metrics",
                  $self.executor_name,
                  if $future {""} else {"Non-"},
                  if $future {
                      if $futures_timeout != Duration::ZERO {format!(" (with timeouts of {:?})", $futures_timeout)} else {" (NO timeouts)".to_string()}
                  } else {
                      "".to_string()
                  },
                  if $fallible {""} else {"Non-"},
                  if !METRICS {"NO "} else {""});
        }
    }
}

/// logs & registers metrics (if opted in) for an `Ok` item yielded by this stream for which we don't have timings for -- most likely a non-future or future with metrics disabled\
///   - `self`: this executor's instance. Example: `cloned_self`
///   - `item`: the just yielded `Ok` item
macro_rules! on_non_timed_ok_item {
    ($self: expr, $item: expr) => {
        if METRICS {
            $self.ok_events_avg_future_duration.inc(-1.0);  // since there is no time measurement (item is non-future or METRICS=false), the convention is to use -1.0
        }
        if LOG {
            trace!("✓✓✓✓ Executor '{}' yielded '{:?}'", $self.executor_name, $item);
        }
    }
}

/// logs & registers metrics (if opted in) for an `Ok` item yielded by this stream for which we DO have timings for -- most certainly, a future
///   - `self`: this executor's instance. Example: `cloned_self`
///   - `item`: the just yielded `Ok` item
///   - `elapsed`: the `Duration` it took to resolve the this item's `Future`
macro_rules! on_timed_ok_item {
    ($self: expr, $item: expr, $elapsed: expr) => {
        if METRICS {
            $self.ok_events_avg_future_duration.inc($elapsed.as_secs_f32());
        } else {
            panic!("\nThis macro can only be used if METRICS=true -- otherwise you should use `on_non_timed_ok_item!(...)` instead");
        }
        if LOG {
            trace!("✓✓✓✓ Executor '{}' yielded '{:?}' in {:?}", $self.executor_name, $item, $elapsed);
        }
    }
}

/// logs & registers metrics (if opted in) for an `Err` item yielded by this stream for which we don't have timings for -- most likely a non-future or future with metrics disabled\
///   - `self`: this executor's instance. Example: `cloned_self`
///   - `err`: the just yielded `Err` item
macro_rules! on_non_timed_err_item {
    ($self: expr, $err: expr) => {
        if METRICS {
            $self.failed_events_avg_future_duration.inc(-1.0);  // since there is no time measurement (item is non-future or METRICS=false), the convention is to use -1.0
        }
        if LOG {
            error!("✗✗✗✗ Executor '{}' yielded ERROR '{:?}'", $self.executor_name, $err);
        }
    }
}

/// logs & registers metrics (if opted in) for an `Err` item yielded by this stream for which we DO have timings for -- most certainly, a future
///   - `self`: this executor's instance. Example: `cloned_self`
///   - `err`: the just yielded `Err` item
///   - `elapsed`: the `Duration` it took to resolve the this item's `Future`
macro_rules! on_timed_err_item {
    ($self: expr, $err: expr, $elapsed: expr) => {
        if METRICS {
            $self.failed_events_avg_future_duration.inc($elapsed.as_secs_f32());
        } else {
            panic!("This macro can only be used if METRICS=true -- otherwise you should use `on_non_timed_err_item!(...)` instead");
        }
        if LOG {
            error!("✗✗✗✗ Executor '{}' yielded ERROR '{:?}' in {:?}", $self.executor_name, $err, $elapsed);
        }
    }
}

/// registers metrics & logs (if opted in) that the either the stream or the executor ended.\
///   - `self`: this executor's instance. Example: `cloned_self`
///   - `future`: true if items yielded by this stream were of type `Future<Output=ItemType>`
///   - `fallible`: true if items yielded by this stream were of type `Result<DataType, ErrType>`
///   - `futures_timeout`: if different than `Duration::ZERO`, means time out stats could have beem collected and may be logged
macro_rules! on_executor_end {
    ($self: expr, $future: expr, $fallible: expr, $futures_timeout: expr) => {
        $self.register_execution_finish();
        let stream_ended = $self.executor_status.load(Relaxed) == ExecutorStatus::StreamEnded;
        let (ok_counter, ok_avg_seconds) = $self.ok_events_avg_future_duration.probe();
        let (timed_out_counter, timed_out_avg_seconds) = $self.timed_out_events_avg_future_duration.probe();
        let (failed_counter, failed_avg_seconds) = $self.failed_events_avg_future_duration.probe();
        let execution_nanos = $self.execution_finish_delta_nanos.load(Relaxed) - $self.execution_start_delta_nanos.load(Relaxed);
        let execution_secs: f64 = Duration::from_nanos(execution_nanos).as_secs_f64() + f64::MIN_POSITIVE /* dirty way to avoid silly /0 divisions */;
        if LOG && METRICS {
            let ok_stats = if $future {
                               format!("ok: {} events; avg {:?} - {:.5}/sec", ok_counter, Duration::from_secs_f32(ok_avg_seconds), ok_counter as f64 / execution_secs)
                           } else {
                               format!("ok: {} events", ok_counter)
                           };
            let timed_out_stats = if $future && $futures_timeout != Duration::ZERO {
                                      format!(" | time out: {} events; avg {:?} - {:.5}/sec", timed_out_counter, Duration::from_secs_f32(timed_out_avg_seconds), timed_out_counter as f64 / execution_secs)
                                  } else {
                                      format!("")
                                  };
            let failed_stats = if $future && $fallible {
                                   format!(" | failed: {} events; avg {:?} - {:.5}/sec", failed_counter, Duration::from_secs_f32(failed_avg_seconds), failed_counter as f64 / execution_secs)
                               } else if $fallible {
                                   format!(" | failed: {} events", failed_counter)
                               } else {
                                   format!("")
                               };
            warn!("✓✓✓✓ {} '{}' ended after running for {:?} -- stats: | {}{}{}",
                  if stream_ended {"Stream"} else {"Executor"},
                  $self.executor_name,
                  Duration::from_nanos(execution_nanos),
                  ok_stats,
                  timed_out_stats,
                  failed_stats);
        } else if LOG {
            warn!("✓✓✓✓ {} '{}' ended after running for {:?} -- metrics were disabled",
                  if stream_ended {"Stream"} else {"Executor"},
                  $self.executor_name,
                  Duration::from_nanos(execution_nanos));
        }
    }
}


impl<const LOG: bool, const METRICS: bool> StreamExecutor<LOG, METRICS> {

    /// initializes an executor that should not timeout any futures returned by the `Stream`
    pub fn new<IntoString: Into<String>>(executor_name: IntoString) -> Arc<Self> {
        Self::with_futures_timeout(executor_name, Duration::ZERO)
    }

    /// initializes an executor that should `timeout` futures returned by the `Stream`
    pub fn with_futures_timeout<IntoString: Into<String>>(executor_name: IntoString, futures_timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            executor_name: executor_name.into(),
            futures_timeout,
            creation_time:                        Instant::now(),
            executor_status:                      AtomicExecutorStatus::new(ExecutorStatus::NotStarted),
            execution_start_delta_nanos:          AtomicU64::new(0),
            execution_finish_delta_nanos:         AtomicU64::new(0),
            ok_events_avg_future_duration:        AtomicIncrementalAverage64::new(),
            failed_events_avg_future_duration:    AtomicIncrementalAverage64::new(),
            timed_out_events_avg_future_duration: AtomicIncrementalAverage64::new(),
        })
    }

    pub fn log_enabled(self: &Arc<Self>) -> bool {
        LOG
    }

    pub fn metrics_enabled(self: &Arc<Self>) -> bool {
        METRICS
    }

    pub fn executor_name(self: &Arc<Self>) -> String {
        self.executor_name.clone()
    }

    fn register_execution_start(self: &Arc<Self>) {
        self.executor_status.store(ExecutorStatus::Running, Relaxed);
        self.execution_start_delta_nanos.store(self.creation_time.elapsed().as_nanos() as u64, Relaxed);
    }

    fn register_execution_finish(self: &Arc<Self>) {
        // update the finish status
        loop {
            if self.executor_status.compare_exchange(ExecutorStatus::Running,           ExecutorStatus::StreamEnded,           Relaxed, Relaxed).is_ok() ||
               self.executor_status.compare_exchange(ExecutorStatus::ScheduledToFinish, ExecutorStatus::ProgrammaticallyEnded, Relaxed, Relaxed).is_ok() {
                break
            }
        }
        self.execution_finish_delta_nanos.store(self.creation_time.elapsed().as_nanos() as u64, Relaxed);
    }

    /// tells this executor that its stream will be artificially ended, to cause it to cease its execution
    pub fn report_scheduled_to_finish(self: &Arc<Self>) {
        self.executor_status.store(ExecutorStatus::ScheduledToFinish, Relaxed);
    }

    /// spawns an optimized executor for a Stream of `ItemType`s which are:
    ///   * Futures  -- ItemType = Future<Output=InnerFallibleType>
    ///   * Fallible -- InnerFallibleType = Result<InnerType, Box<dyn std::error::Error>>.\
    /// NOTE: special (optimized) versions are spawned depending if we should or not enforce each item `Future` resolution timeout
    pub fn spawn_executor<OutItemType:        Send + Debug,
                          FutureItemType:     Future<Output=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send,
                          CloseVoidAsyncType: Future<Output=()> + Send + 'static,
                          ErrVoidAsyncType:   Future<Output=()> + Send + 'static>
                         (self:                  Arc<Self>,
                          concurrency_limit:     u32,
                          on_err_callback:       impl Fn(Box<dyn Error + Send + Sync>) -> ErrVoidAsyncType   + Send + Sync + 'static,
                          stream_ended_callback: impl Fn(Arc<StreamExecutor<LOG, METRICS>>) -> CloseVoidAsyncType + Send + Sync + 'static,
                          stream:                impl Stream<Item=FutureItemType> + 'static + Send) {
        let cloned_self = Arc::clone(&self);
        let on_err_callback = Arc::new(on_err_callback);

        match self.futures_timeout {

            // spawns an optimized executor that do not track `Future`s timeouts
            Duration::ZERO => {
                let cloned_self = Arc::clone(&cloned_self);
                tokio::spawn(async move {
                    on_executor_start!(cloned_self, true, true, cloned_self.futures_timeout);
                    let mut start = Instant::now();     // item's Future resolution time -- declared here to allow optimizations when METRICS=false
                    let item_processor = |future_element| {
                        let cloned_self = Arc::clone(&cloned_self);
                        let on_err_callback = Arc::clone(&on_err_callback);
                        async move {
                            if METRICS {
                                start = Instant::now();
                            }
                            match future_element.await {
                                Ok(yielded_item) => {
                                    if METRICS {
                                        let elapsed = start.elapsed();
                                        on_timed_ok_item!(cloned_self, yielded_item, elapsed);
                                    } else {
                                        on_non_timed_ok_item!(cloned_self, yielded_item);
                                    }
                                },
                                Err(err) => {
                                    if METRICS {
                                        let elapsed = start.elapsed();
                                        on_timed_err_item!(cloned_self, err, elapsed);
                                    } else {
                                        on_non_timed_err_item!(cloned_self, err);
                                    }
                                    on_err_callback(err).await;
                                },
                            }
                        }
                    };
                    match concurrency_limit {
                        1 => stream.for_each(item_processor).await,     // faster in `futures 0.3` -- may be useless in the future
                        _ => stream.for_each_concurrent(concurrency_limit as usize, item_processor).await,
                    }
                    on_executor_end!(cloned_self, true, true, Duration::ZERO);
                    stream_ended_callback(cloned_self).await;
                });
            },

            // spawns an optimized executor that tracks `Future`s timeouts
            _ => {
                let cloned_self = Arc::clone(&cloned_self);
                tokio::spawn(async move {
                    on_executor_start!(cloned_self, true, true, cloned_self.futures_timeout);
                    let mut start = Instant::now();     // item's Future resolution time -- declared here to allow optimizations when METRICS=false
                    let item_processor = |future_element| {
                        let cloned_self = Arc::clone(&cloned_self);
                        let on_err_callback = Arc::clone(&on_err_callback);
                        async move {
                            if METRICS {
                                start = Instant::now();
                            }
                            match timeout(cloned_self.futures_timeout, future_element).await {
                                Ok(non_timed_out_result) => match non_timed_out_result {
                                                                                          Ok(yielded_item) => {
                                                                                              if METRICS {
                                                                                                  let elapsed = start.elapsed();
                                                                                                  on_timed_ok_item!(cloned_self, yielded_item, elapsed);
                                                                                              } else {
                                                                                                  on_non_timed_ok_item!(cloned_self, yielded_item);
                                                                                              }
                                                                                          },
                                                                                          Err(err) => {
                                                                                              if METRICS {
                                                                                                  let elapsed = start.elapsed();
                                                                                                  on_timed_err_item!(cloned_self, err, elapsed);
                                                                                              } else {
                                                                                                  on_non_timed_err_item!(cloned_self, err);
                                                                                              }
                                                                                              on_err_callback(err).await;
                                                                                          },
                                                                                      },
                                Err(_time_out_err) => {
                                    if METRICS {
                                        let elapsed = start.elapsed();
                                        cloned_self.timed_out_events_avg_future_duration.inc(elapsed.as_secs_f32());
                                        if LOG {
                                            error!("🕝🕝🕝🕝 Executor '{}' TIMED OUT after {:?}", cloned_self.executor_name, elapsed);
                                        }
                                    } else if LOG {
                                        error!("🕝🕝🕝🕝 Executor '{}' TIMED OUT", cloned_self.executor_name);
                                    }
                                }
                            }
                        }
                    };
                    match concurrency_limit {
                        1 => stream.for_each(item_processor).await,     // faster in `futures 0.3` -- may be useless in the future
                        _ => stream.for_each_concurrent(concurrency_limit as usize, item_processor).await,
                    }
                    on_executor_end!(cloned_self, true, true, cloned_self.futures_timeout);
                    stream_ended_callback(cloned_self).await;
                });
            },

        }

    }

    /// spawns an executor for a Stream of `ItemType`s which are not Futures but are fallible:
    ///   * InnerFallibleType = Result<ItemType, Box<dyn std::error::Error>>
    pub fn spawn_non_futures_executor<ItemType:      Send + Debug,
                                      VoidAsyncType: Future<Output=()> + Send + 'static>
                                     (self:                      Arc<Self>,
                                      concurrency_limit:         u32,
                                      consumption_done_reporter: impl Fn(Arc<StreamExecutor<LOG, METRICS>>) -> VoidAsyncType + Send + Sync + 'static,
                                      stream:                    impl Stream<Item=Result<ItemType, Box<dyn std::error::Error + Send + Sync>>> + 'static + Send) {
        let cloned_self = Arc::clone(&self);
        tokio::spawn(async move {
            info!("✓✓✓ Stream '{}' executor started: Non-Futures / Fallible Items & Metrics", cloned_self.executor_name);
            stream
                .for_each_concurrent(concurrency_limit as usize, |fallible_element| {
                    let cloned_self = Arc::clone(&cloned_self);
                    async move {
                        match fallible_element {
                            Ok(yielded_item) => {
                                cloned_self.ok_events_avg_future_duration.inc(-1.0);
                                trace!("✓✓✓ Stream '{}' yielded '{:?}'", cloned_self.executor_name, yielded_item);
                            },
                            Err(err) => {
                                cloned_self.failed_events_avg_future_duration.inc(-1.0);
                                error!("✗✗✗✗ Stream '{}' yielded ERROR '{:?}'", cloned_self.executor_name, err);
                            },
                        }
                    }
                }).await;
            let (ok_counter, _) = cloned_self.ok_events_avg_future_duration.probe();
            let (failed_counter, _) = cloned_self.failed_events_avg_future_duration.probe();
            warn!("✓✓✓ Stream '{}' ended -- stats: | ok: {} events | failed: {} events",
                  cloned_self.executor_name,
                  ok_counter,
                  failed_counter);
            consumption_done_reporter(cloned_self).await;
        });
    }

    /// spawns an optimized executor for a Stream of `ItemType`s which are not Futures and, also, are not fallible
    pub fn spawn_non_futures_non_fallible_executor<OutItemType:   Send + Debug,
                                                   VoidAsyncType: Future<Output=()> + Send + 'static>
                                                  (self:                  Arc<Self>,
                                                   concurrency_limit:     u32,
                                                   stream_ended_callback: impl FnOnce(Arc<StreamExecutor<LOG, METRICS>>) -> VoidAsyncType + Send + Sync + 'static,
                                                   stream:                impl Stream<Item=OutItemType> + 'static + Send) {

        let cloned_self = Arc::clone(&self);
        tokio::spawn(async move {
            on_executor_start!(cloned_self, false, false, Duration::ZERO);
            let item_processor = |yielded_item| {
                let cloned_self = Arc::clone(&cloned_self);
                async move {
                    on_non_timed_ok_item!(cloned_self, yielded_item);
                }
            };
            match concurrency_limit {
                1 => stream.for_each(item_processor).await,     // faster in `futures 0.3` -- may be useless in the near future?
                _ => stream.for_each_concurrent(concurrency_limit as usize, item_processor).await,
            }
            on_executor_end!(cloned_self, false, false, Duration::ZERO);
            stream_ended_callback(cloned_self).await;
        });
    }

}

/// Unit tests & enforces the requisites of the [stream_executor](self) module.\
/// Tests here mixes manual & automated assertions -- you should manually inspect the output of each one and check if the log outputs make sense
#[cfg(any(test, feature = "dox"))]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;
    use futures::{
        stream::{self, StreamExt},
        channel::mpsc,
        SinkExt,
    };


    #[ctor::ctor]
    fn suite_setup() {
        simple_logger::SimpleLogger::new().with_utc_timestamps().init().unwrap_or_else(|_| eprintln!("--> LOGGER WAS ALREADY STARTED"));
        info!("minstant: is TSC / RDTSC instruction available for time measurement? {}", minstant::is_tsc_available());
    }

    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn spawn_non_timeout_futures_fallible_executor_with_logs_and_metrics() {
        assert_spawn_futures_fallible_executor(StreamExecutor::<true, true>::new("executor with logs & metrics")).await;
    }

    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn spawn_non_timeout_futures_fallible_executor_with_metrics_and_no_logs() {
        assert_spawn_futures_fallible_executor(StreamExecutor::<false, true>::new("executor with metrics & NO logs")).await;
    }

    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn spawn_non_timeout_futures_fallible_executor_with_logs_and_no_metrics() {
        assert_spawn_futures_fallible_executor(StreamExecutor::<true, false>::new("executor with logs & NO metrics")).await;
    }

    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn spawn_non_timeout_futures_fallible_executor_with_no_logs_and_no_metrics() {
        assert_spawn_futures_fallible_executor(StreamExecutor::<false, false>::new("executor with NO logs & NO metrics")).await;
    }

    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn spawn_timeout_futures_fallible_executor_with_logs_and_metrics() {
        assert_spawn_futures_fallible_executor(StreamExecutor::<true, true>::with_futures_timeout("executor with logs & metrics", Duration::from_millis(100))).await;
    }

    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn spawn_timeout_futures_fallible_executor_with_metrics_and_no_logs() {
        assert_spawn_futures_fallible_executor(StreamExecutor::<false, true>::with_futures_timeout("executor with metrics & NO logs", Duration::from_millis(100))).await;
    }

    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn spawn_timeout_futures_fallible_executor_with_logs_and_no_metrics() {
        assert_spawn_futures_fallible_executor(StreamExecutor::<true, false>::with_futures_timeout("executor with logs & NO metrics", Duration::from_millis(100))).await;
    }

    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn spawn_timeout_futures_fallible_executor_with_no_logs_and_no_metrics() {
        assert_spawn_futures_fallible_executor(StreamExecutor::<false, false>::with_futures_timeout("executor with NO logs & NO metrics", Duration::from_millis(100))).await;
    }

    /// executes assertions on the given `executor` by spawning the executor and adding futures / fallible elements to it.\
    async fn assert_spawn_futures_fallible_executor<const LOG: bool,
                                                    const METRICS: bool>
                                                    (executor: Arc<StreamExecutor<LOG, METRICS>>) {

        async fn to_future(item: Result<u32, Box<dyn Error + Send + Sync>>) -> Result<u32, Box<dyn Error + Send + Sync>> {
            // OK items > 100 will sleep for a while -- intended to cause a timeout, provided the given executor is appropriately configured
            if item.is_ok() && item.as_ref().unwrap() > &100 {
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            item
        }

        let (tx, mut rx) = mpsc::channel::<bool>(10);
        let error_counter = Arc::new(AtomicU32::new(0));
        let error_counter_ref = Arc::clone(&error_counter);
        let cloned_executor = Arc::clone(&executor);
        let timeout_enabled = executor.futures_timeout > Duration::ZERO;
        let expected_timeout_count = if timeout_enabled {2} else {0};
        executor.spawn_executor(1,
                                move |_| { let error_counter = Arc::clone(&error_counter_ref); async move {error_counter.fetch_add(1, Relaxed);} },
                                move |_| { let mut tx = tx.clone(); async move {tx.send(true).await.unwrap()} },
                                stream::iter(vec![to_future(Ok(17)),
                                                            to_future(Err(Box::from("17"))),
                                                            to_future(Ok(170)),     // times out
                                                            to_future(Ok(19)),
                                                            to_future(Err(Box::from("19"))),
                                                            to_future(Ok(190))]     // times out
                                ));
        assert!(rx.next().await.expect("consumption_done_reporter() wasn't called"), "consumption_done_reporter yielded the wrong value");
        assert_eq!(error_counter.load(Relaxed), 2, "Error callback wasn't called the right number of times");
        assert_metrics(cloned_executor, 4 - expected_timeout_count, expected_timeout_count, 2);

    }

    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn spawn_non_futures_non_fallible_executor_with_logs_and_metrics() {
        assert_spawn_non_futures_non_fallible_executor(StreamExecutor::<true, true>::new("executor with logs & metrics")).await;
    }

    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn spawn_non_futures_non_fallible_executor_with_metrics_and_no_logs() {
        assert_spawn_non_futures_non_fallible_executor(StreamExecutor::<false, true>::new("executor with metrics & NO logs")).await;
    }

    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn spawn_non_futures_non_fallible_executor_with_logs_and_no_metrics() {
        assert_spawn_non_futures_non_fallible_executor(StreamExecutor::<true, false>::new("executor with logs & NO metrics")).await;
    }

    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn spawn_non_futures_non_fallible_executor_with_no_logs_and_no_metrics() {
        assert_spawn_non_futures_non_fallible_executor(StreamExecutor::<false, false>::new("executor with NO logs & NO metrics")).await;
    }

    /// executes assertions on the given `executor` by spawning the executor and adding non-futures / non-fallible elements to it
    async fn assert_spawn_non_futures_non_fallible_executor<const LOG: bool,
                                                            const METRICS: bool>
                                                           (executor: Arc<StreamExecutor<LOG, METRICS>>) {
        let (mut tx, mut rx) = mpsc::channel::<bool>(10);
        let cloned_executor = Arc::clone(&executor);
        executor.spawn_non_futures_non_fallible_executor(1, move |_| async move {tx.send(true).await.unwrap()}, stream::iter(vec![17, 19]));
        assert!(rx.next().await.expect("consumption_done_reporter() wasn't called"), "consumption_done_reporter yielded the wrong value");
        assert_metrics(cloned_executor, 2, 0, 0);

    }

    /// apply assertions on metrics for the given `executor`
    fn assert_metrics<const LOG: bool,
                      const METRICS: bool>
                     (executor: Arc<StreamExecutor<LOG, METRICS>>,
                      expected_ok_counter:         u32,
                      expected_timed_out_counter:  u32,
                      expected_failed_counter:     u32) {

        println!("### Stats assertions for Stream pipeline executor named '{}' (Logs? {}; Metrics? {}) ####", executor.executor_name, executor.log_enabled(), executor.metrics_enabled());
        let creation_duration = executor.creation_time.elapsed();
        let execution_start_delta_nanos = executor.execution_start_delta_nanos.load(Relaxed);
        let execution_finish_delta_nanos = executor.execution_finish_delta_nanos.load(Relaxed);
        let (ok_counter, ok_average) = executor.ok_events_avg_future_duration.lightweight_probe();
        let (timed_out_counter, timed_out_average) = executor.timed_out_events_avg_future_duration.lightweight_probe();
        let (failed_counter, failed_average) = executor.failed_events_avg_future_duration.lightweight_probe();
        println!("Creation time:    {:?} ago", creation_duration);
        println!("Execution Start:  {:?} after creation", Duration::from_nanos(execution_start_delta_nanos));
        println!("Execution Finish: {:?} after creation", Duration::from_nanos(execution_finish_delta_nanos));
        println!("OK elements count: {ok_counter}; OK elements average Future resolution time: {ok_average}s{}{}",
                 if METRICS {""} else {" -- metrics are DISABLED"},
                 if LOG {" -- verify these values against the \"executor closed\" message"} else {" -- logs are DISABLED"});
        println!("TIMED OUT elements count: {timed_out_counter}; TIMED OUT elements average Future resolution time: {timed_out_average}s{}{}",
                 if METRICS {""} else {" -- metrics are DISABLED"},
                 if LOG {" -- verify these values against the \"executor closed\" message"} else {" -- logs are DISABLED"});
        println!("FAILED elements count: {failed_counter}; FAILED elements average Future resolution time: {failed_average}s{}{}",
                 if METRICS {""} else {" -- metrics are DISABLED"},
                 if LOG {" -- verify these values against the \"executor closed\" message"} else {" -- logs are DISABLED"});

        assert!(execution_start_delta_nanos  > 0, "'execution_start_delta_nanos' wasn't set");
        assert!(execution_finish_delta_nanos > 0, "'execution_finish_delta_nanos' wasn't set");
        assert!(execution_finish_delta_nanos >= execution_start_delta_nanos, "INSTRUMENTATION ERROR: 'execution_start_delta_nanos' was set after 'execution_finish_delta_nanos'");

        if executor.metrics_enabled() {
            assert_eq!(ok_counter,        expected_ok_counter,        "OK elements counter doesn't match -- Metrics are ENABLED");
            assert_eq!(timed_out_counter, expected_timed_out_counter, "TIMED OUT elements counter doesn't match -- Metrics are ENABLED");
            assert_eq!(failed_counter,    expected_failed_counter,    "FAILED elements counter doesn't match -- Metrics are ENABLED");
        } else {
            assert_eq!(ok_counter,        0, "Metrics are DISABLED, so the reported OK elements should be ZERO");
            assert_eq!(timed_out_counter, 0, "Metrics are DISABLED, so the reported TIMED OUT elements should be ZERO");
            assert_eq!(failed_counter,    0, "Metrics are DISABLED, so the reported FAILED elements should be ZERO");
        }


        // if executor.instruments.measure_time is set
        // ...
    }
}

/// will derive `AtomicExecutorStatus`
#[atomic_enum]
#[derive(PartialEq)]
enum ExecutorStatus {
    NotStarted,
    Running,
    ScheduledToFinish,
    ProgrammaticallyEnded,
    StreamEnded,
}
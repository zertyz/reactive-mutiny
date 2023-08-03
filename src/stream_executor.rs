//! Contains the logic for executing [Stream] pipelines on their own Tokio tasks with zero-cost [Instruments] options for gathering stats,
//! logging and so on.\
//! Four executors are provided to attend to different Stream output item types:
//!   1. Items that are non-futures & non-fallible. For instance, `Stream::Item = String`
//!   2. Items that are futures, but are non-fallible: `Stream::Item = Future<Output=DataType>` -- DataType may be, for instance, String
//!   3. Items that are non-futures, but are fallible: `Stream::Item = Result<DataType, Box<dyn std::error::Error>>`
//!   4. Items that are fallible futures: `Stream::Item = Future<Output=Result<DataType, Box<dyn std::error::Error>>>` -- allowing futures to time out
//!
//! Apart from handling the specific return types, logging & filling in the available metrics, all executors do the same things:
//!   1. Register start & finish metrics
//!   2. Log start (info), items (trace) and finish (warn)
//!
//! Specific executors do additional work:
//!   1. Streams that returns a `Result`: logs & counts untreated-errors (errors that made it all the way through the pipeline and
//!      ended up in the executor), calling the provided `on_err_callback()`;
//!   2. Streams that returns a `Future`: they will, optionally, compute the time for each future to complete and, also optionally,
//!      register a time out for each future to be completed -- cancelling the `Future` if it exceeds the time budget.


use super::{
    instruments::Instruments,
    incremental_averages::AtomicIncrementalAverage64,
};
use std::{
    sync::{
        Arc,
        atomic::{
            AtomicU64,
            Ordering::Relaxed,
        },
    },
    future::Future,
    fmt::Debug,
    time::Duration,
    error::Error,
    future,
};
use atomic_enum::atomic_enum;
use futures::stream::{Stream,StreamExt};
use tokio::time::timeout;
// using this instead of Tokio's Instant -- or even std's Instant -- saves a system call (and a context switch), avoiding a huge performance prejudice
use minstant::Instant;
use log::{trace,info,warn,error};


/// See [Instruments]
#[derive(Debug)]
pub struct StreamExecutor {
    executor_name:                 String,
    futures_timeout:               Duration,
    creation_time:                 Instant,
    executor_status:               AtomicExecutorStatus,
    execution_start_delta_nanos:   AtomicU64,
    execution_finish_delta_nanos:  AtomicU64,

    // data to the fields bellow will depend on computations being enabled when instantiating this struct

    /// computes: counter of successful events & average times (seconds) for the future resolution
    pub ok_events_avg_future_duration: AtomicIncrementalAverage64,

    /// computes: counter of timed out events & average times (seconds) for the failed future resolution (even if it should be ~ constant)
    /// -- events computed here are NOT computed at [failed_events_avg_future_duration]
    pub timed_out_events_avg_future_duration: AtomicIncrementalAverage64,

    /// computes: counter of all *other* failed events & average times (seconds) for the failed future resolution
    /// (timed out events, computed by [timed_out_events_avg_future_duration], are NOT computed here)
    pub failed_events_avg_future_duration: AtomicIncrementalAverage64,

    // currently, we're only able to measure how long it took to execute the future returned by the stream.
    // to automatically measure the time it took for the pipeline to process the event (time taken to build & return the future),
    // a new type has to be introduced -- which would wrap around the payload before passing it to the pipeline... and it should
    // be available at the end, even if the actual "return" type changes along the way...
    // this may be done transparently if I have my own Stream implementation doing this... but it seems too much...
    // if needed, applications should do it by their own for now.
}

// /// A snapshot taken from a (running or not) [StreamExecutor] that may be easily shared around
// pub struct StreamExecutorStatsSnapshot {
//     pub executor
// }


/// registers metrics & logs (if opted in) that an executor with the given capabilities has started.\
///   - `self`: this executor's instance. Example: `self_ref`
///   - `future`: true if items yielded by this stream are of type `Future<Output=ItemType>`
///   - `fallible`: true if items yielded by this stream are of type `Result<DataType, ErrType>`
///   - `futures_timeout`: if different than `Duration::ZERO` means we enforce a timeout for every item (`Future<Outut=ItemType>`) when resolving them
macro_rules! on_executor_start {
    ($self: expr, $future: expr, $fallible: expr, $futures_timeout: expr, $INSTRUMENTS: ident) => {
        $self.register_execution_start();
        if Instruments::from($INSTRUMENTS).logging() {
            info!("✓✓✓✓ Stream Executor '{}' started: {}Futures{} / {}Fallible Items & {}Metrics",
                  $self.executor_name,
                  if $future {""} else {"Non-"},
                  if $future {
                      if $futures_timeout != Duration::ZERO {format!(" (with timeouts of {:?})", $futures_timeout)} else {" (NO timeouts)".to_string()}
                  } else {
                      "".to_string()
                  },
                  if $fallible {""} else {"Non-"},
                  if !Instruments::from($INSTRUMENTS).metrics() {"NO "} else {""});
        }
    }
}

/// logs & registers metrics (if opted in) for an `Ok` item yielded by this stream for which we don't have timings for -- most likely a non-future or future with metrics disabled\
///   - `self`: this executor's instance. Example: `self_ref`
///   - `item`: the just yielded `Ok` item
macro_rules! on_non_timed_ok_item {
    ($self: expr, $item: expr, $INSTRUMENTS: ident) => {
        {
            if Instruments::from($INSTRUMENTS).cheap_profiling() {
                $self.ok_events_avg_future_duration.inc(-1.0);  // since there is no time measurement (item is non-future or METRICS=false), the convention is to use -1.0
            }
            if Instruments::from($INSTRUMENTS).tracing() {
                trace!("✓✓✓✓ Executor '{}' yielded '{:?}'", $self.executor_name, $item);
            }
        }
    }
}

/// logs & registers metrics (if opted in) for an `Ok` item yielded by this stream for which we DO have timings for -- most certainly, a future
///   - `self`: this executor's instance. Example: `self_ref`
///   - `item`: the just yielded `Ok` item
///   - `elapsed`: the `Duration` it took to resolve the this item's `Future`
macro_rules! on_timed_ok_item {
    ($self: expr, $item: expr, $elapsed: expr, $INSTRUMENTS: ident) => {
        {
            if Instruments::from($INSTRUMENTS).cheap_profiling() {
                $self.ok_events_avg_future_duration.inc($elapsed.as_secs_f32());
            } else {
                panic!("\nThis macro can only be used if at least one of the Instruments' PROFILING are enabled -- otherwise you should use `on_non_timed_ok_item!(...)` instead");
            }
            if Instruments::from($INSTRUMENTS).tracing() {
                trace!("✓✓✓✓ Executor '{}' yielded '{:?}' in {:?}", $self.executor_name, $item, $elapsed);
            }
        }
    }
}

/// logs & registers metrics (if opted in) for an `Err` item yielded by this stream for which we don't have timings for -- most likely a non-future or future with metrics disabled\
///   - `self`: this executor's instance. Example: `self_ref`
///   - `err`: the just yielded `Err` item
macro_rules! on_non_timed_err_item {
    ($self: expr, $err: expr, $INSTRUMENTS: ident) => {
        {
            if Instruments::from($INSTRUMENTS).cheap_profiling() {
                $self.failed_events_avg_future_duration.inc(-1.0);  // since there is no time measurement (item is non-future or METRICS=false), the convention is to use -1.0
            }
            if Instruments::from($INSTRUMENTS).logging() {
                error!("✗✗✗✗ Executor '{}' yielded ERROR '{:?}'", $self.executor_name, $err);
            }
        }
    }
}

/// logs & registers metrics (if opted in) for an `Err` item yielded by this stream for which we DO have timings for -- most certainly, a future
///   - `self`: this executor's instance. Example: `self_ref`
///   - `err`: the just yielded `Err` item
///   - `elapsed`: the `Duration` it took to resolve the this item's `Future`
macro_rules! on_timed_err_item {
    ($self: expr, $err: expr, $elapsed: expr, $INSTRUMENTS: ident) => {
        {
            if Instruments::from($INSTRUMENTS).cheap_profiling() {
                $self.failed_events_avg_future_duration.inc($elapsed.as_secs_f32());
            } else {
                panic!("This macro can only be used if at least one of the Instruments' PROFILING are enabled -- otherwise you should use `on_non_timed_err_item!(...)` instead");
            }
            if Instruments::from($INSTRUMENTS).logging() {
                error!("✗✗✗✗ Executor '{}' yielded ERROR '{:?}' in {:?}", $self.executor_name, $err, $elapsed);
            }
        }
    }
}

/// registers metrics & logs (if opted in) that the either the stream or the executor ended.\
///   - `self`: this executor's instance. Example: `self_ref`
///   - `future`: true if items yielded by this stream were of type `Future<Output=ItemType>`
///   - `fallible`: true if items yielded by this stream were of type `Result<DataType, ErrType>`
///   - `futures_timeout`: if different than `Duration::ZERO`, means time out stats could have beem collected and may be logged
macro_rules! on_executor_end {
    ($self: expr, $future: expr, $fallible: expr, $futures_timeout: expr, $INSTRUMENTS: ident) => {
        $self.register_execution_finish();
        let stream_ended = $self.executor_status.load(Relaxed) == ExecutorStatus::StreamEnded;
        let execution_nanos = $self.execution_finish_delta_nanos.load(Relaxed) - $self.execution_start_delta_nanos.load(Relaxed);
        if Instruments::from($INSTRUMENTS).logging() && Instruments::from($INSTRUMENTS).cheap_profiling() {
            let (ok_counter, ok_avg_seconds) = $self.ok_events_avg_future_duration.probe();
            let (timed_out_counter, timed_out_avg_seconds) = $self.timed_out_events_avg_future_duration.probe();
            let (failed_counter, failed_avg_seconds) = $self.failed_events_avg_future_duration.probe();
            let execution_secs: f64 = Duration::from_nanos(execution_nanos).as_secs_f64() + f64::MIN_POSITIVE /* dirty way to avoid silly /0 divisions */;
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
        } else if Instruments::from($INSTRUMENTS).logging() {
            warn!("✓✓✓✓ {} '{}' ended after running for {:?} -- metrics were disabled",
                  if stream_ended {"Stream"} else {"Executor"},
                  $self.executor_name,
                  Duration::from_nanos(execution_nanos));
        }
    }
}


impl StreamExecutor {

    /// Initializes an executor that should not timeout any futures returned by the Stream
    pub fn new<IntoString: Into<String>>(executor_name: IntoString) -> Arc<Self> {
        Self::with_futures_timeout(executor_name, Duration::ZERO)
    }

    /// Initializes an executor that should be able to `timeout` Futures returned by the Stream
    pub fn with_futures_timeout<IntoString: Into<String>>(executor_name: IntoString, futures_timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            executor_name: executor_name.into(),
            futures_timeout,
            creation_time:                        Instant::now(),
            executor_status:                      AtomicExecutorStatus::new(ExecutorStatus::NotStarted),
            execution_start_delta_nanos:          AtomicU64::new(u64::MAX),
            execution_finish_delta_nanos:         AtomicU64::new(u64::MAX),
            ok_events_avg_future_duration:        AtomicIncrementalAverage64::new(),
            failed_events_avg_future_duration:    AtomicIncrementalAverage64::new(),
            timed_out_events_avg_future_duration: AtomicIncrementalAverage64::new(),
        })
    }


    pub fn executor_name(&self) -> String {
        self.executor_name.clone()
    }

    /// Sets the executor state & computes some always-enabled metrics
    fn register_execution_start(&self) {
        self.executor_status.store(ExecutorStatus::Running, Relaxed);
        self.execution_start_delta_nanos.store(self.creation_time.elapsed().as_nanos() as u64, Relaxed);
    }

    /// Sets the executor state & computes some always-enabled metrics
    fn register_execution_finish(&self) {
        loop {
            if self.executor_status.compare_exchange(ExecutorStatus::Running,           ExecutorStatus::StreamEnded,           Relaxed, Relaxed).is_ok() ||
               self.executor_status.compare_exchange(ExecutorStatus::ScheduledToFinish, ExecutorStatus::ProgrammaticallyEnded, Relaxed, Relaxed).is_ok() {
                break
            }
        }
        self.execution_finish_delta_nanos.store(self.creation_time.elapsed().as_nanos() as u64, Relaxed);
    }

    /// Tells this executor that its Stream will be artificially ended -- so to cause it to cease its execution
    pub fn report_scheduled_to_finish(&self) {
        self.executor_status.store(ExecutorStatus::ScheduledToFinish, Relaxed);
    }

    /// Spawns an optimized executor for a Stream of `ItemType`s which are:
    ///   * Futures  -- `ItemType := Future<Output=InnerFallibleType>`
    ///   * Fallible -- `InnerFallibleType := Result<InnerType, Box<dyn std::error::Error>>.`\
    /// NOTE: special (optimized) versions are spawned depending if we should or not enforce each item's `Future` a resolution timeout
    pub fn spawn_executor<const INSTRUMENTS:  usize,
                          OutItemType:        Send + Debug,
                          FutureItemType:     Future<Output=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send,
                          CloseVoidAsyncType: Future<Output=()> + Send + 'static,
                          ErrVoidAsyncType:   Future<Output=()> + Send + 'static>
                         (self:                  Arc<Self>,
                          concurrency_limit:     u32,
                          on_err_callback:       impl Fn(Box<dyn Error + Send + Sync>) -> ErrVoidAsyncType   + Send + Sync + 'static,
                          stream_ended_callback: impl FnOnce(Arc<StreamExecutor>) -> CloseVoidAsyncType + Send + Sync + 'static,
                          stream:                impl Stream<Item=FutureItemType> + 'static + Send) {

        match self.futures_timeout {

            // spawns an optimized executor that do not track `Future`s timeouts
            Duration::ZERO => {
                tokio::spawn(async move {
                    let self_ref: &Self = &self;
                    let on_err_callback_ref = &on_err_callback;
                    on_executor_start!(self_ref, true, true, self_ref.futures_timeout, INSTRUMENTS);
                    let mut start = Instant::now();     // item's Future resolution time -- declared here to allow optimizations when METRICS=false
                    let item_processor = |future_element| {
                        async move {
                            if Instruments::from(INSTRUMENTS).cheap_profiling() {
                                start = Instant::now();
                            }
                            match future_element.await {
                                Ok(yielded_item) => {
                                    if Instruments::from(INSTRUMENTS).cheap_profiling() {
                                        let elapsed = start.elapsed();
                                        on_timed_ok_item!(self_ref, yielded_item, elapsed, INSTRUMENTS);
                                    } else {
                                        on_non_timed_ok_item!(self_ref, yielded_item, INSTRUMENTS);
                                    }
                                },
                                Err(err) => {
                                    if Instruments::from(INSTRUMENTS).cheap_profiling() {
                                        let elapsed = start.elapsed();
                                        on_timed_err_item!(self_ref, err, elapsed, INSTRUMENTS);
                                    } else {
                                        on_non_timed_err_item!(self_ref, err, INSTRUMENTS);
                                    }
                                    on_err_callback_ref(err).await;
                                },
                            }
                        }
                    };
                    match concurrency_limit {
                        1 => stream.for_each(item_processor).await,     // faster in `futures 0.3` -- may be useless in the future
                        _ => stream.for_each_concurrent(concurrency_limit as usize, item_processor).await,
                    }
                    on_executor_end!(self_ref, true, true, Duration::ZERO, INSTRUMENTS);
                    stream_ended_callback(self).await;
                });
            },

            // spawns an optimized executor that tracks `Future`s timeouts
            _ => {
                tokio::spawn(async move {
                    let self_ref: &Self = &self;
                    let on_err_callback_ref = &on_err_callback;
                    on_executor_start!(self_ref, true, true, self_ref.futures_timeout, INSTRUMENTS);
                    let mut start = Instant::now();     // item's Future resolution time -- declared here to allow optimizations when METRICS=false
                    let item_processor = |future_element| {
                        async move {
                            if Instruments::from(INSTRUMENTS).cheap_profiling() {
                                start = Instant::now();
                            }
                            match timeout(self_ref.futures_timeout, future_element).await {
                                Ok(non_timed_out_result) => match non_timed_out_result {
                                                                                          Ok(yielded_item) => {
                                                                                              if Instruments::from(INSTRUMENTS).cheap_profiling() {
                                                                                                  let elapsed = start.elapsed();
                                                                                                  on_timed_ok_item!(self_ref, yielded_item, elapsed, INSTRUMENTS);
                                                                                              } else {
                                                                                                  on_non_timed_ok_item!(self_ref, yielded_item, INSTRUMENTS);
                                                                                              }
                                                                                          },
                                                                                          Err(err) => {
                                                                                              if Instruments::from(INSTRUMENTS).cheap_profiling() {
                                                                                                  let elapsed = start.elapsed();
                                                                                                  on_timed_err_item!(self_ref, err, elapsed, INSTRUMENTS);
                                                                                              } else {
                                                                                                  on_non_timed_err_item!(self_ref, err, INSTRUMENTS);
                                                                                              }
                                                                                              on_err_callback_ref(err).await;
                                                                                          },
                                                                                      },
                                Err(_time_out_err) => {
                                    if Instruments::from(INSTRUMENTS).cheap_profiling() {
                                        let elapsed = start.elapsed();
                                        self_ref.timed_out_events_avg_future_duration.inc(elapsed.as_secs_f32());
                                        if Instruments::from(INSTRUMENTS).logging() {
                                            error!("🕝🕝🕝🕝 Executor '{}' TIMED OUT after {:?}", self_ref.executor_name, elapsed);
                                        }
                                    } else if Instruments::from(INSTRUMENTS).logging() {
                                        error!("🕝🕝🕝🕝 Executor '{}' TIMED OUT", self_ref.executor_name);
                                    }
                                }
                            }
                        }
                    };
                    match concurrency_limit {
                        1 => stream.for_each(item_processor).await,     // faster in `futures 0.3` -- may be useless in other versions
                        _ => stream.for_each_concurrent(concurrency_limit as usize, item_processor).await,
                    }
                    on_executor_end!(self_ref, true, true, self_ref.futures_timeout, INSTRUMENTS);
                    stream_ended_callback(self).await;
                });
            },

        }
    }

    /// Spawns an optimized executor for a `Stream` of `FutureItemType`s (non fallible Futures).\
    /// NOTE: Since this is non-fallible, timeouts are enforced but not detectable. If that is not acceptable, use [Self::spawn_executor()] instead
    pub fn spawn_futures_executor<const INSTRUMENTS:  usize,
                                  OutItemType:        Send + Debug,
                                  FutureItemType:     Future<Output=OutItemType> + Send,
                                  CloseVoidAsyncType: Future<Output=()> + Send + 'static>
                                 (self:                  Arc<Self>,
                                  concurrency_limit:     u32,
                                  stream_ended_callback: impl FnOnce(Arc<StreamExecutor>) -> CloseVoidAsyncType + Send + Sync + 'static,
                                  stream:                impl Stream<Item=FutureItemType> + 'static + Send) {

        match self.futures_timeout {

            // spawns an optimized executor that do not track `Future`s timeouts
            Duration::ZERO => {
                tokio::spawn(async move {
                    let self_ref: &Self = &self;
                    on_executor_start!(self_ref, true, true, self_ref.futures_timeout, INSTRUMENTS);
                    let mut start = Instant::now();     // item's Future resolution time -- declared here to allow optimizations when METRICS=false
                    let item_processor = |future_element| {
                        async move {
                            if Instruments::from(INSTRUMENTS).cheap_profiling() {
                                start = Instant::now();
                            }
                            let yielded_item = future_element.await;
                            if Instruments::from(INSTRUMENTS).cheap_profiling() {
                                let elapsed = start.elapsed();
                                on_timed_ok_item!(self_ref, yielded_item, elapsed, INSTRUMENTS);
                            } else {
                                on_non_timed_ok_item!(self_ref, yielded_item, INSTRUMENTS);
                            }
                        }
                    };
                    match concurrency_limit {
                        1 => stream.for_each(item_processor).await,     // faster in `futures 0.3` -- may be useless in the future
                        _ => stream.for_each_concurrent(concurrency_limit as usize, item_processor).await,
                    }
                    on_executor_end!(self_ref, true, true, Duration::ZERO, INSTRUMENTS);
                    stream_ended_callback(self).await;
                });
            },

            // spawns an optimized executor that tracks `Future`s timeouts
            _ => {
                tokio::spawn(async move {
                    let self_ref: &Self = &self;
                    on_executor_start!(self_ref, true, true, self_ref.futures_timeout, INSTRUMENTS);
                    let mut start = Instant::now();     // item's Future resolution time -- declared here to allow optimizations when METRICS=false
                    let item_processor = |future_element| {
                        async move {
                            if Instruments::from(INSTRUMENTS).cheap_profiling() {
                                start = Instant::now();
                            }
                            match timeout(self_ref.futures_timeout, future_element).await {
                                Ok(non_timed_out_result) => {
                                    if Instruments::from(INSTRUMENTS).cheap_profiling() {
                                        let elapsed = start.elapsed();
                                        on_timed_ok_item!(self_ref, non_timed_out_result, elapsed, INSTRUMENTS);
                                    } else {
                                        on_non_timed_ok_item!(self_ref, non_timed_out_result, INSTRUMENTS);
                                    }
                                }
                                Err(_time_out_err) => {
                                    if Instruments::from(INSTRUMENTS).cheap_profiling() {
                                        let elapsed = start.elapsed();
                                        self_ref.timed_out_events_avg_future_duration.inc(elapsed.as_secs_f32());
                                        if Instruments::from(INSTRUMENTS).logging() {
                                            error!("🕝🕝🕝🕝 Executor '{}' TIMED OUT after {:?}", self_ref.executor_name, elapsed);
                                        }
                                    } else if Instruments::from(INSTRUMENTS).logging() {
                                        error!("🕝🕝🕝🕝 Executor '{}' TIMED OUT", self_ref.executor_name);
                                    }
                                }
                            }
                        }
                    };
                    match concurrency_limit {
                        1 => stream.for_each(item_processor).await,     // faster in `futures 0.3` -- may be useless in other versions
                        _ => stream.for_each_concurrent(concurrency_limit as usize, item_processor).await,
                    }
                    on_executor_end!(self_ref, true, true, self_ref.futures_timeout, INSTRUMENTS);     // notice the `fallible = true` here -- this is due to the timeouts, that shows as errors
                    stream_ended_callback(self).await;
                });
            },

        }
    }

    /// Spawns an optimized executor for a Stream of `FallibleItemType`s, where:\
    ///   * `FallibleItemType := Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>`
    pub fn spawn_fallibles_executor<const INSTRUMENTS:  usize,
                                    OutItemType:        Send + Debug,
                                    CloseVoidAsyncType: Future<Output=()> + Send + 'static>
                                   (self:                  Arc<Self>,
                                    concurrency_limit:     u32,
                                    on_err_callback:       impl Fn(Box<dyn Error + Send + Sync>)                               + Send + Sync + 'static,
                                    stream_ended_callback: impl FnOnce(Arc<StreamExecutor>) -> CloseVoidAsyncType + Send + Sync + 'static,
                                    stream:                impl Stream<Item=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + 'static + Send) {

        tokio::spawn(async move {
            on_executor_start!(self, true, true, Duration::ZERO, INSTRUMENTS);
            let item_processor = |element| {
                match element {
                    Ok(yielded_item) => {
                        on_non_timed_ok_item!(self, yielded_item, INSTRUMENTS);
                    },
                    Err(err) => {
                        on_non_timed_err_item!(self, err, INSTRUMENTS);
                        on_err_callback(err);
                    }
                }
            };
            match concurrency_limit {
                1 => stream.for_each(|item| future::ready(item_processor(item))).await,     // faster in `futures 0.3` -- may be useless in the future
                _ => stream.for_each_concurrent(concurrency_limit as usize, |item| future::ready(item_processor(item))).await,
            }
            on_executor_end!(self, false, true, Duration::ZERO, INSTRUMENTS);
            stream_ended_callback(self).await;
        });
    }

    /// Spawns an executor for a Stream of `ItemType`s which are not Futures but are fallible:
    ///   * `InnerFallibleType := Result<ItemType, Box<dyn std::error::Error>>`
    pub fn spawn_non_futures_executor<const INSTRUMENTS: usize,
                                      ItemType:          Send + Debug,
                                      VoidAsyncType:     Future<Output=()> + Send + 'static>
                                     (self:                      Arc<Self>,
                                      concurrency_limit:         u32,
                                      stream_ended_callback:     impl FnOnce(Arc<StreamExecutor>) -> VoidAsyncType + Send + Sync + 'static,
                                      stream:                    impl Stream<Item=Result<ItemType, Box<dyn std::error::Error + Send + Sync>>> + 'static + Send) {
        tokio::spawn(async move {
            on_executor_start!(self, false, true, Duration::ZERO, INSTRUMENTS);
            let item_processor = |fallible_element| {
                match fallible_element {
                    Ok(yielded_item) => on_non_timed_ok_item!(self, yielded_item, INSTRUMENTS),
                    Err(err) => on_non_timed_err_item!(self, err, INSTRUMENTS),
                }
            };
            match concurrency_limit {
                1 => stream.for_each(|fallible_item| future::ready(item_processor(fallible_item))).await,     // faster in `futures 0.3` -- may be useless in other versions
                _ => stream.for_each_concurrent(concurrency_limit as usize, |fallible_item| future::ready(item_processor(fallible_item))).await,
            }
            on_executor_end!(self, false, true, Duration::ZERO, INSTRUMENTS);
            stream_ended_callback(self).await;
        });
    }

    /// Spawns an optimized executor for a Stream of `ItemType`s which are not Futures and, also, are not fallible
    pub fn spawn_non_futures_non_fallibles_executor<const INSTRUMENTS: usize,
                                                    OutItemType:       Send + Debug,
                                                    VoidAsyncType:     Future<Output=()> + Send + 'static>
                                                  (self:                  Arc<Self>,
                                                   concurrency_limit:     u32,
                                                   stream_ended_callback: impl FnOnce(Arc<StreamExecutor>) -> VoidAsyncType + Send + Sync + 'static,
                                                   stream:                impl Stream<Item=OutItemType> + 'static + Send) {

        tokio::spawn(async move {
            on_executor_start!(self, false, false, Duration::ZERO, INSTRUMENTS);
            let item_processor = |yielded_item| on_non_timed_ok_item!(self, yielded_item, INSTRUMENTS);
            match concurrency_limit {
                1 => stream.for_each(|item| future::ready(item_processor(item))).await,     // faster in `futures 0.3` -- may be useless in the near future?
                _ => stream.for_each_concurrent(concurrency_limit as usize, |item| future::ready(item_processor(item))).await,
            }
            on_executor_end!(self, false, false, Duration::ZERO, INSTRUMENTS);
            stream_ended_callback(self).await;
        });
    }

}

/// Unit tests & enforces the requisites of the [stream_executor](self) module.\
/// Tests here mixes manual & automated assertions -- you should manually inspect the output of each one and check if the log outputs make sense
#[cfg(any(test,doc))]
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

    // fallible futures
    ///////////////////

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_non_timeout_futures_fallible_executor_with_logs_and_metrics() {
        assert_spawn_futures_fallible_executor::<{Instruments::LogsWithMetrics.into()}>(StreamExecutor::new("executor with logs & metrics")).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_non_timeout_futures_fallible_executor_with_metrics_and_no_logs() {
        assert_spawn_futures_fallible_executor::<{Instruments::MetricsWithoutLogs.into()}>(StreamExecutor::new("executor with metrics & NO logs")).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_non_timeout_futures_fallible_executor_with_logs_and_no_metrics() {
        assert_spawn_futures_fallible_executor::<{Instruments::LogsWithoutMetrics.into()}>(StreamExecutor::new("executor with logs & NO metrics")).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_non_timeout_futures_fallible_executor_with_no_logs_and_no_metrics() {
        assert_spawn_futures_fallible_executor::<{Instruments::NoInstruments.into()}>(StreamExecutor::new("executor with NO logs & NO metrics")).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_timeout_futures_fallible_executor_with_logs_and_metrics() {
        assert_spawn_futures_fallible_executor::<{Instruments::LogsWithMetrics.into()}>(StreamExecutor::with_futures_timeout("executor with logs & metrics", Duration::from_millis(100))).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_timeout_futures_fallible_executor_with_metrics_and_no_logs() {
        assert_spawn_futures_fallible_executor::<{Instruments::MetricsWithoutLogs.into()}>(StreamExecutor::with_futures_timeout("executor with metrics & NO logs", Duration::from_millis(100))).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_timeout_futures_fallible_executor_with_logs_and_no_metrics() {
        assert_spawn_futures_fallible_executor::<{Instruments::LogsWithoutMetrics.into()}>(StreamExecutor::with_futures_timeout("executor with logs & NO metrics", Duration::from_millis(100))).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_timeout_futures_fallible_executor_with_no_logs_and_no_metrics() {
        assert_spawn_futures_fallible_executor::<{Instruments::NoInstruments.into()}>(StreamExecutor::with_futures_timeout("executor with NO logs & NO metrics", Duration::from_millis(100))).await;
    }

    /// executes assertions on the given `executor` by spawning the executor to operate on streams yielding `Future<Result>` elements
    async fn assert_spawn_futures_fallible_executor<const INSTRUMENTS: usize>
                                                   (executor: Arc<StreamExecutor>) {

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
        executor.spawn_executor::<INSTRUMENTS, _, _, _, _>
                                 (1,
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
        assert_metrics::<INSTRUMENTS>
                        (cloned_executor, 4 - expected_timeout_count, expected_timeout_count, 2);

    }

    // non-fallible futures
    ///////////////////////

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_non_timeout_futures_executor_with_logs_and_metrics() {
        assert_spawn_futures_executor::<{Instruments::LogsWithMetrics.into()}>(StreamExecutor::new("executor with logs & metrics")).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_non_timeout_futures_executor_with_metrics_and_no_logs() {
        assert_spawn_futures_executor::<{Instruments::MetricsWithoutLogs.into()}>(StreamExecutor::new("executor with metrics & NO logs")).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_non_timeout_futures_executor_with_logs_and_no_metrics() {
        assert_spawn_futures_executor::<{Instruments::LogsWithoutMetrics.into()}>(StreamExecutor::new("executor with logs & NO metrics")).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_non_timeout_futures_executor_with_no_logs_and_no_metrics() {
        assert_spawn_futures_executor::<{Instruments::NoInstruments.into()}>(StreamExecutor::new("executor with NO logs & NO metrics")).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_timeout_futures_executor_with_logs_and_metrics() {
        assert_spawn_futures_executor::<{Instruments::LogsWithMetrics.into()}>(StreamExecutor::with_futures_timeout("executor with logs & metrics", Duration::from_millis(100))).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_timeout_futures_executor_with_metrics_and_no_logs() {
        assert_spawn_futures_executor::<{Instruments::MetricsWithoutLogs.into()}>(StreamExecutor::with_futures_timeout("executor with metrics & NO logs", Duration::from_millis(100))).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_timeout_futures_executor_with_logs_and_no_metrics() {
        assert_spawn_futures_executor::<{Instruments::LogsWithoutMetrics.into()}>(StreamExecutor::with_futures_timeout("executor with logs & NO metrics", Duration::from_millis(100))).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_timeout_futures_executor_with_no_logs_and_no_metrics() {
        assert_spawn_futures_executor::<{Instruments::NoInstruments.into()}>(StreamExecutor::with_futures_timeout("executor with NO logs & NO metrics", Duration::from_millis(100))).await;
    }

    /// executes assertions on the given `executor` by spawning the executor to operate on streams yielding `Future` elements
    async fn assert_spawn_futures_executor<const INSTRUMENTS: usize>
                                          (executor: Arc<StreamExecutor>) {

        async fn to_future(item: u32) -> u32 {
            // OK items > 100 will sleep for a while -- intended to cause a timeout, provided the given executor is appropriately configured
            if item > 100 {
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            item
        }

        let (tx, mut rx) = mpsc::channel::<bool>(10);
        let cloned_executor = Arc::clone(&executor);
        let timeout_enabled = executor.futures_timeout > Duration::ZERO;
        let expected_timeout_count = if timeout_enabled {2} else {0};
        executor.spawn_futures_executor::<INSTRUMENTS, _, _, _>
                                         (1,
                                          move |_| { let mut tx = tx.clone(); async move {tx.send(true).await.unwrap()} },
                                          stream::iter(vec![to_future(17),
                                                                      to_future(170),     // times out
                                                                      to_future(19),
                                                                      to_future(190)]     // times out
                                          ));
        assert!(rx.next().await.expect("consumption_done_reporter() wasn't called"), "consumption_done_reporter yielded the wrong value");
        assert_metrics::<INSTRUMENTS>
                        (cloned_executor, 4 - expected_timeout_count, expected_timeout_count, 0);

    }

    // fallible (non-futures)
    /////////////////////////

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_fallibles_executor_with_logs_and_metrics() {
        assert_spawn_fallibles_executor::<{Instruments::LogsWithMetrics.into()}>(StreamExecutor::new("executor with logs & metrics")).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_fallibles_executor_with_metrics_and_no_logs() {
        assert_spawn_fallibles_executor::<{Instruments::MetricsWithoutLogs.into()}>(StreamExecutor::new("executor with metrics & NO logs")).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_fallibles_executor_with_logs_and_no_metrics() {
        assert_spawn_fallibles_executor::<{Instruments::LogsWithoutMetrics.into()}>(StreamExecutor::new("executor with logs & NO metrics")).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_fallibles_executor_with_no_logs_and_no_metrics() {
        assert_spawn_fallibles_executor::<{Instruments::NoInstruments.into()}>(StreamExecutor::new("executor with NO logs & NO metrics")).await;
    }

    /// executes assertions on the given `executor` by spawning the executor to operate on streams yielding fallible elements
    async fn assert_spawn_fallibles_executor<const INSTRUMENTS: usize>
                                            (executor: Arc<StreamExecutor>) {
        let error_count = Arc::new(AtomicU32::new(0));
        let error_count_ref = Arc::clone(&error_count);
        let (mut tx, mut rx) = mpsc::channel::<bool>(10);
        let cloned_executor = Arc::clone(&executor);
        executor.spawn_fallibles_executor::<INSTRUMENTS, _, _>
                                           (1,
                                            move |_err| {
                                                error_count_ref.fetch_add(1, Relaxed);
                                            },
                                            move |_| async move {
                                                tx.send(true).await.unwrap()
                                            },
                                            stream::iter(vec![Ok(17), Ok(19), Err(Box::from(String::from("Error on 20th")))]) );
        assert!(rx.next().await.expect("consumption_done_reporter() wasn't called"), "consumption_done_reporter yielded the wrong value");
        assert_eq!(error_count.load(Relaxed), 1, "Error count is wrong, as computed by the error callback");
        assert_metrics::<INSTRUMENTS>
                        (cloned_executor, 2, 0, 1);

    }

    // non-fallible & non-futures
    /////////////////////////////

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_non_futures_non_fallibles_executor_with_logs_and_metrics() {
        assert_spawn_non_futures_non_fallibles_executor::<{Instruments::LogsWithMetrics.into()}>(StreamExecutor::new("executor with logs & metrics")).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_non_futures_non_fallibles_executor_with_metrics_and_no_logs() {
        assert_spawn_non_futures_non_fallibles_executor::<{Instruments::MetricsWithoutLogs.into()}>(StreamExecutor::new("executor with metrics & NO logs")).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_non_futures_non_fallibles_executor_with_logs_and_no_metrics() {
        assert_spawn_non_futures_non_fallibles_executor::<{Instruments::LogsWithoutMetrics.into()}>(StreamExecutor::new("executor with logs & NO metrics")).await;
    }

    #[cfg_attr(not(doc),tokio::test)]
    async fn spawn_non_futures_non_fallibles_executor_with_no_logs_and_no_metrics() {
        assert_spawn_non_futures_non_fallibles_executor::<{Instruments::NoInstruments.into()}>(StreamExecutor::new("executor with NO logs & NO metrics")).await;
    }

    /// executes assertions on the given `executor` by spawning the executor to operate on streams yielding non-futures / non-fallible elements
    async fn assert_spawn_non_futures_non_fallibles_executor<const INSTRUMENTS: usize>
                                                            (executor: Arc<StreamExecutor>) {
        let (mut tx, mut rx) = mpsc::channel::<bool>(10);
        let cloned_executor = Arc::clone(&executor);
        executor.spawn_non_futures_non_fallibles_executor::<INSTRUMENTS, _, _>
                                                         (1,
                                                          move |_| async move {tx.send(true).await.unwrap()},
                                                          stream::iter(vec![17, 19]) );
        assert!(rx.next().await.expect("consumption_done_reporter() wasn't called"), "consumption_done_reporter yielded the wrong value");
        assert_metrics::<INSTRUMENTS>
                        (cloned_executor, 2, 0, 0);

    }

    // auxiliary functions
    //////////////////////

    /// apply assertions on metrics for the given `executor`
    fn assert_metrics<const INSTRUMENTS: usize>
                     (executor: Arc<StreamExecutor>,
                      expected_ok_counter:         u32,
                      expected_timed_out_counter:  u32,
                      expected_failed_counter:     u32) {

        println!("### Stats assertions for Stream pipeline executor named '{}' (Logs? {}; Metrics? {}) ####",
                 executor.executor_name, Instruments::from(INSTRUMENTS).logging(), Instruments::from(INSTRUMENTS).metrics());
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
                 if Instruments::from(INSTRUMENTS).metrics() {""} else {" -- metrics are DISABLED"},
                 if Instruments::from(INSTRUMENTS).logging() {" -- verify these values against the \"executor closed\" message"} else {" -- logs are DISABLED"});
        println!("TIMED OUT elements count: {timed_out_counter}; TIMED OUT elements average Future resolution time: {timed_out_average}s{}{}",
                 if Instruments::from(INSTRUMENTS).metrics() {""} else {" -- metrics are DISABLED"},
                 if Instruments::from(INSTRUMENTS).logging() {" -- verify these values against the \"executor closed\" message"} else {" -- logs are DISABLED"});
        println!("FAILED elements count: {failed_counter}; FAILED elements average Future resolution time: {failed_average}s{}{}",
                 if Instruments::from(INSTRUMENTS).metrics() {""} else {" -- metrics are DISABLED"},
                 if Instruments::from(INSTRUMENTS).logging() {" -- verify these values against the \"executor closed\" message"} else {" -- logs are DISABLED"});

        assert_ne!(execution_start_delta_nanos,  u64::MAX, "'execution_start_delta_nanos' wasn't set");
        assert_ne!(execution_finish_delta_nanos, u64::MAX, "'execution_finish_delta_nanos' wasn't set");
        assert!(execution_finish_delta_nanos >= execution_start_delta_nanos, "INSTRUMENTATION ERROR: 'execution_start_delta_nanos' was set after 'execution_finish_delta_nanos'");

        if Instruments::from(INSTRUMENTS).metrics() {
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
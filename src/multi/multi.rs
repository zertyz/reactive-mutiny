//! See [super]

use super::super::{
    ogre_std::ogre_queues::full_sync_queues::full_sync_meta::FullSyncMeta,
    instruments::Instruments,
    multi::channels::{self,multi_stream::MultiStream},
    stream_executor::StreamExecutor,
};
use std::{
    sync::Arc,
    fmt::Debug,
    pin::Pin,
    time::Duration,
    future::Future,
};
use indexmap::IndexMap;
use futures::{Stream,StreamExt};
use tokio::{
    sync::{RwLock},
};


/// this is the fastest [MultiChannel] for general use, as revealed in performance tests
type MultiChannelType<'a, ItemType,
                          const BUFFER_SIZE: usize,
                          const MAX_STREAMS: usize> = channels::crossbeam::Crossbeam<'a, ItemType, BUFFER_SIZE, MAX_STREAMS>;
pub type MultiStreamType<'a, ItemType,
                             const BUFFER_SIZE: usize,
                             const MAX_STREAMS: usize> = MultiStream<'a, ItemType, MultiChannelType<'a, ItemType, BUFFER_SIZE, MAX_STREAMS>>;


/// `Multi` is an event handler capable of having several "listeners" -- all of which receives all events.\
/// With this struct, it is possible to:
///   - produce events
///   - spawn new `Stream`s & executors
///   - close `Stream`s (and executors)
/// Example:
/// ```nocompile
/// {reactive_mutiny::Instruments::MetricsWithoutLogs.into()}
pub struct Multi<'a, ItemType:          Send + Sync + Debug,
                     const BUFFER_SIZE: usize,
                     const MAX_STREAMS: usize,
                     const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}> {
    pub multi_name:     String,
    pub channel:        Arc<MultiChannelType<'a, ItemType, BUFFER_SIZE, MAX_STREAMS>>,
    pub executor_infos: RwLock<IndexMap<String, ExecutorInfo<INSTRUMENTS>>>,
}

impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize,
         const INSTRUMENTS: usize>
Multi<'a, ItemType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS> {

    pub fn new<IntoString: Into<String>>(multi_name: IntoString) -> Self {
        let multi_name = multi_name.into();
        Multi {
            multi_name:     multi_name.clone(),
            channel:        MultiChannelType::<ItemType, BUFFER_SIZE, MAX_STREAMS>::new(multi_name.clone()),
            executor_infos: RwLock::new(IndexMap::new()),
        }
    }

    pub fn stream_name(self: &Self) -> &str {
        &self.multi_name
    }

    #[inline(always)]
    pub fn send(&self, item: ItemType) {
        self.channel.send(item);
    }

    #[inline(always)]
    pub fn send_arc(&self, arc_item: &Arc<ItemType>) {
        self.channel.send_arc(arc_item);
    }

    #[inline(always)]
    pub fn buffer_size(&self) -> u32 {
        BUFFER_SIZE as u32
    }

    #[inline(always)]
    pub fn pending_items_count(&self) -> u32 {
        self.channel.pending_items_count()
    }

    /// Companion of [spawn_executor_ref()]
    pub async fn spawn_executor<IntoString:             Into<String>,
                                OutItemType:            Send + Debug,
                                OutStreamType:          Stream<Item=OutType> + Send + 'static,
                                OutType:                Future<Output=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send,
                                ErrVoidAsyncType:       Future<Output=()> + Send + 'static,
                                CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                               (self,
                                concurrency_limit:         u32,
                                futures_timeout:           Duration,
                                pipeline_name:             IntoString,
                                pipeline_builder:          impl FnOnce(MultiStreamType<'a, ItemType, BUFFER_SIZE, MAX_STREAMS>) -> OutStreamType,
                                on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>)                    -> ErrVoidAsyncType   + Send + Sync + 'static,
                                on_close_callback:         impl Fn(Arc<StreamExecutor<INSTRUMENTS>>)                            -> CloseVoidAsyncType + Send + Sync + 'static)

                                -> Result</*Self*/Multi<'a, ItemType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>, Box<dyn std::error::Error>> {

        self.spawn_executor_ref(concurrency_limit, futures_timeout, pipeline_name, pipeline_builder, on_err_callback, on_close_callback).await?;
        Ok(self)
    }

    /// Spawns a new listener of all subsequent events sent to this `Multi`, processing them through the `Stream` returned by `pipeline_builder()`,
    /// which generates events that are Futures & Fallible.
    pub async fn spawn_executor_ref<IntoString:             Into<String>,
                                    OutItemType:            Send + Debug,
                                    OutStreamType:          Stream<Item=OutType> + Send + 'static,
                                    OutType:                Future<Output=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send,
                                    ErrVoidAsyncType:       Future<Output=()> + Send + 'static,
                                    CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                   (&self,
                                    concurrency_limit:         u32,
                                    futures_timeout:           Duration,
                                    pipeline_name:             IntoString,
                                    pipeline_builder:          impl FnOnce(MultiStreamType<'a, ItemType, BUFFER_SIZE, MAX_STREAMS>) -> OutStreamType,
                                    on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>)                    -> ErrVoidAsyncType   + Send + Sync + 'static,
                                    on_close_callback:         impl Fn(Arc<StreamExecutor<INSTRUMENTS>>)                            -> CloseVoidAsyncType + Send + Sync + 'static)

                                    -> Result</*&Self*/&Multi<'a, ItemType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>, Box<dyn std::error::Error>> {

        let executor = StreamExecutor::with_futures_timeout(format!("{}: {}", self.stream_name(), pipeline_name.into()), futures_timeout);
        let (in_stream, in_stream_id) = self.channel.listener_stream();
        self.add_executor(Arc::clone(&executor), in_stream_id).await?;
        let out_stream = pipeline_builder(in_stream);
        executor
            .spawn_executor(
                concurrency_limit,
                on_err_callback,
                move |executor| on_close_callback(executor),
                out_stream
            );
        Ok(self)
    }

    /// Companion of [spawn_non_futures_non_fallible_executor_ref()]
    pub async fn spawn_non_futures_non_fallible_executor<IntoString:             Into<String>,
                                                         OutItemType:            Send + Debug,
                                                         OutStreamType:          Stream<Item=OutItemType> + Send + 'static,
                                                         CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                                        (self,
                                                         concurrency_limit:        u32,
                                                         pipeline_name:            IntoString,
                                                         pipeline_builder:         impl FnOnce(MultiStreamType<'a, ItemType, BUFFER_SIZE, MAX_STREAMS>) -> OutStreamType,
                                                         on_close_callback:        impl Fn(Arc<StreamExecutor<INSTRUMENTS>>)                            -> CloseVoidAsyncType + Send + Sync + 'static)

                                                        -> Result</*Self*/Multi<'a, ItemType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>, Box<dyn std::error::Error>> {
        self.spawn_non_futures_non_fallible_executor_ref(concurrency_limit, pipeline_name, pipeline_builder, on_close_callback).await?;
        Ok(self)
    }

    /// Spawns a new listener of all subsequent events sent to this `Multi`, processing them through the `Stream` returned by `pipeline_builder()`,
    /// which generates events that are Non-Futures & Non-Fallible.
    pub async fn spawn_non_futures_non_fallible_executor_ref<IntoString:             Into<String>,
                                                             OutItemType:            Send + Debug,
                                                             OutStreamType:          Stream<Item=OutItemType> + Send + 'static,
                                                             CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                                            (&self,
                                                             concurrency_limit:        u32,
                                                             pipeline_name:            IntoString,
                                                             pipeline_builder:         impl FnOnce(MultiStreamType<'a, ItemType, BUFFER_SIZE, MAX_STREAMS>) -> OutStreamType,
                                                             on_close_callback:        impl Fn(Arc<StreamExecutor<INSTRUMENTS>>)                            -> CloseVoidAsyncType + Send + Sync + 'static)

                                                            -> Result</*&Self*/&Multi<'a, ItemType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>, Box<dyn std::error::Error>> {

        let executor = StreamExecutor::new(format!("{}: {}", self.stream_name(), pipeline_name.into()));
        let (in_stream, in_stream_id) = self.channel.listener_stream();
        self.add_executor(Arc::clone(&executor), in_stream_id).await?;
        let out_stream = pipeline_builder(in_stream);
        executor
            .spawn_non_futures_non_fallible_executor(
                concurrency_limit,
                move |executor| on_close_callback(executor),
                out_stream
            );
        Ok(self)
    }

    /// Closes this `Multi`, in isolation -- flushing pending events, closing the producers,
    /// waiting for all events to be fully processed and calling all executor's "on close" callbacks.\
    /// If this `Multi` share resources with another one (which will get dumped by the "on close"
    /// callback), most probably you want to close them atomically -- see [multis_close_async!()]
    pub async fn close(self: &Self, timeout: Duration) {
        self.channel.end_all_streams(timeout).await;
    }

    /// Asynchronously blocks until all resources associated with the executor responsible for `pipeline_name` are freed:
    ///   1) immediately causes `pipeline_name` to cease receiving new elements by removing it from the active list
    ///   2) wait for all pending elements to be processed
    ///   3) remove the queue/channel and wake the Stream to see that it has ended
    ///   4) waits for the executor to inform it ceased its execution
    ///   5) return, dropping all resources\
    /// Note it might make sense to spawn this operation by a `Tokio task`, for it may block indefinitely if the Stream has no timeout.\
    /// Also note that timing out this operation is not advisable, for resources won't be freed until it reaches the last step.\
    /// Returns false if there was no executor associated with `pipeline_name`.
    pub async fn flush_and_cancel_executor<IntoString: Into<String>>
                                          (self:          &Self,
                                           pipeline_name: IntoString,
                                           timeout:       Duration) -> bool {

        let executor_name = format!("{}: {}", self.multi_name, pipeline_name.into());
        // remove the pipeline from the active list
        let mut executor_infos = self.executor_infos.write().await;
        let executor_info = match executor_infos.remove(&executor_name) {
            Some(executor) => executor,
            None => return false,
        };
        drop(executor_infos);

        // wait until all elements are taken out from the queue
        executor_info.stream_executor.report_scheduled_to_finish();
        self.channel.end_stream(executor_info.stream_id, timeout).await;
        true
    }

    /// Registers an executor within this `Multi` so it can be managed -- closed, inquired for stats, etc
    async fn add_executor(&self, stream_executor: Arc<StreamExecutor<INSTRUMENTS>>, stream_id: u32) -> Result<(), Box<dyn std::error::Error>> {
        let mut internal_multis = self.executor_infos.write().await;
        if internal_multis.contains_key(&stream_executor.executor_name()) {
            Err(Box::from(format!("Can't add a new listener pipeline to a Multi: an executor with the same name is already present: '{}'", stream_executor.executor_name())))
        } else {
            internal_multis.insert(stream_executor.executor_name(), ExecutorInfo { stream_executor, stream_id });
            Ok(())
        }
    }

}

/// Macro to close, atomically, all [Multi]s passed in as parameters
#[macro_export]
macro_rules! multis_close_async {
    ($timeout: expr,
     $($multi: expr),+) => {
        {
            tokio::join!( $( $multi.channel.flush($timeout), )+ );
            tokio::join!( $( $multi.channel.end_all_streams($timeout), )+ );
        }
    }
}
pub use multis_close_async;
use crate::ogre_std::ogre_queues::atomic_queues::atomic_meta::AtomicMeta;

/// Keeps track of the `stream_executor` associated to each `stream_id`
pub struct ExecutorInfo<const INSTRUMENTS: usize> {
    pub stream_executor: Arc<StreamExecutor<INSTRUMENTS>>,
    pub stream_id: u32,
}

//! Builders for [Multi], responsible for creating the executors and creating the producers.\
//! For more, see [super]

use super::{
    multi::{Multi},
    super::{
        instruments::Instruments,
        stream_executor::StreamExecutor,
        types::*,
    },
};
use std::{
    fmt::Debug,
    future::Future,
    sync::Arc,
    time::Duration,
};
use futures::{Stream,StreamExt};
use crate::multi::MultiStreamType;


/// this will simply be ripped off...
/// pub type OnMultiCloseFnType = Box<dyn Fn(Arc<StreamExecutor<true, true>>) -> BoxFuture<'static, ()> + Send + Sync + 'static>;
/// Example:
/// ```
/// {reactive_mutiny::Instruments::MetricsWithoutLogs.into()}
pub struct MultiBuilder<ItemType:            Send + Sync + Debug,
                        const BUFFER_SIZE:   usize = 1024,
                        const MAX_STREAMS:   usize = 1,
                        const INSTRUMENTS:   usize = {Instruments::LogsWithMetrics.into()}> {

    pub handle: Multi<ItemType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>,

}
impl<'a, ItemType:            Send + Sync + Debug + 'a,
         const BUFFER_SIZE:   usize,
         const MAX_STREAMS:   usize,
         const INSTRUMENTS:   usize>
MultiBuilder<ItemType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS> {

    pub fn new<IntoString: Into<String>>
              (multi_name: IntoString) -> Self {
        Self {
            handle: Multi::new(multi_name),
        }
    }

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

                                -> Result<Self, Box<dyn std::error::Error>> {

        self.spawn_executor_ref(concurrency_limit, futures_timeout, pipeline_name, pipeline_builder, on_err_callback, on_close_callback).await?;
        Ok(self)
    }

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

                                    -> Result<&Self, Box<dyn std::error::Error>> {

        let executor = StreamExecutor::with_futures_timeout(format!("{}: {}", self.handle.stream_name(), pipeline_name.into()), futures_timeout);
        let (in_stream, in_stream_id) = self.handle.listener_stream();
        self.handle.add_executor(Arc::clone(&executor), in_stream_id).await?;
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

    pub async fn spawn_non_futures_non_fallible_executor<IntoString:             Into<String>,
                                                         OutItemType:            Send + Debug,
                                                         OutStreamType:          Stream<Item=OutItemType> + Send + 'static,
                                                         CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                                        (self,
                                                         concurrency_limit:        u32,
                                                         pipeline_name:            IntoString,
                                                         pipeline_builder:         impl FnOnce(MultiStreamType<'a, ItemType, BUFFER_SIZE, MAX_STREAMS>) -> OutStreamType,
                                                         on_close_callback:        impl Fn(Arc<StreamExecutor<INSTRUMENTS>>)                            -> CloseVoidAsyncType + Send + Sync + 'static)

                                                        -> Result<Self, Box<dyn std::error::Error>> {
        self.spawn_non_futures_non_fallible_executor_ref(concurrency_limit, pipeline_name, pipeline_builder, on_close_callback).await?;
        Ok(self)
    }

    pub async fn spawn_non_futures_non_fallible_executor_ref<IntoString:             Into<String>,
                                                             OutItemType:            Send + Debug,
                                                             OutStreamType:          Stream<Item=OutItemType> + Send + 'static,
                                                             CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                                            (&self,
                                                             concurrency_limit:        u32,
                                                             pipeline_name:            IntoString,
                                                             pipeline_builder:         impl FnOnce(MultiStreamType<'a, ItemType, BUFFER_SIZE, MAX_STREAMS>) -> OutStreamType,
                                                             on_close_callback:        impl Fn(Arc<StreamExecutor<INSTRUMENTS>>)                            -> CloseVoidAsyncType + Send + Sync + 'static)

                                                            -> Result<&Self, Box<dyn std::error::Error>> {

        let executor = StreamExecutor::new(format!("{}: {}", self.handle.stream_name(), pipeline_name.into()));
        let (in_stream, in_stream_id) = self.handle.listener_stream();
        self.handle.add_executor(Arc::clone(&executor), in_stream_id).await?;
        let out_stream = pipeline_builder(in_stream);
        executor
            .spawn_non_futures_non_fallible_executor(
                concurrency_limit,
                move |executor| on_close_callback(executor),
                out_stream
            );
        Ok(self)
    }

    /// See [Multi::flush_and_cancel_executor()]
    pub async fn flush_and_cancel_executor<IntoString: Into<String>>
                                          (&self,
                                           pipeline_name: IntoString,
                                           timeout:       Duration) -> bool {
        self.handle.flush_and_cancel_executor(pipeline_name, timeout).await
    }

    pub fn handle(self) -> Multi<ItemType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS> {
        self.handle
    }

    pub fn handle_ref(&'a self) -> &'a Multi<ItemType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS> {
        &self.handle
    }

}
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
use futures::Stream;


/// this will simply be ripped off...
/// pub type OnMultiCloseFnType = Box<dyn Fn(Arc<StreamExecutor<true, true>>) -> BoxFuture<'static, ()> + Send + Sync + 'static>;


pub struct MultiBuilder<InType:              Clone + Unpin + Send + Sync + Debug,
                        const BUFFER_SIZE:   usize = 1024,
                        const MAX_STREAMS:   usize = 1,
                        const INSTRUMENTS:   usize = {Instruments::LogsWithMetrics.into()}> {

    pub handle:                   Arc<Multi<InType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>>,

}
impl<'a, InType:              Clone + Unpin + Send + Sync + Debug + 'static,
         const BUFFER_SIZE:   usize,
         const MAX_STREAMS:   usize,
         const INSTRUMENTS:   usize>
MultiBuilder<InType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS> {

    pub fn new<IntoString: Into<String>>
              (multi_name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            handle:                   Multi::new(multi_name),
        })
    }

    pub async fn spawn_executor<IntoString:             Into<String>,
                                OutItemType:            Send + Debug,
                                PipelineBuilderFnType:  FnOnce(MutinyStream<InType>)                 -> OutStreamType,
                                OutStreamType:          Stream<Item=OutType> + Send + 'static,
                                OutType:                Future<Output=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send,
                                OnErrFnType:            Fn(Box<dyn std::error::Error + Send + Sync>) -> ErrVoidAsyncType   + Send + Sync + 'static,
                                ErrVoidAsyncType:       Future<Output=()> + Send + 'static,
                                OnCloseFnType:          Fn(Arc<StreamExecutor<INSTRUMENTS>>)         -> CloseVoidAsyncType + Send + Sync + 'static,
                                CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                               (self:                      &'a Arc<Self>,
                                concurrency_limit:         u32,
                                futures_timeout:           Duration,
                                pipeline_name:             IntoString,
                                pipeline_builder:          PipelineBuilderFnType,
                                on_err_callback:           OnErrFnType,
                                on_close_callback:         OnCloseFnType)

                                -> Result<&'a Arc<Self>, Box<dyn std::error::Error>> {

        let executor = StreamExecutor::with_futures_timeout(format!("{}: {}", self.handle.stream_name(), pipeline_name.into()), futures_timeout);
        let (in_stream, in_stream_id) = self.handle.listener_stream();
        self.handle.add_executor(Arc::clone(&executor), in_stream_id).await?;
        let out_stream = pipeline_builder(MutinyStream { stream: Box::new(in_stream) });
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
                                                         PipelineBuilderFnType:  FnOnce(MutinyStream<InType>)          -> OutStreamType,
                                                         OutStreamType:          Stream<Item=OutItemType> + Send + 'static,
                                                         OnCloseFnType:          Fn(Arc<StreamExecutor<INSTRUMENTS>>)  -> CloseVoidAsyncType + Send + Sync + 'static,
                                                         CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                                        (self:                     &'a Arc<Self>,
                                                         concurrency_limit:        u32,
                                                         pipeline_name:            IntoString,
                                                         pipeline_builder:         PipelineBuilderFnType,
                                                         on_close_callback:        OnCloseFnType)

                                                        -> Result<&'a Arc<Self>, Box<dyn std::error::Error>> {

        let executor = StreamExecutor::new(format!("{}: {}", self.handle.stream_name(), pipeline_name.into()));
        let (in_stream, in_stream_id) = self.handle.listener_stream();
        self.handle.add_executor(Arc::clone(&executor), in_stream_id).await?;
        let out_stream = pipeline_builder(MutinyStream { stream: Box::new(in_stream) });
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
                                          (self:          &Arc<Self>,
                                           pipeline_name: IntoString,
                                           timeout:       Duration) -> bool {
        self.handle.flush_and_cancel_executor(pipeline_name, timeout).await
    }

    pub fn handle(self: &Arc<Self>) -> Arc<Multi<InType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>> {
        Arc::clone(&self.handle)
    }

}
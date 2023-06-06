//! Builders for [Uni], responsible for creating the executor and creating the producers.\
//! For more, see [super]

use super::{
    super::{
        instruments::Instruments,
        stream_executor::StreamExecutor,
        mutiny_stream::MutinyStream,
        types::{FullDuplexUniChannel},
    },
    uni::Uni,
};
use std::{
    fmt::Debug,
    future::Future,
    marker::PhantomData,
    sync::{
        Arc,
        atomic::Ordering::Relaxed,
    },
    time::Duration,
};
use futures::Stream;


pub struct UniBuilder<InType:              'static + Debug + Sync + Send,
                      UniChannelType:      FullDuplexUniChannel<'static, InType, DerivedItemType> + Sync + Send + 'static,
                      const INSTRUMENTS:   usize,
                      DerivedItemType:     'static + Debug + Sync + Send> {

    pub concurrency_limit:        u32,
    /// if Some, will cause the pipeline to be wrapped with [tokio::time::timeout] so that an Err will be produced whenever the pipeline takes too much time to process
    pub futures_timeout:          Duration,
    /// Instrumentation this instance gathers / produces
    pub instruments:              Instruments,
    // phantoms
    _phantom:                     PhantomData<(InType, UniChannelType, DerivedItemType)>,

}
impl<InType:              'static + Sync + Send + Debug,
     UniChannelType:      FullDuplexUniChannel<'static, InType, DerivedItemType> + Sync + Send + 'static,
     const INSTRUMENTS:   usize,
     DerivedItemType:     'static + Debug + Sync + Send>
UniBuilder<InType, UniChannelType, INSTRUMENTS, DerivedItemType> {

    pub fn new() -> Self {
        Self {
            concurrency_limit:        1,
            futures_timeout:          Duration::ZERO,
            instruments:              Instruments::from(INSTRUMENTS),
            _phantom:                 Default::default(),
        }
    }

    pub fn concurrency_limit(mut self, concurrency_limit: u32) -> Self {
        self.concurrency_limit = concurrency_limit;
        self
    }

    pub fn futures_timeout(mut self, futures_timeout: Duration) -> Self {
        self.futures_timeout = futures_timeout;
        self
    }

    #[must_use]
    pub fn spawn_executor<IntoString:             Into<String>,
                          OutItemType:            Send + Debug,
                          OutStreamType:          Stream<Item=OutType> + Send + 'static,
                          OutType:                Future<Output=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send,
                          ErrVoidAsyncType:       Future<Output=()> + Send + 'static,
                          CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                         (self,
                          stream_name:               IntoString,
                          pipeline_builder:          impl FnOnce(MutinyStream<'static, InType, UniChannelType, DerivedItemType>) -> OutStreamType,
                          on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>)                           -> ErrVoidAsyncType   + Send + Sync + 'static,
                          on_close_callback:         impl Fn(Arc<StreamExecutor<INSTRUMENTS>>)                                   -> CloseVoidAsyncType + Send + Sync + 'static)

                         -> Arc<Uni<'static, InType, UniChannelType, INSTRUMENTS, DerivedItemType>> {

        let stream_name = stream_name.into();
        let executor = StreamExecutor::with_futures_timeout(stream_name.to_string(), self.futures_timeout);
        let handle = Arc::new(Uni::new(stream_name, Arc::clone(&executor)));
        let in_stream = handle.consumer_stream();
        let returned_handle = Arc::clone(&handle);
        let out_stream = pipeline_builder(in_stream);
        let on_close_callback = Arc::new(on_close_callback);
        executor
            .spawn_executor(
                self.concurrency_limit,
                on_err_callback,
                move |executor| {
                    let on_close_callback = Arc::clone(&on_close_callback);
                    let handle = Arc::clone(&handle);
                        async move {
                        handle.finished_executors_count.fetch_add(1, Relaxed);
                        on_close_callback(executor).await;
                    }
                },
                out_stream
            );
        returned_handle
    }

    #[must_use]
    pub fn spawn_fallibles_executor<IntoString:             Into<String>,
                                    OutItemType:            Send + Debug,
                                    OutStreamType:          Stream<Item=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
                                    CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                   (self,
                                    stream_name:               IntoString,
                                    pipeline_builder:          impl FnOnce(MutinyStream<'static, InType, UniChannelType, DerivedItemType>) -> OutStreamType,
                                    on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>)                                                 + Send + Sync + 'static,
                                    on_close_callback:         impl Fn(Arc<StreamExecutor<INSTRUMENTS>>)                                   -> CloseVoidAsyncType + Send + Sync + 'static)

                                   -> Arc<Uni<'static, InType, UniChannelType, INSTRUMENTS, DerivedItemType>> {

        let stream_name = stream_name.into();
        let executor = StreamExecutor::with_futures_timeout(stream_name.to_string(), self.futures_timeout);
        let handle = Arc::new(Uni::new(stream_name, Arc::clone(&executor)));
        let in_stream = handle.consumer_stream();
        let returned_handle = Arc::clone(&handle);
        let out_stream = pipeline_builder(in_stream);
        let on_close_callback = Arc::new(on_close_callback);
        executor
            .spawn_fallibles_executor(
                self.concurrency_limit,
                on_err_callback,
                move |executor| {
                    let on_close_callback = Arc::clone(&on_close_callback);
                    let handle = Arc::clone(&handle);
                        async move {
                        handle.finished_executors_count.fetch_add(1, Relaxed);
                        on_close_callback(executor).await;
                    }
                },
                out_stream
            );
        returned_handle
    }

    #[must_use]
    pub fn spawn_futures_executor<IntoString:             Into<String>,
                                  OutItemType:            Send + Debug,
                                  OutStreamType:          Stream<Item=OutType> + Send + 'static,
                                  OutType:                Future<Output=OutItemType> + Send,
                                  CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                 (self,
                                  stream_name:               IntoString,
                                  pipeline_builder:          impl FnOnce(MutinyStream<'static, InType, UniChannelType, DerivedItemType>) -> OutStreamType,
                                  on_close_callback:         impl Fn(Arc<StreamExecutor<INSTRUMENTS>>)                                   -> CloseVoidAsyncType + Send + Sync + 'static)

                                 -> Arc<Uni<'static, InType, UniChannelType, INSTRUMENTS, DerivedItemType>> {

        let stream_name = stream_name.into();
        let executor = StreamExecutor::with_futures_timeout(stream_name.to_string(), self.futures_timeout);
        let handle = Arc::new(Uni::new(stream_name, Arc::clone(&executor)));
        let in_stream = handle.consumer_stream();
        let returned_handle = Arc::clone(&handle);
        let out_stream = pipeline_builder(in_stream);
        let on_close_callback = Arc::new(on_close_callback);
        executor
            .spawn_futures_executor(
                self.concurrency_limit,
                move |executor| {
                    let on_close_callback = Arc::clone(&on_close_callback);
                    let handle = Arc::clone(&handle);
                        async move {
                        handle.finished_executors_count.fetch_add(1, Relaxed);
                        on_close_callback(executor).await;
                    }
                },
                out_stream
            );
        returned_handle
    }

    #[must_use]
    pub fn spawn_non_futures_non_fallibles_executor<IntoString:             Into<String>,
                                                   OutItemType:            Send + Debug,
                                                   OutStreamType:          Stream<Item=OutItemType> + Send + 'static,
                                                   CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                                  (self,
                                                   stream_name:              IntoString,
                                                   pipeline_builder:         impl FnOnce(MutinyStream<'static, InType, UniChannelType, DerivedItemType>) -> OutStreamType,
                                                   on_close_callback:        impl FnOnce(Arc<StreamExecutor<INSTRUMENTS>>)                               -> CloseVoidAsyncType + Send + Sync + 'static)

                                                  -> Arc<Uni<'static, InType, UniChannelType, INSTRUMENTS, DerivedItemType>> {

        let stream_name = stream_name.into();
        let executor = StreamExecutor::new(stream_name.to_string());
        let handle = Arc::new(Uni::new(stream_name, Arc::clone(&executor)));
        let in_stream = handle.consumer_stream();
        let returned_handle = Arc::clone(&handle);
        let out_stream = pipeline_builder(in_stream);
        executor
            .spawn_non_futures_non_fallibles_executor(
                self.concurrency_limit,
                move |executor| {
                    let handle = Arc::clone(&handle);
                    async move {
                        handle.finished_executors_count.fetch_add(1, Relaxed);
                        on_close_callback(executor).await;
                    }
                },
                out_stream
            );
        returned_handle
    }

}
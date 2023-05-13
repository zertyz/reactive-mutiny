//! Builders for [Uni], responsible for creating the executor and creating the producers.\
//! For more, see [super]

use super::{
    super::{
        instruments::Instruments,
        stream_executor::StreamExecutor,
        mutiny_stream::MutinyStream,
        types::ChannelConsumer,
    },
    uni::Uni,
    channels::{ChannelProducer, ChannelCommon},
};
use std::{
    fmt::Debug,
    future::Future,
    marker::PhantomData,
    sync::Arc,
    time::Duration,
};
use std::sync::atomic::Ordering::Relaxed;
use futures::Stream;
use crate::uni::channels::FullDuplexChannel;


pub struct UniBuilder<InType:              'static + Debug + Sync + Send,
                      UniChannelType:      FullDuplexChannel<'static, InType, DerivedItemType> + Sync + Send + 'static,
                      const INSTRUMENTS:   usize,
                      DerivedItemType:     'static + Debug + Sync + Send,
                      OnStreamCloseFnType: Fn(Arc<StreamExecutor<INSTRUMENTS>>) -> CloseVoidAsyncType + Send + Sync + 'static,
                      CloseVoidAsyncType:  Future<Output=()> + Send + 'static> {

    pub on_stream_close_callback: Option<Arc<OnStreamCloseFnType>>,
    pub concurrency_limit:        u32,
    /// if Some, will cause the pipeline to be wrapped with [tokio::time::timeout] so that an Err will be produced whenever the pipeline takes too much time to process
    pub futures_timeout:          Duration,
    /// Instrumentation this instance gathers / produces
    pub instruments:              Instruments,
    // phantoms
    _phantom:                     PhantomData<(InType, UniChannelType, DerivedItemType)>,

}
impl<InType:              'static + Sync + Send + Debug,
     UniChannelType:      FullDuplexChannel<'static, InType, DerivedItemType> + Sync + Send + 'static,
     const INSTRUMENTS:   usize,
     DerivedItemType:     'static + Debug + Sync + Send,
     OnStreamCloseFnType: Fn(Arc<StreamExecutor<INSTRUMENTS>>) -> CloseVoidAsyncType + Send + Sync + 'static,
     CloseVoidAsyncType:  Future<Output=()> + Send + 'static>
UniBuilder<InType, UniChannelType, INSTRUMENTS, DerivedItemType, OnStreamCloseFnType, CloseVoidAsyncType> {

    pub fn new() -> Self {
        Self {
            on_stream_close_callback: None,
            concurrency_limit:        1,
            futures_timeout:          Duration::ZERO,
            instruments:              Instruments::from(INSTRUMENTS),
            _phantom:                 Default::default(),
        }
    }

    pub fn on_stream_close(mut self, on_stream_close_callback: OnStreamCloseFnType) -> Self {
        self.on_stream_close_callback = Some(Arc::new(on_stream_close_callback));
        self
    }

    pub fn concurrency_limit(mut self, concurrency_limit: u32) -> Self {
        self.concurrency_limit = concurrency_limit;
        self
    }

    pub fn futures_timeout(mut self, futures_timeout: Duration) -> Self {
        self.futures_timeout = futures_timeout;
        self
    }

    pub fn spawn_executor<IntoString:             Into<String>,
                          OutItemType:            Send + Debug,
                          ErrVoidAsyncType:       Future<Output=()> + Send + 'static,
                          OutStreamType:          Stream<Item=OutType> + Send + 'static,
                          OutType:                Future<Output=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send>

                         (self,
                          stream_name:               IntoString,
                          pipeline_builder:          impl FnOnce(MutinyStream<'static, InType, UniChannelType, DerivedItemType>) -> OutStreamType,
                          on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>) -> ErrVoidAsyncType   + Send + Sync + 'static)

                         -> Arc<Uni<'static, InType, UniChannelType, INSTRUMENTS, DerivedItemType>> {

        let stream_name = stream_name.into();
        let executor = StreamExecutor::with_futures_timeout(stream_name.to_string(), self.futures_timeout);
        let handle = Arc::new(Uni::new(stream_name, Arc::clone(&executor)));
        let in_stream = handle.consumer_stream();
        let returned_handle = Arc::clone(&handle);
        let out_stream = pipeline_builder(in_stream);
        let on_stream_close_callback = self.on_stream_close_callback.expect("how could you compile without a 'on_stream_close_callback'?");
        executor
            .spawn_executor(
                self.concurrency_limit,
                on_err_callback,
                move |executor| {
                    let handle = Arc::clone(&handle);
                    let on_stream_close_callback = Arc::clone(&on_stream_close_callback);
                        async move {
                        handle.finished_executors_count.fetch_add(1, Relaxed);
                        on_stream_close_callback(executor);
                    }
                },
                out_stream
            );
        returned_handle
    }

    pub fn spawn_non_futures_non_fallible_executor<IntoString:             Into<String>,
                                                   OutItemType:            Send + Debug,
                                                   OutStreamType:          Stream<Item=OutItemType> + Send + 'static>

                                                  (self,
                                                   stream_name:              IntoString,
                                                   pipeline_builder:         impl FnOnce(MutinyStream<'static, InType, UniChannelType, DerivedItemType>) -> OutStreamType)

                                                  -> Arc<Uni<'static, InType, UniChannelType, INSTRUMENTS, DerivedItemType>> {

        let stream_name = stream_name.into();
        let executor = StreamExecutor::new(stream_name.to_string());
        let handle = Arc::new(Uni::new(stream_name, Arc::clone(&executor)));
        let in_stream = handle.consumer_stream();
        let returned_handle = Arc::clone(&handle);
        let out_stream = pipeline_builder(in_stream);
        executor
            .spawn_non_futures_non_fallible_executor(
                self.concurrency_limit,
                move |_executor| {
                    let handle = Arc::clone(&handle);
                    async move {
                        handle.finished_executors_count.fetch_add(1, Relaxed);
                        self.on_stream_close_callback.as_ref().expect("how could you compile without a 'on_stream_close_callback'?");
                    }
                },
                out_stream
            );
        returned_handle
    }

}


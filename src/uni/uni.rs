//! See [super]

use super::{
    super::{
        stream_executor::StreamExecutor,
        mutiny_stream::MutinyStream,
        types::{FullDuplexUniChannel},
    },
};
use std::{
    fmt::Debug,
    time::Duration,
    sync::{Arc, atomic::{AtomicU32, Ordering::Relaxed}},
};
use std::future::Future;
use std::marker::PhantomData;
use futures::Stream;
use minstant::Instant;


/// Contains the producer-side [Uni] handle used to interact with the `uni` event
/// -- for closing the stream, requiring stats, ...
pub struct Uni<ItemType:          Send + Sync + Debug + 'static,
               UniChannelType:    FullDuplexUniChannel<'static, ItemType, DerivedItemType> + Send + Sync + 'static,
               const INSTRUMENTS: usize,
               DerivedItemType:   Debug + 'static = ItemType> {
    pub uni_channel:              Arc<UniChannelType>,
    pub stream_executors:         Vec<Arc<StreamExecutor<INSTRUMENTS>>>,
    pub finished_executors_count: AtomicU32,
        _phantom:                 PhantomData<(&'static ItemType, &'static DerivedItemType)>,
}

impl<ItemType:          Send + Sync + Debug + 'static,
     UniChannelType:    FullDuplexUniChannel<'static, ItemType, DerivedItemType> + Send + Sync + 'static,
     const INSTRUMENTS: usize,
     DerivedItemType:   Debug + Sync>
Uni<ItemType, UniChannelType, INSTRUMENTS, DerivedItemType> {

    /// creates & returns a pair (`Uni`, `UniStream`)
    pub fn new<IntoString: Into<String>>(uni_name: IntoString) -> Self {
        Uni {
            uni_channel:              UniChannelType::new(uni_name),
            stream_executors:         vec![],
            finished_executors_count: AtomicU32::new(0),
            _phantom:                 PhantomData::default(),
        }
    }

    #[inline(always)]
    #[must_use]
    pub fn try_send<F: FnOnce(&mut ItemType)>(&self, setter: F) -> bool {
        self.uni_channel.try_send(setter)
    }

    #[inline(always)]
    pub fn send<F: FnOnce(&mut ItemType)>(&self, setter: F) {
        self.uni_channel.send(setter)
    }

    #[inline(always)]
    #[must_use]
    pub fn try_send_movable(&self, item: ItemType) -> bool {
        self.uni_channel.try_send_movable(item)
    }

    pub fn consumer_stream(&self) -> MutinyStream<'static, ItemType, UniChannelType, DerivedItemType> {
        let (stream, _stream_id) = self.uni_channel.create_stream();
        stream
    }

    pub async fn flush(&self, duration: Duration) -> u32 {
        self.uni_channel.flush(duration).await
    }

    /// closes this Uni, in isolation -- flushing pending events, closing the producers,
    /// waiting for all events to be fully processed and calling the "on close" callback.\
    /// If this Uni share resources with another one (which will get dumped by the "on close"
    /// callback), most probably you want to close them atomically -- see [unis_close_async!()]
    #[must_use]
    pub async fn close(&self, timeout: Duration) -> bool {
        let start = Instant::now();
        if self.uni_channel.gracefully_end_all_streams(timeout).await > 0 {
            false
        } else {
            loop {
                if self.finished_executors_count.load(Relaxed) > 0 {
                    break true;
                }
                // enforce timeout
                if timeout != Duration::ZERO && start.elapsed() > timeout {
                    break false
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    }

    #[must_use]
    pub fn spawn_executor<OutItemType:            Send + Debug,
                          OutStreamType:          Stream<Item=OutType> + Send + 'static,
                          OutType:                Future<Output=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send,
                          ErrVoidAsyncType:       Future<Output=()> + Send + 'static,
                          CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                         (mut self,
                          concurrency_limit:         u32,
                          futures_timeout:           Duration,
                          pipeline_builder:          impl FnOnce(MutinyStream<'static, ItemType, UniChannelType, DerivedItemType>) -> OutStreamType,
                          on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>)                             -> ErrVoidAsyncType   + Send + Sync + 'static,
                          on_close_callback:         impl FnOnce(Arc<StreamExecutor<INSTRUMENTS>>)                                 -> CloseVoidAsyncType + Send + Sync + 'static)

                         -> Arc<Uni<ItemType, UniChannelType, INSTRUMENTS, DerivedItemType>> {

        let pipeline_name = format!("Consumer #1 for Uni '{}'", self.uni_channel.name());
        let on_close_callback = Some(on_close_callback);
        // TODO: 2023-06-14: create as many executors as `self.uni_channel.MAX_STREAMS` (new API there might be needed)
        let in_stream = self.consumer_stream();
        let executor = StreamExecutor::with_futures_timeout(pipeline_name.to_string(), futures_timeout);
        self.stream_executors.push(Arc::clone(&executor));
        let out_stream = pipeline_builder(in_stream);
        let arc_self = Arc::new(self);
        let arc_self_ref = Arc::clone(&arc_self);
        executor
            .spawn_executor(
                concurrency_limit,
                on_err_callback,
                move |executor| {
                    let arc_self = Arc::clone(&arc_self);
                        async move {
                        arc_self.finished_executors_count.fetch_add(1, Relaxed);
                        (on_close_callback.expect("TODO: 2023-06-14: to be called when the last executor ends"))(executor).await;
                    }
                },
                out_stream
            );
        arc_self_ref
    }

    #[must_use]
    pub fn spawn_fallibles_executor<OutItemType:            Send + Debug,
                                    OutStreamType:          Stream<Item=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
                                    CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                   (mut self,
                                    concurrency_limit:         u32,
                                    pipeline_builder:          impl FnOnce(MutinyStream<'static, ItemType, UniChannelType, DerivedItemType>) -> OutStreamType,
                                    on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>)                                                   + Send + Sync + 'static,
                                    on_close_callback:         impl FnOnce(Arc<StreamExecutor<INSTRUMENTS>>)                                 -> CloseVoidAsyncType + Send + Sync + 'static)

                                    -> Arc<Uni<ItemType, UniChannelType, INSTRUMENTS, DerivedItemType>> {

        let pipeline_name = format!("Consumer #1 for Uni '{}'", self.uni_channel.name());
        let on_close_callback = Some(on_close_callback);
        // TODO: 2023-06-14: create as many executors as `self.uni_channel.MAX_STREAMS` (new API there might be needed)
        let in_stream = self.consumer_stream();
        let executor = StreamExecutor::new(pipeline_name.to_string());
        self.stream_executors.push(Arc::clone(&executor));
        let out_stream = pipeline_builder(in_stream);
        let arc_self = Arc::new(self);
        let arc_self_ref = Arc::clone(&arc_self);
        executor
            .spawn_fallibles_executor(
                concurrency_limit,
                on_err_callback,
                move |executor| {
                    let arc_self = Arc::clone(&arc_self);
                        async move {
                        arc_self.finished_executors_count.fetch_add(1, Relaxed);
                        (on_close_callback.expect("TODO: 2023-06-14: to be called when the last executor ends"))(executor).await;
                    }
                },
                out_stream
            );
        arc_self_ref
    }

    #[must_use]
    pub fn spawn_futures_executor<OutItemType:            Send + Debug,
                                  OutStreamType:          Stream<Item=OutType> + Send + 'static,
                                  OutType:                Future<Output=OutItemType> + Send,
                                  CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                 (mut self,
                                  concurrency_limit:         u32,
                                  futures_timeout:           Duration,
                                  pipeline_builder:          impl FnOnce(MutinyStream<'static, ItemType, UniChannelType, DerivedItemType>) -> OutStreamType,
                                  on_close_callback:         impl FnOnce(Arc<StreamExecutor<INSTRUMENTS>>)                                 -> CloseVoidAsyncType + Send + Sync + 'static)

                                 -> Arc<Uni<ItemType, UniChannelType, INSTRUMENTS, DerivedItemType>> {

        let pipeline_name = format!("Consumer #1 for Uni '{}'", self.uni_channel.name());
        let on_close_callback = Some(on_close_callback);
        // TODO: 2023-06-14: create as many executors as `self.uni_channel.MAX_STREAMS` (new API there might be needed)
        let in_stream = self.consumer_stream();
        let executor = StreamExecutor::with_futures_timeout(pipeline_name.to_string(), futures_timeout);
        self.stream_executors.push(Arc::clone(&executor));
        let out_stream = pipeline_builder(in_stream);
        let arc_self = Arc::new(self);
        let arc_self_ref = Arc::clone(&arc_self);
        executor
            .spawn_futures_executor(
                concurrency_limit,
                move |executor| {
                    let arc_self = Arc::clone(&arc_self);
                        async move {
                        arc_self.finished_executors_count.fetch_add(1, Relaxed);
                        (on_close_callback.expect("TODO: 2023-06-14: to be called when the last executor ends"))(executor).await;
                    }
                },
                out_stream
            );
        arc_self_ref
    }

    #[must_use]
    pub fn spawn_non_futures_non_fallibles_executor<OutItemType:            Send + Debug,
                                                    OutStreamType:          Stream<Item=OutItemType> + Send + 'static,
                                                    CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                                   (mut self,
                                                    concurrency_limit:        u32,
                                                    pipeline_builder:         impl FnOnce(MutinyStream<'static, ItemType, UniChannelType, DerivedItemType>) -> OutStreamType,
                                                    on_close_callback:        impl FnOnce(Arc<StreamExecutor<INSTRUMENTS>>)                                 -> CloseVoidAsyncType + Send + Sync + 'static)

                                                   -> Arc<Uni<ItemType, UniChannelType, INSTRUMENTS, DerivedItemType>> {

        let pipeline_name = format!("Consumer #1 for Uni '{}'", self.uni_channel.name());
        let on_close_callback = Some(on_close_callback);
        // TODO: 2023-06-14: create as many executors as `self.uni_channel.MAX_STREAMS` (new API there might be needed)
        let in_stream = self.consumer_stream();
        let executor = StreamExecutor::new(pipeline_name.to_string());
        self.stream_executors.push(Arc::clone(&executor));
        let out_stream = pipeline_builder(in_stream);
        let arc_self = Arc::new(self);
        let arc_self_ref = Arc::clone(&arc_self);
        executor
            .spawn_non_futures_non_fallibles_executor(
                concurrency_limit,
                move |executor| {
                    let arc_self = Arc::clone(&arc_self);
                    async move {
                        arc_self.finished_executors_count.fetch_add(1, Relaxed);
                        (on_close_callback.expect("TODO: 2023-06-14: to be called when the last executor ends"))(executor).await;
                    }
                },
                out_stream
            );
        arc_self_ref
    }

}


/// Macro to close, atomically, all [Uni]s passed as parameters
/// TODO the closer may receive a time limit to wait -- returning how many elements were still there after if gave up waiting
/// TODO busy wait ahead -- is it possible to get rid of that without having to poll & sleep?
///      (anyway, this function is not meant to be used in production -- unless when quitting the app... so this is not a priority)
#[macro_export]
macro_rules! unis_close_async {
    ($($producer_handle: tt),+) => {
        let timeout = Duration::ZERO;
        tokio::join!($($producer_handle.uni_channel.gracefully_end_all_streams(timeout), )+);
        loop {
            let mut all_finished = true;
            $(
                if $producer_handle.finished_executors_count.load(Relaxed) == 0 {
                    all_finished = false;
                }
            )+
            if all_finished {
                break
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
}
pub use unis_close_async;

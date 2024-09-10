//! See [super]

use crate::stream_executor::StreamExecutorStats;

use super::super::{
    stream_executor::StreamExecutor,
    mutiny_stream::MutinyStream,
    types::FullDuplexUniChannel,
};
use std::{fmt::Debug, time::Duration, sync::{Arc, atomic::{AtomicU32, Ordering::Relaxed}}};
use std::future::Future;
use std::marker::PhantomData;
use futures::future::BoxFuture;
use futures::Stream;
use tokio::sync::Mutex;


/// Contains the producer-side [Uni] handle used to interact with the `uni` event
/// -- for closing the stream, requiring stats, ...
pub struct Uni<ItemType:          Send + Sync + Debug + 'static,
               UniChannelType:    FullDuplexUniChannel<ItemType=ItemType, DerivedItemType=DerivedItemType> + Send + Sync + 'static,
               const INSTRUMENTS: usize,
               DerivedItemType:   Send + Sync + Debug + 'static = ItemType> {
    pub channel:                  Arc<UniChannelType>,
    pub stream_executors:         Vec<Arc<StreamExecutor<INSTRUMENTS>>>,
    pub finished_executors_count: AtomicU32,
        _phantom:                 PhantomData<(&'static ItemType, &'static DerivedItemType)>,
}

impl<ItemType:          Send + Sync + Debug + 'static,
     UniChannelType:    FullDuplexUniChannel<ItemType=ItemType, DerivedItemType=DerivedItemType> + Send + Sync + 'static,
     const INSTRUMENTS: usize,
     DerivedItemType:   Send + Sync + Debug + 'static>
GenericUni for
Uni<ItemType, UniChannelType, INSTRUMENTS, DerivedItemType> {
    const INSTRUMENTS: usize = INSTRUMENTS;
    type ItemType            = ItemType;
    type UniChannelType      = UniChannelType;
    type DerivedItemType     = DerivedItemType;
    type MutinyStreamType    = MutinyStream<'static, ItemType, UniChannelType, DerivedItemType>;


    fn new<IntoString:Into<String> >(uni_name: IntoString) -> Self {
        Uni {
            channel:                  UniChannelType::new(uni_name),
            stream_executors:         vec![],
            finished_executors_count: AtomicU32::new(0),
            _phantom:                 PhantomData,
        }
    }

    #[inline(always)]
    fn send(&self, item:Self::ItemType) -> keen_retry::RetryConsumerResult<(),Self::ItemType,()>  {
        self.channel.send(item)
    }

    #[inline(always)]
    fn send_with<F:FnOnce(&mut Self::ItemType)>(&self, setter:F) -> keen_retry::RetryConsumerResult<(),F,()>  {
        self.channel.send_with(setter)
    }

    fn consumer_stream(self) -> (Arc<Self> ,Vec<MutinyStream<'static,Self::ItemType,Self::UniChannelType,Self::DerivedItemType> >) {
        let streams = self.consumer_stream_internal();
        let arc_self = Arc::new(self);
        (arc_self, streams)
    }

    #[inline(always)]
    fn pending_items_count(&self) -> u32 {
        self.channel.pending_items_count()
    }

    #[inline(always)]
    fn buffer_size(&self) -> u32 {
        self.channel.buffer_size()
    }

    async fn flush(&self, duration: Duration) -> u32 {
        self.channel.flush(duration).await
    }

    async fn close(&self, timeout: Duration) -> bool {
        self.channel.gracefully_end_all_streams(timeout).await == 0
    }

    fn spawn_executors<OutItemType:        Send + Debug,
                       OutStreamType:      Stream<Item=OutType> + Send + 'static,
                       OutType:            Future<Output=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send,
                       ErrVoidAsyncType:   Future<Output=()> + Send + 'static,
                       CloseVoidAsyncType: Future<Output=()> + Send + 'static>

                      (mut self,
                       concurrency_limit:         u32,
                       futures_timeout:           Duration,
                       pipeline_builder:          impl Fn(MutinyStream<'static, Self::ItemType, Self::UniChannelType, Self::DerivedItemType>) -> OutStreamType,
                       on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>)                                           -> ErrVoidAsyncType   + Send + Sync + 'static,
                       on_close_callback:         impl FnOnce(Arc<dyn StreamExecutorStats + Send + Sync>)                                     -> CloseVoidAsyncType + Send + Sync + 'static)

                      -> Arc<Self> {

        let on_close_callback = Arc::new(latch_callback_1p(UniChannelType::MAX_STREAMS as u32, on_close_callback));
        let on_err_callback = Arc::new(on_err_callback);
        let in_streams = self.consumer_stream_internal();
        for i in 0..=in_streams.len() {
            let pipeline_name = format!("Consumer #{i} for Uni '{}'", self.channel.name());
            let executor = StreamExecutor::<INSTRUMENTS>::with_futures_timeout(pipeline_name, futures_timeout);
            self.stream_executors.push(executor);
        }
        let arc_self = Arc::new(self);
        let arc_self_ref = Arc::clone(&arc_self);
        arc_self.stream_executors.iter().zip(in_streams)
            .for_each(|(executor, in_stream)| {
                let arc_self = Arc::clone(&arc_self);
                let on_close_callback = Arc::clone(&on_close_callback);
                let on_err_callback = Arc::clone(&on_err_callback);
                let out_stream = pipeline_builder(in_stream);
                Arc::clone(executor)
                    .spawn_executor::<_, _, _, _>(
                        concurrency_limit,
                        move |err| on_err_callback(err),
                        move |executor| {
                            async move {
                                arc_self.finished_executors_count.fetch_add(1, Relaxed);
                                on_close_callback(executor).await;
                            }
                        },
                        out_stream
                    );
            });
        arc_self_ref
    }

    fn spawn_fallibles_executors<OutItemType:        Send + Debug,
                                 OutStreamType:      Stream<Item=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
                                 CloseVoidAsyncType: Future<Output=()> + Send + 'static>

                                (mut self,
                                 concurrency_limit:         u32,
                                 pipeline_builder:          impl Fn(MutinyStream<'static, Self::ItemType, Self::UniChannelType, Self::DerivedItemType>) -> OutStreamType,
                                 on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>)                                                                 + Send + Sync + 'static,
                                 on_close_callback:         impl FnOnce(Arc<dyn StreamExecutorStats + Send + Sync>)                                     -> CloseVoidAsyncType + Send + Sync + 'static)

                                -> Arc<Self> {

        let on_close_callback = Arc::new(latch_callback_1p(UniChannelType::MAX_STREAMS as u32, on_close_callback));
        let on_err_callback = Arc::new(on_err_callback);
        let in_streams = self.consumer_stream_internal();
        for i in 0..=in_streams.len() {
            let pipeline_name = format!("Consumer #{i} for Uni '{}'", self.channel.name());
            let executor = StreamExecutor::<INSTRUMENTS>::new(pipeline_name);
            self.stream_executors.push(executor);
        }
        let arc_self = Arc::new(self);
        let arc_self_ref = Arc::clone(&arc_self);
        arc_self.stream_executors.iter().zip(in_streams)
            .for_each(|(executor, in_stream)| {
                let arc_self = Arc::clone(&arc_self);
                let on_close_callback = Arc::clone(&on_close_callback);
                let on_err_callback = Arc::clone(&on_err_callback);
                let out_stream = pipeline_builder(in_stream);
                Arc::clone(executor)
                    .spawn_fallibles_executor::<_, _>(
                        concurrency_limit,
                        move |err| on_err_callback(err),
                        move |executor| {
                            let arc_self = Arc::clone(&arc_self);
                            async move {
                                arc_self.finished_executors_count.fetch_add(1, Relaxed);
                                on_close_callback(executor).await;
                            }
                        },
                        out_stream
                    );
            });
        arc_self_ref
    }

    fn spawn_futures_executors<OutItemType:        Send + Debug,
                               OutStreamType:      Stream<Item=OutType>       + Send + 'static,
                               OutType:            Future<Output=OutItemType> + Send,
                               CloseVoidAsyncType: Future<Output=()>          + Send + 'static>

                              (mut self,
                               concurrency_limit:         u32,
                               futures_timeout:           Duration,
                               pipeline_builder:          impl Fn(MutinyStream<'static, Self::ItemType, Self::UniChannelType, Self::DerivedItemType>) -> OutStreamType,
                               on_close_callback:         impl FnOnce(Arc<dyn StreamExecutorStats + Send + Sync>)                                     -> CloseVoidAsyncType + Send + Sync + 'static)

                              -> Arc<Self> {

        let on_close_callback = Arc::new(latch_callback_1p(UniChannelType::MAX_STREAMS as u32, on_close_callback));
        let in_streams= self.consumer_stream_internal();
        for i in 0..=in_streams.len() {
            let pipeline_name = format!("Consumer #{i} for Uni '{}'", self.channel.name());
            let executor = StreamExecutor::<INSTRUMENTS>::with_futures_timeout(pipeline_name, futures_timeout);
            self.stream_executors.push(executor);
        }
        let arc_self = Arc::new(self);
        let arc_self_ref = Arc::clone(&arc_self);
        arc_self.stream_executors.iter().zip(in_streams)
            .for_each(|(executor, in_stream)| {
                let arc_self = Arc::clone(&arc_self);
                let on_close_callback = Arc::clone(&on_close_callback);
                let out_stream = pipeline_builder(in_stream);
                Arc::clone(executor)
                    .spawn_futures_executor(
                        concurrency_limit,
                        move |executor| {
                            let arc_self = Arc::clone(&arc_self);
                            async move {
                                arc_self.finished_executors_count.fetch_add(1, Relaxed);
                                on_close_callback(executor).await;
                            }
                        },
                        out_stream
                    );
                });
        arc_self_ref
    }

    fn spawn_non_futures_non_fallibles_executors<OutItemType:        Send + Debug,
                                                 OutStreamType:      Stream<Item=OutItemType> + Send + 'static,
                                                 CloseVoidAsyncType: Future<Output=()>        + Send + 'static>

                                                (mut self,
                                                 concurrency_limit:        u32,
                                                 pipeline_builder:         impl Fn(MutinyStream<'static, Self::ItemType, Self::UniChannelType, Self::DerivedItemType>) -> OutStreamType,
                                                 on_close_callback:        impl FnOnce(Arc<dyn StreamExecutorStats + Send + Sync>)                                     -> CloseVoidAsyncType + Send + Sync + 'static)

                                                -> Arc<Self> {

        let on_close_callback = Arc::new(latch_callback_1p(UniChannelType::MAX_STREAMS as u32, on_close_callback));
        let in_streams = self.consumer_stream_internal();
        for i in 0..=in_streams.len() {
            let pipeline_name = format!("Consumer #{i} for Uni '{}'", self.channel.name());
            let executor = StreamExecutor::<INSTRUMENTS>::new(pipeline_name);
            self.stream_executors.push(executor);
        }
        let arc_self = Arc::new(self);
        let arc_self_ref = Arc::clone(&arc_self);
        arc_self.stream_executors.iter().zip(in_streams)
            .for_each(|(executor, in_stream)| {
                let arc_self = Arc::clone(&arc_self);
                let on_close_callback = Arc::clone(&on_close_callback);
                let out_stream = pipeline_builder(in_stream);
                Arc::clone(executor)
                    .spawn_non_futures_non_fallibles_executor(
                        concurrency_limit,
                        move |executor| {
                            let arc_self = Arc::clone(&arc_self);
                            async move {
                                arc_self.finished_executors_count.fetch_add(1, Relaxed);
                                on_close_callback(executor).await;
                            }
                        },
                        out_stream
                    );
            });
        arc_self_ref
    }
}

impl<ItemType:          Send + Sync + Debug + 'static,
     UniChannelType:    FullDuplexUniChannel<ItemType=ItemType, DerivedItemType=DerivedItemType> + Send + Sync + 'static,
     const INSTRUMENTS: usize,
     DerivedItemType:   Send + Sync + Debug + 'static>
Uni<ItemType, UniChannelType, INSTRUMENTS, DerivedItemType> {

    /// similar to [Self::consumer_stream()], but without consuming `self`
    #[must_use]
    fn consumer_stream_internal(&self) -> Vec<MutinyStream<'static, ItemType, UniChannelType, DerivedItemType>> {
        (0..UniChannelType::MAX_STREAMS)
            .map(|_| {
                let (stream, _stream_id) = self.channel.create_stream();
                stream
            })
            .collect()
    }
}


/// This trait exists to allow simplifying generic declarations of concrete [Uni] types.
/// See also [GenericMulti].\
/// Usage:
/// ```nocompile
///     struct MyGenericStruct<T: GenericUni> { the_uni: T }
///     let the_uni = Uni<Lots,And,Lots<Of,Generic,Arguments>>::new();
///     let my_struct = MyGenericStruct { the_uni };
///     // see more at `tests/use_cases.rs`
pub trait GenericUni {
    /// The instruments this Uni will collect/report
    const INSTRUMENTS: usize;
    /// The payload type this Uni's producers will receive
    type ItemType: Send + Sync + Debug + 'static;
    /// The payload type this [Uni]'s `Stream`s will yield
    type DerivedItemType: Send + Sync + Debug + 'static;
    /// The channel through which payloads will travel from producers to consumers (see [Uni] for more info)
    type UniChannelType: FullDuplexUniChannel<ItemType=Self::ItemType, DerivedItemType=Self::DerivedItemType> + Send + Sync + 'static;
    /// Defined as `MutinyStream<'static, ItemType, UniChannelType, DerivedItemType>`,\
    /// the concrete type for the `Stream` of `DerivedItemType`s to be given to consumers
    type MutinyStreamType;

    /// Creates a [Uni], which implements the `consumer pattern`, capable of:
    ///   - creating `Stream`s;
    ///   - applying a user-provided `processor` to the `Stream`s and executing them to depletion --
    ///     the final `Stream`s may produce a combination of fallible/non-fallible &
    ///     futures/non-futures events;
    ///   - producing events that are sent to those `Stream`s.
    /// 
    /// `uni_name` is used for instrumentation purposes, depending on the `INSTRUMENT` generic
    /// argument passed to the [Uni] struct.
    fn new<IntoString: Into<String>>(uni_name: IntoString) -> Self;
    
    #[must_use = "The return type should be examined in case retrying is needed -- or call map(...).into() to transform it into a `Result<(), ItemType>`"]
    fn send(&self, item: Self::ItemType) -> keen_retry::RetryConsumerResult<(), Self::ItemType, ()>;
    
    #[must_use = "The return type should be examined in case retrying is needed -- or call map(...).into() to transform it into a `Result<(), F>`"]
    fn send_with<F: FnOnce(&mut Self::ItemType)>(&self, setter: F) -> keen_retry::RetryConsumerResult<(), F, ()>;
    
    /// Sets this [Uni] to return `Stream`s instead of executing them
    #[must_use = "By calling this method, the Uni gets converted into only providing Streams (rather than executing them) -- so the returned values of (self, Streams) must be used"]
    fn consumer_stream(self) -> (Arc<Self>, Vec<MutinyStream<'static, Self::ItemType, Self::UniChannelType, Self::DerivedItemType>>);

    /// Tells the limit number of events that might be, at any given time, awaiting consumption from the active `Stream`s
    /// -- when exceeded, [Self::send()] & [Self::send_with()] will fail until consumption progresses
    fn buffer_size(&self) -> u32;

    /// Tells how many events (collected by [Self::send()] or [Self::send_with()]) are waiting to be 
    /// consumed by the active `Stream`s
    fn pending_items_count(&self) -> u32;
    
    /// Waits (up to `duration`) until [Self::pending_items_count()] is zero -- possibly waking some tasks awaiting on the active `Stream`s.\
    /// Returns the pending items -- which will be non-zero if `timeout` expired.
    fn flush(&self, timeout: Duration) -> impl Future<Output=u32> + Send;

    /// Closes this Uni, in isolation -- flushing pending events, closing the producers,
    /// waiting for all events to be fully processed and calling the "on close" callback.\
    /// Returns `false` if the timeout kicked-in before it could be attested that the closing was complete.\
    /// If this Uni share resources with another one (which will get dumped by the "on close"
    /// callback), most probably you want to close them atomically -- see [unis_close_async!()]
    #[must_use = "Returns true if the Uni could be closed within the given time"]
    fn close(&self, timeout: Duration) -> impl Future<Output=bool> + Send;
    
    /// Spawns an optimized executor for the `Stream` returned by `pipeline_builder()`, provided it produces elements which are `Future` & fallible
    /// (Actually, as many consumers as `MAX_STREAMS` will be spawned).\
    /// `on_close_callback(stats)` is called when this [Uni] (and all `Stream`s) are closed.\
    /// `on_err_callback(error)` is called whenever the `Stream` returns an `Err` element.
    #[must_use = "`Arc<self>` is returned back, so the return value must be used to send data to this `Uni` and to close it"]
    fn spawn_executors<OutItemType:        Send + Debug,
                       OutStreamType:      Stream<Item=OutType> + Send + 'static,
                       OutType:            Future<Output=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send,
                       ErrVoidAsyncType:   Future<Output=()> + Send + 'static,
                       CloseVoidAsyncType: Future<Output=()> + Send + 'static>

                      (self,
                       concurrency_limit:         u32,
                       futures_timeout:           Duration,
                       pipeline_builder:          impl Fn(MutinyStream<'static, Self::ItemType, Self::UniChannelType, Self::DerivedItemType>) -> OutStreamType,
                       on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>)                                           -> ErrVoidAsyncType   + Send + Sync + 'static,
                       on_close_callback:         impl FnOnce(Arc<dyn StreamExecutorStats + Send + Sync>)                                     -> CloseVoidAsyncType + Send + Sync + 'static)

                      -> Arc<Self>;

    /// Spawns an optimized executor for the `Stream` returned by `pipeline_builder()`, provided it produces elements which are fallible & non-future
    /// (Actually, as many consumers as `MAX_STREAMS` will be spawned).\
    /// `on_close_callback(stats)` is called when this [Uni] (and all `Stream`s) are closed.\
    /// `on_err_callback(error)` is called whenever the `Stream` returns an `Err` element.
    #[must_use = "`Arc<self>` is returned back, so the return value must be used to send data to this `Uni` and to close it"]
    fn spawn_fallibles_executors<OutItemType:        Send + Debug,
                                 OutStreamType:      Stream<Item=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
                                 CloseVoidAsyncType: Future<Output=()> + Send + 'static>

                                (self,
                                 concurrency_limit:         u32,
                                 pipeline_builder:          impl Fn(MutinyStream<'static, Self::ItemType, Self::UniChannelType, Self::DerivedItemType>) -> OutStreamType,
                                 on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>)                                                                 + Send + Sync + 'static,
                                 on_close_callback:         impl FnOnce(Arc<dyn StreamExecutorStats + Send + Sync>)                                     -> CloseVoidAsyncType + Send + Sync + 'static)

                                -> Arc<Self>;
                                    
    /// Spawns an optimized executor for the `Stream` returned by `pipeline_builder()`, provided it produces elements which are `Future` & non-fallible
    /// (Actually, as many consumers as `MAX_STREAMS` will be spawned).\
    /// `on_close_callback(stats)` is called when this [Uni] (and all `Stream`s) are closed.
    #[must_use = "`Arc<self>` is returned back, so the return value must be used to send data to this `Uni` and to close it"]
    fn spawn_futures_executors<OutItemType:        Send + Debug,
                               OutStreamType:      Stream<Item=OutType>       + Send + 'static,
                               OutType:            Future<Output=OutItemType> + Send,
                               CloseVoidAsyncType: Future<Output=()>          + Send + 'static>

                              (self,
                               concurrency_limit:         u32,
                               futures_timeout:           Duration,
                               pipeline_builder:          impl Fn(MutinyStream<'static, Self::ItemType, Self::UniChannelType, Self::DerivedItemType>) -> OutStreamType,
                               on_close_callback:         impl FnOnce(Arc<dyn StreamExecutorStats + Send + Sync>)                                     -> CloseVoidAsyncType + Send + Sync + 'static)

                              -> Arc<Self>;
                                  
    /// Spawns an optimized executor for the `Stream` returned by `pipeline_builder()`, provided it produces elements which are non-future & non-fallible
    /// (Actually, as many consumers as `MAX_STREAMS` will be spawned).\
    /// `on_close_callback(stats)` is called when this [Uni] (and all `Stream`s) are closed.
    #[must_use = "`Arc<self>` is returned back, so the return value must be used to send data to this `Uni` and to close it"]
    fn spawn_non_futures_non_fallibles_executors<OutItemType:        Send + Debug,
                                                 OutStreamType:      Stream<Item=OutItemType> + Send + 'static,
                                                 CloseVoidAsyncType: Future<Output=()>        + Send + 'static>

                                                (self,
                                                 concurrency_limit:        u32,
                                                 pipeline_builder:         impl Fn(MutinyStream<'static, Self::ItemType, Self::UniChannelType, Self::DerivedItemType>) -> OutStreamType,
                                                 on_close_callback:        impl FnOnce(Arc<dyn StreamExecutorStats + Send + Sync>)                                     -> CloseVoidAsyncType + Send + Sync + 'static)

                                                -> Arc<Self>;
}


/// Macro to close, atomically-ish, all [Uni]s passed as parameters
#[macro_export]
macro_rules! unis_close_async {
    ($timeout: expr,
     $($uni: tt),+) => {
        {
            tokio::join!( $( $uni.channel.flush($timeout), )+ );
            tokio::join!( $( $uni.channel.gracefully_end_all_streams($timeout), )+);
        }
    }
}
pub use unis_close_async;


/// returns a closure (receiving 1 parameter) that must be called `latch_count` times before calling `callback(1 parameter)`
fn latch_callback_1p<CallbackParameterType: Send + 'static,
                     CallbackAsyncType:     Send + Future<Output=()>>
                    (latch_count:    u32,
                     async_callback: impl FnOnce(CallbackParameterType) -> CallbackAsyncType + Send + Sync + 'static)
                    -> impl Fn(CallbackParameterType) -> BoxFuture<'static, ()> {
    let async_callback = Arc::new(Mutex::new(Some(async_callback)));
    let latch_counter = Arc::new(AtomicU32::new(latch_count));
    move |p1| {
        let async_callback = Arc::clone(&async_callback);
        let latch_counter = Arc::clone(&latch_counter);
        Box::pin(async move {
            if latch_counter.fetch_sub(1, Relaxed) == 1 {
                let mut async_callback = async_callback.lock().await;
                (async_callback.take().expect("Uni::latch_callback_1p(): BUG! FnOnce() not honored by the algorithm"))(p1).await;
            }
        })
    }
}

//! See [super]

use super::super::{
    stream_executor::StreamExecutor,
    mutiny_stream::MutinyStream,
    types::FullDuplexMultiChannel,
};
use std::{
    sync::Arc,
    fmt::Debug,
    time::Duration,
    future::Future,
    marker::PhantomData,
};
use indexmap::IndexMap;
use futures::Stream;
use tokio::sync::RwLock;


/// `Multi` is an event handler capable of having several "listeners" -- all of which receives all events.\
/// With this struct, it is possible to:
///   - produce events
///   - spawn new `Stream`s & executors
///   - close `Stream`s (and executors)
/// Example:
/// ```nocompile
/// {reactive_mutiny::Instruments::MetricsWithoutLogs.into()}
pub struct Multi<ItemType:          Debug + Sync + Send + 'static,
                 MultiChannelType:  FullDuplexMultiChannel<'static, ItemType, DerivedItemType> + Sync + Send,
                 const INSTRUMENTS: usize,
                 DerivedItemType:   Debug + Sync + Send + 'static> {
    pub multi_name:     String,
    pub channel:        Arc<MultiChannelType>,
    pub executor_infos: RwLock<IndexMap<String, ExecutorInfo>>,
        _phantom:       PhantomData<(ItemType, MultiChannelType, DerivedItemType)>,
}

impl<ItemType:          Debug + Send + Sync + 'static,
     MultiChannelType:  FullDuplexMultiChannel<'static, ItemType, DerivedItemType> + Sync + Send + 'static,
     const INSTRUMENTS: usize,
     DerivedItemType:   Debug + Sync + Send + 'static>
MultiGenericTypes for
Multi<ItemType, MultiChannelType, INSTRUMENTS, DerivedItemType> {
    type ItemType = ItemType;
    type MultiChannelType = MultiChannelType;
    type DerivedItemType = DerivedItemType;
    type MutinyStreamType = MutinyStream<'static, ItemType, MultiChannelType, DerivedItemType>;
}

impl<ItemType:          Debug + Send + Sync + 'static,
     MultiChannelType:  FullDuplexMultiChannel<'static, ItemType, DerivedItemType> + Sync + Send + 'static,
     const INSTRUMENTS: usize,
     DerivedItemType:   Debug + Sync + Send + 'static>
Multi<ItemType, MultiChannelType, INSTRUMENTS, DerivedItemType> {

    pub fn new<IntoString: Into<String>>(multi_name: IntoString) -> Self {
        let multi_name = multi_name.into();
        Multi {
            multi_name:     multi_name.clone(),
            channel:        MultiChannelType::new(multi_name.clone()),
            executor_infos: RwLock::new(IndexMap::new()),
            _phantom:       PhantomData::default(),
        }
    }

    pub fn stream_name(self: &Self) -> &str {
        &self.multi_name
    }

    #[inline(always)]
    #[must_use = "The return type should be examined in case retrying is needed -- or call map(...).into() to transform it into a `Result<(), ItemType>`"]
    pub fn send(&self, item: ItemType) -> keen_retry::RetryConsumerResult<(), ItemType, ()> {
        self.channel.send(item)
    }

    #[inline(always)]
    #[must_use = "The return type should be examined in case retrying is needed -- or call map(...).into() to transform it into a `Result<(), F>`"]
    pub fn send_with<F: FnOnce(&mut ItemType)>(&self, setter: F) -> keen_retry::RetryConsumerResult<(), F, ()> {
        self.channel.send_with(setter)
    }

    #[inline(always)]
    #[must_use = "The return type should be examined in case retrying is needed"]
    pub fn send_derived(&self, arc_item: &DerivedItemType) -> bool {
        self.channel.send_derived(arc_item)
    }

    #[inline(always)]
    #[must_use]
    pub fn buffer_size(&self) -> u32 {
        self.channel.buffer_size()
    }

    #[inline(always)]
    #[must_use]
    pub fn pending_items_count(&self) -> u32 {
        self.channel.pending_items_count()
    }

    /// Spawns a new listener of all subsequent events sent to this `Multi`, processing them through the `Stream` returned by `pipeline_builder()`,
    /// which generates events that are Futures & Fallible.
    pub async fn spawn_executor<IntoString:             Into<String>,
                                OutItemType:            Send + Debug,
                                OutStreamType:          Stream<Item=OutType> + Send + 'static,
                                OutType:                Future<Output=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send,
                                ErrVoidAsyncType:       Future<Output=()> + Send + 'static,
                                CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                               (&self,
                                concurrency_limit:         u32,
                                futures_timeout:           Duration,
                                pipeline_name:             IntoString,
                                pipeline_builder:          impl FnOnce(MutinyStream<'static, ItemType, MultiChannelType, DerivedItemType>) -> OutStreamType,
                                on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>)                               -> ErrVoidAsyncType   + Send + Sync + 'static,
                                on_close_callback:         impl FnOnce(Arc<StreamExecutor>)                                                -> CloseVoidAsyncType + Send + Sync + 'static)

                               -> Result<(), Box<dyn std::error::Error>> {

        let (in_stream, in_stream_id) = self.channel.create_stream_for_new_events();
        let out_stream = pipeline_builder(in_stream);
        self.spawn_executor_from_stream(concurrency_limit, futures_timeout, pipeline_name, in_stream_id, out_stream, on_err_callback, on_close_callback).await
    }

    /// For channels that allow it (like [channels::reference::mmap_log::MmapLog]), spawns two listeners for events sent to this `Multi`:
    ///   1) One for past events -- to be processed by the stream returned by `oldies_pipeline_builder()`;
    ///   2) Another one for subsequent events -- to be processed by the stream returned by `newies_pipeline_builder()`.
    /// By using this method, it is assumed that both pipeline builders returns `Future<Result>` events. If this is not so, see one of the sibling methods.\
    /// The stream splitting is guaranteed not to drop any events and `sequential_transition` may be used to indicate if old events should be processed first or if both old and new events
    /// may be processed simultaneously (in an inevitable out-of-order fashion).
    pub async fn spawn_oldies_executor<IntoString:               Into<String>,
                                       OutItemType:              Send + Debug,
                                       OldiesOutStreamType:      Stream<Item=OldiesOutType> + Sync + Send + 'static,
                                       NewiesOutStreamType:      Stream<Item=NewiesOutType> + Sync + Send + 'static,
                                       OldiesOutType:            Future<Output=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send,
                                       NewiesOutType:            Future<Output=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send,
                                       ErrVoidAsyncType:         Future<Output=()> + Send + 'static,
                                       OldiesCloseVoidAsyncType: Future<Output=()> + Send + 'static,
                                       NewiesCloseVoidAsyncType: Future<Output=()> + Send + 'static>

                                      (self:                      &Arc<Self>,
                                       concurrency_limit:         u32,
                                       sequential_transition:     bool,
                                       futures_timeout:           Duration,
                                       oldies_pipeline_name:      IntoString,
                                       oldies_pipeline_builder:   impl FnOnce(MutinyStream<'static, ItemType, MultiChannelType, DerivedItemType>) -> OldiesOutStreamType,
                                       oldies_on_close_callback:  impl FnOnce(Arc<StreamExecutor>)                                                -> OldiesCloseVoidAsyncType + Send + Sync + 'static,
                                       newies_pipeline_name:      IntoString,
                                       newies_pipeline_builder:   impl FnOnce(MutinyStream<'static, ItemType, MultiChannelType, DerivedItemType>) -> NewiesOutStreamType      + Send + Sync + 'static,
                                       newies_on_close_callback:  impl FnOnce(Arc<StreamExecutor>)                                                -> NewiesCloseVoidAsyncType + Send + Sync + 'static,
                                       on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>)                               -> ErrVoidAsyncType         + Send + Sync + 'static)

                                      -> Result<(), Box<dyn std::error::Error>> {

        let ((oldies_in_stream, oldies_in_stream_id),
             (newies_in_stream, newies_in_stream_id)) = self.channel.create_streams_for_old_and_new_events();

        let cloned_self = Arc::clone(&self);
        let oldies_pipeline_name = oldies_pipeline_name.into();
        let newies_pipeline_name = Arc::new(newies_pipeline_name.into());
        let on_err_callback_ref1 = Arc::new(on_err_callback);
        let on_err_callback_ref2 = Arc::clone(&on_err_callback_ref1);
        let oldies_out_stream = oldies_pipeline_builder(oldies_in_stream);
        let newies_out_stream = newies_pipeline_builder(newies_in_stream);

        match sequential_transition {
            true => {
                self.spawn_executor_from_stream(concurrency_limit, futures_timeout, oldies_pipeline_name, oldies_in_stream_id, oldies_out_stream,
                                                move |err| on_err_callback_ref1(err),
                                                move |executor| {
                                                    let cloned_self = Arc::clone(&cloned_self);
                                                    let on_err_callback_ref2 = Arc::clone(&on_err_callback_ref2);
                                                    let newies_pipeline_name = Arc::clone(&newies_pipeline_name);
                                                    async move {
                                                        cloned_self.spawn_executor_from_stream(concurrency_limit, futures_timeout, newies_pipeline_name.as_str(), newies_in_stream_id, newies_out_stream,
                                                                                              move |err| on_err_callback_ref2(err),
                                                                                              move |executor| newies_on_close_callback(executor)).await
                                                            .map_err(|err| format!("Multi::spawn_oldies_executor(): could not start `newies`/sequential executor: {:?}", err))
                                                            .expect("CANNOT SPAWN NEWIES EXECUTOR AFTER OLDIES HAD COMPLETE");
                                                        oldies_on_close_callback(executor).await;
                                                    }
                                                } ).await
                    .map_err(|err| format!("Multi::spawn_oldies_executor(): could not start `oldies`/sequential executor: {:?}", err))?;
            },
            false => {
                self.spawn_executor_from_stream(concurrency_limit, futures_timeout, oldies_pipeline_name, oldies_in_stream_id, oldies_out_stream,
                                                move |err| on_err_callback_ref1(err),
                                                move |executor| oldies_on_close_callback(executor)).await
                    .map_err(|err| format!("Multi::spawn_oldies_executor(): could not start `oldies` executor: {:?}", err))?;
                self.spawn_executor_from_stream(concurrency_limit, futures_timeout, newies_pipeline_name.as_str(), newies_in_stream_id, newies_out_stream,
                                                move |err| on_err_callback_ref2(err),
                                                move |executor| newies_on_close_callback(executor)).await
                    .map_err(|err| format!("Multi::spawn_oldies_executor(): could not start `newies` executor: {:?}", err))?;
            },
        }
        Ok(())
    }

    /// Internal method with common code for [Self::spawn_executor()] & [Self::spawn_oldies_executor()].
    async fn spawn_executor_from_stream<IntoString:             Into<String>,
                                        OutItemType:            Send + Debug,
                                        OutStreamType:          Stream<Item=OutType> + Send + 'static,
                                        OutType:                Future<Output=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send,
                                        ErrVoidAsyncType:       Future<Output=()> + Send + 'static,
                                        CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                       (&self,
                                        concurrency_limit:         u32,
                                        futures_timeout:           Duration,
                                        pipeline_name:             IntoString,
                                        stream_id:                 u32,
                                        pipelined_stream:          OutStreamType,
                                        on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>) -> ErrVoidAsyncType   + Send + Sync + 'static,
                                        on_close_callback:         impl FnOnce(Arc<StreamExecutor>)                  -> CloseVoidAsyncType + Send + Sync + 'static)

                                       -> Result<(), Box<dyn std::error::Error>> {

        let executor = StreamExecutor::with_futures_timeout(format!("{}: {}", self.stream_name(), pipeline_name.into()), futures_timeout);
        self.add_executor(Arc::clone(&executor), stream_id).await?;
        executor
            .spawn_executor::<INSTRUMENTS, _, _, _, _>(
                concurrency_limit,
                on_err_callback,
                move |executor| on_close_callback(executor),
                pipelined_stream
            );
        Ok(())
    }

    /// Spawns a new listener of all subsequent events sent to this `Multi`, processing them through the `Stream` returned by `pipeline_builder()`,
    /// which generates events that are Futures.
    pub async fn spawn_futures_executor<IntoString:             Into<String>,
                                        OutItemType:            Send + Debug,
                                        OutStreamType:          Stream<Item=OutType> + Send + 'static,
                                        OutType:                Future<Output=OutItemType> + Send,
                                        CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                       (&self,
                                        concurrency_limit:         u32,
                                        futures_timeout:           Duration,
                                        pipeline_name:             IntoString,
                                        pipeline_builder:          impl FnOnce(MutinyStream<'static, ItemType, MultiChannelType, DerivedItemType>) -> OutStreamType,
                                        on_close_callback:         impl FnOnce(Arc<StreamExecutor>)                                                -> CloseVoidAsyncType + Send + Sync + 'static)

                                       -> Result<(), Box<dyn std::error::Error>> {

        let (in_stream, in_stream_id) = self.channel.create_stream_for_new_events();
        let out_stream = pipeline_builder(in_stream);
        self.spawn_futures_executor_from_stream(concurrency_limit, futures_timeout, pipeline_name, in_stream_id, out_stream, on_close_callback).await
    }

    /// For channels that allow it (like [channels::reference::mmap_log::MmapLog]), spawns two listeners for events sent to this `Multi`:
    ///   1) One for past events -- to be processed by the stream returned by `oldies_pipeline_builder()`;
    ///   2) Another one for subsequent events -- to be processed by the stream returned by `newies_pipeline_builder()`.
    /// By using this method, it is assumed that both pipeline builders returns `Future` events. If this is not so, see one of the sibling methods.\
    /// The stream splitting is guaranteed not to drop any events and `sequential_transition` may be used to indicate if old events should be processed first or if both old and new events
    /// may be processed simultaneously (in an inevitable out-of-order fashion).
    pub async fn spawn_futures_oldies_executor<IntoString:               Into<String>,
                                               OutItemType:              Send + Debug,
                                               OldiesOutStreamType:      Stream<Item=OldiesOutType> + Sync + Send + 'static,
                                               NewiesOutStreamType:      Stream<Item=NewiesOutType> + Sync + Send + 'static,
                                               OldiesOutType:            Future<Output=OutItemType> + Send,
                                               NewiesOutType:            Future<Output=OutItemType> + Send,
                                               OldiesCloseVoidAsyncType: Future<Output=()> + Send + 'static,
                                               NewiesCloseVoidAsyncType: Future<Output=()> + Send + 'static>

                                              (self:                      &Arc<Self>,
                                               concurrency_limit:         u32,
                                               sequential_transition:     bool,
                                               futures_timeout:           Duration,
                                               oldies_pipeline_name:      IntoString,
                                               oldies_pipeline_builder:   impl FnOnce(MutinyStream<'static, ItemType, MultiChannelType, DerivedItemType>) -> OldiesOutStreamType,
                                               oldies_on_close_callback:  impl FnOnce(Arc<StreamExecutor>)                                                -> OldiesCloseVoidAsyncType + Send + Sync + 'static,
                                               newies_pipeline_name:      IntoString,
                                               newies_pipeline_builder:   impl FnOnce(MutinyStream<'static, ItemType, MultiChannelType, DerivedItemType>) -> NewiesOutStreamType       + Send + Sync + 'static,
                                               newies_on_close_callback:  impl FnOnce(Arc<StreamExecutor>)                                                -> NewiesCloseVoidAsyncType  + Send + Sync + 'static)

                                              -> Result<(), Box<dyn std::error::Error>> {

        let ((oldies_in_stream, oldies_in_stream_id),
             (newies_in_stream, newies_in_stream_id)) = self.channel.create_streams_for_old_and_new_events();

        let cloned_self = Arc::clone(&self);
        let oldies_pipeline_name = oldies_pipeline_name.into();
        let newies_pipeline_name = Arc::new(newies_pipeline_name.into());
        let oldies_out_stream = oldies_pipeline_builder(oldies_in_stream);
        let newies_out_stream = newies_pipeline_builder(newies_in_stream);

        match sequential_transition {
            true => {
                self.spawn_futures_executor_from_stream(concurrency_limit, futures_timeout, oldies_pipeline_name, oldies_in_stream_id, oldies_out_stream,
                                                        move |executor| {
                                                            let cloned_self = Arc::clone(&cloned_self);
                                                            let newies_pipeline_name = Arc::clone(&newies_pipeline_name);
                                                            async move {
                                                                cloned_self.spawn_futures_executor_from_stream(concurrency_limit, futures_timeout, newies_pipeline_name.as_str(), newies_in_stream_id, newies_out_stream,
                                                                                                              move |executor| newies_on_close_callback(executor)).await
                                                                    .map_err(|err| format!("Multi::spawn_oldies_executor(): could not start `newies`/sequential executor: {:?}", err))
                                                                    .expect("CANNOT SPAWN NEWIES EXECUTOR AFTER OLDIES HAD COMPLETE");
                                                                oldies_on_close_callback(executor).await;
                                                            }
                                                        } ).await
                    .map_err(|err| format!("Multi::spawn_oldies_executor(): could not start `oldies`/sequential executor: {:?}", err))?;
            },
            false => {
                self.spawn_futures_executor_from_stream(concurrency_limit, futures_timeout, oldies_pipeline_name, oldies_in_stream_id, oldies_out_stream,
                                                        move |executor| oldies_on_close_callback(executor)).await
                    .map_err(|err| format!("Multi::spawn_oldies_executor(): could not start `oldies` executor: {:?}", err))?;
                self.spawn_futures_executor_from_stream(concurrency_limit, futures_timeout, newies_pipeline_name.as_str(), newies_in_stream_id, newies_out_stream,
                                                        move |executor| newies_on_close_callback(executor)).await
                    .map_err(|err| format!("Multi::spawn_oldies_executor(): could not start `newies` executor: {:?}", err))?;
            },
        }
        Ok(())
    }

    /// Internal method with common code for [Self::spawn_futures_executor()] & [Self::spawn_futures_oldies_executor()].
    async fn spawn_futures_executor_from_stream<IntoString:             Into<String>,
                                                OutItemType:            Send + Debug,
                                                OutStreamType:          Stream<Item=OutType> + Send + 'static,
                                                OutType:                Future<Output=OutItemType> + Send,
                                                CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                               (&self,
                                                concurrency_limit:         u32,
                                                futures_timeout:           Duration,
                                                pipeline_name:             IntoString,
                                                stream_id:                 u32,
                                                pipelined_stream:          OutStreamType,
                                                on_close_callback:         impl FnOnce(Arc<StreamExecutor>) -> CloseVoidAsyncType + Send + Sync + 'static)

                                               -> Result<(), Box<dyn std::error::Error>> {

        let executor = StreamExecutor::with_futures_timeout(format!("{}: {}", self.stream_name(), pipeline_name.into()), futures_timeout);
        self.add_executor(Arc::clone(&executor), stream_id).await?;
        executor
            .spawn_futures_executor::<INSTRUMENTS, _, _, _>(
                concurrency_limit,
                move |executor| on_close_callback(executor),
                pipelined_stream
            );
        Ok(())
    }

    /// Spawns a new listener of all subsequent events sent to this `Multi`, processing them through the `Stream` returned by `pipeline_builder()`,
    /// which generates events that are Fallible.
    pub async fn spawn_fallibles_executor<IntoString:             Into<String>,
                                          OutItemType:            Send + Debug,
                                          OutStreamType:          Stream<Item=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
                                          CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                         (&self,
                                          concurrency_limit:         u32,
                                          pipeline_name:             IntoString,
                                          pipeline_builder:          impl FnOnce(MutinyStream<'static, ItemType, MultiChannelType, DerivedItemType>) -> OutStreamType,
                                          on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>)                                                     + Send + Sync + 'static,
                                          on_close_callback:         impl FnOnce(Arc<StreamExecutor>)                                                -> CloseVoidAsyncType + Send + Sync + 'static)

                                         -> Result<(), Box<dyn std::error::Error>> {

        let (in_stream, in_stream_id) = self.channel.create_stream_for_new_events();
        let out_stream = pipeline_builder(in_stream);
        self.spawn_fallibles_executor_from_stream(concurrency_limit, pipeline_name, in_stream_id, out_stream, on_err_callback, on_close_callback).await
    }

    /// For channels that allow it (like [channels::reference::mmap_log::MmapLog]), spawns two listeners for events sent to this `Multi`:
    ///   1) One for past events -- to be processed by the stream returned by `oldies_pipeline_builder()`;
    ///   2) Another one for subsequent events -- to be processed by the stream returned by `newies_pipeline_builder()`.
    /// By using this method, it is assumed that both pipeline builders returns Fallible events. If this is not so, see one of the sibling methods.\
    /// The stream splitting is guaranteed not to drop any events and `sequential_transition` may be used to indicate if old events should be processed first or if both old and new events
    /// may be processed simultaneously (in an inevitable out-of-order fashion).
    pub async fn spawn_fallibles_oldies_executor<IntoString:               Into<String>,
                                                 OutItemType:              Send + Debug,
                                                 OldiesOutStreamType:      Stream<Item=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Sync + Send + 'static,
                                                 NewiesOutStreamType:      Stream<Item=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Sync + Send + 'static,
                                                 OldiesCloseVoidAsyncType: Future<Output=()> + Send + 'static,
                                                 NewiesCloseVoidAsyncType: Future<Output=()> + Send + 'static>

                                                (self:                      &Arc<Self>,
                                                 concurrency_limit:         u32,
                                                 sequential_transition:     bool,
                                                 oldies_pipeline_name:      IntoString,
                                                 oldies_pipeline_builder:   impl FnOnce(MutinyStream<'static, ItemType, MultiChannelType, DerivedItemType>) -> OldiesOutStreamType,
                                                 oldies_on_close_callback:  impl FnOnce(Arc<StreamExecutor>)                                                -> OldiesCloseVoidAsyncType + Send + Sync + 'static,
                                                 newies_pipeline_name:      IntoString,
                                                 newies_pipeline_builder:   impl FnOnce(MutinyStream<'static, ItemType, MultiChannelType, DerivedItemType>) -> NewiesOutStreamType      + Send + Sync + 'static,
                                                 newies_on_close_callback:  impl FnOnce(Arc<StreamExecutor>)                                                -> NewiesCloseVoidAsyncType + Send + Sync + 'static,
                                                 on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>)                                                           + Send + Sync + 'static)

                                                -> Result<(), Box<dyn std::error::Error>> {

        let ((oldies_in_stream, oldies_in_stream_id),
             (newies_in_stream, newies_in_stream_id)) = self.channel.create_streams_for_old_and_new_events();

        let cloned_self = Arc::clone(&self);
        let oldies_pipeline_name = oldies_pipeline_name.into();
        let newies_pipeline_name = Arc::new(newies_pipeline_name.into());
        let on_err_callback_ref1 = Arc::new(on_err_callback);
        let on_err_callback_ref2 = Arc::clone(&on_err_callback_ref1);
        let oldies_out_stream = oldies_pipeline_builder(oldies_in_stream);
        let newies_out_stream = newies_pipeline_builder(newies_in_stream);

        match sequential_transition {
            true => {
                self.spawn_fallibles_executor_from_stream(concurrency_limit, oldies_pipeline_name, oldies_in_stream_id, oldies_out_stream,
                                                         move |err| on_err_callback_ref1(err),
                                                         move |executor| {
                                                             let cloned_self = Arc::clone(&cloned_self);
                                                             let on_err_callback_ref2 = Arc::clone(&on_err_callback_ref2);
                                                             let newies_pipeline_name = Arc::clone(&newies_pipeline_name);
                                                             async move {
                                                                 cloned_self.spawn_fallibles_executor_from_stream(concurrency_limit, newies_pipeline_name.as_str(), newies_in_stream_id, newies_out_stream,
                                                                                                                  move |err| on_err_callback_ref2(err),
                                                                                                                  move |executor| newies_on_close_callback(executor)).await
                                                                     .map_err(|err| format!("Multi::spawn_oldies_executor(): could not start `newies`/sequential executor: {:?}", err))
                                                                     .expect("CANNOT SPAWN NEWIES EXECUTOR AFTER OLDIES HAD COMPLETE");
                                                                 oldies_on_close_callback(executor).await;
                                                             }
                                                         } ).await
                    .map_err(|err| format!("Multi::spawn_oldies_executor(): could not start `oldies`/sequential executor: {:?}", err))?;
            },
            false => {
                self.spawn_fallibles_executor_from_stream(concurrency_limit, oldies_pipeline_name, oldies_in_stream_id, oldies_out_stream,
                                                         move |err| on_err_callback_ref1(err),
                                                          move |executor| oldies_on_close_callback(executor)).await
                    .map_err(|err| format!("Multi::spawn_oldies_executor(): could not start `oldies` executor: {:?}", err))?;
                self.spawn_fallibles_executor_from_stream(concurrency_limit, newies_pipeline_name.as_str(), newies_in_stream_id, newies_out_stream,
                                                          move |err| on_err_callback_ref2(err),
                                                          move |executor| newies_on_close_callback(executor)).await
                    .map_err(|err| format!("Multi::spawn_oldies_executor(): could not start `newies` executor: {:?}", err))?;
            },
        }
        Ok(())
    }

    /// Internal method with common code for [Self::spawn_fallibles_executor()] & [Self::spawn_oldies_fallibles_executor()].
    async fn spawn_fallibles_executor_from_stream<IntoString:             Into<String>,
                                                  OutItemType:            Send + Debug,
                                                  OutStreamType:          Stream<Item=Result<OutItemType, Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
                                                  CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                       (&self,
                                        concurrency_limit:         u32,
                                        pipeline_name:             IntoString,
                                        stream_id:                 u32,
                                        pipelined_stream:          OutStreamType,
                                        on_err_callback:           impl Fn(Box<dyn std::error::Error + Send + Sync>)      + Send + Sync + 'static,
                                        on_close_callback:         impl FnOnce(Arc<StreamExecutor>) -> CloseVoidAsyncType + Send + Sync + 'static)

                                       -> Result<(), Box<dyn std::error::Error>> {

        let executor = StreamExecutor::new(format!("{}: {}", self.stream_name(), pipeline_name.into()));
        self.add_executor(Arc::clone(&executor), stream_id).await?;
        executor
            .spawn_fallibles_executor::<INSTRUMENTS, _, _>(
                concurrency_limit,
                on_err_callback,
                move |executor| on_close_callback(executor),
                pipelined_stream
            );
        Ok(())
    }

    /// Spawns a new listener of all subsequent events sent to this `Multi`, processing them through the `Stream` returned by `pipeline_builder()`,
    /// which generates events that are Non-Futures & Non-Fallible.
    pub async fn spawn_non_futures_non_fallible_executor<IntoString:             Into<String>,
                                                         OutItemType:            Send + Debug,
                                                         OutStreamType:          Stream<Item=OutItemType> + Send + 'static,
                                                         CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                                        (&self,
                                                         concurrency_limit:        u32,
                                                         pipeline_name:            IntoString,
                                                         pipeline_builder:         impl FnOnce(MutinyStream<'static, ItemType, MultiChannelType, DerivedItemType>) -> OutStreamType,
                                                         on_close_callback:        impl FnOnce(Arc<StreamExecutor>)                                                -> CloseVoidAsyncType + Send + Sync + 'static)

                                                        -> Result<(), Box<dyn std::error::Error>> {

        let (in_stream, in_stream_id) = self.channel.create_stream_for_new_events();
        let out_stream = pipeline_builder(in_stream);
        self.spawn_non_futures_non_fallible_executor_from_stream(concurrency_limit, pipeline_name, in_stream_id, out_stream, on_close_callback).await
    }

    /// For channels that allow it (like [channels::reference::mmap_log::MmapLog]), spawns two listeners for events sent to this `Multi`:
    ///   1) One for past events -- to be processed by the stream returned by `oldies_pipeline_builder()`;
    ///   2) Another one for subsequent events -- to be processed by the stream returned by `newies_pipeline_builder()`.
    /// By using this method, it is assumed that both pipeline builders returns non-Futures & non-Fallible events. If this is not so, see [spawn_oldies_executor].\
    /// The stream splitting is guaranteed not to drop any events and `sequential_transition` may be used to indicate if old events should be processed first or if both old and new events
    /// may be processed simultaneously (in an inevitable out-of-order fashion).
    pub async fn spawn_non_futures_non_fallible_oldies_executor<IntoString:               Into<String>,
                                                                OldiesOutItemType:        Send + Debug,
                                                                NewiesOutItemType:        Send + Debug,
                                                                OldiesOutStreamType:      Stream<Item=OldiesOutItemType> + Sync + Send + 'static,
                                                                NewiesOutStreamType:      Stream<Item=NewiesOutItemType> + Sync + Send + 'static,
                                                                OldiesCloseVoidAsyncType: Future<Output=()> + Send + 'static,
                                                                NewiesCloseVoidAsyncType: Future<Output=()> + Send + 'static>

                                                               (self:                     &Arc<Self>,
                                                                concurrency_limit:        u32,
                                                                sequential_transition:    bool,
                                                                oldies_pipeline_name:     IntoString,
                                                                oldies_pipeline_builder:  impl FnOnce(MutinyStream<'static, ItemType, MultiChannelType, DerivedItemType>) -> OldiesOutStreamType,
                                                                oldies_on_close_callback: impl FnOnce(Arc<StreamExecutor>)                                                -> OldiesCloseVoidAsyncType + Send + Sync + 'static,
                                                                newies_pipeline_name:     IntoString,
                                                                newies_pipeline_builder:  impl FnOnce(MutinyStream<'static, ItemType, MultiChannelType, DerivedItemType>) -> NewiesOutStreamType,
                                                                newies_on_close_callback: impl FnOnce(Arc<StreamExecutor>)                                                -> NewiesCloseVoidAsyncType + Send + Sync + 'static)

                                                               -> Result<(), Box<dyn std::error::Error>> {

        let ((oldies_in_stream, oldies_in_stream_id),
             (newies_in_stream, newies_in_stream_id)) = self.channel.create_streams_for_old_and_new_events();

        let cloned_self = Arc::clone(&self);
        let oldies_pipeline_name = oldies_pipeline_name.into();
        let newies_pipeline_name = Arc::new(newies_pipeline_name.into());
        let oldies_out_stream = oldies_pipeline_builder(oldies_in_stream);
        let newies_out_stream = newies_pipeline_builder(newies_in_stream);

        match sequential_transition {
            true => {
                self.spawn_non_futures_non_fallible_executor_from_stream(concurrency_limit, oldies_pipeline_name, oldies_in_stream_id, oldies_out_stream,
                                                                         move |executor| {
                                                                             let cloned_self = Arc::clone(&cloned_self);
                                                                             let newies_pipeline_name = Arc::clone(&newies_pipeline_name);
                                                                             async move {
                                                                                 cloned_self.spawn_non_futures_non_fallible_executor_from_stream(concurrency_limit, newies_pipeline_name.as_str(), newies_in_stream_id, newies_out_stream,
                                                                                                                                                move |executor| newies_on_close_callback(executor)).await
                                                                                     .map_err(|err| format!("Multi::spawn_non_futures_non_fallible_oldies_executor(): could not start `newies` executor: {:?}", err))
                                                                                     .expect("CANNOT SPAWN NEWIES EXECUTOR AFTER OLDIES HAD COMPLETE");
                                                                                 oldies_on_close_callback(executor).await;
                                                                             }
                                                                         }).await
                    .map_err(|err| format!("Multi::spawn_non_futures_non_fallible_oldies_executor(): could not start `oldies`/sequential executor: {:?}", err))?;

            },
            false => {
                self.spawn_non_futures_non_fallible_executor_from_stream(concurrency_limit, oldies_pipeline_name, oldies_in_stream_id, oldies_out_stream,
                                                          move |executor| oldies_on_close_callback(executor)).await
                    .map_err(|err| format!("Multi::spawn_non_futures_non_fallible_oldies_executor(): could not start `oldies` executor: {:?}", err))?;
                self.spawn_non_futures_non_fallible_executor_from_stream(concurrency_limit, newies_pipeline_name.as_str(), newies_in_stream_id, newies_out_stream,
                                                                         move |executor| newies_on_close_callback(executor)).await
                    .map_err(|err| format!("Multi::spawn_non_futures_non_fallible_oldies_executor(): could not start `newies` executor: {:?}", err))?;
            },
        }
        Ok(())
    }

    /// Internal method with common code for [spawn_non_futures_non_fallible_executor()] & [spawn_non_futures_non_fallible_oldies_executor()].
    async fn spawn_non_futures_non_fallible_executor_from_stream<IntoString:             Into<String>,
                                                                 OutItemType:            Send + Debug,
                                                                 OutStreamType:          Stream<Item=OutItemType> + Send + 'static,
                                                                 CloseVoidAsyncType:     Future<Output=()> + Send + 'static>

                                                                (&self,
                                                                 concurrency_limit:         u32,
                                                                 pipeline_name:             IntoString,
                                                                 stream_id:                 u32,
                                                                 pipelined_stream:          OutStreamType,
                                                                 on_close_callback:         impl FnOnce(Arc<StreamExecutor>) -> CloseVoidAsyncType + Send + Sync + 'static)

                                                                -> Result<(), Box<dyn std::error::Error>> {

        let executor = StreamExecutor::new(format!("{}: {}", self.stream_name(), pipeline_name.into()));
        self.add_executor(Arc::clone(&executor), stream_id).await?;
        executor
            .spawn_non_futures_non_fallibles_executor::<INSTRUMENTS, _, _>(
                concurrency_limit,
                move |executor| on_close_callback(executor),
                pipelined_stream
            );
        Ok(())
    }

    /// Closes this `Multi`, in isolation -- flushing pending events, closing the producers,
    /// waiting for all events to be fully processed and calling all executor's "on close" callbacks.\
    /// If this `Multi` share resources with another one (which will get dumped by the "on close"
    /// callback), most probably you want to close them atomically -- see [multis_close_async!()].\
    /// Returns `true` if all events could be flushed within the given `timeout`.
    pub async fn close(self: &Self, timeout: Duration) -> bool {
        self.channel.gracefully_end_all_streams(timeout).await == 0
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
    #[must_use = "futures do nothing unless you `.await` or poll them"]
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
        self.channel.gracefully_end_stream(executor_info.stream_id, timeout).await;
        true
    }

    /// Registers an executor within this `Multi` so it can be managed -- closed, inquired for stats, etc
    async fn add_executor(&self, stream_executor: Arc<StreamExecutor>, stream_id: u32) -> Result<(), Box<dyn std::error::Error>> {
        let mut internal_multis = self.executor_infos.write().await;
        if internal_multis.contains_key(&stream_executor.executor_name()) {
            Err(Box::from(format!("an executor with the same name is already present: '{}'", stream_executor.executor_name())))
        } else {
            internal_multis.insert(stream_executor.executor_name(), ExecutorInfo { stream_executor, stream_id });
            Ok(())
        }
    }

}


/// This trait exists only to overcome some Rust's limitations (as of 2023-08-01), where it is not possible to infer the types of generic parameters directly.
/// Since having access to internal generic types (of intricate types) my greatly economize typing and increase code readability, this trait justifies its existence for now.\
/// See also [UniGenericTypes].\
/// Usage:
/// ```nocompile
///     let my_multi = CustomMultiType::new();
///     let another_channel = <CustomMultiType as MultiGenericTypes>::MultiChannelTypes::new();
pub trait MultiGenericTypes {
    /// Define it as the homonymous generic parameter
    type ItemType;
    /// Define it as the homonymous generic parameter
    type MultiChannelType;
    /// Define it as the homonymous generic parameter
    type DerivedItemType;
    /// Defined as `MutinyStream<'static, ItemType, MultiChannelType, DerivedItemType>`
    type MutinyStreamType;
}


/// Macro to close, atomically-ish, all [Multi]s passed in as parameters
#[macro_export]
macro_rules! multis_close_async {
    ($timeout: expr,
     $($multi: expr),+) => {
        {
            tokio::join!( $( $multi.channel.flush($timeout), )+ );
            tokio::join!( $( $multi.channel.gracefully_end_all_streams($timeout), )+ );
        }
    }
}
pub use multis_close_async;
pub use crate::types::ChannelCommon;

/// Keeps track of the `stream_executor` associated to each `stream_id`
pub struct ExecutorInfo {
    pub stream_executor: Arc<StreamExecutor>,
    pub stream_id: u32,
}

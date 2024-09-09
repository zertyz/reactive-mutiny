//! Resting place for the [FullSync] Uni Channel

use crate::{
    ogre_std::{
        ogre_queues::{
            full_sync::full_sync_move::FullSyncMove,
            meta_publisher::MovePublisher,
            meta_subscriber::MoveSubscriber,
            meta_container::MoveContainer,
        },
    },
    types::{
        ChannelCommon,
        ChannelProducer,
        ChannelUni,
        ChannelConsumer,
        FullDuplexUniChannel,
    },
    mutiny_stream::MutinyStream,
};
use std::{
    time::Duration,
    pin::Pin,
    fmt::Debug,
    task::Waker,
    sync::Arc,
};
use std::future::Future;
use std::marker::PhantomData;
use crate::streams_manager::StreamsManagerBase;


/// This channel uses the fastest of the queues [FullSyncMove], which are the fastest for general purpose use and for most hardware but requires that elements are copied, due to the full sync characteristics
/// of the backing queue, which doesn't allow enqueueing to happen independently of dequeueing.\
/// Due to that, this channel requires that `ItemType`s are `Clone`, since they will have to be moved around during dequeueing (as there is no way to keep the queue slot allocated during processing),
/// making this channel a typical best fit for small & trivial types.\
/// Please, measure your `Uni`s using all available channels [FullSync], [OgreAtomicQueue] and, possibly, even [OgreMmapLog].\
/// See also [uni::channels::ogre_full_sync_mpmc_queue].\
/// Refresher: the backing queue requires "BUFFER_SIZE" to be a power of 2
pub struct FullSync<'a, ItemType:          Send + Sync + Debug + Default,
                        const BUFFER_SIZE: usize,
                        const MAX_STREAMS: usize> {

    /// common code for dealing with streams
    streams_manager: Arc<StreamsManagerBase<MAX_STREAMS>>,
    /// backing storage for events -- AKA, channels
    container:       Pin<Box<FullSyncMove<ItemType, BUFFER_SIZE>>>,
    _phanrom: PhantomData<&'a ItemType>,

}

impl<'a, ItemType:          Send + Sync + Debug + Default + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelCommon<ItemType, ItemType>
for FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(streams_manager_name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager: Arc::new(StreamsManagerBase::new(streams_manager_name)),
            container:       Box::pin(FullSyncMove::<ItemType, BUFFER_SIZE>::new()),
            _phanrom: PhantomData,
        })
    }

    async fn flush(&self, timeout: Duration) -> u32 {
        self.streams_manager.flush(timeout, || self.pending_items_count()).await
    }

    #[inline(always)]
    fn is_channel_open(&self) -> bool {
        self.streams_manager.is_any_stream_running()
    }

    async fn gracefully_end_stream(&self, stream_id: u32, timeout: Duration) -> bool {
        self.streams_manager.end_stream(stream_id, timeout, || self.pending_items_count()).await
    }

    async fn gracefully_end_all_streams(&self, timeout: Duration) -> u32 {
        self.streams_manager.end_all_streams(timeout, || self.pending_items_count()).await
    }

    fn cancel_all_streams(&self) {
        self.streams_manager.cancel_all_streams();
    }

    fn running_streams_count(&self) -> u32 {
        self.streams_manager.running_streams_count()
    }

    #[inline(always)]
    fn pending_items_count(&self) -> u32 {
        self.container.available_elements_count() as u32
    }

    #[inline(always)]
    fn buffer_size(&self) -> u32 {
        BUFFER_SIZE as u32
    }
}

impl<'a, ItemType:          Send + Sync + Debug + Default + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelUni<'a, ItemType, ItemType>
for FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn create_stream(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, ItemType>, u32) {
        let stream_id = self.streams_manager.create_stream_id();
        (MutinyStream::new(stream_id, self), stream_id)
    }
}

impl<'a, ItemType:          Send + Sync + Debug + Default + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelProducer<'a, ItemType, ItemType>
for FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn send(&self, item: ItemType) -> keen_retry::RetryConsumerResult<(), ItemType, ()> {
        match self.container.publish_movable(item) {
            (Some(len_after), _none_item) => {
                let len_after = len_after.get();
                if len_after <= MAX_STREAMS as u32 {
                    self.streams_manager.wake_stream(len_after-1)
                }
                keen_retry::RetryResult::Ok { reported_input: (), output: () }
            },
            (None, some_item) => {
                keen_retry::RetryResult::Transient { input: some_item.expect("reactive-mutiny: uni movable full_sync::send() BUG! None `some_setter`"), error: () }
            },
        }
    }

    #[inline(always)]
    fn send_with<F: FnOnce(&mut ItemType)>(&self, setter: F) -> keen_retry::RetryConsumerResult<(), F, ()> {
        let setter_option = self.container.publish(setter, || false, |len_after| {
            if len_after <= MAX_STREAMS as u32 {
                self.streams_manager.wake_stream(len_after - 1)
            }
        });
        // implementation note: superior branch prediction over using `.map_or_else()` directly, as `None` is more likely
        match setter_option {
            None => keen_retry::RetryResult::Ok { reported_input: (), output: () },
            Some(setter) => keen_retry::RetryResult::Transient { input: setter, error: () }
        }
    }

    #[inline(always)]
    async fn send_with_async<F:   FnOnce(&'a mut ItemType) -> Fut,
                             Fut: Future<Output=&'a mut ItemType>>
                            (&'a self,
                             setter: F) -> keen_retry::RetryConsumerResult<(), F, ()> {
        if let Some((slot, _slot_id, len_before)) = self.container.leak_slot_internal(|| false) {
            setter(slot).await;
            self.container.publish_leaked_internal();
            if len_before < MAX_STREAMS as u32 {
                self.streams_manager.wake_stream(len_before);
            }
            keen_retry::RetryResult::Ok { reported_input: (), output: () }
        } else {
            keen_retry::RetryResult::Transient { input: setter, error: () }
        }
    }

    #[inline(always)]
    fn reserve_slot(&self) -> Option<&'a mut ItemType> {
        unimplemented!("`reserve_slot()` is not yet implemented for FullSync (movable). It is currently available in Atomic (movable) and zero-copy for both FullSync and Atomic channels.")
    }

    #[inline(always)]
    fn try_send_reserved(&self, _reserved_slot: &mut ItemType) -> bool {
        unimplemented!("`reserve_slot()` / `try_send_reserved()` is not yet implemented for FullSync (movable). It is currently available in Atomic (movable) and zero-copy for both FullSync and Atomic channels.")
    }

    #[inline(always)]
    fn try_cancel_slot_reserve(&self, _reserved_slot: &mut ItemType) -> bool {
        unimplemented!("`reserve_slot()` / `try_cancel_slot_reserve()` is not yet implemented for FullSync (movable). It is currently available in Atomic (movable) and zero-copy for both FullSync and Atomic channels.")
    }
}

impl<'a, ItemType:          'a + Debug + Send + Sync + Default,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelConsumer<'a, ItemType>
for FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn consume(&self, _stream_id: u32) -> Option<ItemType> {
        self.container.consume_movable()
    }

    #[inline(always)]
    fn keep_stream_running(&self, stream_id: u32) -> bool {
        self.streams_manager.keep_stream_running(stream_id)
    }

    #[inline(always)]
    fn register_stream_waker(&self, stream_id: u32, waker: &Waker) {
        self.streams_manager.register_stream_waker(stream_id, waker)
    }

    #[inline(always)]
    fn drop_resources(&self, stream_id: u32) {
        self.streams_manager.report_stream_dropped(stream_id);
    }
}

impl <ItemType:          'static + Debug + Send + Sync + Default,
      const BUFFER_SIZE: usize,
      const MAX_STREAMS: usize>
FullDuplexUniChannel
for FullSync<'static, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    const MAX_STREAMS: usize = MAX_STREAMS;
    const BUFFER_SIZE: usize = BUFFER_SIZE;
    type ItemType            = ItemType;
    type DerivedItemType     = ItemType;

    fn name(&self) -> &str {
        self.streams_manager.name()
    }
}
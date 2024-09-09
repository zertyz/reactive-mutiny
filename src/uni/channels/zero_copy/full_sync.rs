//! Resting place for the Zero-Copy [FullSync] Uni Channel

use crate::{
    ogre_std::{
        ogre_queues::{
            full_sync::full_sync_zero_copy::FullSyncZeroCopy,
            meta_publisher::MetaPublisher,
            meta_subscriber::MetaSubscriber,
            meta_container::MetaContainer,
        },
        ogre_alloc::{
            ogre_unique::OgreUnique,
            BoundedOgreAllocator,
        }
    },
    streams_manager::StreamsManagerBase,
    types::{
        ChannelCommon,
        ChannelUni,
        ChannelProducer,
        ChannelConsumer,
        FullDuplexUniChannel,
    },
    mutiny_stream::MutinyStream,
};
use std::{
    time::Duration,
    fmt::Debug,
    task::Waker,
    sync::Arc,
};
use std::future::Future;
use std::marker::PhantomData;


/// This channel uses the [AtomicZeroCopy] queue and the wrapping type [OgreUnique] to allow a complete zero-copy
/// operation -- no copies either when producing the event nor when consuming it, nor when passing it along to application logic functions.
pub struct FullSync<'a, ItemType:          Debug + Send + Sync,
                        OgreAllocatorType: BoundedOgreAllocator<ItemType> + 'a,
                        const BUFFER_SIZE: usize,
                        const MAX_STREAMS: usize> {

    /// common code for dealing with streams
    streams_manager: Arc<StreamsManagerBase<MAX_STREAMS>>,
    /// backing storage for events
    channel:         FullSyncZeroCopy<ItemType, OgreAllocatorType, BUFFER_SIZE>,
    _phantom:        PhantomData<&'a OgreAllocatorType>,

}


impl<'a, ItemType:          Debug + Send + Sync,
         OgreAllocatorType: BoundedOgreAllocator<ItemType> + 'a + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelCommon<ItemType, OgreUnique<ItemType, OgreAllocatorType>>
for FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(streams_manager_name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager: Arc::new(StreamsManagerBase::new(streams_manager_name)),
            channel:         FullSyncZeroCopy::new(),
            _phantom:        PhantomData,
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
        self.channel.available_elements_count() as u32
    }

    #[inline(always)]
    fn buffer_size(&self) -> u32 {
        BUFFER_SIZE as u32
    }
}


impl<'a, ItemType:          Debug + Send + Sync,
         OgreAllocatorType: BoundedOgreAllocator<ItemType> + 'a + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelUni<'a, ItemType, OgreUnique<ItemType, OgreAllocatorType>>
for FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    fn create_stream(self: &Arc<Self>)
                    -> (MutinyStream<'a, ItemType, Self, OgreUnique<ItemType, OgreAllocatorType>>, u32)
        where Self: ChannelConsumer<'a, OgreUnique<ItemType, OgreAllocatorType>> {
        let stream_id = self.streams_manager.create_stream_id();
        (MutinyStream::new(stream_id, self), stream_id)
    }
}


impl<'a, ItemType:          'a + Debug + Send + Sync,
         OgreAllocatorType: 'a + BoundedOgreAllocator<ItemType> + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelProducer<'a, ItemType, OgreUnique<ItemType, OgreAllocatorType>>
for FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn send(&self, item: ItemType) -> keen_retry::RetryConsumerResult<(), ItemType, ()> {
        match self.channel.publish_movable(item) {
            (Some(len_after), _none_item) => {
                let len_after = len_after.get();
                if len_after <= MAX_STREAMS as u32 {
                    self.streams_manager.wake_stream(len_after-1)
                }
                keen_retry::RetryResult::Ok { reported_input: (), output: () }
            },
            (None, some_item) => {
                keen_retry::RetryResult::Transient { input: some_item.expect("reactive-mutiny: uni zero-copy full_sync::send() BUG! None `some_item`"), error: () }
            },
        }
    }

    #[inline(always)]
    fn send_with<F: FnOnce(&mut ItemType)>(&self, setter: F) -> keen_retry::RetryConsumerResult<(), F, ()> {
        match self.channel.publish(setter) {
            (Some(len_after), _none_setter) => {
                let len_after = len_after.get();
                if len_after <= MAX_STREAMS as u32 {
                    self.streams_manager.wake_stream(len_after-1)
                }
                keen_retry::RetryResult::Ok { reported_input: (), output: () }
            },
            (None, some_setter) => {
                keen_retry::RetryResult::Transient { input: some_setter.expect("reactive-mutiny: uni zero-copy full_sync::send_with() BUG! None `some_setter`"), error: () }
            },
        }
    }

    #[inline(always)]
    async fn send_with_async<F:   FnOnce(&'a mut ItemType) -> Fut,
                             Fut: Future<Output=&'a mut ItemType>>
                            (&'a self,
                             setter: F) -> keen_retry::RetryConsumerResult<(), F, ()> {
        if let Some((slot, _slot_id)) = self.channel.leak_slot() {
            let slot = setter(slot).await;
            let Some(len_after) = self.channel.publish_leaked_ref(slot) else {
                panic!("reactive-mutiny: uni zero-copy full_sync::send_with_async() BUG! could not publish a previously leaked slot");
            };
            let len_after = len_after.get();
            if len_after <= MAX_STREAMS as u32 {
                self.streams_manager.wake_stream(len_after-1)
            }
            keen_retry::RetryResult::Ok { reported_input: (), output: () }
        } else {
            keen_retry::RetryResult::Transient { input: setter, error: () }
        }
    }

    #[inline(always)]
    fn reserve_slot(&'a self) -> Option<&'a mut ItemType> {
        self.channel.leak_slot()
            .map(|(slot_ref, _slot_id)| slot_ref)
    }

    #[inline(always)]
    fn try_send_reserved(&self, reserved_slot: &mut ItemType) -> bool {
        self.channel.publish_leaked_ref(reserved_slot)
            .map(|len_after| {
                // wake the streams, if needed
                let len_after = len_after.get();
                if len_after <= MAX_STREAMS as u32 {
                    self.streams_manager.wake_stream(len_after % MAX_STREAMS as u32);
                }
                true
            }).unwrap_or(false)
    }

    #[inline(always)]
    fn try_cancel_slot_reserve(&self, reserved_slot: &mut ItemType) -> bool {
        self.channel.release_leaked_ref(reserved_slot);
        true
    }
}


impl<'a, ItemType:          'a + Debug + Send + Sync,
         OgreAllocatorType: 'a + BoundedOgreAllocator<ItemType> + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelConsumer<'a, OgreUnique<ItemType, OgreAllocatorType>>
for FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn consume(&self, _stream_id: u32) -> Option<OgreUnique<ItemType, OgreAllocatorType>> {
        self.channel.consume_leaking().map(|(slot_ref, _slot_id)| OgreUnique::<ItemType, OgreAllocatorType>::from_allocated_ref(slot_ref, &self.channel.allocator))
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


impl <ItemType:          'static + Debug + Send + Sync,
      OgreAllocatorType: BoundedOgreAllocator<ItemType> + 'static + Send + Sync,
      const BUFFER_SIZE: usize,
      const MAX_STREAMS: usize>
FullDuplexUniChannel
for FullSync<'static, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    const MAX_STREAMS: usize = MAX_STREAMS;
    const BUFFER_SIZE: usize = BUFFER_SIZE;
    type ItemType            = ItemType;
    type DerivedItemType     = OgreUnique<ItemType, OgreAllocatorType>;

    fn name(&self) -> &str {
        self.streams_manager.name()
    }
}
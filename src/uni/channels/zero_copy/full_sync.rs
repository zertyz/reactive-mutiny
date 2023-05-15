//! Resting place for the Zero-Copy [FullSync] Uni Channel

use crate::{
    ogre_std::{
        ogre_queues::{
            full_sync::full_sync_move::FullSyncMove,
            meta_publisher::MovePublisher,
            meta_subscriber::MoveSubscriber,
            meta_container::MoveContainer,
        },
    },
    ChannelConsumer,
    mutiny_stream::MutinyStream,
};
use std::{
    time::Duration,
    pin::Pin,
    fmt::Debug,
    task::{Waker},
    sync::Arc,
    mem::{ManuallyDrop, MaybeUninit},
    ops::Deref
};
use std::marker::PhantomData;
use crate::streams_manager::StreamsManagerBase;
use async_trait::async_trait;
use crate::ogre_std::ogre_alloc::ogre_unique::OgreUnique;
use crate::ogre_std::ogre_alloc::OgreAllocator;
use crate::ogre_std::ogre_queues::atomic::atomic_zero_copy::AtomicZeroCopy;
use crate::ogre_std::ogre_queues::full_sync::full_sync_zero_copy::FullSyncZeroCopy;
use crate::ogre_std::ogre_queues::meta_container::MetaContainer;
use crate::ogre_std::ogre_queues::meta_publisher::MetaPublisher;
use crate::ogre_std::ogre_queues::meta_subscriber::MetaSubscriber;
use crate::uni::channels::{ChannelCommon, ChannelProducer, FullDuplexChannel};


/// This channel uses the [AtomicZeroCopy] queue and the wrapping type [OgreUnique] to allow a complete zero-copy
/// operation -- no copies either when producing the event nor when consuming it, nor when passing it along to application logic functions.
pub struct FullSync<'a, ItemType:          Debug + Send + Sync,
                        OgreAllocatorType: OgreAllocator<ItemType> + 'a,
                        const BUFFER_SIZE: usize,
                        const MAX_STREAMS: usize> {

    /// common code for dealing with streams
    streams_manager: Arc<StreamsManagerBase<'a, ItemType, MAX_STREAMS>>,
    /// backing storage for events
    channel:         FullSyncZeroCopy<ItemType, OgreAllocatorType, BUFFER_SIZE>,
    _phantom:        PhantomData<OgreAllocatorType>,

}


#[async_trait]
impl<'a, ItemType:          Debug + Send + Sync,
         OgreAllocatorType: OgreAllocator<ItemType> + 'a + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelCommon<'a, ItemType, OgreUnique<ItemType, OgreAllocatorType>>
for FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(streams_manager_name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager: Arc::new(StreamsManagerBase::new(streams_manager_name)),
            channel:         FullSyncZeroCopy::new(),
            _phantom:        PhantomData::default(),
        })
    }

    fn create_stream(self: &Arc<Self>)
                    -> (MutinyStream<'a, ItemType, Self, OgreUnique<ItemType, OgreAllocatorType>>, u32)
                       where Self: ChannelConsumer<'a, OgreUnique<ItemType, OgreAllocatorType>> {
        let stream_id = self.streams_manager.create_stream_id();
        (MutinyStream::new(stream_id, self), stream_id)
    }

    async fn flush(&self, timeout: Duration) -> u32 {
        self.streams_manager.flush(timeout, || self.pending_items_count()).await
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


impl<'a, ItemType:          'a + Debug + Send + Sync,
         OgreAllocatorType: 'a + OgreAllocator<ItemType> + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelProducer<'a, ItemType, OgreUnique<ItemType, OgreAllocatorType>>
for FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn try_send(&self, item: ItemType) -> bool {
        match self.channel.publish(item) {
            Some(len_after) => {
                let len_after = len_after.get();
                if len_after <= MAX_STREAMS as u32 {
                    self.streams_manager.wake_stream(len_after-1)
                }
                true
            },
            None => false,
        }
    }
}


impl<'a, ItemType:          'a + Debug + Send + Sync,
         OgreAllocatorType: 'a + OgreAllocator<ItemType> + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelConsumer<'a, OgreUnique<ItemType, OgreAllocatorType>>
for FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn consume(&self, _stream_id: u32) -> Option<OgreUnique<ItemType, OgreAllocatorType>> {
        match self.channel.consume_leaking() {
            Some( (slot_ref, _slot_id) ) => {
                Some(OgreUnique::<ItemType, OgreAllocatorType>::from_allocated_ref(slot_ref, &self.channel.allocator))
            }
            None => None,
        }
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


impl <'a, ItemType:          'a + Debug + Send + Sync,
          OgreAllocatorType: OgreAllocator<ItemType> + 'a + Send + Sync,
          const BUFFER_SIZE: usize,
          const MAX_STREAMS: usize>
FullDuplexChannel<'a, ItemType, OgreUnique<ItemType, OgreAllocatorType>>
for FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {}
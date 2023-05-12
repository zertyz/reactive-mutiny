//! Resting place for [Atomic]

use crate::{
    types::ChannelConsumer,
    ogre_std::{
        ogre_alloc::{
            OgreAllocator,
            ogre_array_pool_allocator::OgreArrayPoolAllocator,
            ogre_arc::OgreArc,
        },
        ogre_queues::{
            atomic::atomic_zero_copy::AtomicZeroCopy,
            meta_container::MetaContainer,
            meta_publisher::MetaPublisher,
            meta_subscriber::MetaSubscriber,
        },
    },
    uni::channels::{
        ChannelCommon,
        ChannelProducer,
    },
    streams_manager::StreamsManagerBase,
    mutiny_stream::MutinyStream,
};
use std::{
    time::Duration,
    pin::Pin,
    fmt::Debug,
    task::{Waker},
    sync::Arc,
    mem::{ManuallyDrop, MaybeUninit},
    ops::Deref,
    marker::PhantomData,
};
use async_trait::async_trait;
use crate::uni::channels::FullDuplexChannel;


/// This channel uses the [AtomicZeroCopy] queue and the wrapping type [OgreArc] to allow a complete zero-copy
/// operation -- no copies either when producing the event nor when consuming it, nor when passing it along to application logic functions.
pub struct Atomic<'a, ItemType:          Debug + Send + Sync,
                      OgreAllocatorType: OgreAllocator<ItemType> + 'a,
                      const BUFFER_SIZE: usize,
                      const MAX_STREAMS: usize> {

    streams_manager: StreamsManagerBase<'a, ItemType, MAX_STREAMS>,
    channel:         AtomicZeroCopy<ItemType, OgreAllocatorType, BUFFER_SIZE>,
    _phantom:        PhantomData<OgreAllocatorType>,
}


#[async_trait]
impl<'a, ItemType:          Debug + Send + Sync,
         OgreAllocatorType: OgreAllocator<ItemType> + 'a + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelCommon<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>>
for Atomic<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager: StreamsManagerBase::new(name),
            channel:         AtomicZeroCopy::new(),
            _phantom:        PhantomData::default(),
        })
    }

    fn create_stream(self: &Arc<Self>)
                     -> (MutinyStream<'a, ItemType, Self, OgreArc<ItemType, OgreAllocatorType>>, u32)
                         where Self: ChannelConsumer<'a, OgreArc<ItemType, OgreAllocatorType>> {
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


impl<'a, ItemType:          'a + Send + Sync + Debug,
         OgreAllocatorType: OgreAllocator<ItemType> + 'a + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelProducer<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>>
for Atomic<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn try_send(&self, item: ItemType) -> bool {
        match self.channel.publish(item) {
            Some(len_after) => {
                let len_after = len_after.get();
                if len_after <= MAX_STREAMS as u32 {
                    self.streams_manager.wake_stream(len_after-1)
                } else if len_after == 1 + MAX_STREAMS as u32 {
                    // the Atomic queue may enqueue at the same time it dequeues, so,
                    // on high pressure for production / consumption & low event payloads (like in our tests),
                    // the Stream might have dequeued the last element, another enqueue just finished and we triggered the wake before
                    // the Stream had returned, leaving an element stuck. This code works around this and is required only for the Atomic Queue.
                    self.streams_manager.wake_stream(len_after - 2)
                }
                true
            },
            None => false,
        }
    }
}


impl<'a, ItemType:          'static + Debug + Send + Sync,
         OgreAllocatorType: 'a + OgreAllocator<ItemType> + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelConsumer<'a, OgreArc<ItemType, OgreAllocatorType>>
for Atomic<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn consume(&self, _stream_id: u32) -> Option<OgreArc<ItemType, OgreAllocatorType>> {
        match self.channel.consume_leaking() {
            Some( (_event_ref, slot_id) ) => {
                Some(OgreArc::<ItemType, OgreAllocatorType>::from_allocated(slot_id, &self.channel.allocator))
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


impl <'a, ItemType:          'static + Debug + Send + Sync,
          OgreAllocatorType: 'a + OgreAllocator<ItemType> + Send + Sync,
          const BUFFER_SIZE: usize,
          const MAX_STREAMS: usize>
FullDuplexChannel<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>>
for Atomic<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {}
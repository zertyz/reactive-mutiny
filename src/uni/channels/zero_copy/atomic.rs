//! Resting place for [Atomic]

use crate::{
    types::MutinyStreamSource,
    ogre_std::{
        ogre_alloc::ogre_arc::OgreArc,
        ogre_queues::{
            atomic::atomic_zero_copy::AtomicZeroCopy,
            meta_container::MetaContainer,
            meta_publisher::MetaPublisher,
            meta_subscriber::MetaSubscriber,
        },
    },
    uni::channels::{
        UniChannelCommon,
        UniMovableChannel,
        uni_stream::UniStream,
    },
    streams_manager::StreamsManagerBase,
};
use std::{
    time::Duration,
    pin::Pin,
    fmt::Debug,
    task::{Waker},
    sync::Arc,
    mem::{ManuallyDrop, MaybeUninit},
    ops::Deref,
};
use async_trait::async_trait;
use crate::ogre_std::ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator;
use crate::ogre_std::ogre_alloc::OgreAllocator;
use crate::uni::channels::uni_zero_copy_stream::UniZeroCopyStream;


pub struct Atomic<'a, ItemType:          Debug + Send + Sync,
                      const BUFFER_SIZE: usize,
                      const MAX_STREAMS: usize> {

    streams_manager: StreamsManagerBase<'a, ItemType, MAX_STREAMS>,
    channel:         AtomicZeroCopy<ItemType, BUFFER_SIZE>
}

//#[async_trait]
impl<'a, ItemType:          Debug + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
/*UniChannelCommon<'a, OgreArc<ItemType, OgreArrayPoolAllocator<ItemType, BUFFER_SIZE>>>
for */Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

pub    fn new<IntoString: Into<String>>(name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager: StreamsManagerBase::new(name),
            channel:         AtomicZeroCopy::new(),
        })
    }

pub    fn consumer_stream(self: &Arc<Self>)
                      -> UniZeroCopyStream<'a, ItemType, OgreArrayPoolAllocator<ItemType, BUFFER_SIZE>, Self>
                         where Self: MutinyStreamSource<'a, ItemType, OgreArc<ItemType, OgreArrayPoolAllocator<ItemType, BUFFER_SIZE>>> {
        let stream_id = self.streams_manager.create_stream_id();
        UniZeroCopyStream::new(stream_id, self)
    }

    async fn flush(&self, timeout: Duration) -> u32 {
        self.streams_manager.flush(timeout, || self.pending_items_count()).await
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
}

impl<'a, ItemType:          'a + Send + Sync + Debug,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
/*UniMovableChannel<'a, OgreArc<ItemType, OgreArrayPoolAllocator<ItemType, BUFFER_SIZE>>>
for */Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn zero_copy_try_send(&self, item_builder: impl FnOnce(&mut ItemType)) -> bool {
        self.channel.publish(item_builder,
                             || {
                                   self.streams_manager.wake_all_streams();
                                   false
                               },
                             |len| if len <= 1 {
                                                                        self.streams_manager.wake_stream(0)
                                                                    } else if len <= MAX_STREAMS as u32 {
                                                                        self.streams_manager.wake_stream(len-1)
                                                                    })
    }

    #[inline(always)]
pub    fn try_send(&self, item: ItemType) -> bool {
        let item = ManuallyDrop::new(item);       // ensure it won't be dropped when this function ends, since it will be "moved"
        unsafe {
            self.zero_copy_try_send(|slot| {
                // move `item` to `slot`
                std::ptr::copy_nonoverlapping(item.deref() as *const ItemType, (slot as *const ItemType) as *mut ItemType, 1);
            })
        }
    }
}


/*impl<'a, ItemType:          Debug + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
OgreAllocator<ItemType>
for Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn alloc(&self) -> Option<(&mut ItemType, u32)> {
        self.channel.allocator.alloc()
    }

    #[inline(always)]
    fn dealloc_ref(&self, slot: &mut ItemType) {
        self.channel.allocator.dealloc_ref(slot)
    }

    #[inline(always)]
    fn dealloc_id(&self, slot_id: u32) {
        self.channel.allocator.dealloc_id(slot_id)
    }

    #[inline(always)]
    fn id_from_ref(&self, slot: &ItemType) -> u32 {
        self.channel.allocator.id_from_ref(slot)
    }

    #[inline(always)]
    fn ref_from_id(&self, slot_id: u32) -> &mut ItemType {
        self.channel.allocator.ref_from_id(slot_id)
    }
}*/


impl<'a, ItemType:          Debug + Send + Sync + 'static,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
MutinyStreamSource<'a, ItemType, OgreArc<ItemType, OgreArrayPoolAllocator<ItemType, BUFFER_SIZE>>>
for Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn provide(&self, _stream_id: u32) -> Option<OgreArc<ItemType, OgreArrayPoolAllocator<ItemType, BUFFER_SIZE>>> {
        match self.channel.consume_leaking() {
            Some( (_event_ref, slot_id) ) => {
                let allocator = Arc::clone(&self.channel.allocator);
                Some(OgreArc::from_allocated(slot_id, allocator))
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

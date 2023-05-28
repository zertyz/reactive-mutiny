//! Resting place for the `OgreArc` based [Atomic] Zero-Copy Multi Channel

use crate::{
    ogre_std::{
        ogre_queues::{
            atomic::atomic_move::AtomicMove,
            meta_publisher::MovePublisher,
            meta_subscriber::MoveSubscriber,
            meta_container::MoveContainer,
        },
        ogre_alloc::{
            ogre_arc::OgreArc,
            ogre_array_pool_allocator::OgreArrayPoolAllocator,
            OgreAllocator,
        },
    },
    types::{ChannelCommon, ChannelMulti, ChannelProducer, ChannelConsumer, FullDuplexMultiChannel},
    streams_manager::StreamsManagerBase,
    mutiny_stream::MutinyStream,
};
use std::{
    time::Duration,
    sync::{
        Arc,
    },
    pin::Pin,
    fmt::Debug,
    task::{Waker},
    num::NonZeroU32,
};
use async_trait::async_trait;
use log::{warn};


/// This channel uses the queue [AtomicMove] (the lowest latency among all in 'benches/'), which allows zero-copy both when enqueueing / dequeueing and
/// allow enqueueing to happen independently of dequeueing.\
/// Due to that, this channel requires that `ItemType`s are `Clone`, since they will have to be moved around during dequeueing (as there is no way to keep the queue slot allocated during processing),
/// making this channel a typical best fit for small & trivial types.\
/// Please, measure your `Multi`s using all available channels [Atomic], [OgreAtomicQueue] and, possibly, even [OgreMmapLog].\
/// See also [multi::channels::ogre_full_sync_mpmc_queue].\
/// Refresher: the backing queue requires `BUFFER_SIZE` to be a power of 2 -- the same applies to `MAX_STREAMS`, which will also have its own queue
pub struct Atomic<'a, ItemType:          Send + Sync + Debug,
                      OgreAllocatorType: OgreAllocator<ItemType> + 'static + Sync + Send,
                      const BUFFER_SIZE: usize = 1024,
                      const MAX_STREAMS: usize = 16> {

    /// common code for dealing with streams
    streams_manager:     StreamsManagerBase<'a, ItemType, MAX_STREAMS>,
    /// backing storage for events
    allocator:           OgreAllocatorType,
    /// management for dispatching the events -- elements in the container are `slot_id`s in the `allocator`
    dispatcher_managers: [AtomicMove<OgreArc<ItemType, OgreAllocatorType>, BUFFER_SIZE>; MAX_STREAMS],

}


#[async_trait]      // all async functions are out of the hot path, so the `async_trait` won't impose performance penalties
impl<'a, ItemType:          Send + Sync + Debug + 'a,
         OgreAllocatorType: OgreAllocator<ItemType> + 'a + Sync + Send,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelCommon<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>>
for Atomic<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager:     StreamsManagerBase::new(name),
            allocator:           OgreAllocatorType::new(),
            dispatcher_managers: [0; MAX_STREAMS].map(|_| AtomicMove::<OgreArc<ItemType, OgreAllocatorType>, BUFFER_SIZE>::new()),
        })
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

    #[inline(always)]
    fn running_streams_count(&self) -> u32 {
        self.streams_manager.running_streams_count()
    }

    #[inline(always)]
    fn pending_items_count(&self) -> u32 {
        self.streams_manager.used_streams().iter()
            .take_while(|&&stream_id| stream_id != u32::MAX)
            .map(|&stream_id| unsafe { self.dispatcher_managers.get_unchecked(stream_id as usize) }.available_elements_count())
            .max().unwrap_or(0) as u32
    }

    #[inline(always)]
    fn buffer_size(&self) -> u32 {
        BUFFER_SIZE as u32
    }
}

impl<'a, ItemType:          Send + Sync + Debug + 'a,
         OgreAllocatorType: OgreAllocator<ItemType> + 'a + Sync + Send,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelMulti<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>>
for Atomic<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    fn create_stream_for_old_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, OgreArc<ItemType, OgreAllocatorType>>, u32) {
        panic!("multi::channels::ogre_arc::Atomic: this channel doesn't implement the `.create_stream_for_old_events()` method. Use `.create_stream_for_new_events()` instead")
    }

    fn create_stream_for_new_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, OgreArc<ItemType, OgreAllocatorType>>, u32) {
        let stream_id = self.streams_manager.create_stream_id();
        (MutinyStream::new(stream_id, self), stream_id)
    }

    fn create_streams_for_old_and_new_events(self: &Arc<Self>) -> ((MutinyStream<'a, ItemType, Self, OgreArc<ItemType, OgreAllocatorType>>, u32),
                                                                   (MutinyStream<'a, ItemType, Self, OgreArc<ItemType, OgreAllocatorType>>, u32)) {
        panic!("multi::channels::ogre_arc::Atomic: this channel doesn't implement the `.create_streams_for_old_and_new_events()` method. Use `.create_stream_for_new_events()` instead")
    }

    fn create_stream_for_old_and_new_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, OgreArc<ItemType, OgreAllocatorType>>, u32) {
        panic!("multi::channels::ogre_arc::Atomic: this channel doesn't implement the `.create_stream_for_old_and_new_events()` method. Use `.create_stream_for_new_events()` instead")
    }

}

impl<'a, ItemType:          'a + Send + Sync + Debug,
         OgreAllocatorType: OgreAllocator<ItemType> + 'a + Sync + Send,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelProducer<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>>
for Atomic<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn try_send<F: FnOnce(&mut ItemType)>(&self, setter: F) -> bool {
        if let Some(ogre_arc_item) = OgreArc::new(setter, &self.allocator) {
            self.send_derived(&ogre_arc_item);
            true
        } else {
            false
        }
    }

    #[inline(always)]
    fn send<F: FnOnce(&mut ItemType)>(&self, setter: F) {
        loop {
            if let Some((mut slot_ref, slot_id)) = self.allocator.alloc_ref() {
                setter(&mut slot_ref);
                let ogre_arc_item = OgreArc::from_allocated(slot_id, &self.allocator);
                self.send_derived(&ogre_arc_item);
                break
            }
            std::hint::spin_loop();
        }
    }

    #[inline(always)]
    fn send_derived(&self, ogre_arc_item: &OgreArc<ItemType, OgreAllocatorType>) {
        let running_streams_count = self.streams_manager.running_streams_count();
        unsafe { ogre_arc_item.increment_references(running_streams_count) };
        let used_streams = self.streams_manager.used_streams();
        for i in 0..running_streams_count {
            let stream_id = *unsafe { used_streams.get_unchecked(i as usize) };
            if stream_id != u32::MAX {
                let dispatcher_manager = unsafe { self.dispatcher_managers.get_unchecked(stream_id as usize) };
                match dispatcher_manager.publish_movable(unsafe { ogre_arc_item.raw_copy() }) {
                    Some(len_after_publishing) => {
                        if len_after_publishing.get() <= 2 {
                            self.streams_manager.wake_stream(stream_id);
                        }
                    },
                    None => {
                        // WARNING: THIS SHOULD NEVER HAPPEN IF BUFFER_SIZE OF THE ALLOCATOR IS THE SAME AS THE DISPATCHER MANAGERS, as there is no way that there are more elements into a dispatcher manager than elements in the allocator
                        //          if this class ever gets to accept a bigger BUFFER_SIZE for the allocator, this situation may happen (and the loops should be brought back from the "movable" channels). For the time being, it could be replaced by a panic!() instead, saying it is a bug.
                        panic!("BUG! This should never happen! See the comment in the code -- Multi Channel's OgreArc/Atomic (named '{channel_name}', {used_streams_count} streams): One of the streams (#{stream_id}) is full of elements. Multi producing performance has been degraded. Increase the Multi buffer size (currently {BUFFER_SIZE}) to overcome that.",
                               channel_name = self.streams_manager.name(), used_streams_count = self.streams_manager.running_streams_count());
                    },
                }
            }
        }
    }

    #[inline(always)]
    fn try_send_movable(&self, item: ItemType) -> bool {
        self.try_send(|slot| *slot = item)
    }
}

impl<'a, ItemType:          'a + Send + Sync + Debug,
         OgreAllocatorType: OgreAllocator<ItemType> + 'a + Sync + Send,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelConsumer<'a, OgreArc<ItemType, OgreAllocatorType>>
for Atomic<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn consume(&self, stream_id: u32) -> Option<OgreArc<ItemType, OgreAllocatorType>> {
        let dispatcher_manager = unsafe { self.dispatcher_managers.get_unchecked(stream_id as usize) };
        dispatcher_manager.consume_movable()
    }

    #[inline(always)]
    fn keep_stream_running(&self, stream_id: u32) -> bool {
        self.streams_manager.keep_stream_running(stream_id)
    }

    #[inline(always)]
    fn register_stream_waker(&self, stream_id: u32, waker: &Waker) {
        self.streams_manager.register_stream_waker(stream_id, waker);
    }

    #[inline(always)]
    fn drop_resources(&self, stream_id: u32) {
        self.streams_manager.report_stream_dropped(stream_id);
    }
}


impl<'a, ItemType:          Send + Sync + Debug,
         OgreAllocatorType: OgreAllocator<ItemType> + 'static + Sync + Send,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
Drop for
Atomic<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {
    fn drop(&mut self) {
        self.streams_manager.cancel_all_streams();
    }
}


impl <'a, ItemType:          'a + Debug + Send + Sync,
          OgreAllocatorType: OgreAllocator<ItemType> + 'a + Sync + Send,
          const BUFFER_SIZE: usize,
          const MAX_STREAMS: usize>
FullDuplexMultiChannel<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>>
for Atomic<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {}
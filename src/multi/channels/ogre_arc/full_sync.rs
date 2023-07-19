//! Resting place for the `OgreArc` based [FullSync] Zero-Copy Multi Channel

use crate::{
    ogre_std::{
        ogre_queues::{
            full_sync::full_sync_move::FullSyncMove,
            meta_publisher::MovePublisher,
            meta_subscriber::MoveSubscriber,
            meta_container::MoveContainer,
        },
        ogre_alloc::{
            ogre_arc::OgreArc,
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
    fmt::Debug,
    task::{Waker},
};
use std::mem::MaybeUninit;
use async_trait::async_trait;


/// ...
pub struct FullSync<'a, ItemType:          Send + Sync + Debug + 'static,
                        OgreAllocatorType: OgreAllocator<ItemType> + 'static + Sync + Send,
                        const BUFFER_SIZE: usize = 1024,
                        const MAX_STREAMS: usize = 16> {

    /// common code for dealing with streams
    streams_manager:     StreamsManagerBase<'a, ItemType, MAX_STREAMS>,
    /// backing storage for events
    allocator:           OgreAllocatorType,
    /// management for dispatching the events -- elements in the container are `slot_id`s in the `allocator`
    dispatcher_managers: [FullSyncMove<OgreArc<ItemType, OgreAllocatorType>, BUFFER_SIZE>; MAX_STREAMS],
}


#[async_trait]      // all async functions are out of the hot path, so the `async_trait` won't impose performance penalties
impl<'a, ItemType:          Send + Sync + Debug + 'a,
         OgreAllocatorType: OgreAllocator<ItemType> + 'a + Sync + Send,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelCommon<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>>
for FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager:     StreamsManagerBase::new(name),
            allocator:           OgreAllocatorType::new(),
            dispatcher_managers: [0; MAX_STREAMS].map(|_| FullSyncMove::<OgreArc<ItemType, OgreAllocatorType>, BUFFER_SIZE>::with_initializer(|| unsafe {
                let mut slot = MaybeUninit::<OgreArc<ItemType, OgreAllocatorType>>::uninit();
                slot.as_mut_ptr().write_bytes(1u8, 1);  // initializing with a non-zero value makes MIRI happily state there isn't any UB involved (actually, any value is equally good)
                slot.assume_init()
            })),
        })
    }

    #[must_use = "futures do nothing unless you `.await` or poll them"]
    async fn flush(&self, timeout: Duration) -> u32 {
        self.streams_manager.flush(timeout, || self.pending_items_count()).await
    }

    #[must_use = "futures do nothing unless you `.await` or poll them"]
    async fn gracefully_end_stream(&self, stream_id: u32, timeout: Duration) -> bool {
        self.streams_manager.end_stream(stream_id, timeout, || self.pending_items_count()).await
    }

    #[must_use = "futures do nothing unless you `.await` or poll them"]
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
for FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    fn create_stream_for_old_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, OgreArc<ItemType, OgreAllocatorType>>, u32) {
        panic!("multi::channels::ogre_arc::FullSync: this channel doesn't implement the `.create_stream_for_old_events()` method. Use `.create_stream_for_new_events()` instead")
    }

    fn create_stream_for_new_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, OgreArc<ItemType, OgreAllocatorType>>, u32) {
        let stream_id = self.streams_manager.create_stream_id();
        (MutinyStream::new(stream_id, self), stream_id)
    }

    fn create_streams_for_old_and_new_events(self: &Arc<Self>) -> ((MutinyStream<'a, ItemType, Self, OgreArc<ItemType, OgreAllocatorType>>, u32),
                                                                   (MutinyStream<'a, ItemType, Self, OgreArc<ItemType, OgreAllocatorType>>, u32)) {
        panic!("multi::channels::ogre_arc::FullSync: this channel doesn't implement the `.create_streams_for_old_and_new_events()` method. Use `.create_stream_for_new_events()` instead")
    }

    fn create_stream_for_old_and_new_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, OgreArc<ItemType, OgreAllocatorType>>, u32) {
        panic!("multi::channels::ogre_arc::FullSync: this channel doesn't implement the `.create_stream_for_old_and_new_events()` method. Use `.create_stream_for_new_events()` instead")
    }

}


impl<'a, ItemType:          'a + Send + Sync + Debug,
         OgreAllocatorType: OgreAllocator<ItemType> + 'a + Sync + Send,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelProducer<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>>
for FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn try_send<F: FnOnce(&mut ItemType)>(&self, setter: F) -> Option<F> {
        if let Some((ogre_arc_item, mut slot)) = OgreArc::new(&self.allocator) {
            setter(&mut slot);
            self.send_derived(&ogre_arc_item);
            None
        } else {
            Some(setter)
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
    fn send_derived(&self, ogre_arc_item: &OgreArc<ItemType, OgreAllocatorType>) -> bool {
        let running_streams_count = self.streams_manager.running_streams_count();
        unsafe { ogre_arc_item.increment_references(running_streams_count) };
        let used_streams = self.streams_manager.used_streams();
        for i in 0..running_streams_count {
            let stream_id = *unsafe { used_streams.get_unchecked(i as usize) };
            if stream_id != u32::MAX {
                let dispatcher_manager = unsafe { self.dispatcher_managers.get_unchecked(stream_id as usize) };
                match dispatcher_manager.publish_movable(unsafe { ogre_arc_item.raw_copy() }).0 {
                    Some(len_after_publishing) => {
                        if len_after_publishing.get() <= 1 {
                            self.streams_manager.wake_stream(stream_id);
                        }
                    },
                    None => {
                        // TODO f14: re-evaluate if this is really not happening after the f14 changes
                        // WARNING: THIS SHOULD NEVER HAPPEN IF BUFFER_SIZE OF THE ALLOCATOR IS THE SAME AS THE DISPATCHER MANAGERS, as there is no way that there are more elements into a dispatcher manager than elements in the allocator
                        //          if this class ever gets to accept a bigger BUFFER_SIZE for the allocator, this situation may happen (and the loops should be brought back from the "movable" channels). For the time being, it could be replaced by a panic!() instead, saying it is a bug.
                        panic!("BUG! This should never happen! See the comment in the code -- Multi Channel's OgreArc/FullSync (named '{channel_name}', {used_streams_count} streams): One of the streams (#{stream_id}) is full of elements. Multi producing performance has been degraded. Increase the Multi buffer size (currently {BUFFER_SIZE}) to overcome that.",
                               channel_name = self.streams_manager.name(), used_streams_count = self.streams_manager.running_streams_count());
                    },
                }
            }
        }
        true
    }

    #[inline(always)]
    fn try_send_movable(&self, item: ItemType) -> Option<ItemType> {
        todo!("If the refactoring works, copy & adjust the code from `try_send()`");
        self.try_send(|slot| *slot = item);
    }
}


impl<'a, ItemType:          'a + Send + Sync + Debug,
         OgreAllocatorType: OgreAllocator<ItemType> + 'a + Sync + Send,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelConsumer<'a, OgreArc<ItemType, OgreAllocatorType>>
for FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

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
        self.streams_manager.register_stream_waker(stream_id, waker)
    }

    #[inline(always)]
    fn drop_resources(&self, stream_id: u32) {
        self.streams_manager.report_stream_dropped(stream_id);
    }
}


impl<'a, ItemType:          Send + Sync + Debug + 'a,
         OgreAllocatorType: OgreAllocator<ItemType> + 'static + Sync + Send,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
Drop for
FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {
    fn drop(&mut self) {
        self.streams_manager.cancel_all_streams();
    }
}


impl <'a, ItemType:          'a + Debug + Send + Sync,
          OgreAllocatorType: OgreAllocator<ItemType> + 'a + Sync + Send,
          const BUFFER_SIZE: usize,
          const MAX_STREAMS: usize>
FullDuplexMultiChannel<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>>
for FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {}
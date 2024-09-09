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
            BoundedOgreAllocator,
        },
    },
    types::{ChannelCommon, ChannelMulti, ChannelProducer, ChannelConsumer, FullDuplexMultiChannel},
    streams_manager::StreamsManagerBase,
    mutiny_stream::MutinyStream,
};
use std::{
    time::Duration,
    sync::Arc,
    fmt::Debug,
    task::Waker,
};
use std::future::Future;
use std::marker::PhantomData;
use std::mem::MaybeUninit;


/// ...
pub struct FullSync<'a, ItemType:          Send + Sync + Debug + 'static,
                        OgreAllocatorType: BoundedOgreAllocator<ItemType> + 'static + Sync + Send,
                        const BUFFER_SIZE: usize = 1024,
                        const MAX_STREAMS: usize = 16> {

    /// common code for dealing with streams
    streams_manager:     StreamsManagerBase<MAX_STREAMS>,
    /// backing storage for events
    allocator:           OgreAllocatorType,
    /// management for dispatching the events -- elements in the container are `slot_id`s in the `allocator`
    dispatcher_managers: [FullSyncMove<OgreArc<ItemType, OgreAllocatorType>, BUFFER_SIZE>; MAX_STREAMS],
    _phanrom: PhantomData<&'a ItemType>,
}


impl<'a, ItemType:          Send + Sync + Debug + 'a,
         OgreAllocatorType: BoundedOgreAllocator<ItemType> + 'a + Sync + Send,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelCommon<ItemType, OgreArc<ItemType, OgreAllocatorType>> for
FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager:     StreamsManagerBase::new(name),
            allocator:           OgreAllocatorType::new(),
            dispatcher_managers: [0; MAX_STREAMS].map(|_| FullSyncMove::<OgreArc<ItemType, OgreAllocatorType>, BUFFER_SIZE>::with_initializer(|| unsafe {
                let mut slot = MaybeUninit::<OgreArc<ItemType, OgreAllocatorType>>::uninit();
                slot.as_mut_ptr().write_bytes(1u8, 1);  // initializing with a non-zero value makes MIRI happily state there isn't any UB involved (actually, any value is equally good)
                slot.assume_init()
            })),
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
         OgreAllocatorType: BoundedOgreAllocator<ItemType> + 'a + Sync + Send,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelMulti<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>> for
FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

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
         OgreAllocatorType: BoundedOgreAllocator<ItemType> + 'a + Sync + Send,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelProducer<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>> for
FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn send(&self, item: ItemType) -> keen_retry::RetryConsumerResult<(), ItemType, ()> {
        if let Some((ogre_arc_item, slot)) = OgreArc::new(&self.allocator) {
            unsafe { std::ptr::write(slot, item) }
            _ = self.send_derived(&ogre_arc_item);
            keen_retry::RetryResult::Ok { reported_input: (), output: () }
        } else {
            keen_retry::RetryResult::Transient { input: item, error: () }
        }
    }

    #[inline(always)]
    fn send_with<F: FnOnce(&mut ItemType)>(&self, setter: F) -> keen_retry::RetryConsumerResult<(), F, ()> {
        if let Some((ogre_arc_item, slot)) = OgreArc::new(&self.allocator) {
            setter(slot);
            _ = self.send_derived(&ogre_arc_item);
            keen_retry::RetryResult::Ok { reported_input: (), output: () }
        } else {
            keen_retry::RetryResult::Transient { input: setter, error: () }
        }
    }

    #[inline(always)]
    async fn send_with_async<F:   FnOnce(&'a mut ItemType) -> Fut,
                             Fut: Future<Output=&'a mut ItemType>>
                            (&'a self,
                             setter: F) -> keen_retry::RetryConsumerResult<(), F, ()> {
        if let Some((ogre_arc_item, slot)) = OgreArc::new(&self.allocator) {
            setter(slot).await;
            _ = self.send_derived(&ogre_arc_item);
            keen_retry::RetryResult::Ok { reported_input: (), output: () }
        } else {
            keen_retry::RetryResult::Transient { input: setter, error: () }
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
    fn reserve_slot(&'a self) -> Option<&'a mut ItemType> {
        self.allocator.alloc_ref()
            .map(|(slot_ref, _slot_id)| slot_ref)
    }

    #[inline(always)]
    fn try_send_reserved(&self, reserved_slot: &mut ItemType) -> bool {
        let slot_id = self.allocator.id_from_ref(reserved_slot);
        let ogre_arc = OgreArc::from_allocated(slot_id, &self.allocator);
        _ = self.send_derived(&ogre_arc);
        true
    }

    #[inline(always)]
    fn try_cancel_slot_reserve(&self, reserved_slot: &mut ItemType) -> bool {
        self.allocator.dealloc_ref(reserved_slot);
        true
    }
}


impl<'a, ItemType:          'a + Send + Sync + Debug,
         OgreAllocatorType: BoundedOgreAllocator<ItemType> + 'a + Sync + Send,
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
         OgreAllocatorType: BoundedOgreAllocator<ItemType> + 'static + Sync + Send,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
Drop for
FullSync<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {
    fn drop(&mut self) {
        self.streams_manager.cancel_all_streams();
    }
}


impl <ItemType:          'static + Debug + Send + Sync,
      OgreAllocatorType: BoundedOgreAllocator<ItemType> + 'static + Sync + Send,
      const BUFFER_SIZE: usize,
      const MAX_STREAMS: usize>
FullDuplexMultiChannel for
FullSync<'static, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    const MAX_STREAMS: usize = MAX_STREAMS;
    const BUFFER_SIZE: usize = BUFFER_SIZE;
    type ItemType            = ItemType;
    type DerivedItemType     = OgreArc<ItemType, OgreAllocatorType>;
}
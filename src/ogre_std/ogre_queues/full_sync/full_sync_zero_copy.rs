//! Resting place for [FullSyncZeroCopy]

use crate::ogre_std::{
    ogre_queues::{
        full_sync::full_sync_move::FullSyncMove,
        meta_publisher::{MetaPublisher,MovePublisher},
        meta_subscriber::{MetaSubscriber,MoveSubscriber},
        meta_container::{MetaContainer, MoveContainer},
    },
    ogre_alloc::BoundedOgreAllocator,
};
use std::{
    num::NonZeroU32,
    marker::PhantomData,
    sync::Arc,
    fmt::Debug,
};


/// Basis for multiple producer / multiple consumer queues using a quick-and-dirty (but fast)
/// full synchronization through an atomic flag, with a clever & experimentally tuned efficient locking mechanism.
///
/// This queue implements the "zero-copy patterns" through [ZeroCopyPublisher] & [ZeroCopySubscriber] and
/// is a good fit for payloads > 1k.
///
/// For thinner payloads, [FullSyncMove] should be a better fit, as it doesn't require a secondary container to
/// hold the objects.
pub struct FullSyncZeroCopy<SlotType:          Debug + Send + Sync,
                            OgreAllocatorType: BoundedOgreAllocator<SlotType>,
                            const BUFFER_SIZE: usize> {

    pub(crate) allocator: Arc<OgreAllocatorType>,
               queue:     FullSyncMove<u32, BUFFER_SIZE>,
               _phantom:  PhantomData<SlotType>

}


impl<'a, SlotType:          'a + Debug + Send + Sync,
         OgreAllocatorType: BoundedOgreAllocator<SlotType>,
         const BUFFER_SIZE: usize>
MetaContainer<'a, SlotType> for
FullSyncZeroCopy<SlotType, OgreAllocatorType, BUFFER_SIZE> {

    fn new() -> Self {
        Self {
            allocator: Arc::new(OgreAllocatorType::new()),
            queue:     FullSyncMove::new(),
            _phantom:  PhantomData,
        }
    }
}


impl<'a, SlotType:          'a + Debug + Send + Sync,
         OgreAllocatorType: BoundedOgreAllocator<SlotType>,
         const BUFFER_SIZE: usize>
MetaPublisher<'a, SlotType> for
FullSyncZeroCopy<SlotType, OgreAllocatorType, BUFFER_SIZE> {

    #[inline(always)]
    fn publish<F: FnOnce(&mut SlotType)>(&self, setter: F) -> (Option<NonZeroU32>, Option<F>) {
        match self.leak_slot() {
            Some( (slot_ref, slot_id) ) => {
                setter(slot_ref);
                (self.publish_leaked_id(slot_id), None)
            },
            None => (None, Some(setter)),
        }
    }

    #[inline(always)]
    fn publish_movable(&self, item: SlotType) -> (Option<NonZeroU32>, Option<SlotType>) {
        match self.leak_slot() {
            Some( (slot_ref, slot_id) ) => {
                unsafe { std::ptr::write(slot_ref, item); }
                (self.publish_leaked_id(slot_id), None)
            }
            None => (None, Some(item)),
        }
    }

    #[inline(always)]
    fn leak_slot(&self) -> Option<(/*ref:*/ &mut SlotType, /*id: */u32)> {
        self.allocator.alloc_ref()
    }

    #[inline(always)]
    fn publish_leaked_ref(&'a self, slot: &'a SlotType) -> Option<NonZeroU32> {
        self.publish_leaked_id(self.allocator.id_from_ref(slot))
    }

    #[inline(always)]
    fn publish_leaked_id(&'a self, slot_id: u32) -> Option<NonZeroU32> {
        self.queue.publish_movable(slot_id).0
    }

    #[inline(always)]
    fn unleak_slot_ref(&'a self, slot: &'a mut SlotType) {
        self.allocator.dealloc_ref(slot);
    }

    #[inline(always)]
    fn unleak_slot_id(&'a self, slot_id: u32) {
        self.allocator.dealloc_id(slot_id);
    }

    #[inline(always)]
    fn available_elements_count(&self) -> usize {
        self.queue.available_elements_count()
    }

    #[inline(always)]
    fn max_size(&self) -> usize {
        BUFFER_SIZE
    }

    fn debug_info(&self) -> String {
        todo!()
    }
}


impl<'a, SlotType:          'a + Debug + Sync + Send,
         OgreAllocatorType: BoundedOgreAllocator<SlotType>,
         const BUFFER_SIZE: usize>
MetaSubscriber<'a, SlotType> for
FullSyncZeroCopy<SlotType, OgreAllocatorType, BUFFER_SIZE> {

    #[inline(always)]
    fn consume<GetterReturnType: 'a,
               GetterFn:                   FnOnce(&SlotType) -> GetterReturnType,
               ReportEmptyFn:              Fn() -> bool,
               ReportLenAfterDequeueingFn: FnOnce(i32)>
              (&self,
               getter_fn:                      GetterFn,
               report_empty_fn:                ReportEmptyFn,
               report_len_after_dequeueing_fn: ReportLenAfterDequeueingFn)
              -> Option<GetterReturnType> {

        match self.consume_leaking() {
            Some( (slot_ref, slot_id) ) => {
                let len_after_dequeueing = self.queue.available_elements_count() as i32;
                let ret_val = getter_fn(slot_ref);
                self.release_leaked_id(slot_id);
                report_len_after_dequeueing_fn(len_after_dequeueing);
                Some(ret_val)
            },
            None => {
                report_empty_fn();
                None
            },
        }
    }

    #[inline(always)]
    fn consume_leaking(&'a self) -> Option<(/*ref:*/ &'a SlotType, /*id: */u32)> {
        self.queue.consume_movable().map(|slot_id| (&*self.allocator.ref_from_id(slot_id), slot_id))
    }

    #[inline(always)]
    fn release_leaked_ref(&'a self, slot: &'a SlotType) {
        let mutable_slot = unsafe { &mut *(*(slot as *const SlotType as *const std::cell::UnsafeCell<SlotType>)).get() };
        self.allocator.dealloc_ref(mutable_slot);
    }

    #[inline(always)]
    fn release_leaked_id(&'a self, slot_id: u32) {
        self.allocator.dealloc_id(slot_id);
    }

    #[inline(always)]
    fn remaining_elements_count(&self) -> usize {
        self.available_elements_count()
    }


    #[inline(always)]
    unsafe fn peek_remaining(&self) -> Vec<&SlotType> {
        self.queue.peek_remaining().iter()
            .flat_map(|&slice| slice)
            .map(|slot_id| self.allocator.ref_from_id(*slot_id) as &SlotType)
            .collect()
    }
}



#[cfg(any(test,doc))]
mod tests {
    //! Unit tests for [full_sync_zero_copy](super) module

    //use super::*;

}
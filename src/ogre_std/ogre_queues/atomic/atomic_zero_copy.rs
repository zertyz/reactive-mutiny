//! Resting place for [AtomicZeroCopy]

use crate::ogre_std::{
    ogre_queues::{
        atomic::atomic_move::AtomicMove,
        meta_publisher::{MetaPublisher,MovePublisher},
        meta_subscriber::{MetaSubscriber,MoveSubscriber},
        meta_container::{MetaContainer, MoveContainer},
    },
    ogre_alloc::BoundedOgreAllocator,
};
use std::{
    fmt::Debug,
    num::NonZeroU32,
    marker::PhantomData,
    sync::Arc,
};


/// Basis for multiple producer / multiple consumer queues using atomics for synchronization,
/// allowing enqueueing syncing to be (almost) fully detached from the dequeueing syncing.
///
/// This queue implements the "zero-copy patterns" through [ZeroCopyPublisher] & [ZeroCopySubscriber] and
/// is a good fit for payloads > 1k.
///
/// For thinner payloads, [AtomicMove] should be a better fit, as it doesn't require a secondary container to
/// hold the objects.
pub struct AtomicZeroCopy<SlotType:          Debug,
                          OgreAllocatorType: BoundedOgreAllocator<SlotType>,
                          const BUFFER_SIZE: usize> {

    pub(crate) allocator: Arc<OgreAllocatorType>,
               queue:     AtomicMove<u32, BUFFER_SIZE>,
               _phantom:  PhantomData<SlotType>
}


impl<'a, SlotType:          'a + Debug,
         OgreAllocatorType: BoundedOgreAllocator<SlotType> + 'a,
         const BUFFER_SIZE: usize>
MetaContainer<'a, SlotType> for
AtomicZeroCopy<SlotType, OgreAllocatorType, BUFFER_SIZE> {

    fn new() -> Self {
        Self {
            allocator: Arc::new(OgreAllocatorType::new()),
            queue:     AtomicMove::new(),
            _phantom:  PhantomData,
        }
    }
}


impl<'a, SlotType:          'a + Debug,
         OgreAllocatorType: BoundedOgreAllocator<SlotType> + 'a,
         const BUFFER_SIZE: usize>
MetaPublisher<'a, SlotType> for
AtomicZeroCopy<SlotType, OgreAllocatorType, BUFFER_SIZE> {

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
            },
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


impl<'a, SlotType:          'a + Debug,
         OgreAllocatorType: BoundedOgreAllocator<SlotType> + 'a,
         const BUFFER_SIZE: usize>
MetaSubscriber<'a, SlotType> for
AtomicZeroCopy<SlotType, OgreAllocatorType, BUFFER_SIZE> {

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
    //! Unit tests for [atomic_zero_copy](super) module

    use super::*;
    use crate::ogre_std::ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator;
    use crate::ogre_std::test_commons;
    use crate::ogre_std::test_commons::{Blocking, ContainerKind};

    type QueueType<SlotType, const SIZE: usize = 1024> = AtomicZeroCopy::<SlotType, OgreArrayPoolAllocator<SlotType, AtomicMove<u32, SIZE>, SIZE>, SIZE>;

    #[cfg_attr(not(doc),test)]
    fn basic_queue_use_cases() {
        let queue = QueueType::<i32>::new();
        test_commons::basic_container_use_cases("Zero-Copy API",
                                                ContainerKind::Queue, Blocking::NonBlocking, queue.max_size(),
                                                |e| queue.publish_movable(e).0.is_some(),
                                                || queue.consume_leaking().map(|(slot, id)| {
                                                    let val = *slot;
                                                    queue.release_leaked_id(id);
                                                    val
                                                }),
                                                || queue.available_elements_count());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    fn single_producer_multiple_consumers() {
        let queue = QueueType::<u32>::new();
        test_commons::container_single_producer_multiple_consumers("Zero-Copy API",
                                                                   |e| queue.publish_movable(e).0.is_some(),
                                                                   || queue.consume_leaking().map(|(slot, id)| {
                                                                       let val = *slot;
                                                                       queue.release_leaked_id(id);
                                                                       val
                                                                   }));
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    fn multiple_producers_single_consumer() {
        let queue = QueueType::<u32>::new();
        test_commons::container_multiple_producers_single_consumer("Zero-Copy API",
                                                                   |e| queue.publish_movable(e).0.is_some(),
                                                                   || queue.consume_leaking().map(|(slot, id)| {
                                                                       let val = *slot;
                                                                       queue.release_leaked_id(id);
                                                                       val
                                                                   }));
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    pub fn multiple_producers_and_consumers_all_in_and_out() {
        let queue = QueueType::<u32>::new();
        test_commons::container_multiple_producers_and_consumers_all_in_and_out("Zero-Copy API",
                                                                                Blocking::NonBlocking,
                                                                                queue.max_size(),
                                                                                |e| queue.publish_movable(e).0.is_some(),
                                                                                || queue.consume_leaking().map(|(slot, id)| {
                                                                                    let val = *slot;
                                                                                    queue.release_leaked_id(id);
                                                                                    val
                                                                                }));
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    pub fn multiple_producers_and_consumers_single_in_and_out() {
        let queue = QueueType::<u32>::new();
        test_commons::container_multiple_producers_and_consumers_single_in_and_out("Zero-Copy API",
                                                                                   |e| queue.publish_movable(e).0.is_some(),
                                                                                   || queue.consume_leaking().map(|(slot, id)| {
                                                                                       let val = *slot;
                                                                                       queue.release_leaked_id(id);
                                                                                       val
                                                                                   }));
    }

    #[cfg_attr(not(doc),test)]
    pub fn peek_test() {
        let queue = QueueType::<u32, 16>::new();
        test_commons::peak_remaining("Zero-Copy API",
                                     |e| queue.publish_movable(e).0.is_some(),
                                     || queue.consume_leaking().map(|(slot, id)| {
                                         let val = *slot;
                                         queue.release_leaked_id(id);
                                         val
                                     }),
                                     || unsafe { ( queue.peek_remaining().into_iter(), [].into_iter() ) } );
    }


    #[cfg_attr(not(doc),test)]
    fn indexes_and_references_conversions() {
        let queue = QueueType::<u32>::new();
        let Some((first_item, _slot_id)) = queue.leak_slot() else {
            panic!("Can't determine the reference for the element at #0");
        };
        test_commons::indexes_and_references_conversions(first_item,
                                                         |index| queue.allocator.ref_from_id(index),
                                                         |slot_ref: &u32| queue.allocator.id_from_ref(slot_ref));
    }

}

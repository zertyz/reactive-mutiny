//! Resting place for [FullSyncMove]

use super::super::{
    super::ogre_sync,
    meta_publisher::MovePublisher,
    meta_subscriber::MoveSubscriber,
    meta_container::MoveContainer,
};
use std::{
    fmt::Debug,
    mem::{ManuallyDrop, MaybeUninit},
    ptr,
    sync::atomic::{
        AtomicBool,
        Ordering::Relaxed,
    },
    pin::Pin,
    num::NonZeroU32,
};


/// Basis for multiple producer / multiple consumer queues using a quick-and-dirty (but fast)
/// full synchronization through an atomic flag, with a clever & experimentally tuned efficient locking mechanism.
///
/// This queue implements the "movable pattern" through [MovePublisher] & [MoveSubscriber] and is a good fit
/// for raw thin payloads < 1k.
///
/// For fatter payloads, [FullSyncZeroCopy] should be a better fit.
#[repr(C,align(128))]      // aligned to cache line sizes for a careful control over false-sharing performance degradation
pub struct FullSyncMove<SlotType:          Debug,
                        const BUFFER_SIZE: usize> {

    head: u32,
    tail: u32,
    /// guards critical regions to allow concurrency
    concurrency_guard: AtomicBool,
    /// holder for the elements
    buffer: Pin<Box<[ManuallyDrop<SlotType>; BUFFER_SIZE]>>,

}

impl<'a, SlotType:          'a + Debug,
         const BUFFER_SIZE: usize>
MoveContainer<SlotType> for
FullSyncMove<SlotType, BUFFER_SIZE> {

    fn new() -> Self {
        Self::BUFFER_SIZE_MUST_BE_A_POWER_OF_2;     // ignore the compiler warning regarding 'path statement having no effect' -- it does: assures no non-power of 2 buffer may be used
        // if !BUFFER_SIZE.is_power_of_two() {
        //     panic!("FullSyncMeta: BUFFER_SIZE must be a power of 2, but {BUFFER_SIZE} was provided.");
        // }
        Self {
            head:              0,
            tail:              0,
            concurrency_guard: AtomicBool::new(false),
            buffer:            Box::pin(unsafe { MaybeUninit::zeroed().assume_init() }),
        }
    }

}

impl<'a, SlotType:          'a + Debug,
         const BUFFER_SIZE: usize>
MovePublisher<SlotType> for
FullSyncMove<SlotType, BUFFER_SIZE> {

    #[inline(always)]
    fn publish_movable(&self, item: SlotType) -> Option<NonZeroU32> {
        match self.leak_slot_internal(|| false) {
            Some( (slot, len_before) ) => {
                unsafe { ptr::write(slot, item); }
                self.publish_leaked_internal();
                ogre_sync::unlock(&self.concurrency_guard);
                NonZeroU32::new(len_before+1)
            },
            None => None,
        }
    }

    #[inline(always)]
    fn publish<SetterFn:                   FnOnce(&mut SlotType),
               ReportFullFn:               Fn() -> bool,
               ReportLenAfterEnqueueingFn: FnOnce(u32)>
              (&self, setter_fn:                      SetterFn,
                      report_full_fn:                 ReportFullFn,
                      report_len_after_enqueueing_fn: ReportLenAfterEnqueueingFn)
              -> bool {

        match self.leak_slot_internal(report_full_fn) {
            Some( (slot_ref, len_before) ) => {
                setter_fn(slot_ref);
                self.publish_leaked_internal();
                ogre_sync::unlock(&self.concurrency_guard);
                report_len_after_enqueueing_fn(len_before+1);
                true
            }
            None => false
        }
    }

    #[inline(always)]
    fn available_elements_count(&self) -> usize {
        self.tail.overflowing_sub(self.head).0 as usize
    }

    #[inline(always)]
    fn max_size(&self) -> usize {
        BUFFER_SIZE
    }

    fn debug_info(&self) -> String {
        let Self {concurrency_guard, tail, buffer: _, head} = self;
        let concurrency_guard = concurrency_guard.load(Relaxed);
        format!("ogre_queues::full_sync_meta's state: {{head: {head}, tail: {tail}, (len: {}), locked: {concurrency_guard}, elements: {{{}}}'}}",
                self.available_elements_count(),
                unsafe {self.peek_remaining()}.iter().flat_map(|&slice| slice).fold(String::new(), |mut acc, e| {
                    acc.push_str(&format!("'{:?}',", e));
                    acc
                }))
    }
}

impl<'a, SlotType:          'a + Debug,
         const BUFFER_SIZE: usize>
MoveSubscriber<SlotType> for
FullSyncMove<SlotType, BUFFER_SIZE> {

    #[inline(always)]
    fn consume_movable(&self) -> Option<SlotType> {
        match self.consume_leaking_internal(|| false) {
            Some( (slot_ref, _len_before) ) => {
                let item = unsafe { Some(ptr::read(slot_ref)) };
                self.release_leaked_internal();
                ogre_sync::unlock(&self.concurrency_guard);
                item
            }
            None => None,
        }
    }


    #[inline(always)]
    unsafe fn peek_remaining(&self) -> [&[SlotType];2] {
        let head_index = self.head as usize % BUFFER_SIZE;
        let tail_index = self.tail as usize % BUFFER_SIZE;
        if self.head == self.tail {
            [&[],&[]]
        } else if head_index < tail_index {
            unsafe {
                let const_ptr = self.buffer.as_ptr();
                let ptr = const_ptr as *const [SlotType; BUFFER_SIZE];
                let array = &*ptr;
                [&array[head_index ..tail_index], &[]]
            }
        } else {
            unsafe {
                let const_ptr = self.buffer.as_ptr();
                let ptr = const_ptr as *const [SlotType; BUFFER_SIZE];
                let array = &*ptr;
                [&array[head_index..BUFFER_SIZE], &array[0..tail_index]]
            }
        }
    }

}

impl<'a, SlotType:          'a + Debug,
         const BUFFER_SIZE: usize>
FullSyncMove<SlotType, BUFFER_SIZE> {

    /// The ring buffer is required to be a power of 2, so `head` and `tail` may wrap over flawlessly
    const BUFFER_SIZE_MUST_BE_A_POWER_OF_2: usize = 0 / if BUFFER_SIZE.is_power_of_two() {1} else {0};


    /// gets hold of one of the slots available in the pool, LEAVING THE LOCK IN THE ACQUIRED STATE IN CASE IT SUCCEEDS,\
    /// returning: (a mutable reference to the data, the current count of elements available for consumption).\
    /// If the pool is empty, `report_full_fn()` is called to inform the condition. If it was able to release any items,
    /// it should return `true`, which will cause this algorithm to try again instead of giving up with `None`.\
    /// This implementation enables other slots to be returned to the pool while there are allocated (but still unpublished) slots around,
    /// allowing the publication & consumption operations not happen in parallel.
    #[inline(always)]
    fn leak_slot_internal(&self, report_full_fn: impl Fn() -> bool) -> Option<(&'a mut SlotType, /*len_before:*/ u32)> {
        let mutable_buffer = unsafe {
            let const_ptr = self.buffer.as_ptr();
            let mut_ptr = const_ptr as *mut [SlotType; BUFFER_SIZE];
            &mut *mut_ptr
        };
        let mut len_before;
        loop {
            ogre_sync::lock(&self.concurrency_guard);
            len_before = self.tail.overflowing_sub(self.head).0;
            if len_before < BUFFER_SIZE as u32 {
                break unsafe { Some( (mutable_buffer.get_unchecked_mut(self.tail as usize % BUFFER_SIZE), len_before) ) }
            } else {
                ogre_sync::unlock(&self.concurrency_guard);
                let maybe_no_longer_full = report_full_fn();
                if !maybe_no_longer_full {
                    break None;
                }
            }
        }
    }

    /// marks the next slot in the (ring buffer) as ready to be consumed, completing the publishing pattern
    /// that started with a call to [leak_slot_internal()].\
    /// -- assumes the lock is in the acquired state
    #[inline(always)]
    fn publish_leaked_internal(&self) {
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        mutable_self.tail = self.tail.overflowing_add(1).0;
    }

    /// adds back to the pool the slot just acquired by [leak_slot_internal()], disrupting the publishing pattern\
    /// -- assumes the lock is in the acquired state, leaving it untouched
    #[inline(always)]
    fn unleak_internal(&self) {
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        mutable_self.tail = self.tail.overflowing_sub(1).0;
    }

    /// gets hold of a reference to the slot containing the next data to be processed, LEAVING THE LOCK IN THE ACQUIRED STATE IN CASE IT SUCCEEDS,\
    /// returning: (a mutable reference to the data, the slot id, the count of elements that were available for consumption before this method was executed).\
    /// If the queue is empty, `report_empty_fn()` is called to inform the condition. If it was able to produce any items, it should return true,
    /// which will cause this algorithm to try again instead of giving up with `None`.\
    /// This implementation enables other slots to be allocated from the pool while there are references (still not released) hanging around,
    /// allowing the publication & consumption operations not happen in parallel.
    #[inline(always)]
    fn consume_leaking_internal(&self, report_empty_fn: impl Fn() -> bool) -> Option<(&'a mut SlotType, /*len_before:*/ i32)> {
        let mutable_buffer = unsafe {
            let const_ptr = self.buffer.as_ptr();
            let mut_ptr = const_ptr as *mut [SlotType; BUFFER_SIZE];
            &mut *mut_ptr
        };
        let mut len_before;
        loop {
            ogre_sync::lock(&self.concurrency_guard);
            len_before = self.available_elements_count() as i32;
            if len_before > 0 {
                break unsafe { Some( (mutable_buffer.get_unchecked_mut(self.head as usize % BUFFER_SIZE), len_before) ) }
            } else {
                ogre_sync::unlock(&self.concurrency_guard);
                let maybe_no_longer_empty = report_empty_fn();
                if !maybe_no_longer_empty {
                    break None;
                }
            }
        }
    }

    /// marks the slot just returned by [consume_leaking_internal()] as being available to the pool, for reuse, after the referenced data has been processed (aka, consumed)\
    /// -- assumes the lock is in the acquire state, leaving it untouched
    #[inline(always)]
    fn release_leaked_internal(&self) {
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        mutable_self.head = self.head.overflowing_add(1).0;
    }

    /// returns the id (within the Ring Buffer) that the given `slot` reference occupies
    #[inline(always)]
    fn slot_id_from_slot_ref(&'a self, slot: &'a SlotType) -> u32 {
        (( unsafe { (slot as *const SlotType).offset_from(self.buffer.as_ptr() as *const SlotType) } ) as usize) as u32
    }

    /// returns a reference to the slot pointed to by `slot_id`
    #[inline(always)]
    fn _slot_ref_from_slot_id(&'a self, slot_id: u32) -> &'a SlotType {
        unsafe { self.buffer.get_unchecked(slot_id as usize % BUFFER_SIZE) }
    }

}

// buffered elements are `ManuallyDrop<SlotType>`, so here is where we drop any unconsumed ones
impl<SlotType:          Debug,
     const BUFFER_SIZE: usize>
Drop for
FullSyncMove<SlotType, BUFFER_SIZE> {

    fn drop(&mut self) {
        loop {
            match self.consume_movable() {
                None => break,
                Some(item) => drop(item),
            }
        }
    }
}

#[cfg(any(test,doc))]
mod tests {
    //! Unit tests for [full_sync_meta](super) module

    use super::*;
    use crate::ogre_std::test_commons::{self, ContainerKind,Blocking};

    #[cfg_attr(not(doc),test)]
    fn basic_queue_use_cases() {
        let queue = FullSyncMove::<i32, 16>::new();
        test_commons::basic_container_use_cases("Movable API",
                                                ContainerKind::Queue, Blocking::NonBlocking, queue.max_size(),
                                                |e| queue.publish_movable(e).is_some(),
                                                || queue.consume_movable(),
                                                || queue.available_elements_count());
        test_commons::basic_container_use_cases("Zero-Copy Producer/Movable Subscriber API",
                                                ContainerKind::Queue, Blocking::NonBlocking, queue.max_size(),
                                                |e| queue.publish(|slot| *slot = e, || false, |_| {}),
                                                || queue.consume_movable(),
                                                || queue.available_elements_count());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    fn single_producer_multiple_consumers() {
        let queue = FullSyncMove::<u32, 65536>::new();
        test_commons::container_single_producer_multiple_consumers("Movable API",
                                                                   |e| queue.publish_movable(e).is_some(),
                                                                   || queue.consume_movable());
        test_commons::container_single_producer_multiple_consumers("Zero-Copy Producer/Movable Subscriber API",
                                                                   |e| queue.publish(|slot| *slot = e, || false, |_| {}),
                                                                   || queue.consume_movable());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    fn multiple_producers_single_consumer() {
        let queue = FullSyncMove::<u32, 65536>::new();
        test_commons::container_multiple_producers_single_consumer("Movable API",
                                                                   |e| queue.publish_movable(e).is_some(),
                                                                   || queue.consume_movable());
        test_commons::container_multiple_producers_single_consumer("Zero-Copy Producer/Movable Subscriber API",
                                                                   |e| queue.publish(|slot| *slot = e, || false, |_| {}),
                                                                   || queue.consume_movable());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    pub fn multiple_producers_and_consumers_all_in_and_out() {
        let queue = FullSyncMove::<u32, 65536>::new();
        test_commons::container_multiple_producers_and_consumers_all_in_and_out("Movable API",
                                                                                Blocking::NonBlocking,
                                                                                queue.max_size(),
                                                                                |e| queue.publish_movable(e).is_some(),
                                                                                || queue.consume_movable());
        test_commons::container_multiple_producers_and_consumers_all_in_and_out("Zero-Copy Producer/Movable Subscriber API",
                                                                                Blocking::NonBlocking,
                                                                                queue.max_size(),
                                                                                |e| queue.publish(|slot| *slot = e, || false, |_| {}),
                                                                                || queue.consume_movable());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    pub fn multiple_producers_and_consumers_single_in_and_out() {
        let queue = FullSyncMove::<u32, 65536>::new();
        test_commons::container_multiple_producers_and_consumers_single_in_and_out("Movable API",
                                                                                   |e| queue.publish_movable(e).is_some(),
                                                                                   || queue.consume_movable());
        test_commons::container_multiple_producers_and_consumers_single_in_and_out("Zero-Copy Producer/Movable Subscriber API",
                                                                                   |e| queue.publish(|slot| *slot = e, || false, |_| {}),
                                                                                   || queue.consume_movable());
    }

    #[cfg_attr(not(doc),test)]
    pub fn peek_test() {
        let queue = FullSyncMove::<u32, 16>::new();
        test_commons::peak_remaining("Movable API",
                                     |e| queue.publish_movable(e).is_some(),
                                     || queue.consume_movable(),
                                     || unsafe { queue.peek_remaining() } );
        test_commons::peak_remaining("Zero-Copy Producer/Movable Subscriber API",
                                     |e| queue.publish(|slot| *slot = e, || false, |_| {}),
                                     || queue.consume_movable(),
                                     || unsafe { queue.peek_remaining() } );
    }

}
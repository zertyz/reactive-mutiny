//! Resting place for [FullSyncZeroCopy]

use crate::ogre_std::{
    ogre_queues::{
        meta_publisher::MetaPublisher,
        meta_subscriber::MetaSubscriber,
        meta_container::MetaContainer,
    },
    ogre_sync,
};
use std::{
    fmt::Debug,
    mem::{ManuallyDrop, MaybeUninit},
    sync::atomic::{
        AtomicBool,
        Ordering::Relaxed,
    },
};


/// Basis for multiple producer / multiple consumer queues using a quick-and-dirty (but fast)
/// full synchronization through an atomic flag, with a clever & experimentally tuned efficient locking mechanism.
///
/// This queue implements the "zero-copy patterns" through [ZeroCopyPublisher] & [ZeroCopySubscriber] and
/// is a good fit for payloads > 1k.
///
/// For thinner payloads, [FullSyncMove] should be a better fit, as it doesn't require a secondary container to
/// hold the objects.
#[repr(C,align(128))]      // aligned to cache line sizes for a careful control over false-sharing performance degradation
pub struct FullSyncZeroCopy<SlotType,
                            const BUFFER_SIZE: usize> {

    head: u32,
    tail: u32,
    /// guards critical regions to allow concurrency
    concurrency_guard: AtomicBool,
    /// holder for the elements
    buffer: [ManuallyDrop<SlotType>; BUFFER_SIZE],

}

impl<'a, SlotType:          'a + Debug,
         const BUFFER_SIZE: usize>
MetaContainer<'a, SlotType> for
FullSyncZeroCopy<SlotType, BUFFER_SIZE> {

    fn new() -> Self {
        Self::BUFFER_SIZE_MUST_BE_A_POWER_OF_2;
        // if !BUFFER_SIZE.is_power_of_two() {
        //     panic!("FullSyncMeta: BUFFER_SIZE must be a power of 2, but {BUFFER_SIZE} was provided.");
        // }
        Self {
            head:              0,
            tail:              0,
            concurrency_guard: AtomicBool::new(false),
            buffer:            unsafe { MaybeUninit::zeroed().assume_init() },
        }
    }

}

impl<'a, SlotType:          'a + Debug,
         const BUFFER_SIZE: usize>
MetaPublisher<'a, SlotType> for
FullSyncZeroCopy<SlotType, BUFFER_SIZE> {

    #[inline(always)]
    fn publish<SetterFn:                   FnOnce(&'a mut SlotType),
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
    fn leak_slot(&self) -> Option<&SlotType> {
        self.leak_slot_internal(|| false)
            .map(|(slot_ref, _len_before)| {
                ogre_sync::unlock(&self.concurrency_guard);
                &*slot_ref
            })
    }

    #[inline(always)]
    fn publish_leaked(&'a self, slot: &'a SlotType) {
        let slot_id = self.slot_id_from_slot_ref(slot);
        ogre_sync::lock(&self.concurrency_guard);
        debug_assert_eq!(self.tail, slot_id, "FullSyncMeta: Use of the `leak_slot()` / `publish_leaked()` pattern is only available for single-threaded producers -- and it seems not to be the case: `slot_id` and expected `tail` don't match");
        self.publish_leaked_internal();
        ogre_sync::unlock(&self.concurrency_guard);
    }

    #[inline(always)]
    fn unleak_slot(&'a self, slot: &'a SlotType) {
        let slot_id = self.slot_id_from_slot_ref(slot);
        ogre_sync::lock(&self.concurrency_guard);
        debug_assert_eq!(self.tail, slot_id, "FullSyncMeta: `unleak_slot()` was called after another slot has been published / leaked: `slot_id` and expected `tail` don't match");
        self.unleak_internal();
        ogre_sync::unlock(&self.concurrency_guard);
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
MetaSubscriber<'a, SlotType> for
FullSyncZeroCopy<SlotType, BUFFER_SIZE> {

    #[inline(always)]
    fn consume<GetterReturnType: 'a,
               GetterFn:                   FnOnce(&'a mut SlotType) -> GetterReturnType,
               ReportEmptyFn:              Fn() -> bool,
               ReportLenAfterDequeueingFn: FnOnce(i32)>
              (&self,
               getter_fn:                      GetterFn,
               report_empty_fn:                ReportEmptyFn,
               report_len_after_dequeueing_fn: ReportLenAfterDequeueingFn)
              -> Option<GetterReturnType> {

        match self.consume_leaking_internal(report_empty_fn) {
            Some( (slot_ref, len_before) ) => {
                let ret_val = getter_fn(slot_ref);
                self.release_leaked_internal();
                ogre_sync::unlock(&self.concurrency_guard);
                report_len_after_dequeueing_fn(len_before-1);
                Some(ret_val)
            },
            None => None,
        }
    }

    fn consume_leaking(&'a self) -> Option<&'a SlotType> {
        self.consume_leaking_internal(|| false)
            .map(|(slot_ref, _len_before)| &*slot_ref)
    }

    fn release_leaked(&'a self, slot: &'a SlotType) {
        let slot_id = self.slot_id_from_slot_ref(slot);
        ogre_sync::lock(&self.concurrency_guard);
        debug_assert_eq!(self.head, slot_id, "FullSyncMeta: Use of the `consume_leaking()` / `release_leaked()` pattern is only available for single-threaded consumers -- and it seems not to be the case: `slot_id` and expected `head` don't match");
        self.release_leaked_internal();
        ogre_sync::unlock(&self.concurrency_guard);

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
FullSyncZeroCopy<SlotType, BUFFER_SIZE> {

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
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        let mut len_before;
        loop {
            ogre_sync::lock(&self.concurrency_guard);
            len_before = self.tail.overflowing_sub(self.head).0;
            if len_before < BUFFER_SIZE as u32 {
                break unsafe { Some( (mutable_self.buffer.get_unchecked_mut(self.tail as usize % BUFFER_SIZE), len_before) ) }
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
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        let mut len_before;
        loop {
            ogre_sync::lock(&self.concurrency_guard);
            len_before = self.available_elements_count() as i32;
            if len_before > 0 {
                break unsafe { Some( (mutable_self.buffer.get_unchecked_mut(self.head as usize % BUFFER_SIZE), len_before) ) }
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


#[cfg(any(test,doc))]
mod tests {
    //! Unit tests for [full_sync_meta](super) module

    use super::{
        *,
        super::super::super::test_commons::measure_syncing_independency
    };
    use std::sync::{
        Arc,
        atomic::AtomicU32,
    };

    /// the full sync base queue should not allow independent produce/consume operations (one blocks the other while it is happening)
    #[cfg_attr(not(doc),test)]
    #[ignore]   // time-dependent: should run on single-threaded tests
    pub fn assure_syncing_dependency() {
        let queue = Arc::new(FullSyncZeroCopy::<u32, 128>::new());
        let queue_ref1 = Arc::clone(&queue);
        let queue_ref2 = Arc::clone(&queue);
        let (_independent_productions_count, dependent_productions_count, _independent_consumptions_count, _dependent_consumptions_count) = measure_syncing_independency(
            |i, callback: &dyn Fn()| {
                queue_ref1.publish(
                    |slot| {
                        *slot = i;
                        callback();
                    },
                    || false,
                    |_| {}
                )
            },
            |result: &AtomicU32, callback: &dyn Fn()| {
                queue_ref2.consume(
                    |slot| {
                        result.store(*slot, Relaxed);
                        callback();
                    },
                    || false,
                    |_| {}
                )
            }
        );

        assert!(dependent_productions_count > 0, "This queue should wait for dequeue to finish before a new dequeueing is allowed to happen. We are not seeing this behavior here...");
    }

}
//! Resting place for [AtomicMove]

use super::super::{
    meta_publisher::MovePublisher,
    meta_subscriber::MoveSubscriber,
    meta_container::MoveContainer,
};
use std::{fmt::Debug, sync::atomic::{
    AtomicU32,
    Ordering::{Relaxed, Release},
}, ptr, cell::UnsafeCell, num::NonZeroU32, pin::Pin, mem::ManuallyDrop};
use crossbeam::utils::CachePadded;


/// Basis for multiple producer / multiple consumer queues using atomics for synchronization,
/// allowing enqueueing syncing to be (almost) fully detached from the dequeueing syncing.
///
/// This queue implements the "movable pattern" through [MovePublisher] & [MoveSubscriber] and
/// is a good fit for raw thin payload < 1k.
///
/// For fatter payloads, [AtomicZeroCopy] should be a better fit.
pub struct AtomicMove<SlotType:          Debug + Default,
                      const BUFFER_SIZE: usize> {
    /// marks the first element of the queue, ready for dequeue -- increasing when dequeues are complete
    pub(crate) head: CachePadded<AtomicU32>,
    /// increase-before-load field, marking where the next element should be written to
    /// -- may receed if exceeded the buffer capacity
    pub(crate) enqueuer_tail: CachePadded<AtomicU32>,
    /// holder for the queue elements
    pub(crate) buffer: UnsafeCell<Pin<Box<[ManuallyDrop<SlotType>; BUFFER_SIZE]>>>,
    /// increase-before-load field, marking where the next element should be retrieved from
    /// -- may receed if it gets ahead of the published `tail`
    pub(crate) dequeuer_head: CachePadded<AtomicU32>,
    /// marks the last element of the queue, ready for dequeue -- increasing when enqueues are complete
    pub(crate) tail: CachePadded<AtomicU32>,
}

impl<'a, SlotType:          'a + Debug + Default,
         const BUFFER_SIZE: usize>
MoveContainer<SlotType> for
AtomicMove<SlotType, BUFFER_SIZE> {

    fn new() -> Self {
        Self::with_initializer(|| SlotType::default())
    }

    fn with_initializer<F: Fn() -> SlotType>(slot_initializer: F) -> Self {
        debug_assert!(Self::BUFFER_SIZE_MUST_BE_A_POWER_OF_2==0);     // assures no non-power of 2 buffer may be used
        // if !BUFFER_SIZE.is_power_of_two() {
        //     panic!("FullSyncMeta: BUFFER_SIZE must be a power of 2, but {BUFFER_SIZE} was provided.");
        // }
        Self {
            head:                 CachePadded::new(AtomicU32::new(0)),
            tail:                 CachePadded::new(AtomicU32::new(0)),
            dequeuer_head:        CachePadded::new(AtomicU32::new(0)),
            enqueuer_tail:        CachePadded::new(AtomicU32::new(0)),
            buffer:               UnsafeCell::new(Box::pin([0; BUFFER_SIZE].map(|_| ManuallyDrop::new(slot_initializer())))),
        }
    }
}

impl<'a, SlotType:          'a + Debug + Default,
         const BUFFER_SIZE: usize>
MovePublisher<SlotType> for
AtomicMove<SlotType, BUFFER_SIZE> {

    #[inline(always)]
    fn publish_movable(&self, item: SlotType) -> (Option<NonZeroU32>, Option<SlotType>) {
        match self.leak_slot_internal(|| false) {
            Some( (slot_ref, slot_id, len_before) ) => {
                unsafe { ptr::write(slot_ref, item); }
                self.publish_leaked_internal(slot_id);
                (NonZeroU32::new(len_before+1), None)
            },
            None => (None, Some(item)),
        }
    }

    #[inline(always)]
    fn publish<SetterFn:                   FnOnce(&mut SlotType),
               ReportFullFn:               Fn() -> bool,
               ReportLenAfterEnqueueingFn: FnOnce(u32)>
              (&self,
               setter_fn:                      SetterFn,
               report_full_fn:                 ReportFullFn,
               report_len_after_enqueueing_fn: ReportLenAfterEnqueueingFn)
              -> Option<SetterFn> {

        match self.leak_slot_internal(report_full_fn) {
            Some( (slot_ref, slot_id, len_before) ) => {
                setter_fn(slot_ref);
                self.publish_leaked_internal(slot_id);
                report_len_after_enqueueing_fn(len_before+1);
                None
            }
            None => Some(setter_fn),
        }
    }

    #[inline(always)]
    fn available_elements_count(&self) -> usize {
        self.tail.load(Relaxed).overflowing_sub(self.head.load(Relaxed)).0 as usize
    }

    fn max_size(&self) -> usize {
        BUFFER_SIZE
    }

    fn debug_info(&self) -> String {
        let Self {head, tail, dequeuer_head, enqueuer_tail, buffer: _} = self;
        let head = head.load(Relaxed);
        let tail = tail.load(Relaxed);
        let dequeuer_head = dequeuer_head.load(Relaxed);
        let enqueuer_tail = enqueuer_tail.load(Relaxed);
        format!("ogre_queues::atomic_meta's state: {{head: {head}, tail: {tail}, dequeuer_head: {dequeuer_head}, enqueuer_tail: {enqueuer_tail}, (len: {}), elements: {{{}}}'}}",
                self.available_elements_count(),
                unsafe {self.peek_remaining()}.iter().flat_map(|&slice| slice).fold(String::new(), |mut acc, e| {
                    acc.push_str(&format!("'{:?}',", e));
                    acc
                }))
    }
}

impl<'a, SlotType:          'a + Debug + Default,
         const BUFFER_SIZE: usize>
MoveSubscriber<SlotType> for
AtomicMove<SlotType, BUFFER_SIZE> {

    #[inline(always)]
    fn consume_movable(&self) -> Option<SlotType> {
        match self.consume_leaking_internal(|| false) {
            Some( (slot_ref, slot_id, _len_before) ) => {
                let item = unsafe { Some(ptr::read(slot_ref)) };
                self.release_leaked_internal(slot_id);
                item
            }
            None => None,
        }
    }

    unsafe fn peek_remaining(&self) -> [&[SlotType]; 2] {
        let head_index = self.head.load(Relaxed) as usize % BUFFER_SIZE;
        let tail_index = self.tail.load(Relaxed) as usize % BUFFER_SIZE;
        if self.head.load(Relaxed) == self.tail.load(Relaxed) {
            [&[],&[]]
        } else if head_index < tail_index {
            unsafe {
                let ptr = self.buffer.get() as *mut Box<[SlotType; BUFFER_SIZE]>;
                let array = &*ptr;
                [&array[head_index ..tail_index], &[]]
            }
        } else {
            unsafe {
                let ptr = self.buffer.get() as *mut Box<[SlotType; BUFFER_SIZE]>;
                let array = &*ptr;
                [&array[head_index..BUFFER_SIZE], &array[0..tail_index]]
            }
        }
    }

}

impl<'a, SlotType:          'a + Debug + Default,
         const BUFFER_SIZE: usize>
AtomicMove<SlotType, BUFFER_SIZE> {

    /// The ring buffer is required to be a power of 2, so `head` and `tail` may wrap over flawlessly
    const BUFFER_SIZE_MUST_BE_A_POWER_OF_2: usize = usize::MAX / if BUFFER_SIZE.is_power_of_two() {1} else {0};


    /// gets hold of one of the slots available in the pool,\
    /// returning: (a mutable reference to the data, the slot id, the current count of elements available for consumption).\
    /// If the pool is empty, `report_full_fn()` is called to inform the condition. If it was able to release any items,
    /// it should return `true`, which will cause this algorithm to try again instead of giving up with `None`.\
    /// This implementation enables other slots to be returned to the pool while there are allocated (but still unpublished) slots around,
    /// allowing the publication & consumption operations not happen in parallel.
    #[inline(always)]
    pub fn leak_slot_internal(&self, report_full_fn: impl Fn() -> bool) -> Option<(&mut SlotType, /*slot_id:*/ u32, /*len_before:*/ u32)> {
        let mutable_buffer = unsafe { &mut * (self.buffer.get() as *mut Box<[SlotType; BUFFER_SIZE]>) };
        let mut slot_id = self.enqueuer_tail.fetch_add(1, Relaxed);
        let mut len_before;
        loop {
            let head = self.head.load(Relaxed);
            len_before = slot_id.overflowing_sub(head).0;
            // is queue not full?
            if len_before < BUFFER_SIZE as u32 {
                break unsafe { Some( (mutable_buffer.get_unchecked_mut(slot_id as usize % BUFFER_SIZE), slot_id, len_before) ) }
            } else {
                // queue is full: reestablish the correct `enqueuer_tail` (receding it to its original value)
                if self.try_unleak_slot_internal(slot_id) {
                    // report the queue is full (allowing a retry) if the method says we recovered from the condition
                    if report_full_fn() {
                        slot_id = self.enqueuer_tail.fetch_add(1, Relaxed);
                    } else {
                        return None;
                    }
                }
            }
        }
    }
    
    /// Marks the given data, under `slot_id`, as ready to be consumed -- completing the publishing pattern
    /// that started with a call to [Self::leak_slot_internal()].\
    /// Keep in mind that `slot_id` is not an index -- as it can grow bigger than `BUFFER_SIZE`.
    /// If you want to use an index instead, see [Self::published_leaked_internal_index()].\
    /// IMPORTANT: use this function only when `slot_id` is guaranteed to progress sequentially
    ///            (this channel requires so) -- otherwise this function may spin indefinitely,
    ///            with deadlock potential if used in async calls with just 1 thread running.
    ///            If you cannot guarantee `slot_id` will progress sequentially, use
    ///            [Self::try_publish_leaked_internal()] instead and implement your own spin loop
    ///            (probably using `tokio::sync::yield_now().await`)
    #[inline(always)]
    pub fn publish_leaked_internal(&'a self, slot_id: u32) {
        while !self.try_publish_leaked_internal(slot_id) {
            relaxed_wait();
        }
    }

    /// Equivalent to [Self::publish_leaked_internal()], but without spinning
    /// (suitable for use by operations that cannot guarantee that `slot_id` will progress sequentially).
    pub fn try_publish_leaked_internal(&'a self, slot_id: u32) -> bool {
        match self.tail.compare_exchange_weak(slot_id, slot_id.overflowing_add(1).0, Release, Relaxed) {
            Ok(_) => true,
            Err(_reloaded_tail) => {
                false
            }
        }
    }

    /// Similar to [Self::try_publish_leaked_internal()], but receives an index (that can only be inside `0..BUFFER_SIZE`)
    /// instead of an id (that may get any value).\
    /// Returns the available elements for consumption after the operation completes, or `None` if you should retry the operation.
    #[inline(always)]
    pub fn try_publish_leaked_internal_index(&'a self, slot_index: u32) -> Option<NonZeroU32> {
        let mut slot_id = slot_index;
        loop {
            match self.tail.compare_exchange_weak(slot_id, slot_id.overflowing_add(1).0, Release, Relaxed) {
                Ok(new_tail) => break NonZeroU32::new(u32::max(1, new_tail.overflowing_sub(self.head.load(Relaxed)).0)),
                Err(reloaded_tail) => {
                    if reloaded_tail / BUFFER_SIZE as u32 > slot_id / BUFFER_SIZE as u32 {
                        // the ring buffer cycled over -- adjust `slot_id` accordingly
                        slot_id = slot_index + ( (reloaded_tail / BUFFER_SIZE as u32) * BUFFER_SIZE as u32 );
                    } else {
                        relaxed_wait();
                        break None
                    }
                },
            }
        }
    }

    /// Attempt to return the slot under `slot_id` to the pool of available slots -- disrupting the publishing of an event
    /// that was started with a call to [Self::leak_slot_internal()].\
    /// Keep in mind that `slot_id` is not an index -- as it can grow bigger than `BUFFER_SIZE`.
    /// If you want to use an index instead, see [Self::try_unleak_slot_index_internal()].
    /// IMPORTANT: for this channel, the reserve cancellation (unleaking) must be done in the reversed order.
    #[inline(always)]
    fn try_unleak_slot_internal(&'a self, slot_id: u32) -> bool {
        match self.enqueuer_tail.compare_exchange_weak(slot_id.overflowing_add(1).0, slot_id, Release, Relaxed) {
            Ok(_) => true,
            Err(_reloaded_enqueuer_tail) => {
                false
            }
        }
    }

    /// Similar to [Self::try_unleak_slot_internal()], but receives an index (that can only be inside `0..BUFFER_SIZE`)
    /// instead of an id (that may get any value)
    #[inline(always)]
    pub fn try_unleak_slot_index_internal(&'a self, slot_index: u32) -> bool {
        let mut slot_id = slot_index;
        loop {
            match self.enqueuer_tail.compare_exchange_weak(slot_id.overflowing_add(1).0, slot_id, Release, Relaxed) {
                Ok(_) => break true,
                Err(reloaded_enqueuer_tail) => {
                    if (reloaded_enqueuer_tail-1) / BUFFER_SIZE as u32 > slot_id / BUFFER_SIZE as u32 {
                        // the ring buffer cycled over -- adjust `slot_id` accordingly
                        slot_id = slot_index + ( ( (reloaded_enqueuer_tail-1) / BUFFER_SIZE as u32) * BUFFER_SIZE as u32 );
                    } else {
                        break false
                    }
                }
            }
        }
    }

    /// gets hold of a reference to the slot containing the next data to be processed,\
    /// returning: (a mutable reference to the data, the slot id, the count of elements that were available for consumption before this method was executed).\
    /// If the queue is empty, `report_empty_fn()` is called to inform the condition. If it was able to produce any items, it should return true,
    /// which will cause this algorithm to try again instead of giving up with `None`.\
    /// This implementation enables other slots to be allocated from the pool while there are references (still not released) hanging around,
    /// allowing the publication & consumption operations not happen in parallel.
    #[inline(always)]
    fn consume_leaking_internal(&self, report_empty_fn: impl Fn() -> bool) -> Option<(&'a mut SlotType, /*slot_id:*/ u32, /*len_before:*/ i32)> {
        let mutable_buffer = unsafe { &mut * (self.buffer.get() as *mut Box<[SlotType; BUFFER_SIZE]>) };

        let mut slot_id = self.dequeuer_head.fetch_add(1, Relaxed);
        let mut len_before;
        loop {
            let tail = self.tail.load(Relaxed);
            len_before = tail.overflowing_sub(slot_id).0 as i32;
            // queue has elements?
            if len_before > 0 {
                let slot_value = unsafe { mutable_buffer.get_unchecked_mut(slot_id as usize % BUFFER_SIZE) };
                break Some( (slot_value, slot_id, len_before) )
            } else {
                // queue is empty: reestablish the correct `dequeuer_head` (receding it to its original value)
                match self.dequeuer_head.compare_exchange_weak(slot_id.overflowing_add(1).0, slot_id, Relaxed, Relaxed) {
                    Ok(_) => {
                        if !report_empty_fn() {
                            return None;
                        } else {
                            slot_id = self.dequeuer_head.fetch_add(1, Relaxed);
                        }
                    },
                    Err(_reloaded_dequeuer_head) => {
                        relaxed_wait();
                    }
                }
            }
        }
    }

    /// makes the `slot_id` available to the pool, for reuse,
    /// after the referenced data has been processed (aka, consumed)
    #[inline(always)]
    pub fn release_leaked_internal(&self, slot_id: u32) {
        loop {
            match self.head.compare_exchange_weak(slot_id, slot_id.overflowing_add(1).0, Relaxed, Relaxed) {
                Ok(_) => break,
                Err(_reloaded_head) => {
                    relaxed_wait();
                }
            }
        }
    }

    /// Returns the index (within the Ring Buffer) that the given `slot` reference occupies
    /// -- this is the reverse operation of [Self::slot_ref_from_slot_index()]
    #[inline(always)]
    pub fn slot_index_from_slot_ref(&'a self, slot: &'a SlotType) -> u32 {
        unsafe {
            let mutable_buffer = &mut * (self.buffer.get() as *mut Box<[SlotType; BUFFER_SIZE]>);
            (slot as *const SlotType).offset_from(mutable_buffer.get_unchecked(0) as *const SlotType) as u32
        }
    }

    /// Returns a reference to the slot pointed to by `slot_index`
    /// -- this is the reverse operation of [Self::slot_index_from_slot_ref()]
    #[inline(always)]
    pub fn slot_ref_from_slot_index(&'a self, slot_index: u32) -> &'a SlotType {
        unsafe {
            let buffer = &*self.buffer.get();
            buffer.get_unchecked(slot_index as usize % BUFFER_SIZE)
        }
    }
}

// buffered elements are `ManuallyDrop<SlotType>`, so here is where we drop any unconsumed ones
impl<SlotType:          Debug + Default,
     const BUFFER_SIZE: usize>
Drop for
AtomicMove<SlotType, BUFFER_SIZE> {

    fn drop(&mut self) {
        loop {
            match self.consume_movable() {
                None => break,
                Some(item) => drop(item),
            }
        }
    }
}

// TODO: 2023-06-14: Needed while `SyncUnsafeCell` is still not stabilized
unsafe impl<SlotType:          Debug + Default,
            const BUFFER_SIZE: usize>
Sync for
AtomicMove<SlotType, BUFFER_SIZE> {}

// TODO: 2023-06-14: Needed while `SyncUnsafeCell` is still not stabilized
unsafe impl<SlotType:          Debug + Default,
            const BUFFER_SIZE: usize>
Send for
AtomicMove<SlotType, BUFFER_SIZE> {}


#[inline(always)]
fn relaxed_wait() {
    std::hint::spin_loop();
}

#[cfg(any(test,doc))]
mod tests {
    //! Unit tests for [atomic_meta](super) module

    use super::*;
    use crate::ogre_std::test_commons::{self, ContainerKind,Blocking};


    #[cfg_attr(not(doc),test)]
    fn basic_queue_use_cases() {
        let queue = AtomicMove::<i32, 16>::new();
        test_commons::basic_container_use_cases("Movable API",
                                                ContainerKind::Queue, Blocking::NonBlocking, queue.max_size(),
                                                |e| queue.publish_movable(e).0.is_some(),
                                                || queue.consume_movable(),
                                                || queue.available_elements_count());
        test_commons::basic_container_use_cases("Zero-Copy Producer/Movable Subscriber API",
                                                ContainerKind::Queue, Blocking::NonBlocking, queue.max_size(),
                                                |e| queue.publish(|slot| *slot = e, || false, |_| {}).is_none(),
                                                || queue.consume_movable(),
                                                || queue.available_elements_count());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    fn single_producer_multiple_consumers() {
        let queue = AtomicMove::<u32, 65536>::new();
        test_commons::container_single_producer_multiple_consumers("Movable API",
                                                                   |e| queue.publish_movable(e).0.is_some(),
                                                                   || queue.consume_movable());
        test_commons::container_single_producer_multiple_consumers("Zero-Copy Producer/Movable Subscriber API",
                                                                   |e| queue.publish(|slot| *slot = e, || false, |_| {}).is_none(),
                                                                   || queue.consume_movable());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    fn multiple_producers_single_consumer() {
        let queue = AtomicMove::<u32, 65536>::new();
        test_commons::container_multiple_producers_single_consumer("Movable API",
                                                                   |e| queue.publish_movable(e).0.is_some(),
                                                                   || queue.consume_movable());
        test_commons::container_multiple_producers_single_consumer("Zero-Copy Producer/Movable Subscriber API",
                                                                   |e| queue.publish(|slot| *slot = e, || false, |_| {}).is_none(),
                                                                   || queue.consume_movable());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    pub fn multiple_producers_and_consumers_all_in_and_out() {
        let queue = AtomicMove::<u32, 65536>::new();
        test_commons::container_multiple_producers_and_consumers_all_in_and_out("Movable API",
                                                                                Blocking::NonBlocking,
                                                                                queue.max_size(),
                                                                                |e| queue.publish_movable(e).0.is_some(),
                                                                                || queue.consume_movable());
        test_commons::container_multiple_producers_and_consumers_all_in_and_out("Zero-Copy Producer/Movable Subscriber API",
                                                                                Blocking::NonBlocking,
                                                                                queue.max_size(),
                                                                                |e| queue.publish(|slot| *slot = e, || false, |_| {}).is_none(),
                                                                                || queue.consume_movable());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    pub fn multiple_producers_and_consumers_single_in_and_out() {
        let queue = AtomicMove::<u32, 65536>::new();
        test_commons::container_multiple_producers_and_consumers_single_in_and_out("Movable API",
                                                                                   |e| queue.publish_movable(e).0.is_some(),
                                                                                   || queue.consume_movable());
        test_commons::container_multiple_producers_and_consumers_single_in_and_out("Zero-Copy Producer/Movable Subscriber API",
                                                                                   |e| queue.publish(|slot| *slot = e, || false, |_| {}).is_none(),
                                                                                   || queue.consume_movable());
    }

    #[cfg_attr(not(doc),test)]
    pub fn peek_test() {
        let queue = AtomicMove::<u32, 16>::new();
        test_commons::peak_remaining("Movable API",
                                     |e| queue.publish_movable(e).0.is_some(),
                                     || queue.consume_movable(),
                                     || unsafe {
                                         let mut iter = queue.peek_remaining().into_iter();
                                         ( iter.next().expect("no item @0").iter(),
                                           iter.next().expect("no item @1").iter() )
                                     } );
        test_commons::peak_remaining("Zero-Copy Producer/Movable Subscriber API",
                                     |e| queue.publish(|slot| *slot = e, || false, |_| {}).is_none(),
                                     || queue.consume_movable(),
                                     || unsafe {
                                         let mut iter = queue.peek_remaining().into_iter();
                                         ( iter.next().expect("no item @0").iter(),
                                           iter.next().expect("no item @1").iter() )
                                     } );
    }

    #[cfg_attr(not(doc),test)]
    fn indexes_and_references_conversions() {
        let queue = AtomicMove::<i32, 1024>::new();
        let Some((first_item, _, _)) = queue.leak_slot_internal(|| false) else {
            panic!("Can't determine the reference for the element at #0");
        };
        test_commons::indexes_and_references_conversions(first_item,
                                                         |index| queue.slot_ref_from_slot_index(index),
                                                         |slot_ref| queue.slot_index_from_slot_ref(slot_ref));
    }

}
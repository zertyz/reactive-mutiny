//! Resting place for [AtomicMeta]

use super::super::{
    meta_publisher::MetaPublisher,
    meta_subscriber::MetaSubscriber,
    meta_container::MetaContainer,
};
use std::{
    fmt::Debug,
    sync::atomic::{AtomicU32,Ordering::{Acquire,Relaxed,Release}},
    mem::MaybeUninit,
};


/// Basis for multiple producer / multiple consumer queues using atomics for synchronization,
/// allowing enqueueing syncing to be (almost) fully detached from the dequeueing syncing.
///
/// Allows multiple-producer / multiple-consumers when using both patterns --
/// either the `publish()` / `consume()` pattern or\
/// the `leak_slot()` & `publish_leaked()` / `consume_leaking()` & `release_leaked()` one --
/// although the best performance is attainable if all publishers take the same time & all consumers take the same time as well.\
/// If that is not acceptable, see [BufferedBase] -- for an array backed ring buffer queue, which might be a little slower, but is
/// efficient if the publication & consumption of elements takes variable time.
#[repr(C,align(128))]      // aligned to cache line sizes for a careful control over false-sharing performance degradation
pub struct AtomicMeta<SlotType,
                      const BUFFER_SIZE: usize> {
    /// marks the first element of the queue, ready for dequeue -- increasing when dequeues are complete
    pub(crate) head: AtomicU32,
    /// increase-before-load field, marking where the next element should be written to
    /// -- may receed if exceeded the buffer capacity
    pub(crate) enqueuer_tail: AtomicU32,
    /// holder for the queue elements
    pub(crate) buffer: [SlotType; BUFFER_SIZE],
    /// increase-before-load field, marking where the next element should be retrieved from
    /// -- may receed if it gets ahead of the published `tail`
    pub(crate) dequeuer_head: AtomicU32,
    /// marks the last element of the queue, ready for dequeue -- increasing when enqueues are complete
    pub(crate) tail: AtomicU32,
}

impl<'a, SlotType:          'a + Debug,
         const BUFFER_SIZE: usize>
MetaContainer<'a, SlotType> for
AtomicMeta<SlotType, BUFFER_SIZE> {

    fn new() -> Self {
        Self::BUFFER_SIZE_MUST_BE_A_POWER_OF_2;
        // if !BUFFER_SIZE.is_power_of_two() {
        //     panic!("FullSyncMeta: BUFFER_SIZE must be a power of 2, but {BUFFER_SIZE} was provided.");
        // }
        Self {
            head:                 AtomicU32::new(0),
            tail:                 AtomicU32::new(0),
            dequeuer_head:        AtomicU32::new(0),
            enqueuer_tail:        AtomicU32::new(0),
            buffer:               unsafe { MaybeUninit::zeroed().assume_init() },
        }
    }
}

impl<'a, SlotType:          'a + Debug,
         const BUFFER_SIZE: usize>
MetaPublisher<'a, SlotType> for
AtomicMeta<SlotType, BUFFER_SIZE> {

    #[inline(always)]
    fn publish<SetterFn:                   FnOnce(&'a mut SlotType),
               ReportFullFn:               Fn() -> bool,
               ReportLenAfterEnqueueingFn: FnOnce(u32)>
              (&'a self,
               setter_fn:                      SetterFn,
               report_full_fn:                 ReportFullFn,
               report_len_after_enqueueing_fn: ReportLenAfterEnqueueingFn)
              -> bool {

        match self.leak_slot_internal(report_full_fn) {
            Some( (slot, slot_id, len_before) ) => {
                setter_fn(slot);
                self.publish_leaked_internal(slot_id);
                report_len_after_enqueueing_fn(len_before+1);
                true
            },
            None => false,
        }
    }

    #[inline(always)]
    fn leak_slot(&self) -> Option<&SlotType> {
        self.leak_slot_internal(|| false)
            .map(|(slot_ref, _slot_id, _len_before)| &*slot_ref)
    }

    #[inline(always)]
    fn publish_leaked(&'a self, slot: &'a SlotType) {
        let slot_id = self.slot_id_from_slot_ref(slot);
        self.publish_leaked_internal(slot_id)
    }

    #[inline(always)]
    fn unleak_slot(&'a self, slot: &'a SlotType) {
        let slot_id = self.slot_id_from_slot_ref(slot);
        while !self.try_unleak_slot_internal(slot_id) {
            std::hint::spin_loop();
        };
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

impl<'a, SlotType:          'a + Debug,
         const BUFFER_SIZE: usize>
MetaSubscriber<'a, SlotType> for
AtomicMeta<SlotType, BUFFER_SIZE> {

    #[inline(always)]
    fn consume<GetterReturnType: 'a,
               GetterFn:                   Fn(&'a mut SlotType) -> GetterReturnType,
               ReportEmptyFn:              Fn() -> bool,
               ReportLenAfterDequeueingFn: FnOnce(i32)>
              (&self,
               getter_fn:                      GetterFn,
               report_empty_fn:                ReportEmptyFn,
               report_len_after_dequeueing_fn: ReportLenAfterDequeueingFn)
              -> Option<GetterReturnType> {

        match self.consume_leaking_internal(report_empty_fn) {
            Some( (slot_ref, slot_id, len_before) ) => {
                let ret_val = getter_fn(slot_ref);
                self.release_leaked_internal(slot_id);
                report_len_after_dequeueing_fn(len_before-1);
                Some(ret_val)
            }
            None => None,
        }
    }

    #[inline(always)]
    fn consume_leaking(&'a self) -> Option<&'a SlotType> {
        self.consume_leaking_internal(|| false)
            .map(|(slot_ref, slot_id, _len_before)| &*slot_ref)
    }

    #[inline(always)]
    fn release_leaked(&'a self, slot: &'a SlotType) {
        let slot_id = self.slot_id_from_slot_ref(slot);
        self.release_leaked_internal(slot_id);
    }

    unsafe fn peek_remaining(&self) -> [&[SlotType]; 2] {
        let head_index = self.head.load(Relaxed) as usize % BUFFER_SIZE;
        let tail_index = self.tail.load(Relaxed) as usize % BUFFER_SIZE;
        if self.head.load(Relaxed) == self.tail.load(Relaxed) {
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
AtomicMeta<SlotType, BUFFER_SIZE> {

    /// The ring buffer is required to be a power of 2, so `head` and `tail` may wrap over flawlessly
    const BUFFER_SIZE_MUST_BE_A_POWER_OF_2: usize = 0 / if BUFFER_SIZE.is_power_of_two() {1} else {0};


    /// gets hold of one of the slots available in the pool,\
    /// returning: (a mutable reference to the data, the slot id, the current count of elements available for consumption).\
    /// If the pool is empty, `report_full_fn()` is called to inform the condition. If it was able to release any items,
    /// it should return `true`, which will cause this algorithm to try again instead of giving up with `None`.\
    /// This implementation enables other slots to be returned to the pool while there are allocated (but still unpublished) slots around,
    /// allowing the publication & consumption operations not happen in parallel.
    #[inline(always)]
    fn leak_slot_internal(&self, report_full_fn: impl Fn() -> bool) -> Option<(&mut SlotType, /*slot_id:*/ u32, /*len_before:*/ u32)> {
        let mutable_buffer = unsafe {
            let const_ptr = self.buffer.as_ptr();
            let mut_ptr = const_ptr as *mut [SlotType; BUFFER_SIZE];
            &mut *mut_ptr
        };
        let mut slot_id = self.enqueuer_tail.fetch_add(1, Relaxed);
        let mut len_before;
        loop {
            let head = self.head.load(Relaxed);
            len_before = slot_id.overflowing_sub(head).0;
            // is queue full?
            if len_before < BUFFER_SIZE as u32 {
                break
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
        Some( (&mut mutable_buffer[slot_id as usize % BUFFER_SIZE], slot_id, len_before) )
    }

    /// marks the given data sitting at `slot_id` as ready to be consumed, completing the publishing pattern
    /// that started with a call to [leak_slot_internal()]
    #[inline(always)]
    fn publish_leaked_internal(&'a self, slot_id: u32) {
        loop {
            match self.tail.compare_exchange_weak(slot_id, slot_id + 1, Release, Acquire) {
                Ok(_) => break,
                Err(reloaded_tail) => {
                    if slot_id > reloaded_tail {
                        relaxed_wait::<SlotType>(slot_id - reloaded_tail)
                    }
                },
            }
        }
    }

    /// attempt to return the slot at `slot_id` to the pool of available slots -- disrupting the publishing of an event
    /// that was started with a call to [leak_slot_internal()].\
    #[inline(always)]
    fn try_unleak_slot_internal(&'a self, slot_id: u32) -> bool {
        match self.enqueuer_tail.compare_exchange_weak(slot_id + 1, slot_id, Relaxed, Relaxed) {
            Ok(_) => true,
            Err(reloaded_enqueuer_tail) => {
                if reloaded_enqueuer_tail > slot_id {
                    // a new cmp_exch will be attempted due to concurrent modification of the variable
                    // we'll relax the cpu proportional to our position in the "concurrent modification order"
                    relaxed_wait::<SlotType>(reloaded_enqueuer_tail - slot_id);
                } else {
                    //#[cfg(debug_assertions)]
                    panic!("AtomicMeta: unleak_slot_internal: AN ERROR THAT SHOULD NOT HAPPEN!");
                    //panic!("Detected an attempt to `unleak_slot(#{slot_id}` after another allocation (issueing slot #{reloaded_enqueuer_tail})`");
                }
                false
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
        let mutable_buffer = unsafe {
            let const_ptr = self.buffer.as_ptr();
            let mut_ptr = const_ptr as *mut [SlotType; BUFFER_SIZE];
            &mut *mut_ptr
        };
        let mut slot_id = self.dequeuer_head.fetch_add(1, Relaxed);
        let mut len_before;
        loop {
            let tail = self.tail.load(Relaxed);
            len_before = tail.overflowing_sub(slot_id).0 as i32;
            // is queue empty?
            if len_before > 0 {
                let slot_value = &mut mutable_buffer[slot_id as usize % BUFFER_SIZE];
                break Some( (slot_value, slot_id, len_before) )
            } else {
                // queue is empty: reestablish the correct `dequeuer_head` (receding it to its original value)
                match self.dequeuer_head.compare_exchange(slot_id + 1, slot_id, Release, Acquire) {
                    Ok(_) => {
                        if !report_empty_fn() {
                            return None;
                        } else {
                            slot_id = self.dequeuer_head.fetch_add(1, Relaxed);
                        }
                    },
                    Err(reloaded_dequeuer_head) => {
                        if reloaded_dequeuer_head > slot_id {
                            relaxed_wait::<SlotType>(reloaded_dequeuer_head - slot_id);
                        }
                    }
                }
            }
        }
    }

    /// makes the `slot_id` available to the pool, for reuse,
    /// after the referenced data has been processed (aka, consumed)
    #[inline(always)]
    fn release_leaked_internal(&self, slot_id: u32) {
        loop {
            match self.head.compare_exchange_weak(slot_id, slot_id + 1, Relaxed, Relaxed) {
                Ok(_) => break,
                Err(reloaded_head) => {
                    if slot_id > reloaded_head {
                        relaxed_wait::<SlotType>(slot_id - reloaded_head);
                    }
                }
            }
        }
    }

    /// returns the id (within the Ring Buffer) that the given `slot` reference occupies
    #[inline(always)]
    fn slot_id_from_slot_ref(&'a self, slot: &'a SlotType) -> u32 {
        (( unsafe { (slot as *const SlotType).offset_from(self.buffer.as_ptr() as *const SlotType) } ) as usize) as u32
    }

    /// returns a reference to the slot pointed to by `slot_id`
    #[inline(always)]
    fn _slot_ref_from_slot_id(&'a self, slot_id: u32) -> &'a SlotType {
        &self.buffer[slot_id as usize % BUFFER_SIZE]
    }
}

// NOTE: spin_loop instruction disabled for now, as Tokio is faster without it (frees the CPU to do other tasks)
// /// relax the cpu for a time proportional to how much we are expected to wait for the concurrent CAS operation to succeed
// /// (5x * number of bytes to be set * wait_factor)
#[inline(always)]
fn relaxed_wait<SlotType>(_wait_factor: u32) {
    // for _ in 0..wait_factor {
    //     // subject to loop unrolling optimization
    //     for _ in 0..std::mem::size_of::<SlotType>() {
    //         std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop();
    //     }
    // }
}

#[cfg(any(test,doc))]
mod tests {
    //! Unit tests for [atomic_meta](super) module

    use super::{
        *,
        super::super::super::test_commons::measure_syncing_independency
    };
    use std::sync::{
        Arc,
        atomic::AtomicU32,
    };

    /// the atomic meta queue should allow independent enqueueing (if dequeue takes too long to complete, enqueueing should not be affected)
    #[cfg_attr(not(doc),test)]
    #[ignore]   // time-dependent: should run on single-threaded tests
    pub fn assure_enqueue_syncing_independency() {
        let queue = AtomicMeta::<u32, 128>::new();
        let (independent_productions_count, dependent_productions_count, _independent_consumptions_count, _dependent_consumptions_count) = measure_syncing_independency(
            |i, callback: &dyn Fn()| {
                queue.publish(
                    |slot| {
                        *slot = i;
                        callback();
                    },
                    || false,
                    |_| {}
                )
            },
            |result: &AtomicU32, callback: &dyn Fn()| {
                queue.consume(
                    |slot| {
                        result.store(*slot, Relaxed);
                        callback();
                    },
                    || false,
                    |_| {}
                )
            }
        );

        assert!(independent_productions_count > 0, "No independent production was detected");
        assert_eq!(dependent_productions_count, 0, "a > 0 number here means enqueueing had to wait for dequeueing (which is not supposed to happen in ths Atomic meta queue) -- BTW, for this test, the queue is never full");
    }

}

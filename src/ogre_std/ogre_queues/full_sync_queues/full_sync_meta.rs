//! Basis for multiple producer / multiple consumer queues using a quick-and-dirty (but fast)
//! full synchronization through an atomic flag, with a clever & experimentally tuned efficient locking mechanism.

use crate::ogre_std::ogre_queues::{
    meta_publisher::MetaPublisher,
    meta_subscriber::MetaSubscriber,
    meta_queue::MetaQueue,
};
use std::{
    fmt::Debug,
    mem::{ManuallyDrop, MaybeUninit},
    ops::Deref,
    sync::atomic::{
        AtomicBool,
        Ordering::Relaxed,
    },
};


/// to make the most of performance, let BUFFER_SIZE be a power of 2 (so that 'i % BUFFER_SIZE' modulus will be optimized)
// #[repr(C,align(64))]      // users of this class, if uncertain, are advised to declare this as their first field and have this annotation to cause alignment to cache line sizes, for a careful control over false-sharing performance degradation
pub struct FullSyncMeta<SlotType,
                        const BUFFER_SIZE: usize> {

    /// locked when the queue is full, unlocked when it is no longer full
    full_guard: AtomicBool,
    /// guards critical regions to allow concurrency
    concurrency_guard: AtomicBool,
    tail: u32,
    /// holder for the elements
    buffer: [ManuallyDrop<SlotType>; BUFFER_SIZE],
    head: u32,
    /// locked when the queue is empty, unlocked when it is no longer empty
    empty_guard: AtomicBool,

}

impl<'a, SlotType:          'a + Unpin + Debug,
         const BUFFER_SIZE: usize>
MetaQueue<'a, SlotType> for
FullSyncMeta<SlotType, BUFFER_SIZE> {

    fn new() -> Self {
        Self {
            full_guard:        AtomicBool::new(false),
            concurrency_guard: AtomicBool::new(false),
            tail:              0,
            buffer:            unsafe { MaybeUninit::zeroed().assume_init() },
            head:              0,
            empty_guard:       AtomicBool::new(false),
        }
    }

}

impl<'a, SlotType:          'a + Unpin + Debug,
         const BUFFER_SIZE: usize>
MetaPublisher<'a, SlotType> for
FullSyncMeta<SlotType, BUFFER_SIZE> {

    #[inline(always)]
    fn publish<SetterFn:                   FnOnce(&'a mut SlotType),
               ReportFullFn:               Fn() -> bool,
               ReportLenAfterEnqueueingFn: FnOnce(u32)>
              (&self, setter_fn:                      SetterFn,
                      report_full_fn:                 ReportFullFn,
                      report_len_after_enqueueing_fn: ReportLenAfterEnqueueingFn)
              -> bool {

        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        let mut len_before;

        // lock & acquire a slot to set the item
        loop {
            lock(&self.concurrency_guard);
            len_before = self.tail.overflowing_sub(self.head).0;
            if len_before < BUFFER_SIZE as u32 {
                break
            } else {
                unlock(&self.concurrency_guard);
                let maybe_no_longer_full = report_full_fn();
                if !maybe_no_longer_full {
                    return false;
                }
            }
        }

        setter_fn(&mut mutable_self.buffer[self.tail as usize % BUFFER_SIZE]);
        mutable_self.tail = self.tail.overflowing_add(1).0;

        unlock(&self.concurrency_guard);
        report_len_after_enqueueing_fn(len_before+1);
        true
    }

    #[inline(always)]
    fn available_elements(&self) -> usize {
        self.tail.overflowing_sub(self.head).0 as usize
    }

    #[inline(always)]
    fn max_size(&self) -> usize {
        BUFFER_SIZE
    }

    fn debug_info(&self) -> String {
        let Self {full_guard, concurrency_guard, tail, buffer: _, head, empty_guard} = self;
        let full_guard = full_guard.load(Relaxed);
        let concurrency_guard = concurrency_guard.load(Relaxed);
        let empty_guard = empty_guard.load(Relaxed);
        format!("ogre_queues::full_sync_meta's state: {{head: {head}, tail: {tail}, (len: {}), empty: {empty_guard}, full: {full_guard}, locked: {concurrency_guard}, elements: {{{}}}'}}",
                self.available_elements(),
                unsafe {self.peek_remaining()}.iter().flat_map(|&slice| slice).fold(String::new(), |mut acc, e| {
                    acc.push_str(&format!("'{:?}',", e));
                    acc
                }))
    }
}

impl<'a, SlotType:          'a + Unpin + Debug,
         const BUFFER_SIZE: usize>
MetaSubscriber<'a, SlotType> for
FullSyncMeta<SlotType, BUFFER_SIZE> {

    #[inline(always)]
    fn consume<GetterReturnType,
               GetterFn:                   FnOnce(&'a mut SlotType) -> GetterReturnType,
               ReportEmptyFn:              Fn() -> bool,
               ReportLenAfterDequeueingFn: FnOnce(i32)>
              (&self,
               getter_fn:                      GetterFn,
               report_empty_fn:                ReportEmptyFn,
               report_len_after_dequeueing_fn: ReportLenAfterDequeueingFn)
              -> Option<GetterReturnType> {

        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};

        let mut len_before;
        loop {
            lock(&self.concurrency_guard);
            len_before = self.available_elements() as i32;
            if len_before > 0 {
                break
            } else {
                unlock(&self.concurrency_guard);
                let maybe_no_longer_empty = report_empty_fn();
                if !maybe_no_longer_empty {
                    return None;
                }
            }
        }

        let ret_val = getter_fn(&mut mutable_self.buffer[self.head as usize % BUFFER_SIZE]);
        mutable_self.head = self.head.overflowing_add(1).0;

        unlock(&self.concurrency_guard);
        report_len_after_dequeueing_fn(len_before-1);
        Some(ret_val)
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

/// Returns when the lock was acquired -- inspired by `parking-lot`
/// Unlocked: false; locked: true
#[inline(always)]
fn lock(raw_mutex: &AtomicBool) {
    // attempt to lock -- spinning for 10 times, relaxing the CPU between attempts
    if raw_mutex.compare_exchange_weak(false, true, Relaxed, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if raw_mutex.compare_exchange_weak(false, true, Relaxed, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if raw_mutex.compare_exchange_weak(false, true, Relaxed, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if raw_mutex.compare_exchange_weak(false, true, Relaxed, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if raw_mutex.compare_exchange_weak(false, true, Relaxed, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if raw_mutex.compare_exchange_weak(false, true, Relaxed, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if raw_mutex.compare_exchange_weak(false, true, Relaxed, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if raw_mutex.compare_exchange_weak(false, true, Relaxed, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if raw_mutex.compare_exchange_weak(false, true, Relaxed, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if raw_mutex.compare_exchange_weak(false, true, Relaxed, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    // no deal -- fallback without using the _weak version of compare_exchange
    while !raw_mutex.compare_exchange(false, true, Relaxed, Relaxed).is_ok() { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
}

/// Releases any locks, returning immediately
#[inline(always)]
fn unlock(raw_mutex: &AtomicBool) {
    raw_mutex.store(false, Relaxed);
}


#[cfg(any(test, feature="dox"))]
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
    #[test]
    pub fn assure_syncing_dependency() {
        let queue = Arc::new(FullSyncMeta::<u32, 128>::new());
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
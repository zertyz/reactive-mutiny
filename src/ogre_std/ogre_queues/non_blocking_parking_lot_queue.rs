//! multiple producer / multiple consumer non-blocking queue with a full locking synchronization --
//! only a single enqueue (or dequeue) may execute the critical region at a time

use super::{
    OgreQueue,
};
use std::{
    fmt::Debug,
    mem::MaybeUninit,
};
use std::pin::Pin;
use parking_lot::lock_api::RawMutex as RawMutex_api;
use parking_lot::RawMutex;

/// to make the most of performance, let BUFFER_SIZE be a power of 2 (so that 'i % BUFFER_SIZE' modulus will be optimized)
#[repr(C,align(64))]      // aligned to cache line sizes to reduce false-sharing performance degradation
pub struct Queue<SlotType, const BUFFER_SIZE: usize, const METRICS: bool, const DEBUG: bool> {
    /// buffer is 64-bytes aligned as well (notice the fields before it), to reduce false-sharing
    buffer: [SlotType; BUFFER_SIZE],
    /// guards the code's critical regions to allow concurrency
    concurrency_guard: RawMutex,
    head: u32,
    tail: u32,
    // for use when metrics are enabled
    enqueue_count:      u64,
    dequeue_count:      u64,
    enqueue_collisions: u64,
    dequeue_collisions: u64,
    queue_full_count:   u64,
    queue_empty_count:  u64,
    /// for use when debug is enabled
    queue_name: String,
}
impl<SlotType: Copy+Debug, const BUFFER_SIZE: usize, const METRICS: bool, const DEBUG: bool>
OgreQueue<SlotType> for Queue<SlotType, BUFFER_SIZE, METRICS, DEBUG> {
    fn new<IntoString: Into<String>>(queue_name: IntoString) -> Pin<Box<Self>> {
        Box::pin(Self {
            buffer:               unsafe { MaybeUninit::zeroed().assume_init() },
            concurrency_guard:    RawMutex::INIT,
            head:                 0,
            tail:                 0,
            enqueue_count:        0,
            dequeue_count:        0,
            enqueue_collisions:   0,
            dequeue_collisions:   0,
            queue_full_count:     0,
            queue_empty_count:    0,
            queue_name:           queue_name.into(),
        })
    }

    #[inline(always)]
    fn enqueue(&self, element: SlotType) -> bool {
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        self.concurrency_guard.lock();
        if self.tail.overflowing_sub(self.head).0 >= BUFFER_SIZE as u32 {
            if METRICS {
                mutable_self.queue_full_count += 1;
            }
            unsafe {self.concurrency_guard.unlock()};
            return false;
        }
        if DEBUG {
            eprintln!("### ENQUEUE: enqueueing element '{:?}' at #{}: new enqueuer tail={}; head={}", element, self.tail as usize % BUFFER_SIZE, self.tail+1, self.head);
        }
        mutable_self.buffer[self.tail as usize % BUFFER_SIZE] = element;
        mutable_self.tail = self.tail.overflowing_add(1).0;
        if METRICS {
            mutable_self.enqueue_count += 1;
        }
        unsafe {self.concurrency_guard.unlock()};
        true
    }

    #[inline(always)]
    fn dequeue(&self) -> Option<SlotType> {
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        self.concurrency_guard.lock();
        if self.tail == self.head {
            if METRICS {
                mutable_self.queue_empty_count += 1;
            }
            unsafe {self.concurrency_guard.unlock()};
            return None;
        }
        let slot_value = self.buffer[self.head as usize % BUFFER_SIZE];
        mutable_self.head = self.head.overflowing_add(1).0;
        if METRICS {
            mutable_self.dequeue_count += 1;
        }
        unsafe {self.concurrency_guard.unlock()};
        if DEBUG {
            eprintln!("### DEQUEUE: dequeued element '{:?}' from #{}: dequeuer tail={}; new head={}", slot_value, self.head as usize % BUFFER_SIZE, self.tail, self.head + 1);
        }
        Some(slot_value)
    }

    fn len(&self) -> usize {
        self.tail.overflowing_sub(self.head).0 as usize
    }

    fn max_size(&self) -> usize {
        BUFFER_SIZE
    }

    fn debug_enabled(&self) -> bool {
        todo!()
    }

    fn metrics_enabled(&self) -> bool {
        todo!()
    }

    fn queue_name(&self) -> &str {
        todo!()
    }

    fn implementation_name(&self) -> &str {
        todo!()
    }

    fn interrupt(&self) {
        todo!()
    }
}

#[cfg(any(test,doc))]
mod tests {
    //! Unit tests for [queue](super) module

    use super::*;
    use super::super::super::test_commons::{self,ContainerKind,Blocking};

    #[cfg_attr(not(doc),test)]
    fn basic_queue_use_cases() {
        let queue = Queue::<i32, 16, false, false>::new("'basic_use_cases' test queue".to_string());
        test_commons::basic_container_use_cases(ContainerKind::Queue, Blocking::NonBlocking, queue.max_size(), |e| queue.enqueue(e), || queue.dequeue(), || queue.len());
    }

    #[cfg_attr(not(doc),test)]
    fn single_producer_multiple_consumers() {
        let queue = Queue::<u32, 65536, false, false>::new("'single_producer_multiple_consumers' test queue".to_string());
        test_commons::container_single_producer_multiple_consumers(|e| queue.enqueue(e), || queue.dequeue());
    }

    #[cfg_attr(not(doc),test)]
    fn multiple_producers_single_consumer() {
        let queue = Queue::<u32, 65536, false, false>::new("'multiple_producers_single_consumer' test queue".to_string());
        test_commons::container_multiple_producers_single_consumer(|e| queue.enqueue(e), || queue.dequeue());
    }

    #[cfg_attr(not(doc),test)]
    pub fn multiple_producers_and_consumers_all_in_and_out() {
        let queue = Queue::<u32, 102400, false, false>::new("'multiple_producers_and_consumers_all_in_and_out' test queue".to_string());
        test_commons::container_multiple_producers_and_consumers_all_in_and_out(Blocking::NonBlocking, queue.max_size(), |e| queue.enqueue(e), || queue.dequeue());
    }

    #[cfg_attr(not(doc),test)]
    pub fn multiple_producers_and_consumers_single_in_and_out() {
        let queue = Queue::<u32, 128, false, false>::new("'multiple_producers_and_consumers_single_in_and_out' test queue".to_string());
        test_commons::container_multiple_producers_and_consumers_single_in_and_out(|e| queue.enqueue(e), || queue.dequeue());
    }
}
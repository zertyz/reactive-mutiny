//! Multiple producer / multiple consumer lock-free / blocking queue --
//! blocks on empty or full scenarios, using parking-lot mutexes

use super::{
    OgreQueue,
    OgreBlockingQueue
};
use std::{
    fmt::Debug,
    mem::MaybeUninit,
    time::Duration,
};
use std::marker::PhantomPinned;
use std::pin::Pin;
use parking_lot::lock_api::{RawMutex as RawMutex_api, RawMutexTimed};
use parking_lot::RawMutex;

/// to make the most of performance, let BUFFER_SIZE be a power of 2 (so that 'i % BUFFER_SIZE' modulus will be optimized)
#[repr(C,align(64))]      // aligned to cache line sizes for a careful control over false-sharing performance degradation
pub struct Queue<'a, SlotType, const BUFFER_SIZE: usize, const METRICS: bool, const DEBUG: bool, const LOCK_TIMEOUT_MILLIS: usize> {
    buffer: [SlotType; BUFFER_SIZE],
    /// locked when the queue is empty, unlocked when it is no longer empty
    _empty_guard:      RawMutex,
    empty_guard_ref:   &'a RawMutex,
    /// locked when the queue is full, unlocked when it is no longer full
    full_guard:        RawMutex,
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
    _pin: PhantomPinned,
}
impl<'a, SlotType: Copy+Debug, const BUFFER_SIZE: usize, const METRICS: bool, const DEBUG: bool, const LOCK_TIMEOUT_MILLIS: usize>
Queue<'a, SlotType, BUFFER_SIZE, METRICS, DEBUG, LOCK_TIMEOUT_MILLIS> {

    const TRY_LOCK_DURATION: Duration = Duration::from_millis(LOCK_TIMEOUT_MILLIS as u64);

    /// called when the queue is empty: waits for a slot to filled in for up to `LOCK_TIMEOUT_MILLIS`\
    /// -- returns true if we recovered from the empty condition; false otherwise.\
    /// Notice, however, `true` may also be returned if the mutex is being tainted elsewhere ([interrupt()], for example)
    #[inline(always)]
    fn report_empty(&mut self) -> bool {
        // lock 'empty_guard' with a timeout (if applicable)
        if LOCK_TIMEOUT_MILLIS == 0 {
            if DEBUG {
                eprintln!("### QUEUE '{}' is empty. Waiting for a new element (indefinitely) so DEQUEUE may proceed...", self.queue_name);
            }
            self.empty_guard_ref.lock();
        } else {
            if DEBUG {
                eprintln!("### QUEUE '{}' is empty. Waiting for a new element (for up to {}ms) so DEQUEUE may proceed...", self.queue_name, LOCK_TIMEOUT_MILLIS);
            }
            if !self.empty_guard_ref.try_lock_for(Self::TRY_LOCK_DURATION) {
                if DEBUG {
                    eprintln!("Blocking queue '{}', said to be empty, waited too much for an element to be available so it could be dequeued. Bailing out after waiting for ~{}ms", self.queue_name, LOCK_TIMEOUT_MILLIS);
                }
                return false;
            }
        }
        true
    }

    #[inline(always)]
    fn report_no_longer_empty(&self) {
        if DEBUG {
            eprintln!("Blocking queue '{}' is said to have just come out of the EMPTY state...", self.queue_name);
        }
        unsafe {self.empty_guard_ref.unlock()};
        self.empty_guard_ref.try_lock();
    }

    /// called when the queue is full: waits for a slot to be freed up to `LOCK_TIMEOUT_MILLIS`\
    /// -- returns true if we recovered from the full condition; false otherwise.\
    /// Notice, however, `true` may also be returned if the mutex is being tainted elsewhere ([interrupt()], for example)
    #[inline(always)]
    fn report_full(&mut self) -> bool {
        // lock 'full_guard' with a timeout (if applicable)
        if LOCK_TIMEOUT_MILLIS == 0 {
            if DEBUG {
                eprintln!("### QUEUE '{}' is full. Waiting for a free slot (indefinitely) so ENQUEUE may proceed...", self.queue_name);
            }
            self.full_guard.lock();
        } else {
            if DEBUG {
                eprintln!("### QUEUE '{}' is full. Waiting for a free slot (for up to {}ms) so ENQUEUE may proceed...", self.queue_name, LOCK_TIMEOUT_MILLIS);
            }
            if !self.full_guard.try_lock_for(Self::TRY_LOCK_DURATION) {
                if DEBUG {
                    println!("Blocking queue '{}', said to be full, waited too much for a free slot to enqueue a new element. Bailing out after waiting for ~{}ms", self.queue_name, LOCK_TIMEOUT_MILLIS);
                }
                return false;
            }
        }
        true
    }

    #[inline(always)]
    fn report_no_longer_full(&self) {
        if DEBUG {
            eprintln!("Blocking queue '{}' is said to have just come out of the FULL state...", self.queue_name);
        }
        unsafe { self.full_guard.unlock() }
        self.full_guard.try_lock();
    }

}
impl<'a, SlotType: Copy+Debug, const BUFFER_SIZE: usize, const METRICS: bool, const DEBUG: bool, const LOCK_TIMEOUT_MILLIS: usize>
OgreBlockingQueue<'a, SlotType>
for Queue<'a, SlotType, BUFFER_SIZE, METRICS, DEBUG, LOCK_TIMEOUT_MILLIS> {

    fn set_empty_guard_ref(&mut self, empty_guard_ref: &'a RawMutex) {
        self.empty_guard_ref = empty_guard_ref;
    }
/*
    to continue -- after 2022-04-01: we were refactoring the Blocking / non-blocking traits for Queues & Stacks. The objective was to be able to run the async example with a
    blocking queue, so that it wouldn't use CPU. For that, the non-blocking queue is the base trait of the blocking queue, which added 3 new methods: 2 of them are counter-parts
    that behave as non-blocking -- try_enqueue() & try_dequeue() and the other one is to set the full_guard_reference to a common mutex.
    The idea is that shared loop callbacks will require the use of blocking event channels while solo loop callbacks may accept both. For shared loop callbacks to work, they will
    set all queues & stacks of all their event channels to use the same full_guard mutex: this way, only the try_*() methods will be used and, if all of them said the structures
    were empty, the loop would lock on that mutex to sleep; when coming back from a sleep, the counter would be reset to 0 (so that all priorities execute); if nobody from a given
    counter has elements, instead of incrementing by one, the counter will be set to the next power of 2, overflowing back to 0 -- in such a way that if only the lowest possible
    priority has pending elements, up to 15 attempts to get elements from the hightest possible priority will be made (instead of 32768).
    CORRECTION: the shared mutex should be the empty_guard_ref (instead of the full_guard_ref), as if somebody is full, the producer will lock, not the shared thread loop -- which
    must, indeed, only lock when the channels are devoided of elements, so to avoid busy waiting.

    --------------------------------------------------------------------
    to add to the previous:
    sync & async events naming DO create confusion with the Rust's async feature -- specially because sync events may have async consumers & listeners.
    From now on -- and, please, update the sources & documentation, "events" will be "async events" and "answerable events" will stand for "sync events".

    Answerable Events will have their producers called on an async context -- and the producer operation will return a Future with an answer...
    ... on the other hand, "regular events" can be called on normal running context... and, of course, on async contexts as well... but they won't return any Future.
    Nonetheless, consumers, listeners, producer-listeners, consumer-listeners... each one of them may freely be either "normal" functions or "async functions".

    It is believed that ogre-workers may keep the initialization coordination for both types of callback -- provided the Tokio runtime is passed along, it would be just a metter of calling tokio::task() on the new async loop function.
    The great deal of change comes in the event pipeline: now we will have the double the arrays for callbacks. For instance, for "regular events" we will fill
    the consumers: [], async_consumers: [], listeners: [], async_listeners: [].
    And the callback generators must create a vector of regular callbacks and a vector of async callbacks -- maybe the great deal is here.
    Lets leave the answerable events to be addressed once we resolve the new naming & design issues with the regular events, what should be big enough of a task.
    Don't forget to come up with an Example using "regular events" with async consumers or listeners
*/
    fn try_enqueue(&self, element: SlotType) -> bool {
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        self.concurrency_guard.lock();
        if self.tail.overflowing_sub(self.head).0 >= BUFFER_SIZE as u32 {
            unsafe {self.concurrency_guard.unlock()};
            return false;
        }
        if DEBUG {
            eprintln!("### TRY ENQUEUE: enqueueing element '{:?}' at #{}: new enqueuer tail={}; head={}", element, self.tail as usize % BUFFER_SIZE, self.tail+1, self.head);
        }
        mutable_self.buffer[self.tail as usize % BUFFER_SIZE] = element;
        let was_empty = self.head == self.tail;
        mutable_self.tail = self.tail.overflowing_add(1).0;
        if METRICS {
            mutable_self.enqueue_count += 1;
        }
        unsafe {self.concurrency_guard.unlock()};
        if was_empty {
            self.report_no_longer_empty();
        }
        true
    }

    fn try_dequeue(&self) -> Option<SlotType> {
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        self.concurrency_guard.lock();
        if self.tail == self.head {
            unsafe {self.concurrency_guard.unlock()};
            return None;
        }
        let slot_value = self.buffer[self.head as usize % BUFFER_SIZE];
        let was_full = self.tail.overflowing_sub(self.head).0 >= BUFFER_SIZE as u32;
        mutable_self.head = self.head.overflowing_add(1).0;
        if METRICS {
            mutable_self.dequeue_count += 1;
        }
        unsafe {self.concurrency_guard.unlock()};
        if was_full {
            self.report_no_longer_full();
        }
        if DEBUG {
            eprintln!("### DEQUEUE: dequeued element '{:?}' from #{}: dequeuer tail={}; new head={}", slot_value, self.head as usize % BUFFER_SIZE, self.tail, self.head + 1);
        }
        Some(slot_value)
    }

}
impl<'a, SlotType: Copy+Debug, const BUFFER_SIZE: usize, const METRICS: bool, const DEBUG: bool, const LOCK_TIMEOUT_MILLIS: usize>
OgreQueue<SlotType>
for Queue<'a, SlotType, BUFFER_SIZE, METRICS, DEBUG, LOCK_TIMEOUT_MILLIS> {
    fn new<IntoString: Into<String>>(queue_name: IntoString) -> Pin<Box<Self>> {
        let mut instance = Box::pin(Self {
            buffer:               unsafe { MaybeUninit::zeroed().assume_init() },
            _empty_guard:         RawMutex::INIT,
            empty_guard_ref:      unsafe { &*std::ptr::null() as &RawMutex },     // TODO implement interior mutability to get rid of this annoying warning
            full_guard:           RawMutex::INIT,
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
            _pin: PhantomPinned,
        });
        instance._empty_guard.lock();
        instance.full_guard.lock();
        unsafe {
            let empty_guard_ptr = &instance._empty_guard as *const RawMutex;
            let empty_guard_ref = &*(empty_guard_ptr);
            let mut_ref: Pin<&mut Self> = Pin::as_mut(&mut instance);
            Pin::get_unchecked_mut(mut_ref).empty_guard_ref = empty_guard_ref;
        }
        instance
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
            let maybe_no_longer_full = mutable_self.report_full();
            if !maybe_no_longer_full {
                return false;
            }
            self.concurrency_guard.lock();
            if self.tail.overflowing_sub(self.head).0 >= BUFFER_SIZE as u32 {
                unsafe {self.concurrency_guard.unlock()};
                return false;
            }
        }
        if DEBUG {
            eprintln!("### ENQUEUE: enqueueing element '{:?}' at #{}: new enqueuer tail={}; head={}", element, self.tail as usize % BUFFER_SIZE, self.tail+1, self.head);
        }
        mutable_self.buffer[self.tail as usize % BUFFER_SIZE] = element;
        let was_empty = self.head == self.tail;
        mutable_self.tail = self.tail.overflowing_add(1).0;
        if METRICS {
            mutable_self.enqueue_count += 1;
        }
        unsafe {self.concurrency_guard.unlock()};
        if was_empty {
            self.report_no_longer_empty();
        }
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
            let maybe_no_longer_empty = mutable_self.report_empty();
            if !maybe_no_longer_empty {
                return None;
            }
            self.concurrency_guard.lock();
            if self.tail == self.head {
                unsafe {self.concurrency_guard.unlock()};
                return None;
            }
        }
        let slot_value = self.buffer[self.head as usize % BUFFER_SIZE];
        let was_full = self.tail.overflowing_sub(self.head).0 >= BUFFER_SIZE as u32;
        mutable_self.head = self.head.overflowing_add(1).0;
        if METRICS {
            mutable_self.dequeue_count += 1;
        }
        unsafe {self.concurrency_guard.unlock()};
        if was_full {
            self.report_no_longer_full();
        }
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
        DEBUG
    }

    fn metrics_enabled(&self) -> bool {
        METRICS
    }

    fn queue_name(&self) -> &str {
        &self.queue_name
    }

    fn implementation_name(&self) -> &str {
        "blocking_queue"
    }

    fn interrupt(&self) {
        unsafe {
            self.empty_guard_ref.unlock();
            self.full_guard.unlock();
            self.empty_guard_ref.unlock();
            self.full_guard.unlock();
            self.concurrency_guard.unlock();
        }
    }
}
impl <SlotType: Copy+Debug, const BUFFER_SIZE: usize, const METRICS: bool, const DEBUG: bool, const LOCK_TIMEOUT_MILLIS: usize>
     Queue<'_, SlotType, BUFFER_SIZE, METRICS, DEBUG, LOCK_TIMEOUT_MILLIS> {

    pub fn _debug(&self) {
        eprintln!("========================= QUEUE DEBUG ===========================");
        eprintln!("||  head:                 {}", self.head);
        eprintln!("||  Tail:                 {}", self.tail);
    }
}

#[cfg(any(test, feature="dox"))]
mod tests {
    //! Unit tests for [queue](super) module

    use super::*;
    use super::super::super::test_commons::{self,ContainerKind,Blocking};
    use std::{
        time::SystemTime,
        io::Write,
    };
    
    #[test]
    fn basic_queue_use_cases_blocking() {
        let queue = Queue::<i32, 16, false, false, 1000>::new("'basic_use_cases' test queue".to_string());
        test_commons::basic_container_use_cases(ContainerKind::Queue, Blocking::Blocking, queue.max_size(), |e| queue.enqueue(e), || queue.dequeue(), || queue.len());
    }

    #[test]
    fn basic_queue_use_cases_non_blocking() {
        let queue = Queue::<i32, 16, false, false, 1000>::new("'basic_use_cases' test queue".to_string());
        test_commons::basic_container_use_cases(ContainerKind::Queue, Blocking::Blocking, queue.max_size(), |e| queue.try_enqueue(e), || queue.try_dequeue(), || queue.len());
    }

    #[test]
    fn single_producer_multiple_consumers() {
        let queue = Queue::<u32, 65536, false, false, 1000>::new("'single_producer_multiple_consumers' test queue".to_string());
        test_commons::container_single_producer_multiple_consumers(|e| queue.enqueue(e), || queue.dequeue());
    }

    #[test]
    fn multiple_producers_single_consumer() {
        let queue = Queue::<u32, 65536, false, false, 1000>::new("'multiple_producers_single_consumer' test queue".to_string());
        test_commons::container_multiple_producers_single_consumer(|e| queue.enqueue(e), || queue.dequeue());
    }

    #[test]
    fn multiple_producers_and_consumers_all_in_and_out() {
        let queue = Queue::<u32, 102400, false, false, 1000>::new("'multiple_producers_and_consumers_all_in_and_out' test queue".to_string());
        test_commons::container_multiple_producers_and_consumers_all_in_and_out(Blocking::Blocking, queue.max_size(), |e| queue.enqueue(e), || queue.dequeue());
    }

    #[test]
    fn multiple_producers_and_consumers_single_in_and_out() {
        let queue = Queue::<u32, 128, false, false, 1000>::new("'multiple_producers_and_consumers_single_in_and_out' test queue".to_string());
        test_commons::container_multiple_producers_and_consumers_single_in_and_out(|e| queue.enqueue(e), || queue.dequeue());
    }

    /// makes sure the queue waits on a mutex when appropriate -- dequeueing from empty, enqueueing when full --
    /// and doesn't wait when not needed -- dequeueing an existing element, enqueueing when there are free slots available
    #[test]
    fn test_blocking() {
        const TIMEOUT_MILLIS:   usize = 100;
        const QUEUE_SIZE:       usize = 16;
        const TOLERANCE_MILLIS: usize = 10;
        let queue = Queue::<usize, QUEUE_SIZE, false, false, TIMEOUT_MILLIS>::new("'test_blocking' queue".to_string());

        // asserts in several passes, so we're sure blocking side effects on mutexes are fine
        for pass in ["virgin", "non-virgin", "promiscuous"] {
            println!("  Asserting pass '{}'", pass);
            assert_blocking(|| queue.dequeue(), None, &format!("  Blocking on empty (from a {} queue)", pass));
            assert_blocking(|| queue.dequeue(), None, "  Blocking on empty (again)");
            assert_non_blocking(|| queue.try_dequeue(), "  Non-Blocking 'try_dequeue()'");

            assert_non_blocking(|| for i in 0..QUEUE_SIZE {
                queue.enqueue(i);
            }, "  Blocking enqueueing (won't block as there are free slots)");

            assert_blocking(|| queue.enqueue(999), false, "  Blocking on full");
            assert_blocking(|| queue.enqueue(999), false, "  Blocking on full (again)");
            assert_non_blocking(|| queue.try_enqueue(999), "  Non-Blocking 'try_enqueue()'");

            assert_non_blocking(|| for _i in 0..QUEUE_SIZE {
                queue.dequeue();
            }, "  Blocking dequeueing (won't block as there are elements)");
        }

        // interrupt()
        assert_non_blocking(|| {
            queue.interrupt();
            queue.dequeue();
            queue.enqueue(999);
        }, "'interrupt()' causing blocking operations to return immediately");

        fn assert_blocking<R: PartialEq+Debug>(left_with: impl Fn() -> R, right: R, msg: &str) {
            print!("{}:  ", msg); std::io::stdout().flush().unwrap();
            let start = SystemTime::now();
            let result = left_with();
            let elapsed = start.elapsed().unwrap();
            println!("{:?}", elapsed);
            assert!((elapsed.as_millis() as i32 - TIMEOUT_MILLIS as i32).abs() < TOLERANCE_MILLIS as i32,
                    "Expected and observed time spent in the locked mutex exceeded the tolerance of {}ms: observed (blocking?) time: {}ms; expected: {}ms ",
                    TOLERANCE_MILLIS, elapsed.as_millis(), TIMEOUT_MILLIS);
            assert_eq!(result, right, "Even if the queue is blocking, it should behaving as non-blocking after the timeout exceeds");
        }

        fn assert_non_blocking<R: PartialEq+Debug>(op: impl Fn() -> R, msg: &str) {
            print!("{}:  ", msg); std::io::stdout().flush().unwrap();
            let start = SystemTime::now();
            let _result = op();
            let elapsed = start.elapsed().unwrap();
            println!("{:?}", elapsed);
            assert!((elapsed.as_millis() as i32).abs() < TOLERANCE_MILLIS as i32,
                    "Non-blocking operations should not take considerable time: tolerance of {}ms; observed (non-blocking?) time: {}ms; expected: 0ms ",
                    TOLERANCE_MILLIS, elapsed.as_millis());
        }
    }

}

//! See [BlockingQueue]

use super::super::super::{
    instruments::Instruments,
    ogre_queues::{
        OgreQueue,
        OgreBlockingQueue,
        atomic::atomic_move::AtomicMove,
        meta_publisher::MetaPublisher,
        meta_subscriber::MetaSubscriber,
        meta_container::MetaContainer,
    },
};
use std::{
    fmt::Debug,
    sync::atomic::{AtomicU64,Ordering::Relaxed},
    time::Duration,
};
use parking_lot::{
    RawMutex,
    lock_api::{
        RawMutex as _RawMutex,
        RawMutexTimed,
    },
};
use log::{trace};


/// Multiple producer / multiple consumer lock-free / blocking queue --
/// uses atomics for synchronization but blocks (on empty or full scenarios) using parking-lot mutexes
#[repr(C,align(64))]      // aligned to cache line sizes for a careful control over false-sharing performance degradation
pub struct BlockingQueue<SlotType:                  Copy+Debug,
                         const BUFFER_SIZE:         usize,
                         const LOCK_TIMEOUT_MILLIS: usize = 0,
                         const INSTRUMENTS:         usize = 0> {
    // metrics for enqueue
    enqueue_count:      AtomicU64,
    queue_full_count:   AtomicU64,
    /// locked when the queue is full, unlocked when it is no longer full
    full_guard:         RawMutex,
    /// queue
    base_queue:         AtomicMove<SlotType, BUFFER_SIZE>,
    /// locked when the queue is empty, unlocked when it is no longer empty
    empty_guard:        RawMutex,
    // metrics for dequeue
    dequeue_count:      AtomicU64,
    queue_empty_count:  AtomicU64,
    /// for use when debug is enabled
    queue_name: String,
}

impl<SlotType:                  Copy+Debug,
     const BUFFER_SIZE:         usize,
     const LOCK_TIMEOUT_MILLIS: usize,
     const INSTRUMENTS:         usize>
BlockingQueue<SlotType, BUFFER_SIZE, LOCK_TIMEOUT_MILLIS, INSTRUMENTS> {

    const TRY_LOCK_DURATION: Duration = Duration::from_millis(LOCK_TIMEOUT_MILLIS as u64);

    /// called when the queue is empty: waits for a slot to filled in for up to `LOCK_TIMEOUT_MILLIS`\
    /// -- returns true if we recovered from the empty condition; false otherwise.\
    /// Notice, however, `true` may also be returned if the mutex is being tainted elsewhere ([interrupt()], for example)
    #[inline(always)]
    fn report_empty(&self) -> bool {
        if Instruments::from(INSTRUMENTS).metrics() {
            self.queue_empty_count.fetch_add(1, Relaxed);
        }
        if Instruments::from(INSTRUMENTS).metrics_diagnostics() {
            self.metrics_diagnostics();
        }
        self.empty_guard.try_lock();
        // lock 'empty_guard' with a timeout (if applicable)
        if LOCK_TIMEOUT_MILLIS == 0 {
            if Instruments::from(INSTRUMENTS).tracing() {
                trace!("### QUEUE '{}' is empty. Waiting for a new element (indefinitely) so DEQUEUE may proceed...", self.queue_name);
            }
            self.empty_guard.lock();
        } else {
            if Instruments::from(INSTRUMENTS).tracing() {
                trace!("### QUEUE '{}' is empty. Waiting for a new element (for up to {}ms) so DEQUEUE may proceed...", self.queue_name, LOCK_TIMEOUT_MILLIS);
            }
            if !self.empty_guard.try_lock_for(Self::TRY_LOCK_DURATION) {
                if Instruments::from(INSTRUMENTS).tracing() {
                    trace!("Blocking QUEUE '{}', said to be empty, waited too much for an element to be available so it could be dequeued. Bailing out after waiting for ~{}ms", self.queue_name, LOCK_TIMEOUT_MILLIS);
                }
                return false;
            }
        }
        true
    }

    #[inline(always)]
    fn report_no_longer_empty(&self) {
        if self.empty_guard.is_locked() {
            if Instruments::from(INSTRUMENTS).tracing() {
                trace!("Blocking QUEUE '{}' is said to have just come out of the EMPTY state...", self.queue_name);
            }
            unsafe { self.empty_guard.unlock() };
        }
    }

    /// called when the queue is full: waits for a slot to be freed up to `LOCK_TIMEOUT_MILLIS`\
    /// -- returns true if we recovered from the full condition; false otherwise.\
    /// Notice, however, `true` may also be returned if the mutex is being tainted elsewhere ([interrupt()], for example)
    #[inline(always)]
    fn report_full(&self) -> bool {
        if Instruments::from(INSTRUMENTS).metrics() {
            self.queue_full_count.fetch_add(1, Relaxed);
        }
        if Instruments::from(INSTRUMENTS).metrics_diagnostics() {
            self.metrics_diagnostics();
        }
        self.full_guard.try_lock();
        // lock 'full_guard' with a timeout (if applicable)
        if LOCK_TIMEOUT_MILLIS == 0 {
            if Instruments::from(INSTRUMENTS).tracing() {
                trace!("### QUEUE '{}' is full. Waiting for a free slot (indefinitely) so ENQUEUE may proceed...", self.queue_name);
            }
            self.full_guard.lock();
        } else {
            if Instruments::from(INSTRUMENTS).tracing() {
                trace!("### QUEUE '{}' is full. Waiting for a free slot (for up to {}ms) so ENQUEUE may proceed...", self.queue_name, LOCK_TIMEOUT_MILLIS);
            }
            if !self.full_guard.try_lock_for(Self::TRY_LOCK_DURATION) {
                if Instruments::from(INSTRUMENTS).tracing() {
                    trace!("Blocking QUEUE '{}', said to be full, waited too much for a free slot to enqueue a new element. Bailing out after waiting for ~{}ms", self.queue_name, LOCK_TIMEOUT_MILLIS);
                }
                return false;
            }
        }
        true
    }

    #[inline(always)]
    fn report_no_longer_full(&self) {
        if self.full_guard.is_locked() {
            if Instruments::from(INSTRUMENTS).tracing() {
                trace!("Blocking QUEUE '{}' is said to have just come out of the FULL state...", self.queue_name);
            }
            unsafe { self.full_guard.unlock() }
        }
    }

    #[inline(always)]
    fn metrics_diagnostics(&self) {
        let locked_for_enqueueing = self.full_guard.is_locked();
        let locked_for_dequeueing = self.empty_guard.is_locked();
        let len                  = self.len();
        let enqueue_count         = self.enqueue_count.load(Relaxed);
        let queue_full_count      = self.queue_full_count.load(Relaxed);
        let dequeue_count         = self.dequeue_count.load(Relaxed);
        let queue_empty_count     = self.queue_empty_count.load(Relaxed);

        if (enqueue_count + queue_full_count + dequeue_count + queue_empty_count) % (1<<20) == 0 {
            println!("Atomic BlockingQueue '{}' state:", self.queue_name);
            println!("    CONTENTS:     {:12} elements,   {:12} buffer -- locks: enqueueing={locked_for_enqueueing}; dequeueing={locked_for_dequeueing}", len, BUFFER_SIZE);
            println!("    PRODUCTION:   {:12} successful, {:12} reported queue was full",  enqueue_count, queue_full_count);
            println!("    CONSUMPTION:  {:12} successful, {:12} reported queue was empty", dequeue_count, queue_empty_count);
        }
    }

}

impl<SlotType:                  Copy+Debug,
     const BUFFER_SIZE:         usize,
     const LOCK_TIMEOUT_MILLIS: usize,
     const INSTRUMENTS:         usize>
OgreQueue<SlotType>
for BlockingQueue<SlotType, BUFFER_SIZE, LOCK_TIMEOUT_MILLIS, INSTRUMENTS> {

    fn new<IntoString: Into<String>>(queue_name: IntoString) -> Self {
        let instance = Self {
            enqueue_count:      AtomicU64::new(0),
            queue_full_count:   AtomicU64::new(0),
            full_guard:         RawMutex::INIT,
            base_queue:         AtomicMove::new(),
            empty_guard:        RawMutex::INIT,
            dequeue_count:      AtomicU64::new(0),
            queue_empty_count:  AtomicU64::new(0),
            queue_name:         queue_name.into(),
        };
        instance.full_guard.try_lock();
        instance.empty_guard.try_lock();
        instance
    }

    #[inline(always)]
    fn enqueue(&self, element: SlotType) -> bool {
        self.base_queue.publish(|slot| {
                                    *slot = element;
                                    if Instruments::from(INSTRUMENTS).tracing() {
                                        trace!("### QUEUE '{}' enqueued element '{:?}'", self.queue_name, element);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics() {
                                        self.enqueue_count.fetch_add(1, Relaxed);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics_diagnostics() {
                                        self.metrics_diagnostics();
                                    }
                                },
                                || {
                                    self.report_full()
                                },
                                |len| if len == 1 {
                                    self.report_no_longer_empty();
                                })
    }

    #[inline(always)]
    fn dequeue(&self) -> Option<SlotType> {
        self.base_queue.consume(|slot| *slot,
                                || self.report_empty(),
                                |len| {
                                    if Instruments::from(INSTRUMENTS).tracing() {
                                        trace!("### QUEUE '{}' dequeued an element", self.queue_name);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics() {
                                        self.dequeue_count.fetch_add(1, Relaxed);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics_diagnostics() {
                                        self.metrics_diagnostics();
                                    }
                                    if len == self.base_queue.max_size() as i32 - 1 {
                                        self.report_no_longer_full();
                                    }
                                })
    }

    #[inline(always)]
    fn len(&self) -> usize {
        self.base_queue.available_elements_count()
    }

    #[inline(always)]
    fn max_size(&self) -> usize {
        self.base_queue.max_size()
    }

    fn debug_enabled(&self) -> bool {
        Instruments::from(INSTRUMENTS).tracing()
    }

    fn metrics_enabled(&self) -> bool {
        Instruments::from(INSTRUMENTS).metrics()
    }

    fn queue_name(&self) -> &str {
        self.queue_name.as_str()
    }

    fn implementation_name(&self) -> &str {
        "Atomic/Parking-lot-Blocking Queue"
    }

    fn interrupt(&self) {
        unsafe {
            self.empty_guard.unlock();
            self.full_guard.unlock();
        }
        panic!("interrupt() is not fully implemented: an 'interrupted' flag with checks on report_full() and report_empty() is missing. Since checking it impacts performance, it is not implemented by now");
    }
}


impl<SlotType:                  Copy+Debug,
     const BUFFER_SIZE:         usize,
     const LOCK_TIMEOUT_MILLIS: usize,
     const INSTRUMENTS:         usize>
OgreBlockingQueue<'_, SlotType>
for BlockingQueue<SlotType, BUFFER_SIZE, LOCK_TIMEOUT_MILLIS, INSTRUMENTS> {
    fn set_empty_guard_ref(&mut self, _empty_guard_ref: &'_ RawMutex) {
        todo!("no longer used method. remove as soon as it is guaranteed to be useless")
    }

    fn try_enqueue(&self, element: SlotType) -> bool {
        self.base_queue.publish(|slot| {
                                    *slot = element;
                                    if Instruments::from(INSTRUMENTS).tracing() {
                                        trace!("### QUEUE '{}' enqueued element '{:?}' in non-blocking mode", self.queue_name, element);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics() {
                                        self.enqueue_count.fetch_add(1, Relaxed);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics_diagnostics() {
                                        self.metrics_diagnostics();
                                    }
                                 },
                                || {
                                    if Instruments::from(INSTRUMENTS).tracing() {
                                        trace!("### QUEUE '{}' is full when enqueueing element '{:?}' in non-blocking mode", self.queue_name, element);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics() {
                                        self.queue_full_count.fetch_add(1, Relaxed);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics_diagnostics() {
                                        self.metrics_diagnostics();
                                    }
                                    self.full_guard.try_lock();
                                    false
                                },
                                |len|  if len == 1 {
                                    self.report_no_longer_empty();
                                })
    }

    fn try_dequeue(&self) -> Option<SlotType> {
        self.base_queue.consume(|slot| *slot,
                                || {
                                    if Instruments::from(INSTRUMENTS).tracing() {
                                        trace!("### QUEUE '{}' is empty when dequeueing an element in non-blocking mode", self.queue_name);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics() {
                                        self.queue_empty_count.fetch_add(1, Relaxed);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics_diagnostics() {
                                        self.metrics_diagnostics();
                                    }
                                    self.empty_guard.try_lock();
                                    false
                                },
                                |len| {
                                    if Instruments::from(INSTRUMENTS).tracing() {
                                        trace!("### QUEUE '{}' dequeued an element in non-blocking mode", self.queue_name);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics() {
                                        self.dequeue_count.fetch_add(1, Relaxed);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics_diagnostics() {
                                        self.metrics_diagnostics();
                                    }
                                    if len == self.base_queue.available_elements_count() as i32 - 1 {
                                        self.report_no_longer_full();
                                    }
                                })
    }
}


#[cfg(any(test,doc))]
mod tests {
    //! Unit tests for [atomic_base](super) module

    use super::*;
    use super::super::super::super::test_commons::{self,ContainerKind,Blocking};

    #[cfg_attr(not(doc),test)]
    fn basic_queue_use_cases() {
        let queue = BlockingQueue::<i32, 16, {Instruments::MetricsWithDiagnostics.into()}>::new("basic_use_cases' test queue".to_string());
        test_commons::basic_container_use_cases(ContainerKind::Queue, Blocking::Blocking, queue.max_size(), |e| queue.enqueue(e), || queue.dequeue(), || queue.len());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    fn single_producer_multiple_consumers() {
        let queue = BlockingQueue::<u32, 65536, {Instruments::MetricsWithDiagnostics.into()}>::new("single_producer_multiple_consumers' test queue".to_string());
        test_commons::container_single_producer_multiple_consumers(|e| queue.enqueue(e), || queue.dequeue());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    fn multiple_producers_single_consumer() {
        let queue = BlockingQueue::<u32, 65536, {Instruments::MetricsWithDiagnostics.into()}>::new("multiple_producers_single_consumer' test queue".to_string());
        test_commons::container_multiple_producers_single_consumer(|e| queue.enqueue(e), || queue.dequeue());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    pub fn multiple_producers_and_consumers_all_in_and_out() {
        let queue = BlockingQueue::<u32, 65536, {Instruments::MetricsWithDiagnostics.into()}>::new("multiple_producers_and_consumers_all_in_and_out' test queue".to_string());
        test_commons::container_multiple_producers_and_consumers_all_in_and_out(Blocking::Blocking, queue.max_size(), |e| queue.enqueue(e), || queue.dequeue());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    pub fn multiple_producers_and_consumers_single_in_and_out() {
        let queue = BlockingQueue::<u32, 65536, {Instruments::MetricsWithDiagnostics.into()}>::new("multiple_producers_and_consumers_single_in_and_out' test queue".to_string());
        test_commons::container_multiple_producers_and_consumers_single_in_and_out(|e| queue.enqueue(e), || queue.dequeue());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread: timeout measurements go south
    fn test_blocking() {
        const TIMEOUT_MILLIS: usize = 100;
        const QUEUE_SIZE:     usize = 16;
        let queue = BlockingQueue::<usize, QUEUE_SIZE, TIMEOUT_MILLIS, {Instruments::MetricsWithDiagnostics.into()}>::new("test_blocking' queue".to_string());

        test_commons::blocking_behavior(QUEUE_SIZE,
                                        |e| queue.enqueue(e),
                                        || queue.dequeue(),
                                        |e| queue.try_enqueue(e),
                                        || queue.try_dequeue(),
                                        false, || {});
    }

}
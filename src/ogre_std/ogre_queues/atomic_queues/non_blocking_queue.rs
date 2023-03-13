//! See [NonBlockingQueue]


use super::super::super::{
    ogre_queues::{
        OgreQueue,
        meta_queue::MetaQueue,
        atomic_queues::atomic_meta::AtomicMeta,
    },
    instruments::Instruments,
};
use std::{
    pin::Pin,
    fmt::Debug,
    sync::atomic::{AtomicU64,Ordering::Relaxed},
};
use std::time::Duration;
use log::trace;


/// Multiple producer / multiple consumer lock-free & non-blocking queue
/// using atomics for synchronization
#[repr(C,align(64))]      // aligned to cache line sizes for a careful control over false-sharing performance degradation
pub struct NonBlockingQueue<SlotType:          Copy+Debug,
                            const BUFFER_SIZE: usize,
                            const INSTRUMENTS: usize = 0> {
    // metrics for enqueue
    enqueue_count:      AtomicU64,
    queue_full_count:   AtomicU64,
    /// queue
    base_queue:         AtomicMeta<SlotType, BUFFER_SIZE>,
    // metrics for dequeue
    dequeue_count:      AtomicU64,
    queue_empty_count:  AtomicU64,
    /// for use when debug is enabled
    queue_name:         String,
}

impl<SlotType:          Copy+Debug,
    const BUFFER_SIZE: usize,
    const INSTRUMENTS: usize>
NonBlockingQueue<SlotType, BUFFER_SIZE, INSTRUMENTS> {

    #[inline(always)]
    fn metrics_diagnostics(&self) {
        let enqueue_count         = self.enqueue_count.load(Relaxed);
        let queue_full_count      = self.queue_full_count.load(Relaxed);
        let dequeue_count         = self.dequeue_count.load(Relaxed);
        let queue_empty_count     = self.queue_empty_count.load(Relaxed);
        let len                 = self.len();
        let head                  = self.base_queue.head.load(Relaxed);
        let tail                  = self.base_queue.head.load(Relaxed);
        let dequeuer_head         = self.base_queue.dequeuer_head.load(Relaxed);
        let enqueuer_tail         = self.base_queue.enqueuer_tail.load(Relaxed);

        if (enqueue_count + queue_full_count + dequeue_count + queue_empty_count) % 1024000 == 0 {
            println!("Atomic BlockingQueue '{}'", self.queue_name);
            println!("    STATE:        head: {:8}, tail: {:8}, dequeuer_head: {:8}, enqueuer_tail: {:8} ", head, tail, dequeuer_head, enqueuer_tail);
            println!("    CONTENTS:     {:12} elements,   {:12} buffer", len, BUFFER_SIZE);
            println!("    PRODUCTION:   {:12} successful, {:12} reported queue was full",  enqueue_count, queue_full_count);
            println!("    CONSUMPTION:  {:12} successful, {:12} reported queue was empty", dequeue_count, queue_empty_count);
        }
    }

}

impl<SlotType:          Copy+Debug,
     const BUFFER_SIZE: usize,
     const INSTRUMENTS: usize>
OgreQueue<SlotType>
for NonBlockingQueue<SlotType, BUFFER_SIZE, INSTRUMENTS> {

    fn new<IntoString: Into<String>>(queue_name: IntoString) -> Pin<Box<Self>> where Self: Sized {
        Box::pin(Self {
            enqueue_count:      AtomicU64::new(0),
            queue_full_count:   AtomicU64::new(0),
            base_queue:         AtomicMeta::new(),
            dequeue_count:      AtomicU64::new(0),
            queue_empty_count:  AtomicU64::new(0),
            queue_name:         queue_name.into(),
        })
    }

    #[inline(always)]
    fn enqueue(&self, element: SlotType) -> bool {
        self.base_queue.enqueue(|slot| {
                                    *slot = element;
                                    if Instruments::from(INSTRUMENTS).tracing() {
                                        trace!("### '{}' ENQUEUE: enqueueing element '{:?}'", self.queue_name, element);
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
                                        trace!("### '{}' ENQUEUE: queue is full when attempting to enqueue element '{:?}'", self.queue_name, element);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics() {
                                        self.queue_full_count.fetch_add(1, Relaxed);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics_diagnostics() {
                                        self.metrics_diagnostics();
                                    }
                                    false
                                },
                                |_| {})
    }

    #[inline(always)]
    fn dequeue(&self) -> Option<SlotType> {
        self.base_queue.dequeue(|slot| *slot,
                       || {
                                    if Instruments::from(INSTRUMENTS).tracing() {
                                        trace!("### '{}' DEQUEUE: queue is empty when attempting to dequeue an element", self.queue_name);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics() {
                                        self.queue_empty_count.fetch_add(1, Relaxed);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics_diagnostics() {
                                        self.metrics_diagnostics();
                                    }
                                    false
                                },
                                |_| {
                                    if Instruments::from(INSTRUMENTS).tracing() {
                                        trace!("### '{}' DEQUEUE: dequeued an element", self.queue_name);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics() {
                                        self.dequeue_count.fetch_add(1, Relaxed);
                                    }
                                    if Instruments::from(INSTRUMENTS).metrics_diagnostics() {
                                        self.metrics_diagnostics();
                                    }
                                })
    }

    fn len(&self) -> usize {
        self.base_queue.len()
    }

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
        "Atomic/Non-Blocking Queue"
    }

    fn interrupt(&self) {
        todo!()
    }
}


#[cfg(any(test, feature="dox"))]
mod tests {
    //! Unit tests for [non_blocking_queue](super) module

    use super::*;
    use super::super::super::super::test_commons::{self,ContainerKind,Blocking};

    #[test]
    fn basic_queue_use_cases() {
        let queue = NonBlockingQueue::<i32, 16, {Instruments::NoInstruments.into()}>::new("basic_use_cases' test queue".to_string());
        test_commons::basic_container_use_cases(ContainerKind::Queue, Blocking::NonBlocking, queue.max_size(), |e| queue.enqueue(e), || queue.dequeue(), || queue.len());
    }

    #[test]
    fn single_producer_multiple_consumers() {
        let queue = NonBlockingQueue::<u32, 65536, {Instruments::NoInstruments.into()}>::new("single_producer_multiple_consumers' test queue".to_string());
        test_commons::container_single_producer_multiple_consumers(|e| queue.enqueue(e), || queue.dequeue());
    }

    #[test]
    fn multiple_producers_single_consumer() {
        let queue = NonBlockingQueue::<u32, 65536, {Instruments::NoInstruments.into()}>::new("multiple_producers_single_consumer' test queue".to_string());
        test_commons::container_multiple_producers_single_consumer(|e| queue.enqueue(e), || queue.dequeue());
    }

    #[test]
    pub fn multiple_producers_and_consumers_all_in_and_out() {
        let queue = NonBlockingQueue::<u32, {1024*64}, {Instruments::NoInstruments.into()}>::new("multiple_producers_and_consumers_all_in_and_out' test queue".to_string());
        test_commons::container_multiple_producers_and_consumers_all_in_and_out(Blocking::NonBlocking, queue.max_size(), |e| queue.enqueue(e), || queue.dequeue());
    }

    #[test]
    pub fn multiple_producers_and_consumers_single_in_and_out() {
        let queue = NonBlockingQueue::<u32, 128, {Instruments::NoInstruments.into()}>::new("multiple_producers_and_consumers_single_in_and_out' test queue".to_string());
        test_commons::container_multiple_producers_and_consumers_single_in_and_out(|e| queue.enqueue(e), || queue.dequeue());
    }
}
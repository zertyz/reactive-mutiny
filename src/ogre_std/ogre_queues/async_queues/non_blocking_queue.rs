//! See [NonBlockingQueue]

use super::super::super::{
    ogre_queues::{
        OgreQueue,
        async_queues::async_base::AsyncBase,
    },
};
use std::{
    pin::Pin,
    fmt::Debug,
    sync::atomic::{AtomicU32,AtomicU64,Ordering::Relaxed},
    mem::MaybeUninit,
};
use log::trace;


/// Multiple producer / multiple consumer queues using full synchronization & async functions
#[repr(C,align(64))]      // aligned to cache line sizes for a careful control over false-sharing performance degradation
pub struct NonBlockingQueue<SlotType:          Copy+Debug,
                            const BUFFER_SIZE: usize,
                            const METRICS:     bool = false,
                            const DEBUG:       bool = false> {
    // metrics for enqueue
    enqueue_count:      AtomicU64,
    queue_full_count:   AtomicU64,
    /// queue
    base_queue:         AsyncBase<SlotType, BUFFER_SIZE>,
    // metrics for dequeue
    dequeue_count:      AtomicU64,
    queue_empty_count:  AtomicU64,
    /// for use when debug is enabled
    queue_name:         String,
}
impl<SlotType:          Copy+Debug,
     const BUFFER_SIZE: usize,
     const METRICS:     bool,
     const DEBUG:       bool>
/*OgreQueue<SlotType>
for */NonBlockingQueue<SlotType, BUFFER_SIZE, METRICS, DEBUG> {

    fn new<IntoString: Into<String>>(queue_name: IntoString) -> Pin<Box<Self>> where Self: Sized {
        Box::pin(Self {
            enqueue_count:      AtomicU64::new(0),
            queue_full_count:   AtomicU64::new(0),
            base_queue:         AsyncBase::new(),
            dequeue_count:      AtomicU64::new(0),
            queue_empty_count:  AtomicU64::new(0),
            queue_name:         queue_name.into(),
        })
    }

    #[inline(always)]
    pub async fn enqueue(&self, element: SlotType) -> bool {
        self.base_queue.enqueue(|slot| {
                                    if DEBUG {
                                        trace!("### '{}' ENQUEUE: enqueueing element '{:?}'", self.queue_name, element);
                                    }
                                    if METRICS {
                                        self.enqueue_count.fetch_add(1, Relaxed);
                                    }
                                    *slot = element;
                                 },
                                || async {
                                    if DEBUG {
                                        trace!("### '{}' ENQUEUE: queue is full when attempting to enqueue element '{:?}'", self.queue_name, element);
                                    }
                                    if METRICS {
                                        self.queue_full_count.fetch_add(1, Relaxed);
                                    }
                                    false
                                },
                                |_| async {}).await
    }

    #[inline(always)]
    pub async fn dequeue(&self) -> Option<SlotType> {
        self.base_queue.dequeue(|| async {
                                if DEBUG {
                                    trace!("### '{}' DEQUEUE: queue is empty when attempting to dequeue an element", self.queue_name);
                                }
                                if METRICS {
                                    self.queue_empty_count.fetch_add(1, Relaxed);
                                }
                                    false
                                },
                                |_| async {
                                    if DEBUG {
                                        trace!("### '{}' DEQUEUE: dequeued an element", self.queue_name);
                                    }
                                    if METRICS {
                                        self.dequeue_count.fetch_add(1, Relaxed);
                                    }
                                }).await
    }

    pub fn len(&self) -> usize {
        self.base_queue.len()
    }

    pub fn buffer_size(&self) -> usize {
        self.base_queue.buffer_size()
    }

    pub fn debug_enabled(&self) -> bool {
        DEBUG
    }

    pub fn metrics_enabled(&self) -> bool {
        METRICS
    }

    pub fn queue_name(&self) -> &str {
        self.queue_name.as_str()
    }

    pub fn implementation_name(&self) -> &str {
        "Async Queue"
    }

    fn interrupt(&self) {
        todo!()
    }
}


#[cfg(any(test, feature="dox"))]
mod tests {
    //! Unit tests for [non_blocking_queue](super) module
    //! NOTE: we won't unit test this module as it is async and Rust 1.63 still don't support zero-cost async traits
/*
    use super::*;
    use crate::test_commons::{self,ContainerKind,Blocking};

    #[test]
    fn basic_queue_use_cases() {
        let queue = NonBlockingQueue::<i32, 16, false, false>::new("'basic_use_cases' test queue".to_string());
        test_commons::basic_container_use_cases(ContainerKind::Queue, Blocking::NonBlocking, queue.buffer_size(), |e| queue.enqueue(e), || queue.dequeue(), || queue.len());
    }

    #[test]
    fn single_producer_multiple_consumers() {
        let queue = NonBlockingQueue::<u32, 65536, false, false>::new("'single_producer_multiple_consumers' test queue".to_string());
        test_commons::container_single_producer_multiple_consumers(|e| queue.enqueue(e), || queue.dequeue());
    }

    #[test]
    fn multiple_producers_single_consumer() {
        let queue = NonBlockingQueue::<u32, 65536, false, false>::new("'multiple_producers_single_consumer' test queue".to_string());
        test_commons::container_multiple_producers_single_consumer(|e| queue.enqueue(e), || queue.dequeue());
    }

    #[test]
    pub fn multiple_producers_and_consumers_all_in_and_out() {
        let queue = NonBlockingQueue::<u32, 102400, false, false>::new("'multiple_producers_and_consumers_all_in_and_out' test queue".to_string());
        test_commons::container_multiple_producers_and_consumers_all_in_and_out(Blocking::NonBlocking, queue.buffer_size(), |e| queue.enqueue(e), || queue.dequeue());
    }

    #[test]
    pub fn multiple_producers_and_consumers_single_in_and_out() {
        let queue = NonBlockingQueue::<u32, 128, false, false>::new("'multiple_producers_and_consumers_single_in_and_out' test queue".to_string());
        test_commons::container_multiple_producers_and_consumers_single_in_and_out(|e| queue.enqueue(e), || queue.dequeue());
    }*/
}
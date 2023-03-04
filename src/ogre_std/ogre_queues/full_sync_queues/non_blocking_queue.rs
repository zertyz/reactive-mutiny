//! See [NonBlockingQueue]

use super::super::super::{
    ogre_queues::{
        OgreQueue,
        meta_queue::MetaQueue,
        full_sync_queues::full_sync_base::FullSyncBase,
    },
};
use std::{
    pin::Pin,
    fmt::Debug,
    sync::atomic::{AtomicU32,AtomicU64,Ordering::Relaxed},
    mem::MaybeUninit,
};
use std::mem::ManuallyDrop;
use std::ops::Deref;
use log::trace;


/// Multiple producer / multiple consumer queues using full synchronization & async functions
#[repr(C,align(64))]      // aligned to cache line sizes for a careful control over false-sharing performance degradation
pub struct NonBlockingQueue<SlotType:          Unpin + Debug,
                            const BUFFER_SIZE: usize,
                            const METRICS:     bool = false,
                            const DEBUG:       bool = false> {
    // metrics for enqueue
    enqueue_count:      AtomicU64,
    queue_full_count:   AtomicU64,
    /// queue
    base_queue:         FullSyncBase<SlotType, BUFFER_SIZE>,
    // metrics for dequeue
    dequeue_count:      AtomicU64,
    queue_empty_count:  AtomicU64,
    /// for use when debug is enabled
    queue_name:         String,
}
impl<SlotType:          Unpin + Debug,
     const BUFFER_SIZE: usize,
     const METRICS:     bool,
     const DEBUG:       bool>
OgreQueue<SlotType>
for NonBlockingQueue<SlotType, BUFFER_SIZE, METRICS, DEBUG> {

    fn new<IntoString: Into<String>>(queue_name: IntoString) -> Pin<Box<Self>> where Self: Sized {
        Box::pin(Self {
            enqueue_count:      AtomicU64::new(0),
            queue_full_count:   AtomicU64::new(0),
            base_queue:         FullSyncBase::new(),
            dequeue_count:      AtomicU64::new(0),
            queue_empty_count:  AtomicU64::new(0),
            queue_name:         queue_name.into(),
        })
    }

    #[inline(always)]
    fn enqueue(&self, element: SlotType) -> bool {
        let element = ManuallyDrop::new(element);       // ensure it won't be dropped when this function ends, since it will be "moved"
        self.base_queue.enqueue(|slot| {
                                    if DEBUG {
                                        trace!("### '{}' ENQUEUE: enqueueing element '{:?}'", self.queue_name, element);
                                    }
                                    if METRICS {
                                        self.enqueue_count.fetch_add(1, Relaxed);
                                    }
                                    // moves `element` to the buffer -- it won't be dropped after this function ends since it has been wrapped into a `ManuallyDrop`
                                    unsafe { std::ptr::copy_nonoverlapping(element.deref() as *const SlotType, (slot as *const SlotType) as *mut SlotType, 1) }
                                 },
                                || {
                                    if DEBUG {
                                        trace!("### '{}' ENQUEUE: queue is full when attempting to enqueue element '{:?}'", self.queue_name, element);
                                    }
                                    if METRICS {
                                        self.queue_full_count.fetch_add(1, Relaxed);
                                    }
                                    false
                                },
                                |_| {})
    }

    #[inline(always)]
    fn dequeue(&self) -> Option<SlotType> {
        self.base_queue.dequeue(|slot| {
                                    let mut moved_value = MaybeUninit::<SlotType>::uninit();
                                    unsafe { std::ptr::copy_nonoverlapping(slot as *const SlotType, moved_value.as_mut_ptr(), 1) }
                                    unsafe { moved_value.assume_init() }
                                },
                                || {
                                if DEBUG {
                                    trace!("### '{}' DEQUEUE: queue is empty when attempting to dequeue an element", self.queue_name);
                                }
                                if METRICS {
                                    self.queue_empty_count.fetch_add(1, Relaxed);
                                }
                                    false
                                },
                                |_| {
                                    if DEBUG {
                                        trace!("### '{}' DEQUEUE: dequeued an element", self.queue_name);
                                    }
                                    if METRICS {
                                        self.dequeue_count.fetch_add(1, Relaxed);
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
        DEBUG
    }

    fn metrics_enabled(&self) -> bool {
        METRICS
    }

    fn queue_name(&self) -> &str {
        self.queue_name.as_str()
    }

    fn implementation_name(&self) -> &str {
        "Full Sync / Non-Blocking Queue"
    }

    fn interrupt(&self) {
        todo!()
    }
}

impl<SlotType:          Unpin + Debug,
     const BUFFER_SIZE: usize,
     const METRICS:     bool,
     const DEBUG:       bool>
NonBlockingQueue<SlotType, BUFFER_SIZE, METRICS, DEBUG> {

    /// See [MetaQueue::peek_all()]
    #[inline(always)]
    pub unsafe fn peek_all(&self) -> [&[SlotType];2] {
        self.base_queue.peek_all()
    }

}


#[cfg(any(test, feature="dox"))]
mod tests {
    //! Unit tests for [non_blocking_queue](super) module

    use super::*;
    use super::super::super::super::test_commons::{self,ContainerKind,Blocking};

    #[test]
    fn basic_queue_use_cases() {
        let queue = NonBlockingQueue::<i32, 16, false, false>::new("'basic_use_cases' test queue");
        test_commons::basic_container_use_cases(ContainerKind::Queue, Blocking::NonBlocking, queue.max_size(), |e| queue.enqueue(e), || queue.dequeue(), || queue.len());
    }

    #[test]
    fn single_producer_multiple_consumers() {
        let queue = NonBlockingQueue::<u32, 65536, false, false>::new("'single_producer_multiple_consumers' test queue");
        test_commons::container_single_producer_multiple_consumers(|e| queue.enqueue(e), || queue.dequeue());
    }

    #[test]
    fn multiple_producers_single_consumer() {
        let queue = NonBlockingQueue::<u32, 65536, false, false>::new("'multiple_producers_single_consumer' test queue");
        test_commons::container_multiple_producers_single_consumer(|e| queue.enqueue(e), || queue.dequeue());
    }

    #[test]
    pub fn multiple_producers_and_consumers_all_in_and_out() {
        let queue = NonBlockingQueue::<u32, 102400, false, false>::new("'multiple_producers_and_consumers_all_in_and_out' test queue");
        test_commons::container_multiple_producers_and_consumers_all_in_and_out(Blocking::NonBlocking, queue.max_size(), |e| queue.enqueue(e), || queue.dequeue());
    }

    #[test]
    pub fn multiple_producers_and_consumers_single_in_and_out() {
        let queue = NonBlockingQueue::<u32, 128, false, false>::new("'multiple_producers_and_consumers_single_in_and_out' test queue");
        test_commons::container_multiple_producers_and_consumers_single_in_and_out(|e| queue.enqueue(e), || queue.dequeue());
    }

    #[test]
    pub fn peek_test() {
        let queue = NonBlockingQueue::<u32, 16, false, false>::new("'peek_test' queue");

        // tests peeking [&[0..n], &[]]
        for i in 1..=16 {
            queue.enqueue(i);
        }
        let expected_sum = (1+16)*(16/2);
        let mut observed_sum = 0;
        for item in unsafe { queue.peek_all().iter().flat_map(|&slice| slice) } {
            observed_sum += item;
        }
        assert_eq!(observed_sum, expected_sum, "peeking elements from [&[0..n], &[]] didn't work");

        // tests peeking [&[8..n], &[0..8]]
        for i in 1..=8 {
            assert_eq!(queue.dequeue(), Some(i), "Dequeued element is wrong. This test is likely to be wrong.")
        }
        for i in 17..=(17+8) {
            queue.enqueue(i);
        }
        let expected_sum = (9+9+16-1)*(16/2);
        let mut observed_sum = 0;
        for item in unsafe { queue.peek_all().iter().flat_map(|&slice| slice) } {
            observed_sum += item;
        }
        assert_eq!(observed_sum, expected_sum, "peeking elements from [&[8..n], &[0..8]] didn't work");

    }
}
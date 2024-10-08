//! See [NonBlockingQueue]

use super::super::super::{
    instruments::Instruments,
    ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator,
    ogre_queues::{
        OgreQueue,
        full_sync::full_sync_zero_copy::FullSyncZeroCopy,
        meta_publisher::MetaPublisher,
        meta_subscriber::MetaSubscriber,
        meta_container::MetaContainer,
    },
};
use std::{
    fmt::Debug,
    sync::atomic::{AtomicU64,Ordering::Relaxed},
    mem::MaybeUninit,
};
use log::trace;


/// Multiple producer / multiple consumer queues using full synchronization & async functions
#[repr(C,align(64))]      // aligned to cache line sizes for a careful control over false-sharing performance degradation
pub struct NonBlockingQueue<SlotType:          Unpin + Debug + Send + Sync,
                            const BUFFER_SIZE: usize,
                            const INSTRUMENTS: usize = 0> {
    // metrics for enqueue
    enqueue_count:      AtomicU64,
    queue_full_count:   AtomicU64,
    /// queue
    base_queue:         FullSyncZeroCopy<SlotType, OgreArrayPoolAllocator<SlotType, super::full_sync_move::FullSyncMove<u32, BUFFER_SIZE>, BUFFER_SIZE>, BUFFER_SIZE>,
    // metrics for dequeue
    dequeue_count:      AtomicU64,
    queue_empty_count:  AtomicU64,
    /// for use when debug is enabled
    queue_name:         String,
}
impl<SlotType:          Unpin + Debug + Send + Sync,
     const BUFFER_SIZE: usize,
     const INSTRUMENTS: usize>
OgreQueue<SlotType>
for NonBlockingQueue<SlotType, BUFFER_SIZE, INSTRUMENTS> {

    fn new<IntoString: Into<String>>(queue_name: IntoString) -> Self {
        Self {
            enqueue_count:      AtomicU64::new(0),
            queue_full_count:   AtomicU64::new(0),
            base_queue:         FullSyncZeroCopy::new(),
            dequeue_count:      AtomicU64::new(0),
            queue_empty_count:  AtomicU64::new(0),
            queue_name:         queue_name.into(),
        }
    }

    /// TODO: 2023-05-17: make it zero-copy <F: FnOnce(&mut ItemType)>
    #[inline(always)]
    fn enqueue(&self, element: SlotType) -> Option<SlotType> {
        if Instruments::from(INSTRUMENTS).tracing() {
            trace!("### '{}' ENQUEUE: enqueueing element '{:?}'", self.queue_name, element);
        }
        match self.base_queue.publish_movable(element) {
            ( Some(_len_after), _none_element ) => {
                if Instruments::from(INSTRUMENTS).metrics() {
                    self.enqueue_count.fetch_add(1, Relaxed);
                }
                None
            }
            ( None, some_element ) => {
                if Instruments::from(INSTRUMENTS).tracing() {
                    trace!("### '{}' ENQUEUE: queue is full. Last enqueueing failed", self.queue_name);
                }
                if Instruments::from(INSTRUMENTS).metrics() {
                    self.queue_full_count.fetch_add(1, Relaxed);
                }
                some_element
            }
        }
    }

    #[inline(always)]
    fn dequeue(&self) -> Option<SlotType> {
        self.base_queue.consume(|slot| {
                                    let mut moved_value = MaybeUninit::<SlotType>::uninit();
                                    unsafe { std::ptr::copy_nonoverlapping(slot as *const SlotType, moved_value.as_mut_ptr(), 1) }
                                    unsafe { moved_value.assume_init() }
                                },
                                || {
                                if Instruments::from(INSTRUMENTS).tracing() {
                                    trace!("### '{}' DEQUEUE: queue is empty when attempting to dequeue an element", self.queue_name);
                                }
                                if Instruments::from(INSTRUMENTS).metrics() {
                                    self.queue_empty_count.fetch_add(1, Relaxed);
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
                                })
    }

    #[inline(always)]
    fn len(&self) -> usize {
        self.base_queue.available_elements_count()
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
        "Full Sync / Non-Blocking Queue"
    }

    fn interrupt(&self) {
        todo!()
    }
}

impl<SlotType:          Unpin + Debug + Send + Sync,
     const BUFFER_SIZE: usize,
     const INSTRUMENTS: usize>
NonBlockingQueue<SlotType, BUFFER_SIZE, INSTRUMENTS> {

    /// # Safety
    /// See [MetaSubscriber::peek_all()]
    #[inline(always)]
    pub unsafe fn peek_remaining(&self) -> [&[SlotType];2] {
        //self.base_queue.peek_remaining()
        [&[],&[]]
    }

}


#[cfg(any(test,doc))]
mod tests {
    //! Unit tests for [non_blocking_queue](super) module

    use super::*;
    use super::super::super::super::test_commons::{self,ContainerKind,Blocking};

    #[cfg_attr(not(doc),test)]
    fn basic_queue_use_cases() {
        let queue = NonBlockingQueue::<i32, 16, {Instruments::MetricsWithDiagnostics.into()}>::new("'basic_use_cases' test queue");
        test_commons::basic_container_use_cases(queue.queue_name(), ContainerKind::Queue, Blocking::NonBlocking, queue.max_size(),
                                                |e| queue.enqueue(e).is_none(), || queue.dequeue(), || queue.len());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    fn single_producer_multiple_consumers() {
        let queue = NonBlockingQueue::<u32, 65536, {Instruments::MetricsWithDiagnostics.into()}>::new("'single_producer_multiple_consumers' test queue");
        test_commons::container_single_producer_multiple_consumers(queue.queue_name(),
                                                                   |e| queue.enqueue(e).is_none(),
                                                                   || queue.dequeue());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    fn multiple_producers_single_consumer() {
        let queue = NonBlockingQueue::<u32, 65536, {Instruments::MetricsWithDiagnostics.into()}>::new("'multiple_producers_single_consumer' test queue");
        test_commons::container_multiple_producers_single_consumer(queue.queue_name(),
                                                                   |e| queue.enqueue(e).is_none(),
                                                                   || queue.dequeue());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    pub fn multiple_producers_and_consumers_all_in_and_out() {
        let queue = NonBlockingQueue::<u32, 65536, {Instruments::MetricsWithDiagnostics.into()}>::new("'multiple_producers_and_consumers_all_in_and_out' test queue");
        test_commons::container_multiple_producers_and_consumers_all_in_and_out(queue.queue_name(),
                                                                                Blocking::NonBlocking,
                                                                                queue.max_size(),
                                                                                |e| queue.enqueue(e).is_none(),
                                                                                || queue.dequeue());
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // flaky if ran in multi-thread?
    pub fn multiple_producers_and_consumers_single_in_and_out() {
        let queue = NonBlockingQueue::<u32, 65536, {Instruments::MetricsWithDiagnostics.into()}>::new("'multiple_producers_and_consumers_single_in_and_out' test queue");
        test_commons::container_multiple_producers_and_consumers_single_in_and_out(queue.queue_name(),
                                                                                   |e| queue.enqueue(e).is_none(),
                                                                                   || queue.dequeue());
    }

    #[cfg_attr(not(doc),test)]
    pub fn peek_test() {
        let queue = NonBlockingQueue::<u32, 16, {Instruments::MetricsWithDiagnostics.into()}>::new("'peek_test' queue");
        test_commons::peak_remaining(queue.queue_name(),
                                     |e| queue.enqueue(e).is_none(),
                                     || queue.dequeue(),
                                     || unsafe {
                                         let mut iter = queue.peek_remaining().into_iter();
                                         ( iter.next().expect("no item @0").iter(),
                                           iter.next().expect("no item @1").iter() )
                                     } );
    }
}
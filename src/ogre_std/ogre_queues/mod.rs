use std::pin::Pin;
use parking_lot::RawMutex;

pub mod atomic_queues;
pub mod full_sync_queues;
pub mod async_queues;
pub mod blocking_queue;
pub mod non_blocking_parking_lot_queue;


/// multi-producer / multi-consumer queue
pub trait OgreQueue<SlotType> {
    fn new<IntoString: Into<String>>(queue_name: IntoString) -> Pin<Box<Self>> where Self: Sized;
    /// Attempts to add `element` to queue, returning immediately for non-blocking queues... possibly blocking otherwise.
    /// Returns `true` if the element was added, `false` if the queue was full... blocking queues might never return `false`.
    fn enqueue(&self, element: SlotType) -> bool;
    /// Attempts to dequeue an element from the queue, returning immediately for non-blocking queues... possibly blocking otherwise.
    /// Returns `Some(element)` if it was possible to dequeue, `None` if the queue was empty... blocking queues might never return `None`.
    fn dequeue(&self) -> Option<SlotType>;
    /// tells how many elements are awaiting on the queue
    fn len(&self) -> usize;
    fn buffer_size(&self) -> usize;
    fn debug_enabled(&self) -> bool;
    fn metrics_enabled(&self) -> bool;
    fn queue_name(&self) -> &str;
    fn implementation_name(&self) -> &str;
    /// informs the queue of the caller's intention of dropping it or shutdown the application, so it may, immediately,
    /// wake all mutexes & stop any spin loops and cause any waiting methods ([enqueue()] or [dequeue()]) to return immediately
    fn interrupt(&self);
}

/// multi-producer / multi-consumer queue with *blocking* behavior\
/// -- queues implementing this trait will have their `enqueue()` & `dequeue()` methods possibly blocking,
/// but the new methods, `try_enqueue()` and `try_dequeue()` are guaranteed never to block.
pub trait OgreBlockingQueue<'a, SlotType>: OgreQueue<SlotType> {
    /// uses the given value as the `full_guard`, instead of the default one created when [new()] was called\
    /// -- if multiple queues use the same mutex for the full_guard guard,
    fn set_empty_guard_ref(&mut self, empty_guard_ref: &'a RawMutex);
    /// similar to [enqueue()], but is guaranteed to return immediately (never blocks)
    fn try_enqueue(&self, element: SlotType) -> bool;
    /// similar to [dequeue()], but is guaranteed to return immediately (never blocks)
    fn try_dequeue(&self) -> Option<SlotType>;
}


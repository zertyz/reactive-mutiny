use parking_lot::RawMutex;

pub mod meta_publisher;
pub mod meta_subscriber;
pub mod meta_container;
pub mod meta_topic;
pub mod atomic;
pub mod full_sync;
pub mod log_topics;


/// multi-producer / multi-consumer non-blocking queue
pub trait OgreQueue<SlotType> {
    fn new<IntoString: Into<String>>(queue_name: IntoString) -> Self;
    /// Attempts to add `element` to queue, returning immediately for non-blocking queues... possibly blocking otherwise.
    /// Returns:
    ///   - `None` if the element was added
    ///   - `Some<element>` if the enqueueing was denied (queue full) -- returning back the element to the caller to allow zero-copy retrying actions.
    /// TODO 2023-08-01 review this whole... is this a blocking or non-blocking queue? does it still make sense when put together with all the other queue traits?
    fn enqueue(&self, element: SlotType) -> Option<SlotType>;
    /// Attempts to dequeue an element from the queue, returning immediately for non-blocking queues... possibly blocking otherwise.
    /// Returns `Some(element)` if it was possible to dequeue, `None` if the queue was empty... blocking queues might never return `None`.
    fn dequeue(&self) -> Option<SlotType>;
    /// tells how many elements are awaiting on the queue
    fn len(&self) -> usize;
    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn max_size(&self) -> usize;
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
    fn try_enqueue(&self, element: SlotType) -> Option<SlotType>;
    /// similar to [dequeue()], but is guaranteed to return immediately (never blocks)
    fn try_dequeue(&self) -> Option<SlotType>;
}


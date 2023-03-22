//! Resting place for the [MetaSubscriber] trait.


/// API for consuming elements from a [meta_queue] or [meta_topic].
pub trait MetaSubscriber<SlotType> {

    /// Zero-copy dequeue method with the following characteristics:
    ///   - If the queue is found to be empty, `report_empty_fn()` is called. Specializations of this `meta-subscriber` might use it to build a blocking queue, for instance;
    ///   - `getter_fn(&slot)` will be called to inform what is the dequeued element;
    ///   - `report_len_after_dequeueing_fn(len)` might be used by specializations of this `meta-subscriber` to, for instance, set the hardware's clock down.
    /// Caveats:
    ///   1) The caller must ensure the `getter_fn()` operation returns as soon as possible, or else the whole queue is likely to hang. If so, one sould consider to pass in a `getter_fn()`
    ///      that would clone/copy the value and release the queue as soon as possible.
    ///   2) Note the `getter_fn()` is not `FnOnce()`. Some implementors might require calling this function more than once, on contention scenarios.
    fn consume<GetterReturnType,
               GetterFn:                   Fn(&mut SlotType) -> GetterReturnType,
               ReportEmptyFn:              Fn() -> bool,
               ReportLenAfterDequeueingFn: FnOnce(i32)>
              (&self,
               getter_fn:                      GetterFn,
               report_empty_fn:                ReportEmptyFn,
               report_len_after_dequeueing_fn: ReportLenAfterDequeueingFn)
              -> Option<GetterReturnType>;

    /// Considering parallelism, this method *might* provide access to all elements available for [consume()].\
    /// This method is totally not thread safe for ring-buffer based implementations -- the moment it returns, all those elements might have already
    /// been consumed (furthermore, by the time the references are used, several generations of elements might have
    /// already lived and died at those slots). It is, thought, safe for unbounded implementations that never collect published elements, like a `log_topic`\
    /// ... So, use this method only when you're sure the publishing / consumption operations won't interfere with the results
    /// -- for this reason, this method is marked as `unsafe` (**it is only safe -- and consistent -- to call this method if you're behind a lock
    /// guarding against these mentioned scenarios**).
    ///
    /// The rather wired return type here is to avoid heap allocations: a fixed array of two slices are returned.
    /// Ring-buffer based implementations will use it to reference the internal buffer -- the second slice is used
    /// if the sequence of references cycles through the buffer.\
    /// Use this method like the following:
    /// ```nocompile
    ///   // if you don't care for allocating a vector:
    ///   let peeked_references = queue.peek_all().concat();
    ///   // if you require zero-allocations:
    ///   for peeked_chunk in queue.peek_all() {
    ///     for peeked_reference in peeked_chunk {
    ///       println!("your_logic_goes_here: {:#?}", *peeked_reference);
    ///     }
    ///   }
    ///   // or, in the condensed, functional form:
    ///   for peeked_reference in queue.peek_all().iter().flat_map(|&slice| slice) {
    ///       println!("your_logic_goes_here: {:#?}", *peeked_reference);
    ///   }
    unsafe fn peek_all(&self) -> [&[SlotType];2];
    // TODO: rename to "peek_remaining()"
}
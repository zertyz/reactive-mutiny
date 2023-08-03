//! Resting place for the [MetaSubscriber] trait.


/// API for consuming elements from a [meta_queue] or [meta_topic].\
/// Two zero-copy patterns are available:
///   1) `consume()` is the simples & safest: a callback closure is provided, which receives a reference to the data -- with the ability to return a value, also returned by `consume()`;
///   2) `consume_leaking()` / `release_leaked()` offers a more flexible (but more dangerous) option, allowing the reference slot to participate in more complex logics.
pub trait MetaSubscriber<'a, SlotType: 'a> {

    /// Zero-copy dequeue method with the following characteristics:
    ///   - If the queue is found to be empty, `report_empty_fn()` is called. Specializations of this `meta-subscriber` might use it to build a blocking queue, for instance;
    ///   - `getter_fn(&slot)` will be called to inform what is the dequeued element;
    ///   - `report_len_after_dequeueing_fn(len)` might be used by specializations of this `meta-subscriber` to, for instance, set the hardware's clock down.
    /// Caveats:
    ///   1) The caller must ensure the `getter_fn()` operation returns as soon as possible, or else the whole queue is likely to hang. If so, one sould consider to pass in a `getter_fn()`
    ///      that would clone/copy the value and release the queue as soon as possible.
    ///   2) Note the `getter_fn()` is not `FnOnce()`. Some implementors might require calling this function more than once, on contention scenarios.
    /// IMPLEMENTORS: #[inline(always)]
    fn consume<GetterReturnType: 'a,
               GetterFn:                   Fn(&SlotType) -> GetterReturnType,
               ReportEmptyFn:              Fn() -> bool,
               ReportLenAfterDequeueingFn: FnOnce(i32)>
              (&self,
               getter_fn:                      GetterFn,
               report_empty_fn:                ReportEmptyFn,
               report_len_after_dequeueing_fn: ReportLenAfterDequeueingFn)
              -> Option<GetterReturnType>;

    /// Advanced method to "consume" the next element from the pool, returning a reference to the data.\
    /// The slot in which the data sits won't be put back into the pool (for reuse) until [release_leaked()] is called.\
    /// Please notice that misuse of this method may bring the underlying container into an unusable state, as it may run out of slots
    /// for new elements to be published in.\
    /// IMPLEMENTORS: #[inline(always)]
    fn consume_leaking(&'a self) -> Option<(/*ref:*/ &'a SlotType, /*id: */u32)>;

    /// Put the `slot` returned by [consume_leaking()] back into the pool, so it may be reused.\
    /// See the mentioned method for more info.
    /// IMPLEMENTORS: #[inline(always)]
    fn release_leaked_ref(&'a self, slot: &'a SlotType);

    /// The same as [release_leaked_ref()], but slightly more efficient
    /// IMPLEMENTORS: #[inline(always)]
    fn release_leaked_id(&'a self, slot_id: u32);

    /// Returns the same information as [MetaPublisher::available_elements_count()] for implementors that doesn't allow several subscribers;
    /// For those that allow it (that is, use the "Listener Pattern"), returns how much elements are left for consumption
    /// IMPLEMENTORS: #[inline(always)]
    fn remaining_elements_count(&self) -> usize;

    /// # Safety
    /// Considering parallelism, this method *might* provide access to all elements available for [consume()].\
    /// This method is totally not thread safe for ring-buffer based implementations -- the moment it returns, all those elements might have already
    /// been consumed (furthermore, by the time the references are used, several generations of elements might have
    /// already lived and died at those slots). It is, thought, safe for unbounded implementations that never collect published elements, like a `log_topic`\
    /// ... So, use this method only when you're sure the publishing / consumption operations won't interfere with the results
    /// -- for this reason, this method is marked as `unsafe` (**it is only safe -- and consistent -- to call this method if you're behind a lock
    /// guarding against these mentioned scenarios**).
    unsafe fn peek_remaining(&self) -> Vec<&SlotType>;
}

/// API for consuming elements from a [meta_queue] or [meta_topic] that will always move the value out of their internal buffer and pass it to the caller.\
pub trait MoveSubscriber<SlotType> {

    /// Move the next available item out of the pool of objects (copying & forgetting) and hand it over to the caller
    /// IMPLEMENTORS: #[inline(always)]
    fn consume_movable(&self) -> Option<SlotType>;

    /// # Safety
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
    ///   let peeked_references = queue.peek_remaining().concat();
    ///   // if you require zero-allocations:
    ///   for peeked_chunk in queue.peek_remaining() {
    ///     for peeked_reference in peeked_chunk {
    ///       println!("your_logic_goes_here: {:#?}", *peeked_reference);
    ///     }
    ///   }
    ///   // or, in the condensed, functional form:
    ///   for peeked_reference in queue.peek_remaining().iter().flat_map(|&slice| slice) {
    ///       println!("your_logic_goes_here: {:#?}", *peeked_reference);
    ///   }
    unsafe fn peek_remaining(&self) -> [&[SlotType];2];
}
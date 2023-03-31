//! Resting place for the [MetaPublisher] trait.


/// API for producing elements to a [meta_queue] or [meta_topic].
pub trait MetaPublisher<'a, SlotType: 'a> {

    /// Zero-copy enqueue method with the following characteristics:
    ///   - After allocating a slot for the new element, `setter_fn(&mut slot)` is called to fill it;
    ///   - If the queue is found to be full, `report_full_fn()` is called. Through this function, specializations may turn implementing structures into a blocking queue and/or compute metrics.
    ///     Furthermore, if this method returns `true`, the enqueuer should try an allocation again and again;
    ///   - `report_len_after_enqueueing_fn(len)` is called after the enqueueing was done, received the number of not-yet-collected elements it contained then -- a similar info as [available_elements()].
    ///     Specializers might use this to awake a number of consumers.
    /// Returns:
    ///   - `true` if the enqueueing was done;
    ///   - `false` otherwise -- possibly due to a bounded queue being full (generally meaning the caller should try again after spin-waiting or sleeping a bit).
    ///     Keep in mind that blocking-queues are likely never to return false (unless some sort of enqueueing timeout happens).
    /// Caveats:
    ///   1) Slots are reused, so the `setter_fn()` must care to set all fields. No `default()` or any kind of zeroing will be applied to them prior to that function call;
    ///   2) `setter_fn()` should complete instantly, or else the whole queue is likely to hang. If building a `SlotType` is lengthy, one might consider creating it before
    ///      calling this method and using the `setter_fn()` to simply clone/copy the value.
    fn publish<SetterFn:                   FnOnce(&'a mut SlotType),
               ReportFullFn:               Fn() -> bool,
               ReportLenAfterEnqueueingFn: FnOnce(u32)>
              (&'a self,
               setter_fn:                      SetterFn,
               report_full_fn:                 ReportFullFn,
               report_len_after_enqueueing_fn: ReportLenAfterEnqueueingFn)
              -> bool;

    /// Possibly returns the number of published (but not-yet-collected) elements by this `meta_publisher`, at the moment of the call -- not synchronized.\
    /// Some implementations do "collect" enqueued elements once they are dequeued (for instance, a `ring-buffer` queue), while others (like an unbounded `meta_topic`)
    /// never collect them -- in practice, allowing new subscribers to access all elements ever produced.
    fn available_elements(&self) -> usize;

    /// Returns the maximum number of elements this `meta_publisher` can hold -- \
    /// 0, if the implementor offers an unbounded container,\
    /// > 0, if the implementer uses a ring-buffer of that size.
    fn max_size(&self) -> usize;

    /// Returns a string that might be useful to debug or assert algorithms when writing automated tests
    fn debug_info(&self) -> String;
}

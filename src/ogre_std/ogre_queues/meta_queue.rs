//! Resting place for the (rather internal) [MetaQueue] trait

use crate::ogre_std::ogre_queues::blocking_queue;


/// Dictates the API for "base" queues and how they should work.\
/// Base queues are meta-queues (not proper queues yet). They become queues when
/// turned into [blocking_queue::Queue] or [non_blocking_queue::Queue].\
/// Think of base queues as the common code among those queue variants.
pub trait MetaQueue<SlotType> {

    /// Instantiates the base queue
    fn new() -> Self;

    /// Zero-copy enqueue method with the following characteristics:
    ///   - After allocating a slot for the new element, `setter_fn(&mut slot)` is called to fill it;
    ///   - If the queue is found to be full, `report_full_fn()` is called. Through this function, specializations may turn this meta queue into a blocking queue and/or compute metrics.
    ///     Furthermore, if this method returns `true`, the enqueuer algorithm will try an allocation again and again;
    ///   - `report_len_after_enqueueing_fn(len)` is called after the enqueueing was done, and the number of elements it contained then. Specializers might use this to awake a number of consumers.
    /// Returns:
    ///   - `true` if the enqueueing was done;
    ///   - `false` otherwise -- due to the queue being full (generally meaning the caller should try again after spin-waiting or sleeping a bit).
    ///     Keep in mind that blocking-queues are likely never to return false (unless some sort of enqueueing timeout happens).
    /// Caveats:
    ///   1) Slots are reused, so the `setter_fn()` must care to set all fields. No `default()` or any kind of zeroing will be applied to them prior to that function call;
    ///   2) `setter_fn()` should complete instantly, or else the whole queue is likely to hang. If building a `SlotType` is lengthy, one might consider creating it before
    ///      calling this method and using the `setter_fn()` to simply clone/copy the value.
    #[inline(always)]
    fn enqueue<SetterFn:                   FnOnce(&mut SlotType),
               ReportFullFn:               Fn() -> bool,
               ReportLenAfterEnqueueingFn: FnOnce(u32)>
              (&self,
               setter_fn:                      SetterFn,
               report_full_fn:                 ReportFullFn,
               report_len_after_enqueueing_fn: ReportLenAfterEnqueueingFn)
              -> bool;

    /// Zero-copy dequeue method with the following characteristics:
    ///   - If the queue is found to be empty, `report_empty_fn()` is called. Specializations of this meta queue might use it to build a blocking queue;
    ///   - Before deallocating the slot, `getter_fn(&slot)` will be called to inform what is the dequeued element;
    ///   - `report_len_after_dequeueing_fn(len)` might be used by specializations of this meta queue to, for instance, set the hardware's clock down.
    /// Caveats:
    ///   1) The caller must ensure the `getter_fn()` operation returns as soon as possible, or else the whole queue is likely to hang. If so, one sould consider to pass in a `getter_fn()`
    ///      that would clone/copy the value and release the queue as soon as possible.
    #[inline(always)]
    fn dequeue<GetterReturnType,
               GetterFn:                   FnOnce(&SlotType) -> GetterReturnType,
               ReportEmptyFn:              Fn() -> bool,
               ReportLenAfterDequeueingFn: FnOnce(i32)>
              (&self,
               getter_fn:                      GetterFn,
               report_empty_fn:                ReportEmptyFn,
               report_len_after_dequeueing_fn: ReportLenAfterDequeueingFn)
              -> Option<GetterReturnType>;

    /// Possibly returns the number of elements in this meta-queue at the moment of the call -- not synchronized.
    #[inline(always)]
    fn len(&self) -> usize;

    /// Returns the maximum number of elements this queue can hold -- fixed if the implementer uses a ring-buffer
    fn max_size(&self) -> usize;

    /// Taking parallelism into account, this method *might* provide access to all elements available for [dequeue()].\
    /// This method is totally not thread safe -- the moment it returns, all those elements might have already
    /// been dequeued. Furthermore, by the time the references are used, several generations of elements might have
    /// already lived and died at those slots.\
    /// ... So, use this method only when you're sure the enqueue / dequeue operations won't interfere with the results
    /// -- for this reason, this method is marked as `unsafe` (**it is only safe to call this method if you're behind a lock**).
    ///
    /// The rather wired return type here is to avoid heap allocations: a fixed array of two slices of
    /// the internal ring buffer is returned -- the second slice is used if the sequence of references cycles
    /// through the buffer. Use this method like the following:
    /// ```nocompile
    ///   // if you don't care for allocating a vector:
    ///   let peeked_references = queue.peek_all().concat();
    ///   // if you require zero-allocations:
    ///   for peeked_chunk in queue.peek_all() {
    ///     for peeked_reference in peeked_chunk {
    ///       println!("your_logic_goes_here: {:#?}", *peeked_reference);
    ///     }
    ///   }
    ///   // or on the condensed, functional, form:
    ///   for peeked_reference in queue.peek_all().iter().flat_map(|&slice| slice) {
    ///       println!("your_logic_goes_here: {:#?}", *peeked_reference);
    ///   }
    #[inline(always)]
    unsafe fn peek_all(&self) -> [&[SlotType];2];

    /// Returns a string that might be useful to debug or assert algorithms when writing automated tests
    fn debug_info(&self) -> String;
}
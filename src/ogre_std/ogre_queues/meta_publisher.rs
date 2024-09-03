//! Resting place for the [MetaPublisher] trait.


use std::num::NonZeroU32;

/// API for producing elements to a [meta_queue] or [meta_topic].\
/// Two zero-copy patterns are available:
///   1) `publish()` is the simplest & safest: a callback closure is provided to fill in the data;
///   2) `leak_slot()` / `publish_leaked()` offers a more flexible (but more dangerous) option,
///      allowing the allocated slot to participate in more complex logics.
pub trait MetaPublisher<'a, SlotType: 'a> {

    /// Passes a slot reference to `setter`, so to fill in the information for the event to be published,
    /// to be later retrieved with [MoveSubscriber<>], in a way that the compiler won't escape zero-copying it
    /// (even in debug mode).\
    /// The returned values serves two purposes:
    ///   - 1st: If `None`, the container was full and no publishing was done; otherwise, the number of
    ///          elements present just after publishing `item` is returned -- which would be, at a minimum, 1
    ///   - 2nd: If the 1st returned value is `None`, this value will be `Some`, giving back to the caller the
    ///          `setter()` function (FnOnce) for a possible zero-copy reattempt.
    /// 
    /// IMPLEMENTORS: #[inline(always)]
    fn publish<F: FnOnce(&mut SlotType)>
              (&self, setter: F) -> (Option<NonZeroU32>, Option<F>);

    /// Store the `item`, to be later retrieved with [MoveSubscriber<>], in a way that the compiler might
    /// move the data (copy & forget) rather than zero-copying it -- which may be useful for data that
    /// requires a custom dropping function.\
    /// The returned values serves two purposes:
    ///   - 1st: If `None`, the container was full and no publishing was done; otherwise, the number of
    ///          elements present just after publishing `item` is returned -- which would be, at a minimum, 1
    ///   - 2nd: If the 1st returned value is `None`, this value will be `Some`, giving back to the caller the
    ///          `item` for a possible reattempt without incurring in further copying.
    /// 
    /// IMPLEMENTORS: #[inline(always)]
    fn publish_movable(&self, item: SlotType) -> (Option<NonZeroU32>, Option<SlotType>);

    /// Advanced method to publish an element: allocates a slot from the pool, returning a reference to it.\
    /// Once called, either [publish_leaked()] or [unleak_slot()] should also be, eventually, called
    /// --  or else the slot will never be returned to the pool for reuse.
    /// 
    /// IMPLEMENTORS: #[inline(always)]
    fn leak_slot(&self) -> Option<(/*ref:*/ &mut SlotType, /*id: */u32)>;

    /// Advanced method to publish a slot (previously allocated with [leak_slot()]) and populated by the caller.\
    /// The slot will proceed to be handed over to consumers (which should return it to the pool afterwards).\
    /// A return of `None` means the container was full and no publishing was done; otherwise, the number of
    /// elements present just after publishing `item` is returned -- which would be, at a minimum, 1.\
    /// NOTE: unless you're using a shared allocator or an allocator with less slots than this container,
    /// `None` won't ever be returned here.
    /// 
    /// IMPLEMENTORS: #[inline(always)]
    fn publish_leaked_ref(&'a self, slot: &'a SlotType) -> Option<NonZeroU32>;

    /// The same as [publish_leaked_ref()], but slightly more efficient
    /// 
    /// IMPLEMENTORS: #[inline(always)]
    fn publish_leaked_id(&'a self, slot_id: u32) -> Option<NonZeroU32>;

    /// Advanced method to return a slot (obtained by [allocate_slot()]) to the pool, so it may be reused,
    /// in case the publishing of the item should be aborted.
    /// 
    /// IMPLEMENTORS: #[inline(always)]
    fn unleak_slot_ref(&'a self, slot: &'a mut SlotType);

    /// The same as [unleak_slot_ref()], but slightly more efficient
    /// 
    /// IMPLEMENTORS: #[inline(always)]
    fn unleak_slot_id(&'a self, slot_id: u32);

    /// Possibly returns the number of published (but not-yet-collected) elements by this `meta_publisher`, at the moment of the call -- not synchronized.\
    /// Some implementations do "collect" enqueued elements once they are dequeued (for instance, a `ring-buffer` queue), while others (like an unbounded `meta_topic`)
    /// never collect them -- in practice, allowing new subscribers to access all elements ever produced.
    /// 
    /// IMPLEMENTORS: #[inline(always)]
    fn available_elements_count(&self) -> usize;

    /// Returns the maximum number of elements this `meta_publisher` can hold -- \
    ///   * `=0`, if the implementor offers an unbounded container,\
    ///   * `>0`, if the implementer uses a ring-buffer of that size.
    /// 
    /// IMPLEMENTORS: #[inline(always)]
    fn max_size(&self) -> usize;

    /// Returns a string that might be useful to debug or assert algorithms when writing automated tests
    fn debug_info(&self) -> String;
}

/// API for producing elements to a [meta_queue] or [meta_topic] that accepts the possibility that they may be moved,
/// if the compiler is unable to optimize the data transfer to a zero-copy -- typically, publishing will always be
/// zero-copy (provided you're compiling in Release mode), but subscription will always be move.
pub trait MovePublisher<SlotType> {

    /// Store the `item`, to be later retrieved with [MoveSubscriber<>], in a way that the compiler might
    /// move the data (copy & forget) rather than zero-copying it.\
    /// The returned values serves two purposes:
    ///   - 1st: If `None`, the container was full and no publishing was done; otherwise, the number of
    ///          elements present just after publishing `item` is returned -- which would be, at a minimum, 1
    ///   - 2nd: If the 1st returned value is `None`, this value will be `Some`, giving back to the caller the
    ///          `item` for a possible reattempt without incurring in further copying.
    /// 
    /// IMPLEMENTORS: #[inline(always)]
    fn publish_movable(&self, item: SlotType) -> (Option<NonZeroU32>, Option<SlotType>);

    /// Store an item, to be later retrieved with [MoveSubscriber<>], in a way that
    /// no copying nor moving around would ever be made:
    ///   - After securing a slot for the new element, `setter_fn(&mut slot)` is called to fill it;
    ///   - If the queue is found to be full, `report_full_fn()` is called. Through this function, specializations may turn implementing structures into a blocking queue and/or compute metrics.
    ///     The returned boolean indicates if the publisher should try securing a slot again -- `false` meaning "don't retry: nothing could be done to free one up";
    ///   - `report_len_after_enqueueing_fn(len)` is called after the enqueueing is done and receives the number of not-yet-collected elements present -- a similar info as [available_elements()].
    ///     Specializers might use this to awake a number of consumers.
    /// 
    /// Returns:
    ///   - `None` if the enqueueing was done;
    ///   - `Some(setter_fn)` otherwise -- possibly due to a bounded queue being full -- giving the caller the opportunity of retrying (maybe after spin-looping a bit).
    ///     Keep in mind that blocking-queues are likely never to return false (unless some sort of enqueueing timeout happens).
    /// 
    /// Caveats:
    ///   1) Slots are reused, so the `setter_fn()` must care to set all fields. No `default()` or any kind of zeroing will be applied to them prior to that function call.
    ///      As a downside, the `new T { fields }` cannot be used here. If that is needed, please use [publish_movable()]
    ///   2) `setter_fn()` should complete instantly, or else the whole queue is likely to hang. If building a `SlotType` is lengthy, one might consider creating it before
    ///      calling this method and using the `setter_fn()` to simply clone/copy the value.
    /// 
    /// IMPLEMENTORS: #[inline(always)]
    fn publish<SetterFn:                   FnOnce(&mut SlotType),
               ReportFullFn:               Fn() -> bool,
               ReportLenAfterEnqueueingFn: FnOnce(u32)>
              (&self,
               setter_fn:                      SetterFn,
               report_full_fn:                 ReportFullFn,
               report_len_after_enqueueing_fn: ReportLenAfterEnqueueingFn)
              -> Option<SetterFn>;


    /// Possibly returns the number of published (but not-yet-collected) elements by this `meta_publisher`, at the moment of the call -- not synchronized.\
    /// Some implementations do "collect" enqueued elements once they are dequeued (for instance, a `ring-buffer` queue), while others (like an unbounded `meta_topic`)
    /// never collect them -- in practice, allowing new subscribers to access all elements ever produced.
    /// 
    /// IMPLEMENTORS: #[inline(always)]
    fn available_elements_count(&self) -> usize;

    /// Returns the maximum number of elements this `meta_publisher` can hold: \
    ///   * `=0` if the implementor offers an unbounded container,
    ///   * `>0`, if the implementer uses a ring-buffer of that size.
    /// 
    /// IMPLEMENTORS: #[inline(always)]
    fn max_size(&self) -> usize;

    /// Returns a string that might be useful to debug or assert algorithms when writing automated tests
    fn debug_info(&self) -> String;
}

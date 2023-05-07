//! Resting place for the [MetaPublisher] trait.


use std::num::NonZeroU32;

/// API for producing elements to a [meta_queue] or [meta_topic].\
/// Two zero-copy patterns are available:
///   1) `publish()` is the simplest & safest: a callback closure is provided to fill in the data;
///   2) `leak_slot()` / `publish_leaked()` offers a more flexible (but more dangerous) option,
///      allowing the allocated slot to participate in more complex logics.
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

    /// Advanced method to publish an element: allocates a slot from the pool, returning a reference to it.\
    /// Once called, either [publish_leaked()] or [unleak_slot()] should also be, eventually, called
    /// --  or else the slot will never be returned to the pool for reuse.
    fn leak_slot(&self) -> Option<&SlotType>;

    /// Advanced method to publish a slot (previously allocated with [leak_slot()]) and filled by the caller.\
    /// The slot will proceed to b handed over to consumers, and returned to the pool afterwards.\
    /// Please note some caveats:
    ///   1) Ring-Buffer based implementors will, typically, enforce that this method will be executed in the same order in which
    ///      [leak_slot()] was called -- this may either be done through a context-switch or by simply spinning. This is of concern only
    ///      if parallel production is being used and, in this case, publishers will be efficient only if they all take the same time between
    ///      the calls of the two methods;
    ///   2) Nor Log-based nor Array-based implementors suffer any of the restrictions stated above.
    fn publish_leaked(&'a self, slot: &'a SlotType);

    /// Advanced method to return a slot (obtained by [allocate_slot()]) to the pool, so it may be reused.\
    /// Note that not all implementors allow using this method.\
    /// It is known that:
    ///   1) Ring-Buffer based implementors will only accept back the `slot` if no other allocation has been done after that one
    ///      -- they will fail silently if this is attempted, by hanging forever: so this should only be used on single-thread producers;
    ///   2) Log-based implementors won't reuse the cancelled slot, so excessive calling this method may cause the application to consume more resources;
    ///   3) Array-based implementors will deal optimally with the cancelled slots: they will be immediately returned to the pool.
    fn unleak_slot(&'a self, slot: &'a SlotType);

    /// Possibly returns the number of published (but not-yet-collected) elements by this `meta_publisher`, at the moment of the call -- not synchronized.\
    /// Some implementations do "collect" enqueued elements once they are dequeued (for instance, a `ring-buffer` queue), while others (like an unbounded `meta_topic`)
    /// never collect them -- in practice, allowing new subscribers to access all elements ever produced.
    fn available_elements_count(&self) -> usize;

    /// Returns the maximum number of elements this `meta_publisher` can hold -- \
    /// 0, if the implementor offers an unbounded container,\
    /// > 0, if the implementer uses a ring-buffer of that size.
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
    /// If it returns `None`, the container was full and no publishing was done; otherwise, the number of
    /// elements present just after publishing `item` is returned -- which would be, at a minimum, 1.
    fn publish_movable(&self, item: SlotType) -> Option<NonZeroU32>;

    /// Store an item, to be later retrieved with [MoveSubscriber<>], in a way that
    /// no copying nor moving around would ever be made:
    ///   - After securing a slot for the new element, `setter_fn(&mut slot)` is called to fill it;
    ///   - If the queue is found to be full, `report_full_fn()` is called. Through this function, specializations may turn implementing structures into a blocking queue and/or compute metrics.
    ///     The returned boolean indicates if the publisher should try securing a slot again -- `false` meaning "don't retry: nothing could be done to free one up";
    ///   - `report_len_after_enqueueing_fn(len)` is called after the enqueueing is done and receives the number of not-yet-collected elements present -- a similar info as [available_elements()].
    ///     Specializers might use this to awake a number of consumers.
    /// Returns:
    ///   - `true` if the enqueueing was done;
    ///   - `false` otherwise -- possibly due to a bounded queue being full (generally meaning the caller should try again after spin-waiting or sleeping a bit).
    ///     Keep in mind that blocking-queues are likely never to return false (unless some sort of enqueueing timeout happens).
    /// Caveats:
    ///   1) Slots are reused, so the `setter_fn()` must care to set all fields. No `default()` or any kind of zeroing will be applied to them prior to that function call.
    ///      As a down side, the `new T { fields }` cannot be used here. If that is needed, please use [publish_movable()]
    ///   2) `setter_fn()` should complete instantly, or else the whole queue is likely to hang. If building a `SlotType` is lengthy, one might consider creating it before
    ///      calling this method and using the `setter_fn()` to simply clone/copy the value.
    fn publish<SetterFn:                   FnOnce(&mut SlotType),
               ReportFullFn:               Fn() -> bool,
               ReportLenAfterEnqueueingFn: FnOnce(u32)>
              (&self,
               setter_fn:                      SetterFn,
               report_full_fn:                 ReportFullFn,
               report_len_after_enqueueing_fn: ReportLenAfterEnqueueingFn)
              -> bool;


    /// Possibly returns the number of published (but not-yet-collected) elements by this `meta_publisher`, at the moment of the call -- not synchronized.\
    /// Some implementations do "collect" enqueued elements once they are dequeued (for instance, a `ring-buffer` queue), while others (like an unbounded `meta_topic`)
    /// never collect them -- in practice, allowing new subscribers to access all elements ever produced.
    fn available_elements_count(&self) -> usize;

    /// Returns the maximum number of elements this `meta_publisher` can hold -- \
    /// 0, if the implementor offers an unbounded container,\
    /// > 0, if the implementer uses a ring-buffer of that size.
    fn max_size(&self) -> usize;

    /// Returns a string that might be useful to debug or assert algorithms when writing automated tests
    fn debug_info(&self) -> String;
}

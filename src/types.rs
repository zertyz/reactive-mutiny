//! Common types across this module

pub use crate::instruments::Instruments;
use crate::mutiny_stream::MutinyStream;
use std::{
    time::Duration,
    task::Waker,
    fmt::Debug,
};
use std::future::Future;
use std::sync::Arc;
use async_trait::async_trait;


/// Defines common abstractions on how [Uni]s receives produced events and delivers them to `Stream`s.\
/// Implementors should also implement one of [ChannelProducer] or [UniZeroCopyChannel].
/// NOTE: all async functions are out of the hot path, so the `async_trait` won't impose performance penalties
#[async_trait]
pub trait ChannelCommon<'a, ItemType:        Debug + Send + Sync,
                            DerivedItemType: Debug> {

    /// Creates a new instance of this channel, to be referred to (in logs) as `name`
    fn new<IntoString: Into<String>>(name: IntoString) -> Arc<Self>;

    /// Waits until all pending items are taken from this channel, up until `timeout` elapses.\
    /// Returns the number of still unconsumed items -- which is 0 if it was not interrupted by the timeout
    #[must_use = "Returns 0 if all elements could be flushed within the given `timeout` or the number of elements yet flushing"]
    async fn flush(&self, timeout: Duration) -> u32;

    /// Tells weather this channel is still enabled to process elements
    /// (true before calling the "end stream" / "cancel stream" functions)
    fn is_channel_open(&self) -> bool;

    /// Flushes & signals that the given `stream_id` should cease its activities when there are no more elements left
    /// to process, waiting for the operation to complete for up to `timeout`.\
    /// Returns `true` if the stream ended within the given `timeout` or `false` if it is still processing elements.
    #[must_use = "Returns true if the Channel could be closed within the given time"]
    async fn gracefully_end_stream(&self, stream_id: u32, timeout: Duration) -> bool;

    /// Flushes & signals that all streams should cease their activities when there are no more elements left
    /// to process, waiting for the operation to complete for up to `timeout`.\
    /// Returns the number of un-ended streams -- which is 0 if it was not interrupted by the timeout
    #[must_use = "Returns 0 if all elements could be flushed within the given `timeout` or the number of elements that got unsent after the channel closing"]
    async fn gracefully_end_all_streams(&self, timeout: Duration) -> u32;

    /// Sends a signal to all streams, urging them to cease their operations.\
    /// In opposition to [end_all_streams()], this method does not wait for any confirmation,
    /// nor cares if there are remaining elements to be processed.
    fn cancel_all_streams(&self);

    /// Informs the caller how many active streams are currently managed by this channel
    /// IMPLEMENTORS: #[inline(always)]
    fn running_streams_count(&self) -> u32;

    /// Tells how many events are waiting to be taken out of this channel.\
    /// IMPLEMENTORS: #[inline(always)]
    fn pending_items_count(&self) -> u32;

    /// Tells how many events may be produced ahead of the consumers.\
    /// IMPLEMENTORS: #[inline(always)]
    fn buffer_size(&self) -> u32;
}

/// Defines abstractions specific to [Uni] channels
pub trait ChannelUni<'a, ItemType:        Debug + Send + Sync,
                         DerivedItemType: Debug> {

    /// Returns a `Stream` (and its `stream_id`) able to receive elements sent through this channel.\
    /// If called more than once, each `Stream` will receive a different element -- "consumer pattern".\
    /// Currently `panic`s if called more times than allowed by [Uni]'s `MAX_STREAMS`
    fn create_stream(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, DerivedItemType>, u32)
                                          where Self: ChannelConsumer<'a, DerivedItemType>;

}

/// Defines abstractions specific to [Uni] channels
pub trait ChannelMulti<'a, ItemType:        Debug + Send + Sync,
                           DerivedItemType: Debug> {

    /// Implemented only for a few [Multi] channels, returns a `Stream` (and its `stream_id`) able to receive elements
    /// that were sent through this channel *before the call to this method*.\
    /// It is up to each implementor to define how back in the past those events may go, but it is known that `mmap log`
    /// based channels are able to see all past events.\
    /// If called more than once, every stream will see all the past events available.\
    /// Currently `panic`s if called more times than allowed by [Multi]'s `MAX_STREAMS`
    fn create_stream_for_old_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, DerivedItemType>, u32)
                                                         where Self: ChannelConsumer<'a, DerivedItemType>;

    /// Returns a `Stream` (and its `stream_id`) able to receive elements sent through this channel *after the call to this method*.\
    /// If called more than once, each `Stream` will see all new elements -- "listener pattern".\
    /// Currently `panic`s if called more times than allowed by [Multi]'s `MAX_STREAMS`
    fn create_stream_for_new_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, DerivedItemType>, u32)
                                                         where Self: ChannelConsumer<'a, DerivedItemType>;

    /// Implemented only for a few [Multi] channels, returns two `Stream`s (and their `stream_id`s):
    ///   - one for the past events (that, once exhausted, won't see any of the forthcoming events)
    ///   - another for the forthcoming events.
    /// 
    /// The split is guaranteed not to miss any events: no events will be lost between the last of the "past" and
    /// the first of the "forthcoming" events.\
    /// It is up to each implementor to define how back in the past those events may go, but it is known that `mmap log`
    /// based channels are able to see all past events.\
    /// If called more than once, every stream will see all the past events available, as well as all future events after this method call.\
    /// Currently `panic`s if called more times than allowed by [Multi]'s `MAX_STREAMS`
    fn create_streams_for_old_and_new_events(self: &Arc<Self>) -> ((MutinyStream<'a, ItemType, Self, DerivedItemType>, u32),
                                                                   (MutinyStream<'a, ItemType, Self, DerivedItemType>, u32))
                                                                  where Self: ChannelConsumer<'a, DerivedItemType>;

    /// Implemented only for a few [Multi] channels, returns a single `Stream` (and its `stream_id`) able to receive elements
    /// that were sent through this channel either *before and after the call to this method*.\
    /// It is up to each implementor to define how back in the past those events may go, but it is known that `mmap log`
    /// based channels are able to see all past events.\
    /// Notice that, with this method, there is no way of discriminating where the "old" events end and where the "new" events start.\
    /// If called more than once, every stream will see all the past events available, as well as all future events after this method call.\
    /// Currently `panic`s if called more times than allowed by [Multi]'s `MAX_STREAMS`
    fn create_stream_for_old_and_new_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, DerivedItemType>, u32)
                                                                 where Self: ChannelConsumer<'a, DerivedItemType>;

}

/// Defines how to send events (to a [Uni] or [Multi]).
pub trait ChannelProducer<'a, ItemType:        'a + Debug + Send + Sync,
                              DerivedItemType: 'a + Debug> {

    /// Similar to [Self::send_with()], but for sending the already-built `item`.\
    /// See there for how to deal with the returned type.\
    /// IMPLEMENTORS: #[inline(always)]
    #[must_use = "The return type should be examined in case retrying is needed -- or call map(...).into() to transform it into a `Result<(), ItemType>`"]
    fn send(&self, item: ItemType) -> keen_retry::RetryConsumerResult<(), ItemType, ()>;

    /// Calls `setter`, passing a slot so the payload may be filled there, then sends the event through this channel asynchronously.\
    /// The returned type is conversible to `Result<(), F>` by calling .into() on it, returning `Err<setter>` when the buffer is full,
    /// to allow the caller to try again; otherwise you may add any retrying logic using the `keen-retry` crate's API like in:
    /// ```nocompile
    ///     xxxx.send_with(|slot| *slot = 42)
    ///         .retry_with(|setter| xxxx.send_with(setter))
    ///         .spinning_until_timeout(Duration::from_millis(300), ())     // go see the other options
    ///         .map_errors(|_, setter| (setter, _), |e| e)                 // map the unconsumed `setter` payload into `Err(setter)` when converting to `Result` ahead
    ///         .into()?;
    /// ```
    /// NOTE: this type may allow the compiler some extra optimization steps when compared to [Self::send()]. When tuning for performance,
    /// it is advisable to try this method.\
    /// IMPLEMENTORS: #[inline(always)]
    #[must_use = "The return type should be examined in case retrying is needed -- or call map(...).into() to transform it into a `Result<(), F>`"]
    fn send_with<F: FnOnce(&mut ItemType)>
                (&self,
                 setter: F)
                -> keen_retry::RetryConsumerResult<(), F, ()>;
    
    /// Similar to [Self::send_with(), but accepts an async `setter`.
    /// This method is useful for sending operations that depend on data acquired by async blocks, allowing
    /// select loops (like the following) to be built:
    /// ```nocompile
    ///     tokio::select! {
    ///         _ => async {
    ///             channel_producer.send_with_async(|slot| async {
    ///                 let data = data_source.read().await;
    ///                 fill_slot(data, &mut slot);
    ///                 slot
    ///             }).await
    ///         },
    ///        (...other select arms that may execute concurrently with the above arm...)
    ///     }
    /// ```
    /// IMPLEMENTORS: #[inline(always)]
    fn send_with_async<F:   FnOnce(&'a mut ItemType) -> Fut,
                       Fut: Future<Output=&'a mut ItemType>>
                      (&'a self,
                       setter: F) -> impl Future<Output=keen_retry::RetryConsumerResult<(), F, ()>>;
    
    // TODO: 2024-03-04: this is to be filled in by **(f21)**. Possibly an extra dependency on the allocator will be needed for the `BoundedOgreAllocator::OwnedSlotType`
    // fn send_with_external_alloc();

    /// For channels that stores the `DerivedItemType` instead of the `ItemType`, this method may be useful
    /// -- for instance: if the Stream consumes `Arc<String>` (the derived item type) and the channel is for `Strings`, With this method one may send an `Arc` directly.\
    /// The default implementation, though, is made for types that don't have a derived item type.\
    /// IMPLEMENTORS: #[inline(always)]
    #[inline(always)]
    #[must_use = "The return type should be examined in case retrying is needed"]
    fn send_derived(&self, _derived_item: &DerivedItemType) -> bool {
        todo!("The default `ChannelProducer.send_derived()` was not re-implemented, meaning it is not available for this channel -- is only available for channels whose `Stream` items will see different types than the produced one -- example: send(`string`) / Stream<Item=Arc<String>>")
    }
    
    /// Proxy to [crate::prelude::advanced::BoundedOgreAllocator::alloc_ref()] from the underlying allocator,
    /// allowing caller to fill in the data as they wish -- in a non-blocking prone API.\
    /// See also [Self::send_reserved()] and [Self::cancel_slot_reserve()].
    fn reserve_slot(&'a self) -> Option<&'a mut ItemType>;

    /// Attempts to send an item previously reserved by [Self::reserve_slot()].
    /// Failure to do so (when `false` is returned) might be part of the normal channel operation,
    /// so retrying is advised.
    /// More: some channel implementations are optimized (or even only accept) sending the slots
    ///       in the same order they were reserved.
    #[must_use = "You need the returned value to retry the operation until it succeeds, implementing a suitable spin loop logic (like tokio's yield_now())"]
    fn try_send_reserved(&self, reserved_slot: &mut ItemType) -> bool;
    
    /// Attempts to give up sending an item previously reserved by [Self::reserve_slot()], freeing it / setting its resources for reuse.
    /// Two important things to note:
    ///   1. Failure (when `false` is returned) might be part of the normal channel operation,
    ///      so retrying is advised;
    ///   2. Some channel implementations are optimized (or even only accept) cancelling the slots
    ///      in the same order they were reserved;
    ///   3. These, more restricted & more optimized channels, might not allow publishing any reserved
    ///      slots if there are cancelled slots in-between -- in which case, publishing will only be done
    ///      when the missing slots are, eventually, published. So, be careful when using the cancellation
    ///      semantics: ideally, it should only be allowed for the last slot and when no sending occurs in-between.
    #[must_use = "You need the returned value to retry the operation until it succeeds, implementing a suitable spin loop logic (like tokio's yield_now())"]
    fn try_cancel_slot_reserve(&self, reserved_slot: &mut ItemType) -> bool;

}

/// Source of events for [MutinyStream].
pub trait ChannelConsumer<'a, DerivedItemType: 'a + Debug> {

    /// Delivers the next event, whenever the Stream wants it.\
    /// IMPLEMENTORS: use #[inline(always)]
    fn consume(&self, stream_id: u32) -> Option<DerivedItemType>;

    /// Returns `false` if the `Stream` has been signaled to end its operations, causing it to report "out-of-elements" as soon as possible.\
    /// IMPLEMENTORS: use #[inline(always)]
    fn keep_stream_running(&self, stream_id: u32) -> bool;

    /// Shares, to implementors concern, how `stream_id` may be awaken.\
    /// IMPLEMENTORS: use #[inline(always)]
    fn register_stream_waker(&self, stream_id: u32, waker: &Waker);

        /// Reports no more elements will be required through [provide()].\
    /// IMPLEMENTORS: use #[inline(always)]
    fn drop_resources(&self, stream_id: u32);
}


/// Defines a fully fledged `Uni` channel, that has both the producer and consumer parts
/// Also, laverages generic programming by allowing simpler generic parameters:
/// ```nocompile
///     struct MyGenericStruct<T: FullDuplexUniChannel> { the_channel: T }
///     let the_channel = uni::channels::xxx<Lots,And,Lots<Of,Generic,Arguments>>::new();
///     let my_struct = MyGenericStruct { the_channel };
///     // see more at `tests/use_cases.rs`
pub trait FullDuplexUniChannel:
              ChannelCommon<'static, Self::ItemType, Self::DerivedItemType> +
              ChannelUni<'static, Self::ItemType, Self::DerivedItemType> +
              ChannelProducer<'static, Self::ItemType, Self::DerivedItemType> +
              ChannelConsumer<'static, Self::DerivedItemType> {

    const MAX_STREAMS: usize;
    const BUFFER_SIZE: usize;
    type ItemType: 'static + Debug + Send + Sync;
    type DerivedItemType: 'static + Debug + Send + Sync;
            
    /// Returns this channel's name
    fn name(&self) -> &str;
}

/// A fully fledged `Multi` channel, that has both the producer and consumer parts
/// Also, laverages generic programming by allowing simpler generic parameters:
/// ```nocompile
///     struct MyGenericStruct<T: FullDuplexUniChannel> { the_channel: T }
///     let the_channel = uni::channels::xxx<Lots,And,Lots<Of,Generic,Arguments>>::new();
///     let my_struct = MyGenericStruct { the_channel };
///     // see more at `tests/use_cases.rs`
pub trait FullDuplexMultiChannel:
              ChannelCommon<'static, Self::ItemType, Self::DerivedItemType> +
              ChannelMulti<'static, Self::ItemType, Self::DerivedItemType> +
              ChannelProducer<'static, Self::ItemType, Self::DerivedItemType> +
              ChannelConsumer<'static, Self::DerivedItemType> {

    const MAX_STREAMS: usize;
    const BUFFER_SIZE: usize;
    type ItemType: 'static + Debug + Send + Sync;
    type DerivedItemType: 'static + Debug + Send + Sync;
}

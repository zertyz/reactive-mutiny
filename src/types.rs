//! Common types across this module

pub use crate::{
    instruments::Instruments,
};
use crate::{
    mutiny_stream::MutinyStream,
};
use std::{
    time::Duration,
    task::{Waker},
    fmt::Debug,
};
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
    async fn flush(&self, timeout: Duration) -> u32;

    /// Flushes & signals that the given `stream_id` should cease its activities when there are no more elements left
    /// to process, waiting for the operation to complete for up to `timeout`.\
    /// Returns `true` if the stream ended within the given `timeout` or `false` if it is still processing elements.
    async fn gracefully_end_stream(&self, stream_id: u32, timeout: Duration) -> bool;

    /// Flushes & signals that all streams should cease their activities when there are no more elements left
    /// to process, waiting for the operation to complete for up to `timeout`.\
    /// Returns the number of un-ended streams -- which is 0 if it was not interrupted by the timeout
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
pub trait ChannelProducer<'a, ItemType:         Debug + Send + Sync,
                              DerivedItemType: 'a + Debug> {

    /// Calls `setter`, passing a slot so the payload may be filled, then sends the event through this channel asynchronously.\
    /// -- returns `false` if the buffer was full and the `item` wasn't sent; `true` otherwise.\
    /// IMPLEMENTORS: #[inline(always)]
    fn try_send<F: FnOnce(&mut ItemType)>(&self, setter: F) -> bool;

    /// Sends an event through this channel, after calling `setter` to fill the payload.\
    /// If the channel is full, this function may wait until sending it is possible.\
    /// IMPLEMENTORS: #[inline(always]
    #[inline(always)]
    fn send<F: FnOnce(&mut ItemType)>(&self, _setter: F) {
        todo!("`ChannelProducer.send()` is not available for channels where it can't be implemented as zero-copy (including this one)")
    }

    /// For channels that stores the `DerivedItemType` instead of the `ItemType`, this method may be useful
    /// -- for instance: the Stream consumes Arc<String> (the derived item type) and the channel is for Strings. With this method one may send an Arc directly.\
    /// The default implementation, though, is made for types that don't have a derived item type.\
    /// IMPLEMENTORS: #[inline(always)]
    #[inline(always)]
    fn send_derived(&self, _derived_item: &DerivedItemType) {
        todo!("`ChannelProducer.send_derived()` is only available for channels whose Streams will see different types than the produced one -- example: send(`string`) / Stream<Item=Arc<String>>")
    }

    /// Similar to [try_send()], but accepts the penalty that the compiler may impose of copying / moving the data around,
    /// in opposition to set it only once, in its resting place -- useful to send cloned items and other objects with a custom drop
    /// IMPLEMENTORS: #[inline(always)]
    /// TODO 2023-05-17: consider restricting this entry for types that require dropping, and the zero-copy versions for those who don't
    #[must_use]
    fn try_send_movable(&self, item: ItemType) -> bool;

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
pub trait FullDuplexUniChannel<'a, ItemType:        'a + Debug + Send + Sync,
                                   DerivedItemType: 'a + Debug = ItemType>:
          ChannelCommon<'a, ItemType, DerivedItemType> +
          ChannelUni<'a, ItemType, DerivedItemType> +
          ChannelProducer<'a, ItemType, DerivedItemType> +
          ChannelConsumer<'a, DerivedItemType> {}

/// A fully fledged `Multi` channel, that has both the producer and consumer parts
pub trait FullDuplexMultiChannel<'a, ItemType:        'a + Debug + Send + Sync,
                                     DerivedItemType: 'a + Debug = ItemType>:
          ChannelCommon<'a, ItemType, DerivedItemType> +
          ChannelMulti<'a, ItemType, DerivedItemType> +
          ChannelProducer<'a, ItemType, DerivedItemType> +
          ChannelConsumer<'a, DerivedItemType> {}

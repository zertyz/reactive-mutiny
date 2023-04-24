//! Common types across this module

use crate::{
    uni::Uni,
    multi::Multi,
    ogre_std::ogre_queues::meta_subscriber::MetaSubscriber,
};
use std::{
    pin::Pin,
    task::{Context, Poll, Waker},
    fmt::Debug,
    mem::{MaybeUninit},
    ops::Deref,
    sync::Arc,
};
use futures::stream::Stream;
use owning_ref::ArcRef;


/// To be implemented by `Uni` & `Multi` channels, manages the set of streams that consumes elements from them
pub trait StreamsManager<'a, ItemType:           'a,
                             MetaSubscriberType: MetaSubscriber<'a, DerivativeItemType>,
                             DerivativeItemType  : 'a = ItemType> {

    /// Shares a reference to the backing container used in this channel -- implementing [MetaSubscriber]
    fn backing_subscriber(self: &Arc<Self>, stream_id: u32) -> ArcRef<Self, MetaSubscriberType>;

    /// Returns `false` if the Stream was signaled to end its operations, causing it to report as "out-of-elements" as soon as possible.\
    /// IMPLEMENTERS: always inline
    fn keep_stream_running(&self, stream_id: u32) -> bool;

//    fn consumer_stream(&self) -> Option<UniStream>

    /// Registers the `waker` for the given `stream_id`, if not done so already -- for optimal results, typically used on the `Stream`'s `poll_next()`, when None items are ready.\
    /// IMPLEMENTERS: always inline
    fn register_stream_waker(&self, stream_id: u32, waker: &Waker);

    /// Informs the manager that the current stream is being dropped and should be removed & cleaned from the internal list.\
    /// Dropping a single stream, as of 2023-04-15, means that the whole Channel must cease working to avoid undefined behavior.
    /// Due to the algorithm to wake Streams being based on a unique number assigned to it at the moment of creation,
    /// if a stream is created and dropped, the `send` functions will still try to awake stream #0 -- the stream number is determined
    /// based on the elements on the queue before publishing it.
    /// This is specially bad if the first stream is dropped:
    ///   1) the channel gets, eventually empty -- all streams will sleep
    ///   2) An item is sent -- the logic will wake the first stream: since the stream is no longer there, that element won't be consumed!
    ///   3) Eventually, a second item is sent: now the queue has length 2 and the send logic will wake consumer #1
    ///   4) Consumer #1, since it was not dropped, will be awaken and will run until the channel is empty again -- consuming both elements.
    fn report_stream_dropped(&self, stream_id: u32);

}

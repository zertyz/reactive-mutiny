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


//// TODO 2023-04-07: as an optimization, in the future, this may replace the MutinyStream to allow the compiler to fully optimize the whole pipeline processing.
///                   (currently, DropFnOnceType & PollFnType are defined down in the Channel and can't be propagate upstream. Is there a way around it?)
///                   A possible workaround would be to have `UniStream` and `MultiStream` structures (transparent to the user, see the test updates introduced by this commit) such that:
///                     0) Currently the code is optimizable (by the compiler) from the user provided `pipeline_builder()` to the `StreamExecutor`, keeping the input Stream code in a black `Box`. We want to make it optimizable all the way through.
///                     1) Both will have a fixed Drop function and a fixed Poll function, both dependent on the Generic channel -- which will also be passed along with the `Uni` or `Multi` type;
///                     2) With concrete Structs known in advance, both user provided processor functions and the Uni/Multi producer may refer to the same type, allowing optimizations
///                     3) `UniStream` and `MultiStream` will be defined in their modules, but might be re-exported here (for simplicity?) since the user-provided `pipeline_builder()` function should know of it
///                     4) Actually, this file might contain all possible concrete versions (for all possible channels) of `Uni`s, `Multi`s, `UniStream`s and `MultiStream`s... with comments... thus, justifying its existence (which would be to doc it and expose -- in a single place -- to users)
///                     5) Study on why it wouldn't be possible:
///                       5.1) Uni: Unis ask UniChannels to produce the input stream (a `UniStream<UniChannel>`); `Uni` knows its full type would be `Uni<UniChannel>`; Types in this class will also know of `UniStream<UniChannel>`. So far, so good;
///                       5.2) Executor: planned to be simple! It only receives the final impl Stream (with output items), so it is already generic (able to inline the impl Stream code) and doesn't need any tweaking. All good here as well
///                       5.3) Current planning outcome: it is safe to try it. No hard blockers stood out during the study / planning phase. Go ahead and try it for 1 day.
/// When the above was complete for Uni, the results were:
///     25%+ performance increase for Uni for the simple pipelines used in "performance_measurements"
// pub type MutinyStream<DropFnOnceType, PollFnType, ItemType> = CustomDropPollFn<DropFnOnceType, PollFnType, ItemType>;

//
// /// A stream of events for [Uni]s and [Multi]s
// pub struct MutinyStream<ItemType> {
//     pub stream: Box<dyn Stream<Item=ItemType> + Send>,
// }
// impl<ItemType> Stream for MutinyStream<ItemType> {
//     type Item = ItemType;
//     #[inline(always)]
//     fn poll_next(mut self: Pin<&mut Self>, mut cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
//         unsafe { Pin::new_unchecked(self.stream.as_mut()).poll_next(&mut cx) }
//     }
// }
//

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

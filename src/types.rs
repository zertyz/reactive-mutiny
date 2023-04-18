//! Common types across this module

use crate::{
    uni::Uni,
    multi::Multi,
};
use std::{
    pin::Pin,
    task::{Context, Poll},
    fmt::Debug,
    mem::{MaybeUninit},
    ops::Deref,
    sync::Arc,
};
use futures::stream::Stream;


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
// pub type MutinyStream<DropFnOnceType, PollFnType, ItemType> = CustomDropPollFn<DropFnOnceType, PollFnType, ItemType>;


/// A stream of events for [Uni]s and [Multi]s
pub struct MutinyStream<ItemType> {
    pub stream: Box<dyn Stream<Item=ItemType> + Send>,
}
impl<ItemType> Stream for MutinyStream<ItemType> {
    type Item = ItemType;
    #[inline(always)]
    fn poll_next(mut self: Pin<&mut Self>, mut cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { Pin::new_unchecked(self.stream.as_mut()).poll_next(&mut cx) }
    }
}

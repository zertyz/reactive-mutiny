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


/// A stream of events for [Uni]s and [Multi]s
//// TODO 2023-04-07: to be dropped as it is not needed: Functions like Uni::spawn_non_futures_non_fallible_executor() might have their generics improved so that this is not needed
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

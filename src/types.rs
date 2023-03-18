//! Common types across this module

use crate::{
    multi::channels::ogre_mpmc_queue::OgreMPMCQueue,
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

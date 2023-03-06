//! Common types across this module

use crate::{
    multi::channels::ogre_mpmc_queue::OgreMPMCQueue,
    ogre_std::reference_counted_buffer_allocator::ReferenceCountedBufferAllocator,
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


// TODO failed experiment... let it sit for a while, then rip it
pub struct MultiPayload<InnerType:         Clone + Unpin + Send + Sync + Debug,
                        const BUFFER_SIZE: usize,
                        const MAX_STREAMS: usize> {
    slot_id: u32,
    channel: Arc<Pin<Box<OgreMPMCQueue<InnerType, BUFFER_SIZE, MAX_STREAMS>>>>,
}


impl<InnerType:         Clone + Unpin + Send + Sync + Debug,
     const BUFFER_SIZE: usize,
     const MAX_STREAMS: usize>
MultiPayload<InnerType, BUFFER_SIZE, MAX_STREAMS> {

    pub fn new(slot_id: u32, channel: Arc<Pin<Box<OgreMPMCQueue<InnerType, BUFFER_SIZE, MAX_STREAMS>>>>) -> Self {
        Self {
            slot_id,
            channel
        }
    }

    /// This function can only be called if `InnerType` is `Copy` -- otherwise, a double free error / Segfault will occur
    pub unsafe fn copy_unchecked(&self) -> InnerType {
        let mut copy = MaybeUninit::<InnerType>::uninit();
        std::ptr::copy_nonoverlapping(self.deref() as *const InnerType, copy.as_mut_ptr(), 1);
        copy.assume_init()
    }

}


impl<'a, InnerType:         Clone + Unpin + Send + Sync + Debug,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
Deref for
MultiPayload<InnerType, BUFFER_SIZE, MAX_STREAMS> {

    type Target = InnerType;

    fn deref(&self) -> &Self::Target {
        self.channel.buffer.slot_from_slot_id(self.slot_id)
    }
}


impl<'a, InnerType:         Clone + Unpin + Send + Sync + Debug,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
Drop for
MultiPayload<InnerType, BUFFER_SIZE, MAX_STREAMS> {

    fn drop(&mut self) {
        let mut slot = self.deref();
        if self.channel.buffer.ref_dec(slot) {
            unsafe {
                // // TODO can this copy be avoided?
                // let copy = self.copy_unchecked();
                // std::mem::drop(slot);
                std::ptr::drop_in_place(&mut slot);
            }
        }
    }

}
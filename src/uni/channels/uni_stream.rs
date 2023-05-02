//! Resting place for [UniStream]

use super::{
    super::super::{
        ogre_std::ogre_queues::{
            meta_subscriber::MetaSubscriber,
            meta_container::MetaContainer,
        },
    },
};
use std::{
    pin::Pin,
    sync::Arc,
    time::Duration,
    fmt::Debug,
    sync::atomic::Ordering::Relaxed,
    task::{Context, Poll, Waker},
    mem::{self, ManuallyDrop, MaybeUninit},
};
use std::marker::PhantomData;
use futures::stream::Stream;
use owning_ref::ArcRef;
use tokio::io::AsyncBufReadExt;
use crate::streams_manager::StreamsManagerBase;


/// Special type to allow the compiler to fully optimize the whole event consumption chain -- the following paths are covered:
/// from the container's `consume()` (providing `InItemType` items), passing through this Stream implementation, then through the user provided `pipeline_builder()` and, finally, to the `StreamExecutor`,
/// allowing all of them to behave as a single function, that gets optimized together.
pub struct UniStream<'a, ItemType:           Debug + 'a,
                         MetaContainerType:  MetaContainer<'a, ItemType> + 'a,
                         const MAX_STREAMS:  usize> {

    stream_id:         u32,
    streams_manager:   Arc<StreamsManagerBase<'a, ItemType, MetaContainerType, 1, MAX_STREAMS>>,
    _phantom:          PhantomData<&'a ItemType>,

}

impl<'a, ItemType:           Debug,
         MetaContainerType:  MetaContainer<'a, ItemType> + 'a,
         const MAX_STREAMS:  usize>
UniStream<'a, ItemType, MetaContainerType, MAX_STREAMS> {

    pub fn new(stream_id: u32, streams_manager: &Arc<StreamsManagerBase<'a, ItemType, MetaContainerType, 1, MAX_STREAMS>>) -> Self {
        Self {
            stream_id,
            streams_manager: streams_manager.clone(),
            _phantom:        PhantomData::default(),
        }
    }

}

impl<'a, ItemType:           Debug + 'a,
         MetaContainerType:  MetaContainer<'a, ItemType> + 'a,
         const MAX_STREAMS:  usize>
Stream for
UniStream<'a, ItemType, MetaContainerType, MAX_STREAMS> {

    type Item = ItemType;

    #[inline(always)]
    fn poll_next(mut self: Pin<&mut Self>, mut cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let event = self.streams_manager.for_backing_container(0, |container| {
            container.consume(|item| {
                                  let mut moved_value = MaybeUninit::<ItemType>::uninit();
                                  unsafe { std::ptr::copy_nonoverlapping(item as *const ItemType, moved_value.as_mut_ptr(), 1) }
                                  unsafe { moved_value.assume_init() }
                              },
                              || false,
                              |_| {})
        });

        match event {
            Some(_) => Poll::Ready(event),
            None => {
                if self.streams_manager.keep_stream_running(self.stream_id) {
                    self.streams_manager.register_stream_waker(self.stream_id, &cx.waker());
                    Poll::Pending
                } else {
                    Poll::Ready(None)
                }
            },
        }
    }
}

impl<'a, ItemType:          Debug,
         MetaContainerType: MetaContainer<'a, ItemType> + 'a,
         const MAX_STREAMS:  usize>
Drop for UniStream<'a, ItemType, MetaContainerType, MAX_STREAMS> {
    fn drop(&mut self) {
        self.streams_manager.report_stream_dropped(self.stream_id);
    }
}

//! Resting place for [MultiStream]

use super::{
    super::super::{
        StreamsManager,
        ogre_std::ogre_queues::meta_subscriber::MetaSubscriber,
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
use crate::ogre_std::ogre_queues::meta_container::MetaContainer;
use crate::streams_manager::StreamsManagerBase;


/// Special type to allow the compiler to fully optimize the whole event consumption chain -- the following paths are covered:
/// from the container's `consume()` (providing `InItemType` items), passing through this Stream implementation, then through the user provided `pipeline_builder()` and, finally, to the `StreamExecutor`,
/// allowing all of them to behave as a single function, that gets optimized together.
pub struct MultiStream<'a, ItemType:           Debug + 'a,
                           MetaContainerType:  MetaContainer<'a, Option<Arc<ItemType>>> + 'a,
                           const MAX_STREAMS:  usize> {

    stream_id:         u32,
    streams_manager:   Arc<StreamsManagerBase<'a, ItemType, MetaContainerType, MAX_STREAMS, MAX_STREAMS, Option<Arc<ItemType>> >>,
    backing_container: ArcRef<StreamsManagerBase<'a, ItemType, MetaContainerType, MAX_STREAMS, MAX_STREAMS, Option<Arc<ItemType>>>, MetaContainerType>,
    _phantom:          PhantomData<&'a ItemType>,

}

impl<'a, ItemType:           Debug,
         MetaContainerType:  MetaContainer<'a, Option<Arc<ItemType>>> + 'a,
         const MAX_STREAMS:  usize>
MultiStream<'a, ItemType, MetaContainerType, MAX_STREAMS> {

    pub fn new(stream_id: u32, streams_manager: &Arc<StreamsManagerBase<'a, ItemType, MetaContainerType, MAX_STREAMS, MAX_STREAMS, Option<Arc<ItemType>>>>) -> Self {
        let streams_manager = Arc::clone(&streams_manager);
        let backing_container = streams_manager.backing_container(stream_id);
        Self {
            stream_id,
            streams_manager,
            backing_container,
            _phantom: PhantomData::default(),
        }
    }

}

impl<'a, ItemType:           Debug + 'a,
         MetaContainerType:  MetaContainer<'a, Option<Arc<ItemType>>> + 'a,
         const MAX_STREAMS:  usize>
Stream for
MultiStream<'a, ItemType, MetaContainerType, MAX_STREAMS> {

    type Item = Arc<ItemType>;

    #[inline(always)]
    fn poll_next(mut self: Pin<&mut Self>, mut cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let event = self.backing_container.consume(|item| item.take().expect("godshavfty!! element cannot be None here!"),
                                                                       || false,
                                                                       |_| {});

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

impl<'a, ItemType:           Debug,
         MetaContainerType:  MetaContainer<'a, Option<Arc<ItemType>>> + 'a,
         const MAX_STREAMS:  usize>
Drop for MultiStream<'a, ItemType, MetaContainerType, MAX_STREAMS> {
    fn drop(&mut self) {
        self.streams_manager.report_stream_dropped(self.stream_id);
    }
}
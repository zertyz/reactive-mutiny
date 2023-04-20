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


/// Special type to allow the compiler to fully optimize the whole event consumption chain -- the following paths are covered:
/// from the container's `consume()` (providing `InItemType` items), passing through this Stream implementation, then through the user provided `pipeline_builder()` and, finally, to the `StreamExecutor`,
/// allowing all of them to behave as a single function, that gets optimized together.
pub struct MultiStream<'a, ItemType:           Debug + 'a,
                           StreamsManagerType: StreamsManager<'a, ItemType, MetaSubscriberType, Option<ItemType>> + 'a,
                           MetaSubscriberType: MetaSubscriber<'a, Option<ItemType>> + 'a> {

    stream_id:         u32,
    streams_manager:   Arc<StreamsManagerType>,
    backing_container: ArcRef<StreamsManagerType, MetaSubscriberType>,
    _phantom:          PhantomData<&'a ItemType>,

}

impl<'a, ItemType:           Debug,
         StreamsManagerType: StreamsManager<'a, ItemType, MetaSubscriberType, Option<ItemType>> + 'a,
         MetaSubscriberType: MetaSubscriber<'a, Option<ItemType>> + 'a>
MultiStream<'a, ItemType, StreamsManagerType, MetaSubscriberType> {

    pub fn new(stream_id: u32, streams_manager: &Arc<StreamsManagerType>) -> Self {
        let streams_manager = Arc::clone(&streams_manager);
        let backing_container = streams_manager.backing_subscriber(stream_id);
        Self {
            stream_id,
            streams_manager,
            backing_container,
            _phantom: PhantomData::default(),
        }
    }

}

impl<'a, ItemType:           Debug + 'a,
         StreamsManagerType: StreamsManager<'a, ItemType, MetaSubscriberType, Option<ItemType>>,
         MetaSubscriberType: MetaSubscriber<'a, Option<ItemType>>>
Stream for
MultiStream<'a, ItemType, StreamsManagerType, MetaSubscriberType> {

    type Item = ItemType;

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

impl<'a, ItemType:          Debug,
         StreamsManagerType: StreamsManager<'a, ItemType, MetaSubscriberType, Option<ItemType>>,
         MetaSubscriberType: MetaSubscriber<'a, Option<ItemType>>>
Drop for MultiStream<'a, ItemType, StreamsManagerType, MetaSubscriberType> {
    fn drop(&mut self) {
        self.streams_manager.report_stream_dropped(self.stream_id);
    }
}
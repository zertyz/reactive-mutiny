//! Resting place for [MutinyStream]

use crate::{
    types::ChannelConsumer,
};
use std::{
    pin::Pin,
    sync::Arc,
    fmt::Debug,
    task::{Context, Poll},
};
use std::marker::PhantomData;
use futures::stream::Stream;


/// Special `Stream` implementation to avoid using dynamic dispatching, so to allow
/// the compiler to fully optimize the whole event consumption chain.\
/// The following paths are covered:
///   - from the container's `consume()` (providing `InItemType` items),
///   - passing through this Stream implementation,
///   - then through the user provided `pipeline_builder()`
///   - and, finally, to the `StreamExecutor`.
/// ... allowing all of them to behave as a single function, that gets optimized together.
pub struct MutinyStream<'a, ItemType:            Debug + Send + Sync + 'a,
                            ChannelConsumerType: ChannelConsumer<'a, DerivedItemType> + ?Sized,
                            DerivedItemType:     'a + Debug> {
    stream_id:     u32,
    events_source: Arc<ChannelConsumerType>,
    _phantom:      PhantomData<(&'a ItemType, &'a DerivedItemType)>,

}

impl<'a, ItemType:            Debug + Send + Sync,
         ChannelConsumerType: ChannelConsumer<'a, DerivedItemType>,
         DerivedItemType:     Debug>
MutinyStream<'a, ItemType, ChannelConsumerType, DerivedItemType> {

    pub fn new(stream_id: u32, events_source: &Arc<ChannelConsumerType>) -> Self {
        Self {
            stream_id,
            events_source: events_source.clone(),
            _phantom:      PhantomData::default(),
        }
    }

}

impl<'a, ItemType:            Debug + Send + Sync + 'a,
         ChannelConsumerType: ChannelConsumer<'a, DerivedItemType>,
         DerivedItemType:     Debug>
Stream for
MutinyStream<'a, ItemType, ChannelConsumerType, DerivedItemType> {

    type Item = DerivedItemType;

    #[inline(always)]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let event = self.events_source.consume(self.stream_id);
        match event {
            Some(_) => Poll::Ready(event),
            None => {
                if self.events_source.keep_stream_running(self.stream_id) {
                    self.events_source.register_stream_waker(self.stream_id, &cx.waker());
                    Poll::Pending
                } else {
                    Poll::Ready(None)
                }
            },
        }
    }
}

impl<'a, ItemType:            Debug + Send + Sync,
         ChannelConsumerType: ChannelConsumer<'a, DerivedItemType> + ?Sized,
         DerivedItemType:     Debug>
Drop
for MutinyStream<'a, ItemType, ChannelConsumerType, DerivedItemType> {
    fn drop(&mut self) {
        self.events_source.drop_resources(self.stream_id);
    }
}
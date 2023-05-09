//! Resting place for [MutinyStream]

use crate::{
    MutinyStreamSource,
    ogre_std::ogre_alloc::{
        OgreAllocator,
        ogre_arc::OgreArc,
        ogre_array_pool_allocator::OgreArrayPoolAllocator,
    }
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
pub struct MutinyStream<'a, ItemType:          Debug + 'a,
                            StreamSourceType:  MutinyStreamSource<'a, ItemType, DerivedItemType> + ?Sized,
                            DerivedItemType:   'a = ItemType> {
    stream_id:     u32,
    events_source: Arc<StreamSourceType>,
    _phantom:      PhantomData<(&'a ItemType, &'a DerivedItemType)>,

}

impl<'a, ItemType:          Debug,
         StreamSourceType:  MutinyStreamSource<'a, ItemType, DerivedItemType>,
         DerivedItemType>
MutinyStream<'a, ItemType, StreamSourceType, DerivedItemType> {

    pub fn new(stream_id: u32, events_source: &Arc<StreamSourceType>) -> Self {
        Self {
            stream_id,
            events_source: events_source.clone(),
            _phantom:      PhantomData::default(),
        }
    }

}

impl<'a, ItemType:          Debug + 'a,
         StreamSourceType:  MutinyStreamSource<'a, ItemType, DerivedItemType>,
         DerivedItemType>
Stream for
MutinyStream<'a, ItemType, StreamSourceType, DerivedItemType> {

    type Item = DerivedItemType;

    #[inline(always)]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let event = self.events_source.provide(self.stream_id);
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

impl<'a, ItemType:          Debug,
         StreamSourceType:  MutinyStreamSource<'a, ItemType, DerivedItemType> + ?Sized,
         DerivedItemType>
Drop
for MutinyStream<'a, ItemType, StreamSourceType, DerivedItemType> {
    fn drop(&mut self) {
        self.events_source.drop_resources(self.stream_id);
    }
}
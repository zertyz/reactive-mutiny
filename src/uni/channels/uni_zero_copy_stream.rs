//! Resting place for [UniZeroCopyStream]

use std::{
    pin::Pin,
    sync::Arc,
    fmt::Debug,
    task::{Context, Poll},
};
use std::marker::PhantomData;
use futures::stream::Stream;
use crate::MutinyStreamSource;
use crate::ogre_std::ogre_alloc::ogre_arc::OgreArc;
use crate::ogre_std::ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator;
use crate::ogre_std::ogre_alloc::OgreAllocator;


/// Special type to allow the compiler to fully optimize the whole event consumption chain -- the following paths are covered:
/// from the container's `consume()` (providing `InItemType` items), passing through this Stream implementation, then through the user provided `pipeline_builder()` and, finally, to the `StreamExecutor`,
/// allowing all of them to behave as a single function, that gets optimized together.
pub struct UniZeroCopyStream<'a, ItemType:          Debug + 'a,
                                 OgreAllocatorType: OgreAllocator<ItemType> + 'a,
                                 StreamSourceType:  MutinyStreamSource<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>> + ?Sized> {
    stream_id:     u32,
    events_source: Arc<StreamSourceType>,
    _phantom:      PhantomData<(&'a ItemType, &'a OgreAllocatorType)>,

}

impl<'a, ItemType:          Debug,
         OgreAllocatorType: OgreAllocator<ItemType>,
         StreamSourceType:  MutinyStreamSource<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>>>
UniZeroCopyStream<'a, ItemType, OgreAllocatorType, StreamSourceType> {

    pub fn new(stream_id: u32, events_source: &Arc<StreamSourceType>) -> Self {
        Self {
            stream_id,
            events_source: events_source.clone(),
            _phantom:      PhantomData::default(),
        }
    }

}

impl<'a, ItemType:          Debug + 'a,
         OgreAllocatorType: OgreAllocator<ItemType>,
         StreamSourceType:  MutinyStreamSource<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>>>
Stream for
UniZeroCopyStream<'a, ItemType, OgreAllocatorType, StreamSourceType> {

    type Item = OgreArc<ItemType, OgreAllocatorType>;

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
         OgreAllocatorType: OgreAllocator<ItemType>,
         StreamSourceType:  MutinyStreamSource<'a, ItemType, OgreArc<ItemType, OgreAllocatorType>> + ?Sized>
Drop
for UniZeroCopyStream<'a, ItemType, OgreAllocatorType, StreamSourceType> {
    fn drop(&mut self) {
        self.events_source.drop_resources(self.stream_id);
    }
}

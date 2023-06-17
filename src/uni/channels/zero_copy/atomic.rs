//! Resting place for the Zero-Copy [Atomic] Uni Channel

use crate::{
    types::ChannelConsumer,
    ogre_std::{
        ogre_alloc::{
            OgreAllocator,
            ogre_unique::OgreUnique,
        },
        ogre_queues::{
            atomic::atomic_zero_copy::AtomicZeroCopy,
            meta_container::MetaContainer,
            meta_publisher::MetaPublisher,
            meta_subscriber::MetaSubscriber,
        },
    },
    types::{
        ChannelCommon,
        ChannelUni,
        ChannelProducer,
        FullDuplexUniChannel,
    },
    streams_manager::StreamsManagerBase,
    mutiny_stream::MutinyStream,
};
use std::{
    time::Duration,
    fmt::Debug,
    task::{Waker},
    sync::Arc,
    marker::PhantomData,
};
use async_trait::async_trait;


/// This channel uses the [AtomicZeroCopy] queue and the wrapping type [OgreUnique] to allow a complete zero-copy
/// operation -- no copies either when producing the event nor when consuming it, nor when passing it along to application logic functions.
pub struct Atomic<'a, ItemType:          Debug + Send + Sync,
                      OgreAllocatorType: OgreAllocator<ItemType> + 'a,
                      const BUFFER_SIZE: usize,
                      const MAX_STREAMS: usize> {

    /// common code for dealing with streams
    streams_manager: StreamsManagerBase<'a, ItemType, MAX_STREAMS>,
    /// backing storage for events
    channel:         AtomicZeroCopy<ItemType, OgreAllocatorType, BUFFER_SIZE>,
    _phantom:        PhantomData<OgreAllocatorType>,
}


#[async_trait]
impl<'a, ItemType:          Debug + Send + Sync,
         OgreAllocatorType: OgreAllocator<ItemType> + 'a + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelCommon<'a, ItemType, OgreUnique<ItemType, OgreAllocatorType>>
for Atomic<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager: StreamsManagerBase::new(name),
            channel:         AtomicZeroCopy::new(),
            _phantom:        PhantomData::default(),
        })
    }

    async fn flush(&self, timeout: Duration) -> u32 {
        self.streams_manager.flush(timeout, || self.pending_items_count()).await
    }

    async fn gracefully_end_stream(&self, stream_id: u32, timeout: Duration) -> bool {
        self.streams_manager.end_stream(stream_id, timeout, || self.pending_items_count()).await
    }

    async fn gracefully_end_all_streams(&self, timeout: Duration) -> u32 {
        self.streams_manager.end_all_streams(timeout, || self.pending_items_count()).await
    }

    fn cancel_all_streams(&self) {
        self.streams_manager.cancel_all_streams();
    }

    fn running_streams_count(&self) -> u32 {
        self.streams_manager.running_streams_count()
    }

    #[inline(always)]
    fn pending_items_count(&self) -> u32 {
        self.channel.available_elements_count() as u32
    }

    #[inline(always)]
    fn buffer_size(&self) -> u32 {
        BUFFER_SIZE as u32
    }
}


impl<'a, ItemType:          Debug + Send + Sync,
         OgreAllocatorType: OgreAllocator<ItemType> + 'a + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelUni<'a, ItemType, OgreUnique<ItemType, OgreAllocatorType>>
for Atomic<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    fn create_stream(self: &Arc<Self>)
                    -> (MutinyStream<'a, ItemType, Self, OgreUnique<ItemType, OgreAllocatorType>>, u32)
        where Self: ChannelConsumer<'a, OgreUnique<ItemType, OgreAllocatorType>> {
        let stream_id = self.streams_manager.create_stream_id();
        (MutinyStream::new(stream_id, self), stream_id)
    }
}


impl<'a, ItemType:          'a + Send + Sync + Debug,
         OgreAllocatorType: OgreAllocator<ItemType> + 'a + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelProducer<'a, ItemType, OgreUnique<ItemType, OgreAllocatorType>>
for Atomic<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn try_send<F: FnOnce(&mut ItemType)>(&self, setter: F) -> bool {
        match self.channel.publish(setter) {
            Some(len_after) => {
                let len_after = len_after.get();
                if len_after <= MAX_STREAMS as u32 {
                    self.streams_manager.wake_stream(len_after-1)
                } else if len_after == 1 + MAX_STREAMS as u32 {
                    // the Atomic queue may enqueue at the same time it dequeues, so,
                    // on high pressure for production / consumption & low event payloads (like in our tests),
                    // the Stream might have dequeued the last element, another enqueue just finished and we triggered the wake before
                    // the Stream had returned, leaving an element stuck. This code works around this and is required only for the Atomic Queue.
                    self.streams_manager.wake_stream(len_after - 2)
                }
                true
            },
            None => false,
        }
    }

    #[inline(always)]
    fn try_send_movable(&self, item: ItemType) -> bool {
        match self.channel.publish_movable(item) {
            Some(len_after) => {
                let len_after = len_after.get();
                if len_after <= MAX_STREAMS as u32 {
                    self.streams_manager.wake_stream(len_after-1)
                } else if len_after == 1 + MAX_STREAMS as u32 {
                    // the Atomic queue may enqueue at the same time it dequeues, so,
                    // on high pressure for production / consumption & low event payloads (like in our tests),
                    // the Stream might have dequeued the last element, another enqueue just finished and we triggered the wake before
                    // the Stream had returned, leaving an element stuck. This code works around this and is required only for the Atomic Queue.
                    self.streams_manager.wake_stream(len_after - 2)
                }
                true
            },
            None => false,
        }
    }
}


impl<'a, ItemType:          'static + Debug + Send + Sync,
         OgreAllocatorType: 'a + OgreAllocator<ItemType> + Send + Sync,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelConsumer<'a, OgreUnique<ItemType, OgreAllocatorType>>
for Atomic<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn consume(&self, _stream_id: u32) -> Option<OgreUnique<ItemType, OgreAllocatorType>> {
        match self.channel.consume_leaking() {
            Some( (slot_ref, _slot_id) ) => {
                Some(OgreUnique::<ItemType, OgreAllocatorType>::from_allocated_ref(slot_ref, &self.channel.allocator))
            }
            None => None,
        }
    }

    #[inline(always)]
    fn keep_stream_running(&self, stream_id: u32) -> bool {
        self.streams_manager.keep_stream_running(stream_id)
    }

    #[inline(always)]
    fn register_stream_waker(&self, stream_id: u32, waker: &Waker) {
        self.streams_manager.register_stream_waker(stream_id, waker)
    }

    #[inline(always)]
    fn drop_resources(&self, stream_id: u32) {
        self.streams_manager.report_stream_dropped(stream_id);
    }
}


impl <'a, ItemType:          'static + Debug + Send + Sync,
          OgreAllocatorType: OgreAllocator<ItemType> + 'a + Send + Sync,
          const BUFFER_SIZE: usize,
          const MAX_STREAMS: usize>
FullDuplexUniChannel<'a, ItemType, OgreUnique<ItemType, OgreAllocatorType>>
for Atomic<'a, ItemType, OgreAllocatorType, BUFFER_SIZE, MAX_STREAMS> {

    const MAX_STREAMS: usize = MAX_STREAMS;

    fn name(&self) -> &str {
        self.streams_manager.name()
    }
}


#[cfg(any(test,doc))]
mod tests {
    //! Unit tests for zero-copy [atomic](super) module

    use super::*;
    use crate::{
        prelude::advanced::ChannelUniZeroCopyAtomic,
        ogre_std::{
            ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator,
            ogre_queues,
        },
    };


    #[ctor::ctor]
    fn suite_setup() {
        simple_logger::SimpleLogger::new().with_utc_timestamps().init().unwrap_or_else(|_| eprintln!("--> LOGGER WAS ALREADY STARTED"));
    }

    #[cfg_attr(not(doc),test)]
    fn can_we_instantiate() {
        type InType = i32;
        const BUFFER_SIZE: usize = 1024;
        const MAX_STREAMS: usize = 1;
        let _channel = Atomic::<'static, InType,
                                        OgreArrayPoolAllocator<InType, ogre_queues::atomic::atomic_move::AtomicMove<u32, BUFFER_SIZE>, BUFFER_SIZE>,
                                        BUFFER_SIZE,
                                        MAX_STREAMS>::new("can I instantiate this?");
        let _channel2 = ChannelUniZeroCopyAtomic::<&str, 1024, 2>::new("That should be the same channel, but with a ref type instead");
        // all done... this controversial test was used just to guide the refactoring... maybe it can be stripped out in the near future...
    }
}
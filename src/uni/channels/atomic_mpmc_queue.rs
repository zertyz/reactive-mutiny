use crate::{
    ogre_std::{
        ogre_queues::{
            atomic_queues::atomic_meta::AtomicMeta,
            meta_publisher::MetaPublisher,
            meta_subscriber::MetaSubscriber,
            meta_container::MetaContainer,
        },
        ogre_sync,
    },
    uni::channels::{uni_stream::UniStream},
    Instruments,
};
use std::{
    time::Duration,
    sync::atomic::AtomicU32,
    pin::Pin,
    fmt::Debug,
    task::{Poll, Waker},
    sync::{Arc, atomic::Ordering::{Relaxed}},
    mem
};
use std::hint::spin_loop;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ops::Deref;
use std::sync::atomic::AtomicBool;
use futures::{Stream};
use minstant::Instant;
use owning_ref::ArcRef;
use crate::streams_manager::StreamsManagerBase;


/// A Uni channel, backed by an [AtomicMeta], that may be used to create as many streams as `MAX_STREAMS` -- which must only be dropped when it is time to drop this channel
pub struct AtomicMPMCQueue<'a, ItemType:          Send + Sync + Debug,
                               const BUFFER_SIZE: usize,
                               const MAX_STREAMS: usize> {

    streams_manager: Arc<StreamsManagerBase<'a, ItemType, AtomicMeta<ItemType, BUFFER_SIZE>, 1, MAX_STREAMS>>,
}

impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
AtomicMPMCQueue<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    /// Returns as many consumer streams as requested, provided the specified limit [MAX_STREAMS] is respected
    /// -- events will be split for each generated stream, so no two streams  will see the same payload.\
    /// Be aware of the special semantics for dropping streams: no stream should stop consuming elements (don't drop it!) until
    /// your are ready to drop the whole channel.\
    /// DEVELOPMENT NOTE: OUT-OF-TRAIT [UniChannel] implementation. Once Rust allows trait functions to return `impls`, this should be moved there
    pub fn consumer_stream(&self) -> UniStream<'a, ItemType, AtomicMeta<ItemType, BUFFER_SIZE>, MAX_STREAMS> {
        let stream_id = self.streams_manager.create_stream_id();
        UniStream::new(stream_id, &self.streams_manager)
    }

}

//#[async_trait]
/// implementation note: Rust 1.63 does not yet support async traits. See [super::UniChannel]
impl<'a, ItemType:          'a + Send + Sync + Debug,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
/*UniChannel<ItemType>
for*/ AtomicMPMCQueue<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    /// Instantiates
    pub fn new<IntoString: Into<String>>(streams_manager_name: IntoString) -> Self {
        Self {
            streams_manager: Arc::new(StreamsManagerBase::new(streams_manager_name))
        }
    }

    /// clones & shares the internal `streams_manager`, for testing purposes
    pub(crate) fn streams_manager(&self) -> Arc<StreamsManagerBase<'a, ItemType, AtomicMeta<ItemType, BUFFER_SIZE>, 1, MAX_STREAMS>> {
        self.streams_manager.clone()
    }

    #[inline(always)]
    pub unsafe fn zero_copy_try_send(&self, item_builder: impl Fn(&mut ItemType)) -> bool {
        self.streams_manager.for_backing_container(0, |container| {
            container.publish(item_builder,
                              || {
                                  self.streams_manager.wake_all_streams();
                                  false
                              },
                              |len| if len <= MAX_STREAMS as u32 { self.streams_manager.wake_stream(len-1) })
        })
    }

    #[inline(always)]
    pub fn try_send(&self, item: ItemType) -> bool {
        let item = ManuallyDrop::new(item);       // ensure it won't be dropped when this function ends, since it will be "moved"
        unsafe {
            self.zero_copy_try_send(|slot| {
                // move `item` to `slot`
                std::ptr::copy_nonoverlapping(item.deref() as *const ItemType, (slot as *const ItemType) as *mut ItemType, 1);
            })
        }
    }

    pub async fn flush(&self, timeout: Duration) -> u32 {
        self.streams_manager.flush(timeout).await
    }

    pub async fn end_all_streams(&self, timeout: Duration) -> u32 {
        self.streams_manager.end_all_streams(timeout).await
    }

    pub fn cancel_all_streams(&self) {
        self.streams_manager.cancel_all_streams();
    }

    pub fn running_streams_count(&self) -> u32 {
        self.streams_manager.running_streams_count()
    }

    #[inline(always)]
    pub fn pending_items_count(&self) -> u32 {
        self.streams_manager.pending_items_count()
    }

}

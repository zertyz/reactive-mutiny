//! Resting place for the [OgreFullSyncMPMCQueue] Uni Channel

use crate::{
    uni::channels::uni_stream::UniStream,
    ogre_std::{
        ogre_queues::{
            full_sync_queues::full_sync_meta::FullSyncMeta,
            meta_publisher::MetaPublisher,
            meta_subscriber::MetaSubscriber,
            meta_container::MetaContainer,
        },
        ogre_sync,
    }
};
use std::{
    time::Duration,
    sync::atomic::{AtomicU32, AtomicBool, Ordering::Relaxed},
    pin::Pin,
    fmt::Debug,
    task::{Poll, Waker},
    sync::Arc,
    mem::{self, ManuallyDrop, MaybeUninit},
    hint::spin_loop,
    ops::Deref
};
use std::marker::PhantomData;
use futures::{Stream};
use minstant::Instant;
use owning_ref::ArcRef;
use crate::streams_manager::StreamsManagerBase;


/// This channel uses the fastest of the queues [FullSyncMeta], which are the fastest for general purpose use and for most hardware but requires that elements are copied, due to the full sync characteristics
/// of the backing queue, which doesn't allow enqueueing to happen independently of dequeueing.\
/// Due to that, this channel requires that `ItemType`s are `Clone`, since they will have to be moved around during dequeueing (as there is no way to keep the queue slot allocated during processing),
/// making this channel a typical best fit for small & trivial types.\
/// Please, measure your `Uni`s using all available channels [OgreFullSyncMPMCQueue], [OgreAtomicQueue] and, possibly, even [OgreMmapLog].\
/// See also [uni::channels::ogre_full_sync_mpmc_queue].\
/// Refresher: the backing queue requires "BUFFER_SIZE" to be a power of 2
pub struct OgreFullSyncMPMCQueue<'a, ItemType:          Send + Sync + Debug,
                                     const BUFFER_SIZE: usize,
                                     const MAX_STREAMS: usize> {

    streams_manager: Arc<StreamsManagerBase<'a, ItemType, FullSyncMeta<ItemType, BUFFER_SIZE>, 1, MAX_STREAMS>>,
}

impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
OgreFullSyncMPMCQueue<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    /// Returns as many consumer streams as requested, provided the specified limit [MAX_STREAMS] is respected.
    /// -- events will be split for each generated stream, so no two streams  will see the same payload.\
    /// Be aware of the special semantics for dropping streams: no stream should stop consuming elements (don't drop it!) until
    /// your are ready to drop the whole channel.\
    /// DEVELOPMENT NOTE: OUT-OF-TRAIT [UniChannel] implementation. Once Rust allows trait functions to return `impls`, this should be moved there
    pub fn consumer_stream(&self) -> UniStream<'a, ItemType, FullSyncMeta<ItemType, BUFFER_SIZE>, MAX_STREAMS> {
        let stream_id = self.streams_manager.create_stream_id();
        UniStream::new(stream_id, &self.streams_manager)
    }

}

//#[async_trait]
/// implementation note: Rust 1.63 does not yet support async traits. See [super::UniChannel]
impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
/*UniChannel<ItemType>
for*/ OgreFullSyncMPMCQueue<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    /// Instantiates
    pub fn new<IntoString: Into<String>>(streams_manager_name: IntoString) -> Self {
        Self {
            streams_manager: Arc::new(StreamsManagerBase::new(streams_manager_name))
        }
    }

    /// clones & shares the internal `streams_manager`, for testing purposes
    pub(crate) fn streams_manager(&self) -> Arc<StreamsManagerBase<'a, ItemType, FullSyncMeta<ItemType, BUFFER_SIZE>, 1, MAX_STREAMS>> {
        self.streams_manager.clone()
    }

    #[inline(always)]
    pub unsafe fn zero_copy_try_send(&self, item_builder: impl Fn(&mut ItemType)) -> bool {
        self.streams_manager.for_backing_container(0, |container| {
            container.publish(item_builder,
                              || false,
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
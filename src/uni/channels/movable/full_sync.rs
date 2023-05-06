//! Resting place for the [FullSync] Uni Channel

use crate::{
    uni::channels::uni_stream::UniStream, ogre_std::{
        ogre_queues::{
            full_sync_queues::full_sync_meta::FullSyncMeta,
            meta_publisher::MetaPublisher,
            meta_subscriber::MetaSubscriber,
            meta_container::MetaContainer,
        },
    },
    MutinyStreamSource,
};
use std::{
    time::Duration,
    pin::Pin,
    fmt::Debug,
    task::{Waker},
    sync::Arc,
    mem::{ManuallyDrop, MaybeUninit},
    ops::Deref
};
use crate::streams_manager::StreamsManagerBase;
use async_trait::async_trait;
use crate::uni::channels::{UniChannelCommon, UniMovableChannel};


/// This channel uses the fastest of the queues [FullSyncMeta], which are the fastest for general purpose use and for most hardware but requires that elements are copied, due to the full sync characteristics
/// of the backing queue, which doesn't allow enqueueing to happen independently of dequeueing.\
/// Due to that, this channel requires that `ItemType`s are `Clone`, since they will have to be moved around during dequeueing (as there is no way to keep the queue slot allocated during processing),
/// making this channel a typical best fit for small & trivial types.\
/// Please, measure your `Uni`s using all available channels [FullSync], [OgreAtomicQueue] and, possibly, even [OgreMmapLog].\
/// See also [uni::channels::ogre_full_sync_mpmc_queue].\
/// Refresher: the backing queue requires "BUFFER_SIZE" to be a power of 2
pub struct FullSync<'a, ItemType:          Send + Sync + Debug,
                        const BUFFER_SIZE: usize,
                        const MAX_STREAMS: usize> {

    /// common code for dealing with streams
    streams_manager: Arc<StreamsManagerBase<'a, ItemType, MAX_STREAMS>>,
    /// backing storage for events -- AKA, channels
    container:       Pin<Box<FullSyncMeta<ItemType, BUFFER_SIZE>>>,

}

#[async_trait]
impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
UniChannelCommon<'a, ItemType>
for FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(streams_manager_name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager: Arc::new(StreamsManagerBase::new(streams_manager_name)),
            container:       Box::pin(FullSyncMeta::<ItemType, BUFFER_SIZE>::new()),
        })
    }

    fn consumer_stream(self: &Arc<Self>) -> UniStream<'a, ItemType, Self> {
        let stream_id = self.streams_manager.create_stream_id();
        UniStream::new(stream_id, self)
    }

    async fn flush(&self, timeout: Duration) -> u32 {
        self.streams_manager.flush(timeout, || self.pending_items_count()).await
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
        self.container.available_elements_count() as u32
    }
}

impl<'a, ItemType:          Send + Sync + Debug + 'a,
    const BUFFER_SIZE: usize,
    const MAX_STREAMS: usize>
UniMovableChannel<'a, ItemType>
for FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn zero_copy_try_send(&self, item_builder: impl FnOnce(&mut ItemType)) -> bool {
        self.container.publish(item_builder,
                              || false,
                              |len| if len <= MAX_STREAMS as u32 { self.streams_manager.wake_stream(len-1) })
    }

    #[inline(always)]
    fn try_send(&self, item: ItemType) -> bool {
        let item = ManuallyDrop::new(item);       // ensure it won't be dropped when this function ends, since it will be "moved"
        unsafe {
            self.zero_copy_try_send(|slot| {
                // move `item` to `slot`
                std::ptr::copy_nonoverlapping(item.deref() as *const ItemType, (slot as *const ItemType) as *mut ItemType, 1);
            })
        }
    }
}

impl<'a, ItemType:          'a + Send + Sync + Debug,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
MutinyStreamSource<'a, ItemType>
for FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn provide(&self, _stream_id: u32) -> Option<ItemType> {
        self.container.consume(|item| {
                                   let mut moved_value = MaybeUninit::<ItemType>::uninit();
                                   unsafe { std::ptr::copy_nonoverlapping(item as *const ItemType, moved_value.as_mut_ptr(), 1) }
                                   unsafe { moved_value.assume_init() }
                               },
                               || false,
                               |_| {})
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

use crate::{
    types::MutinyStreamSource,
    ogre_std::{
        ogre_queues::{
            atomic_queues::atomic_meta::AtomicMeta,
            meta_publisher::MetaPublisher,
            meta_subscriber::MetaSubscriber,
            meta_container::MetaContainer,
        },
    },
    uni::channels::{
        UniChannelCommon,
        UniMovableChannel,
        uni_stream::UniStream,
    },
    streams_manager::StreamsManagerBase,
};
use std::{time::Duration, pin::Pin, fmt::Debug, task::{Waker}, sync::Arc};
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ops::Deref;
use async_trait::async_trait;


/// A Uni channel, backed by an [AtomicMeta], that may be used to create as many streams as `MAX_STREAMS` -- which must only be dropped when it is time to drop this channel
pub struct Atomic<'a, ItemType:          Send + Sync + Debug,
                      const BUFFER_SIZE: usize,
                      const MAX_STREAMS: usize> {

    /// common code for dealing with streams
    streams_manager: StreamsManagerBase<'a, ItemType, MAX_STREAMS>,
    /// backing storage for events -- AKA, channels
    channel:       Pin<Box<AtomicMeta<ItemType, BUFFER_SIZE>>>,

}

#[async_trait]      // all async functions are out of the hot path, so the `async_trait` won't impose performance penalties
impl<'a, ItemType:          'a + Send + Sync + Debug,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
UniChannelCommon<'a, ItemType>
for Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager: StreamsManagerBase::new(name),
            channel:         Box::pin(AtomicMeta::<ItemType, BUFFER_SIZE>::new()),
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
        self.channel.available_elements_count() as u32
    }
}

impl<'a, ItemType:          'a + Send + Sync + Debug,
    const BUFFER_SIZE: usize,
    const MAX_STREAMS: usize>
UniMovableChannel<'a, ItemType>
for Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn zero_copy_try_send(&self, item_builder: impl FnOnce(&mut ItemType)) -> bool {
        self.channel.publish(item_builder,
                             || {
                                   self.streams_manager.wake_all_streams();
                                   false
                               },
                             |len| if len <= 1 {
                                                                        self.streams_manager.wake_stream(0)
                                                                    } else if len <= MAX_STREAMS as u32 {
                                                                        self.streams_manager.wake_stream(len-1)
                                                                    })
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
for Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn provide(&self, _stream_id: u32) -> Option<ItemType> {
        self.channel.consume(|item| {
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

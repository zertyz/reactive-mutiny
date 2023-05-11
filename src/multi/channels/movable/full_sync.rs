//! Resting place for the [FullSync] Multi Channel

use crate::{
    ogre_std::{
        ogre_queues::{
            full_sync::{
                full_sync_move::FullSyncMove,
            },
            meta_publisher::MovePublisher,
            meta_subscriber::MoveSubscriber,
            meta_container::MoveContainer,
        },
    },
    streams_manager::StreamsManagerBase,
    mutiny_stream::MutinyStream,
    ChannelConsumer,
};
use std::{
    time::Duration,
    sync::{
        Arc,
    },
    pin::Pin,
    fmt::Debug,
    task::{Waker},
};
use async_trait::async_trait;
use log::{warn};
use crate::uni::channels::{ChannelCommon, ChannelProducer, FullDuplexChannel};


/// This channel uses the queues [FullSyncMove] (the highest throughput among all in 'benches/'), which are the fastest for general purpose use and for most hardware but requires that elements are copied when dequeueing,
/// due to the full sync characteristics of the backing queue, which doesn't allow enqueueing to happen independently of dequeueing.\
/// Due to that, this channel requires that `ItemType`s are `Clone`, since they will have to be moved around during dequeueing (as there is no way to keep the queue slot allocated during processing),
/// making this channel a typical best fit for small & trivial types.\
/// Please, measure your `Multi`s using all available channels [FullSync], [OgreAtomicQueue] and, possibly, even [OgreMmapLog].\
/// See also [multi::channels::ogre_full_sync_mpmc_queue].\
/// Refresher: the backing queue requires `BUFFER_SIZE` to be a power of 2 -- the same applies to `MAX_STREAMS`, which will also have its own queue
pub struct FullSync<'a, ItemType:          Send + Sync + Debug,
                        const BUFFER_SIZE: usize = 1024,
                        const MAX_STREAMS: usize = 16> {

    /// common code for dealing with streams
    streams_manager: StreamsManagerBase<'a, ItemType, MAX_STREAMS>,
    /// backing storage for events -- AKA, channels
    channels:        [Pin<Box<FullSyncMove<Arc<ItemType>, BUFFER_SIZE>>>; MAX_STREAMS],
}


#[async_trait]      // all async functions are out of the hot path, so the `async_trait` won't impose performance penalties
impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelCommon<'a, ItemType, Arc<ItemType>>
for FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(streams_manager_name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager: StreamsManagerBase::new(streams_manager_name),
            channels:        [0; MAX_STREAMS].map(|_| Box::pin(FullSyncMove::<Arc<ItemType>, BUFFER_SIZE>::new())),
        })
    }

    fn create_stream(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, Arc<ItemType>>, u32) {
        let stream_id = self.streams_manager.create_stream_id();
        (MutinyStream::new(stream_id, self), stream_id)
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
        self.streams_manager.used_streams().iter()
            .filter(|&&stream_id| stream_id != u32::MAX)
            .map(|&stream_id| unsafe { self.channels.get_unchecked(stream_id as usize) }.available_elements_count())
            .sum::<usize>() as u32
    }

    #[inline(always)]
    fn buffer_size(&self) -> u32 {
        BUFFER_SIZE as u32
    }
}


impl<'a, ItemType:          'a + Send + Sync + Debug,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelProducer<'a, ItemType, Arc<ItemType>>
for FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn try_send(&self, item: ItemType) -> bool {
        todo!("FullSync Arc Multi Channel: try_send() is not implemented for Multis, as not enqueueing an item is an unacceptable condition. Use `send()` or `send_derived()` instead")
    }

    #[inline(always)]
    fn send(&self, item: ItemType) {
        let arc_item = Arc::new(item);
        self.send_derived(&arc_item);
    }

    #[inline(always)]
    fn send_derived(&self, arc_item: &Arc<ItemType>) {
        for stream_id in self.streams_manager.used_streams() {
            if *stream_id == u32::MAX {
                break
            }
            loop {
                let channel = unsafe { self.channels.get_unchecked(*stream_id as usize) };
                match channel.publish_movable(arc_item.clone()) {
                    Some(len_after_publishing) => {
                        if len_after_publishing.get() <= 1 {
                            self.streams_manager.wake_stream(*stream_id);
                        }
                        break;
                    },
                    None => {
                        self.streams_manager.wake_stream(*stream_id);
                        warn!("Multi Channel's FullSync (named '{channel_name}', {used_streams_count} streams): One of the streams (#{stream_id}) is full of elements. Multi producing performance has been degraded. Increase the Multi buffer size (currently {BUFFER_SIZE}) to overcome that.",
                              channel_name = self.streams_manager.name(), used_streams_count = self.streams_manager.running_streams_count());
                        std::thread::sleep(Duration::from_millis(500));
                    },
                }
            }
        }
    }

}


impl<'a, ItemType:          'a + Send + Sync + Debug,
    const BUFFER_SIZE: usize,
    const MAX_STREAMS: usize>
ChannelConsumer<'a, Arc<ItemType>>
for FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn consume(&self, stream_id: u32) -> Option<Arc<ItemType>> {
        let channel = unsafe { self.channels.get_unchecked(stream_id as usize) };
        channel.consume_movable()
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


impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
Drop for
FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {
    fn drop(&mut self) {
        self.streams_manager.cancel_all_streams();
    }
}


impl <'a, ItemType:          'a + Debug + Send + Sync,
          const BUFFER_SIZE: usize,
          const MAX_STREAMS: usize>
FullDuplexChannel<'a, ItemType, Arc<ItemType>>
for FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {}
//! Resting place for the [Atomic] Multi Channel

use crate::{
    ogre_std::{
        ogre_queues::{
            atomic::atomic_move::AtomicMove,
            meta_publisher::MovePublisher,
            meta_subscriber::MoveSubscriber,
            meta_container::MoveContainer,
        },
    },
    multi::channels::{
        MultiChannelCommon,
        MultiMovableChannel,
    },
    streams_manager::StreamsManagerBase,
    mutiny_stream::MutinyStream,
    MutinyStreamSource,
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
use std::num::NonZeroU32;
use log::{warn};
use async_trait::async_trait;


/// This channel uses the queue [AtomicMove] (the lowest latency among all in 'benches/'), which allows zero-copy both when enqueueing / dequeueing and
/// allow enqueueing to happen independently of dequeueing.\
/// Due to that, this channel requires that `ItemType`s are `Clone`, since they will have to be moved around during dequeueing (as there is no way to keep the queue slot allocated during processing),
/// making this channel a typical best fit for small & trivial types.\
/// Please, measure your `Multi`s using all available channels [Atomic], [OgreAtomicQueue] and, possibly, even [OgreMmapLog].\
/// See also [multi::channels::ogre_full_sync_mpmc_queue].\
/// Refresher: the backing queue requires `BUFFER_SIZE` to be a power of 2 -- the same applies to `MAX_STREAMS`, which will also have its own queue
pub struct Atomic<'a, ItemType:          Send + Sync + Debug,
                      const BUFFER_SIZE: usize = 1024,
                      const MAX_STREAMS: usize = 16> {

    /// common code for dealing with streams
    streams_manager: StreamsManagerBase<'a, ItemType, MAX_STREAMS>,
    /// backing storage for events -- AKA, channels
    channels:        [Pin<Box<AtomicMove<Arc<ItemType>, BUFFER_SIZE>>>; MAX_STREAMS],

}

#[async_trait]      // all async functions are out of the hot path, so the `async_trait` won't impose performance penalties
impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
MultiChannelCommon<'a, ItemType>
for Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager: StreamsManagerBase::new(name),
            channels:        [0; MAX_STREAMS].map(|_| Box::pin(AtomicMove::<Arc<ItemType>, BUFFER_SIZE>::new())),
        })
    }

    fn listener_stream(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, Arc<ItemType>>, u32) {
        let stream_id = self.streams_manager.create_stream_id();
        (MutinyStream::new(stream_id, self), stream_id)
    }

    async fn flush(&self, timeout: Duration) -> u32 {
        self.streams_manager.flush(timeout, || self.pending_items_count()).await
    }

    async fn end_stream(&self, stream_id: u32, timeout: Duration) -> bool {
        self.streams_manager.end_stream(stream_id, timeout, || self.pending_items_count()).await
    }

    async fn end_all_streams(&self, timeout: Duration) -> u32 {
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
MultiMovableChannel<'a, ItemType>
for Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn send(&self, item: ItemType) {
        let arc_item = Arc::new(item);
        self.send_arc(&arc_item);
    }

    #[inline(always)]
    fn send_arc(&self, arc_item: &Arc<ItemType>) {
        for stream_id in self.streams_manager.used_streams() {
            if *stream_id == u32::MAX {
                break
            }
            let channel = unsafe { self.channels.get_unchecked(*stream_id as usize) };
            loop {
                match channel.publish_movable(arc_item.clone()) {
                    Some(len_after_publishing) => {
                        if len_after_publishing.get() <= 2 {
                            self.streams_manager.wake_stream(*stream_id);
                        }
                        break;
                    },
                    None => {
                        self.streams_manager.wake_stream(*stream_id);
                        warn!("Multi Channel's Atomic (named '{channel_name}', {used_streams_count} streams): One of the streams (#{stream_id}) is full of elements. Multi producing performance has been degraded. Increase the Multi buffer size (currently {BUFFER_SIZE}) to overcome that.",
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
MutinyStreamSource<'a, ItemType, Arc<ItemType>>
for Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn provide(&self, stream_id: u32) -> Option<Arc<ItemType>> {
        let channel = unsafe { self.channels.get_unchecked(stream_id as usize) };
        channel.consume_movable()
    }

    #[inline(always)]
    fn keep_stream_running(&self, stream_id: u32) -> bool {
        self.streams_manager.keep_stream_running(stream_id)
    }

    #[inline(always)]
    fn register_stream_waker(&self, stream_id: u32, waker: &Waker) {
        self.streams_manager.register_stream_waker(stream_id, waker);
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
Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {
    fn drop(&mut self) {
        self.streams_manager.cancel_all_streams();
    }
}

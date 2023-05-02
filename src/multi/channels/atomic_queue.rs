//! Resting place for the [AtomicQueue] Multi Channel

use crate::{
    ogre_std::{
        ogre_queues::{
            OgreQueue,
            full_sync_queues::{
                full_sync_meta::FullSyncMeta,
                NonBlockingQueue,
            },
            meta_publisher::MetaPublisher,
            meta_subscriber::MetaSubscriber,
            meta_container::MetaContainer,
        },
        ogre_sync,
    },
    multi::channels::multi_stream::MultiStream,
    streams_manager::StreamsManagerBase,
};
use std::{
    time::Duration,
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicBool, Ordering::{Relaxed}},
    },
    pin::Pin,
    fmt::Debug,
    task::{Poll, Waker},
    mem::{self, MaybeUninit},
    hint::spin_loop,
};
use futures::{Stream};
use minstant::Instant;
use owning_ref::ArcRef;
use log::{warn};
use crate::ogre_std::ogre_queues::atomic_queues::atomic_meta::AtomicMeta;


/// This channel uses the the queue [AtomicMeta] (the lowest latency among all in 'benches/'), which allows zero-copy both when enqueueing / dequeueing and
/// allow enqueueing to happen independently of dequeueing.\
/// Due to that, this channel requires that `ItemType`s are `Clone`, since they will have to be moved around during dequeueing (as there is no way to keep the queue slot allocated during processing),
/// making this channel a typical best fit for small & trivial types.\
/// Please, measure your `Multi`s using all available channels [AtomicQueue], [OgreAtomicQueue] and, possibly, even [OgreMmapLog].\
/// See also [multi::channels::ogre_full_sync_mpmc_queue].\
/// Refresher: the backing queue requires `BUFFER_SIZE` to be a power of 2 -- the same applies to `MAX_STREAMS`, which will also have its own queue
pub struct AtomicQueue<'a, ItemType:          Send + Sync + Debug,
                                             const BUFFER_SIZE: usize = 1024,
                                             const MAX_STREAMS: usize = 16> {

    streams_manager: Arc<StreamsManagerBase<'a, ItemType, AtomicMeta<Option<Arc<ItemType>>, BUFFER_SIZE>, MAX_STREAMS, MAX_STREAMS, Option<Arc<ItemType>>>>,
}

impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
AtomicQueue<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    /// Returns a `Stream` -- as many streams as requested may be create, provided the specified limit [MAX_STREAMS] is respected.\
    /// Each stream will see all payloads sent through this channel.
    pub fn listener_stream(&self) -> (MultiStream<'a, ItemType, AtomicMeta<Option<Arc<ItemType>>, BUFFER_SIZE>, MAX_STREAMS>, u32) {
        let stream_id = self.streams_manager.create_stream_id();
        (MultiStream::new(stream_id, &self.streams_manager), stream_id)
    }

    #[inline(always)]
    pub fn buffer_size(&self) -> u32 {
        BUFFER_SIZE as u32
    }
}

//#[async_trait]
/// implementation note: Rust 1.63 does not yet support async traits. See [super::MultiChannel]
impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
/*MultiChannel<ItemType>
for*/ AtomicQueue<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    /// Instantiates
    pub fn new<IntoString: Into<String>>(streams_manager_name: IntoString) -> Self {
        Self {
            streams_manager: Arc::new(StreamsManagerBase::new(streams_manager_name)),
        }
    }

    /// clones & shares the internal `streams_manager`, for testing purposes
    pub fn streams_manager(&self) -> Arc<StreamsManagerBase<'a, ItemType, AtomicMeta<Option<Arc<ItemType>>, BUFFER_SIZE>, MAX_STREAMS, MAX_STREAMS, Option<Arc<ItemType>>>> {
        self.streams_manager.clone()
    }

    #[inline(always)]
    pub fn send_arc(&self, arc_item: &Arc<ItemType>) {
        for stream_id in self.streams_manager.used_streams() {
            if *stream_id == u32::MAX {
                break
            }
            self.streams_manager.for_backing_container(*stream_id, |container| {
                container.publish(|slot| { let _ = slot.insert(Arc::clone(&arc_item)); },
                                  || {
                                      self.streams_manager.wake_stream(*stream_id);
                                      warn!("Multi Channel's AtomicQueue (named '{channel_name}', {used_streams_count} streams): One of the streams (#{stream_id}) is full of elements. Multi producing performance has been degraded. Increase the Multi buffer size (currently {BUFFER_SIZE}) to overcome that.",
                                            channel_name = self.streams_manager.name(), used_streams_count = self.streams_manager.running_streams_count());
                                      std::thread::sleep(Duration::from_millis(500));
                                      true
                                  },
                                  |len| if len <= 2 { self.streams_manager.wake_stream(*stream_id) })
            });
        }
    }

    #[inline(always)]
    pub fn send(&self, item: ItemType) {
        let arc_item = Arc::new(item);
        self.send_arc(&arc_item);
    }

    pub async fn flush(&self, timeout: Duration) -> u32 {
        self.streams_manager.flush(timeout).await
    }

    pub async fn end_stream(&self, stream_id: u32, timeout: Duration) -> bool {
        self.streams_manager.end_stream(stream_id, timeout).await
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

    #[inline(always)]
    pub fn wake_stream(&self, stream_id: u32) {
        self.streams_manager.wake_stream(stream_id);
    }

}

impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
Drop for
AtomicQueue<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {
    fn drop(&mut self) {
        self.streams_manager.cancel_all_streams();
    }
}
//! Resting place for the reference-based [MmapLog] Zero-Copy Multi Channel

use crate::{
    ogre_std::{
        ogre_queues::{
            meta_topic::MetaTopic,
            log_topics::mmap_meta::MMapMeta,
            meta_publisher::MovePublisher,
            meta_subscriber::{MoveSubscriber, MetaSubscriber},
            meta_container::MoveContainer,
        },
        ogre_alloc::{
            ogre_arc::OgreArc,
            ogre_array_pool_allocator::OgreArrayPoolAllocator,
            OgreAllocator,
        },
    },
    uni::channels::{ChannelCommon, ChannelProducer, FullDuplexMultiChannel},
    streams_manager::StreamsManagerBase,
    mutiny_stream::MutinyStream,
    ChannelConsumer,
};
use std::{
    time::Duration,
    sync::{
        Arc,
        atomic::{
            AtomicUsize,
            Ordering::Relaxed,
        }
    },
    pin::Pin,
    fmt::Debug,
    task::{Waker},
    num::NonZeroU32,
};
use async_trait::async_trait;
use log::{warn};
use crate::ogre_std::ogre_queues::log_topics::mmap_meta::{MMapMetaSubscriber};
use crate::ogre_std::ogre_queues::meta_publisher::MetaPublisher;
use crate::uni::channels::ChannelMulti;


/// ...
pub struct MmapLog<'a, ItemType:          Send + Sync + Debug,
                       const MAX_STREAMS: usize = 16> {

    /// common code for dealing with streams
    streams_manager:     StreamsManagerBase<'a, ItemType, MAX_STREAMS>,
    /// backing storage for events
    log_queue:           Arc<MMapMeta<'a, ItemType>>,
    /// tracking of each Stream's next event to send
    subscribers:         [MMapMetaSubscriber<'a, ItemType>; MAX_STREAMS],
}


#[async_trait]      // all async functions are out of the hot path, so the `async_trait` won't impose performance penalties
impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const MAX_STREAMS: usize>
ChannelCommon<'a, ItemType, &'static ItemType>
for MmapLog<'a, ItemType, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(name: IntoString) -> Arc<Self> {
        let name = name.into();
        let mmap_file_path = format!("/tmp/{}.mmap", name.chars().map(|c| if c == ' ' || c >= '0' || c <= '9' || c >= 'A' || c <= 'z' { c } else { '_' }).collect::<String>());
        let log_queue = MMapMeta::new(mmap_file_path, 1<<38).expect("TODO: 2023-05-24: MAKE THIS TRAIT ALLOW RETURNING AN ERROR");
        Arc::new(Self {
            streams_manager: StreamsManagerBase::new(name),
            log_queue:       log_queue.clone(),
            subscribers:     [0; MAX_STREAMS].map(|_| log_queue.subscribe_to_new_events_only()),
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

    #[inline(always)]
    fn running_streams_count(&self) -> u32 {
        self.streams_manager.running_streams_count()
    }

    #[inline(always)]
    fn pending_items_count(&self) -> u32 {
        self.streams_manager.used_streams().iter()
            .take_while(|&&stream_id| stream_id != u32::MAX)
            .map(|&stream_id| unsafe { self.subscribers.get_unchecked(stream_id as usize) }.remaining_elements_count())
            .max().unwrap_or(0) as u32
    }

    #[inline(always)]
    fn buffer_size(&self) -> u32 {
        u32::MAX
    }
}


impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const MAX_STREAMS: usize>
ChannelMulti<'a, ItemType, &'static ItemType>
for MmapLog<'a, ItemType, MAX_STREAMS> {

    fn create_stream_for_old_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, &'static ItemType>, u32) where Self: ChannelConsumer<'a, &'static ItemType> {
        let ref_self: &Self = self;
        let mutable_self = unsafe { &mut *((ref_self as *const Self) as *mut Self) };
        let stream_id = self.streams_manager.create_stream_id();
        mutable_self.subscribers[stream_id as usize] = self.log_queue.subscribe_to_old_events_only();
        (MutinyStream::new(stream_id, self), stream_id)
    }

    fn create_stream_for_new_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, &'static ItemType>, u32) {
        let ref_self: &Self = self;
        let mutable_self = unsafe { &mut *((ref_self as *const Self) as *mut Self) };
        let stream_id = self.streams_manager.create_stream_id();
        mutable_self.subscribers[stream_id as usize] = self.log_queue.subscribe_to_new_events_only();
        (MutinyStream::new(stream_id, self), stream_id)
    }

    fn create_streams_for_old_and_new_events(self: &Arc<Self>) -> ((MutinyStream<'a, ItemType, Self, &'static ItemType>, u32), (MutinyStream<'a, ItemType, Self, &'static ItemType>, u32)) where Self: ChannelConsumer<'a, &'static ItemType> {
        let ref_self: &Self = self;
        let mutable_self = unsafe { &mut *((ref_self as *const Self) as *mut Self) };
        let (stream_of_oldies, stream_of_newies) = self.log_queue.subscribe_to_separated_old_and_new_events();
        let stream_of_oldies_id = self.streams_manager.create_stream_id();
        let stream_of_newies_id = self.streams_manager.create_stream_id();
        mutable_self.subscribers[stream_of_oldies_id as usize] = stream_of_oldies;
        mutable_self.subscribers[stream_of_newies_id as usize] = stream_of_newies;
        ( (MutinyStream::new(stream_of_oldies_id, self), stream_of_oldies_id),
          (MutinyStream::new(stream_of_newies_id, self), stream_of_newies_id) )
    }

    fn create_stream_for_old_and_new_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, &'static ItemType>, u32) where Self: ChannelConsumer<'a, &'static ItemType> {
        let ref_self: &Self = self;
        let mutable_self = unsafe { &mut *((ref_self as *const Self) as *mut Self) };
        let stream_id = self.streams_manager.create_stream_id();
        mutable_self.subscribers[stream_id as usize] = self.log_queue.subscribe_to_joined_old_and_new_events();
        (MutinyStream::new(stream_id, self), stream_id)
    }
}


impl<'a, ItemType:          'a + Send + Sync + Debug,
         const MAX_STREAMS: usize>
ChannelProducer<'a, ItemType, &'static ItemType>
for MmapLog<'a, ItemType, MAX_STREAMS> {

    #[inline(always)]
    fn try_send<F: FnOnce(&mut ItemType)>(&self, setter: F) -> bool {
        if let Some(tail) = self.log_queue.publish(setter) {
            let running_streams_count = self.streams_manager.running_streams_count();
            let used_streams = self.streams_manager.used_streams();
            for i in 0..running_streams_count {
                let stream_id = *unsafe { used_streams.get_unchecked(i as usize) };
                if stream_id != u32::MAX {
                    self.streams_manager.wake_stream(stream_id);
                }
            }
            true
        } else {
            false
        }
    }

    #[inline(always)]
    fn send<F: FnOnce(&mut ItemType)>(&self, setter: F) {
        if !self.try_send(setter) {
            panic!("reactive_mutiny::multi::channels::references::MMapLog: `send()` detected that the mmap file is full for channel named '{}' -- its tail is at #{}. Try increasing this limit.", self.streams_manager.name(), self.log_queue.available_elements_count())
        }
    }

    #[inline(always)]
    fn send_derived(&self, _derived_item: &&'static ItemType) {
        todo!("reactive_mutiny::multi::channels::references::MMapLog: `send_derived()` is not implemented for the MMapLog Multi channel '{}' -- it doesn't make sense to place a reference in an mmap", self.streams_manager.name())
    }

    #[inline(always)]
    fn try_send_movable(&self, item: ItemType) -> bool {
        self.try_send(|slot| *slot = item)
    }
}


impl<'a, ItemType:          'a + Send + Sync + Debug,
         const MAX_STREAMS: usize>
ChannelConsumer<'a, &'static ItemType>
for MmapLog<'a, ItemType, MAX_STREAMS> {

    #[inline(always)]
    fn consume(&self, stream_id: u32) -> Option<&'static ItemType> {
        let subscriber = unsafe { self.subscribers.get_unchecked(stream_id as usize) };
        subscriber.consume(|slot| unsafe {&*(slot as *const ItemType)},
                           || false,
                           |_len_after| {})
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
         const MAX_STREAMS: usize>
Drop for
MmapLog<'a, ItemType, MAX_STREAMS> {
    fn drop(&mut self) {
        self.streams_manager.cancel_all_streams();
    }
}


impl <'a, ItemType:          'a + Debug + Send + Sync,
          const MAX_STREAMS: usize>
FullDuplexMultiChannel<'a, ItemType, &'static ItemType>
for MmapLog<'a, ItemType, MAX_STREAMS> {}
//! Resting place for the Arc based [FullSync] Zero-Copy Multi Channel

use crate::{
    ogre_std::ogre_queues::{
            full_sync::full_sync_move::FullSyncMove,
            meta_publisher::MovePublisher,
            meta_subscriber::MoveSubscriber,
            meta_container::MoveContainer,
        },
    streams_manager::StreamsManagerBase,
    mutiny_stream::MutinyStream,
    types::{ChannelCommon, ChannelMulti, ChannelProducer, ChannelConsumer, FullDuplexMultiChannel},
};
use std::{
    time::Duration,
    sync::Arc,
    fmt::Debug,
    task::Waker,
    mem::MaybeUninit,
};
use std::future::Future;
use std::marker::PhantomData;
use log::warn;


/// This channel uses the queues [FullSyncMove] (the highest throughput among all in 'benches/'), which are the fastest for general purpose use and for most hardware but requires that elements are copied when dequeueing,
/// due to the full sync characteristics of the backing queue, which doesn't allow enqueueing to happen independently of dequeueing.\
/// Due to that, this channel requires that `ItemType`s are `Clone`, since they will have to be moved around during dequeueing (as there is no way to keep the queue slot allocated during processing),
/// making this channel a typical best fit for small & trivial types.\
/// Please, measure your `Multi`s using all available channels [FullSync], [OgreAtomicQueue] and, possibly, even [OgreMmapLog].\
/// See also [multi::channels::ogre_full_sync_mpmc_queue].\
/// Refresher: the backing queue requires `BUFFER_SIZE` to be a power of 2 -- the same applies to `MAX_STREAMS`, which will also have its own queue
pub struct FullSync<'a, ItemType:          Send + Sync + Debug + Default,
                        const BUFFER_SIZE: usize = 1024,
                        const MAX_STREAMS: usize = 16> {

    /// common code for dealing with streams
    streams_manager: StreamsManagerBase<MAX_STREAMS>,
    /// backing storage for events -- AKA, channels
    channels:        [FullSyncMove<Arc<ItemType>, BUFFER_SIZE>; MAX_STREAMS],
    _phanrom: PhantomData<&'a ItemType>,
}


impl<'a, ItemType:          Send + Sync + Debug + Default + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelCommon<ItemType, Arc<ItemType>> for
FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(streams_manager_name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager: StreamsManagerBase::new(streams_manager_name),
            channels:        [0; MAX_STREAMS].map(|_| FullSyncMove::<Arc<ItemType>, BUFFER_SIZE>::with_initializer(|| unsafe {
                let mut slot = MaybeUninit::<Arc<ItemType>>::uninit();
                slot.as_mut_ptr().write_bytes(1u8, 1);  // initializing with a non-zero value makes MIRI happily state there isn't any UB involved (actually, any value is equally good)
                slot.assume_init()
            })),
            _phanrom: PhantomData,
        })
    }

    async fn flush(&self, timeout: Duration) -> u32 {
        self.streams_manager.flush(timeout, || self.pending_items_count()).await
    }

    #[inline(always)]
    fn is_channel_open(&self) -> bool {
        self.streams_manager.is_any_stream_running()
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
            .map(|&stream_id| unsafe { self.channels.get_unchecked(stream_id as usize) }.available_elements_count())
            .max().unwrap_or(0) as u32
    }

    #[inline(always)]
    fn buffer_size(&self) -> u32 {
        BUFFER_SIZE as u32
    }
}


impl<'a, ItemType:          Send + Sync + Debug + Default + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelMulti<'a, ItemType, Arc<ItemType>> for
FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn create_stream_for_old_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, Arc<ItemType>>, u32) where Self: ChannelConsumer<'a, Arc<ItemType>> {
        panic!("multi::channels::arc::FullSync: this channel doesn't implement the `.create_stream_for_old_events()` method. Use `.create_stream_for_new_events()` instead")
    }

    fn create_stream_for_new_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, Arc<ItemType>>, u32) {
        let stream_id = self.streams_manager.create_stream_id();
        (MutinyStream::new(stream_id, self), stream_id)
    }

    fn create_streams_for_old_and_new_events(self: &Arc<Self>) -> ((MutinyStream<'a, ItemType, Self, Arc<ItemType>>, u32), (MutinyStream<'a, ItemType, Self, Arc<ItemType>>, u32)) where Self: ChannelConsumer<'a, Arc<ItemType>> {
        panic!("multi::channels::arc::FullSync: this channel doesn't implement the `.create_streams_for_old_and_new_events()` method. Use `.create_stream_for_new_events()` instead")
    }

    fn create_stream_for_old_and_new_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, Arc<ItemType>>, u32) where Self: ChannelConsumer<'a, Arc<ItemType>> {
        panic!("multi::channels::arc::FullSync: this channel doesn't implement the `.create_stream_for_old_and_new_events()` method. Use `.create_stream_for_new_events()` instead")
    }

}


impl<'a, ItemType:          'a + Send + Sync + Debug + Default,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelProducer<'a, ItemType, Arc<ItemType>> for
FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn send(&self, item: ItemType) -> keen_retry::RetryConsumerResult<(), ItemType, ()> {
        let arc_item = Arc::new(item);
        _ = self.send_derived(&arc_item);
        keen_retry::RetryResult::Ok { reported_input: (), output: () }
    }

    #[inline(always)]
    fn send_with<F: FnOnce(&mut ItemType)>(&self, setter: F) -> keen_retry::RetryConsumerResult<(), F, ()> {
        let mut item = MaybeUninit::uninit();
        let item_ref = unsafe { &mut *item.as_mut_ptr() };
        setter(item_ref);
        let item = unsafe { item.assume_init() };
        let arc_item = Arc::new(item);
        _ = self.send_derived(&arc_item);
        keen_retry::RetryResult::Ok { reported_input: (), output: () }
    }

    #[inline(always)]
    async fn send_with_async<F:   FnOnce(&'a mut ItemType) -> Fut,
                             Fut: Future<Output=&'a mut ItemType>>
                            (&'a self,
                             setter: F) -> keen_retry::RetryConsumerResult<(), F, ()> {
        let mut item = MaybeUninit::uninit();
        let item_ref = unsafe { &mut *item.as_mut_ptr() };
        setter(item_ref).await;
        let item = unsafe { item.assume_init() };
        let arc_item = Arc::new(item);
        _ = self.send_derived(&arc_item);
        keen_retry::RetryResult::Ok { reported_input: (), output: () }
    }

    #[inline(always)]
    fn send_derived(&self, arc_item: &Arc<ItemType>) -> bool {
        for stream_id in self.streams_manager.used_streams() {
            if *stream_id == u32::MAX {
                break
            }
            loop {
                let channel = unsafe { self.channels.get_unchecked(*stream_id as usize) };
                match channel.publish_movable(arc_item.clone()).0 {
                    Some(len_after_publishing) => {
                        if len_after_publishing.get() <= 1 {
                            self.streams_manager.wake_stream(*stream_id);
                        }
                        break;
                    },
                    None => {
                        self.streams_manager.wake_stream(*stream_id);
// TODO 2023-08-02: the possibility of this code indicates all our arc based channels is not a good fit for our retrying semantics. A possible correction would be to use a lock + count all listener's free slots... but OgreArc based ones seem to be a better design
warn!("Multi Channel's for Arc FullSync (named '{channel_name}', {used_streams_count} streams): One of the streams (#{stream_id}) is full of elements. Multi producing performance has been degraded. Increase the Multi buffer size (currently {BUFFER_SIZE}) to overcome that.",
      channel_name = self.streams_manager.name(), used_streams_count = self.streams_manager.running_streams_count());
std::thread::sleep(Duration::from_millis(500));
                    },
                }
            }
        }
        true
    }

    #[inline(always)]
    fn reserve_slot(&self) -> Option<&'a mut ItemType> {
        panic!("If we choose to go on with this Arc channel, this must be implemented")
    }

    #[inline(always)]
    fn try_send_reserved(&self, _reserved_slot: &mut ItemType) -> bool {
        panic!("If we choose to go on with this Arc channel, this must be implemented")
    }

    #[inline(always)]
    fn try_cancel_slot_reserve(&self, _reserved_slot: &mut ItemType) -> bool {
        panic!("If we choose to go on with this Arc channel, this must be implemented")
    }
}


impl<'a, ItemType:          'a + Send + Sync + Debug + Default,
    const BUFFER_SIZE: usize,
    const MAX_STREAMS: usize>
ChannelConsumer<'a, Arc<ItemType>> for
FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

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


impl<'a, ItemType:          Send + Sync + Debug + Default + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
Drop for
FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {
    fn drop(&mut self) {
        self.streams_manager.cancel_all_streams();
    }
}


impl <ItemType:          'static + Debug + Send + Sync + Default,
      const BUFFER_SIZE: usize,
      const MAX_STREAMS: usize>
FullDuplexMultiChannel for
FullSync<'static, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    const MAX_STREAMS: usize = MAX_STREAMS;
    const BUFFER_SIZE: usize = BUFFER_SIZE;
    type ItemType            = ItemType;
    type DerivedItemType     = Arc<ItemType>;
}
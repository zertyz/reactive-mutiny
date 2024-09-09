//! Resting place for the Arc based [Crossbeam] Zero-Copy Multi Channel

use crate::{
    streams_manager::StreamsManagerBase,
    mutiny_stream::MutinyStream,
    types::{ChannelCommon, ChannelMulti, ChannelProducer, ChannelConsumer, FullDuplexMultiChannel},
};
use std::{
    fmt::Debug,
    sync::Arc,
    time::Duration,
    mem::MaybeUninit,
    task::Waker,
};
use std::future::Future;
use std::marker::PhantomData;
use crossbeam_channel::{Sender, Receiver, TryRecvError};
use log::warn;


pub struct Crossbeam<'a, ItemType:          Send + Sync + Debug,
    const BUFFER_SIZE: usize = 1024,
    const MAX_STREAMS: usize = 16> {

    /// common code for dealing with streams
    streams_manager: StreamsManagerBase<MAX_STREAMS>,
    senders:        [Sender<Arc<ItemType>>; MAX_STREAMS],
    receivers:      [Receiver<Arc<ItemType>>; MAX_STREAMS],
    _phanrom: PhantomData<&'a ItemType>,

}


impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelCommon<ItemType, Arc<ItemType>> for
Crossbeam<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(streams_manager_name: IntoString) -> Arc<Self> {
        let (mut senders, mut receivers): (Vec<_>, Vec<_>) = (0 .. MAX_STREAMS)
            .map(|_| crossbeam_channel::bounded::<Arc<ItemType>>(BUFFER_SIZE))
            .unzip();
        Arc::new(Self {
            streams_manager: StreamsManagerBase::new(streams_manager_name),
            senders:         [0; MAX_STREAMS].map(|_| senders.pop().unwrap()),
            receivers:       [0; MAX_STREAMS].map(|_| receivers.pop().unwrap()),
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

    fn running_streams_count(&self) -> u32 {
        self.streams_manager.running_streams_count()
    }

    #[inline(always)]
    fn pending_items_count(&self) -> u32 {
        self.streams_manager.used_streams().iter()
            .take_while(|&&stream_id| stream_id != u32::MAX)
            .map(|&stream_id| unsafe { self.receivers.get_unchecked(stream_id as usize) }.len())
            .max().unwrap_or(0) as u32
    }

    #[inline(always)]
    fn buffer_size(&self) -> u32 {
        BUFFER_SIZE as u32
    }

}


impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelMulti<'a, ItemType, Arc<ItemType>> for
Crossbeam<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn create_stream_for_old_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, Arc<ItemType>>, u32) where Self: ChannelConsumer<'a, Arc<ItemType>> {
        panic!("multi::channels::arc::Crossbeam: this channel doesn't implement the `.create_stream_for_old_events()` method. Use `.create_stream_for_new_events()` instead")
    }

    fn create_stream_for_new_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, Arc<ItemType>>, u32) {
        let stream_id = self.streams_manager.create_stream_id();
        (MutinyStream::new(stream_id, self), stream_id)
    }

    fn create_streams_for_old_and_new_events(self: &Arc<Self>) -> ((MutinyStream<'a, ItemType, Self, Arc<ItemType>>, u32), (MutinyStream<'a, ItemType, Self, Arc<ItemType>>, u32)) where Self: ChannelConsumer<'a, Arc<ItemType>> {
        panic!("multi::channels::arc::Crossbeam: this channel doesn't implement the `.create_streams_for_old_and_new_events()` method. Use `.create_stream_for_new_events()` instead")
    }

    fn create_stream_for_old_and_new_events(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, Arc<ItemType>>, u32) where Self: ChannelConsumer<'a, Arc<ItemType>> {
        panic!("multi::channels::arc::Crossbeam: this channel doesn't implement the `.create_stream_for_old_and_new_events()` method. Use `.create_stream_for_new_events()` instead")
    }

}


impl<'a, ItemType:          'a + Send + Sync + Debug,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelProducer<'a, ItemType, Arc<ItemType>> for
Crossbeam<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

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
            let sender = unsafe { self.senders.get_unchecked(*stream_id as usize) };
            match sender.len() {
                len_before if len_before <= 2 => {
                    let _ = sender.try_send(arc_item.clone());
                    self.streams_manager.wake_stream(*stream_id);
                },
                _ => while sender.try_send(arc_item.clone()).is_err() {
                    self.streams_manager.wake_stream(*stream_id);
// TODO 2023-08-02: the possibility of this code indicates all our arc based channels is not a good fit for our retrying semantics. A possible correction would be to use a lock + count all listener's free slots... but OgreArc based ones seem to be a better design
warn!("Multi Channel's Arc Crossbeam (named '{channel_name}', {used_streams_count} streams): One of the streams (#{stream_id}) is full of elements. Multi producing performance has been degraded. Increase the Multi buffer size (currently {BUFFER_SIZE}) to overcome that.",
      channel_name = self.streams_manager.name(), used_streams_count = self.streams_manager.running_streams_count());
std::thread::sleep(Duration::from_millis(500));
                },
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


impl<'a, ItemType:          'a + Send + Sync + Debug,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelConsumer<'a, Arc<ItemType>> for
Crossbeam<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn consume(&self, stream_id: u32) -> Option<Arc<ItemType>> {
        let receiver = unsafe { self.receivers.get_unchecked(stream_id as usize) };
        match receiver.try_recv() {
            Ok(event) => {
                Some(event)
            },
            Err(status) => match status {
                TryRecvError::Empty        => None,
                TryRecvError::Disconnected => {
                    self.streams_manager.cancel_stream(stream_id);
                    None
                }
            },
        }
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


impl <ItemType:          'static + Debug + Send + Sync,
      const BUFFER_SIZE: usize,
      const MAX_STREAMS: usize>
FullDuplexMultiChannel for
Crossbeam<'static, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    const MAX_STREAMS: usize = MAX_STREAMS;
    const BUFFER_SIZE: usize = BUFFER_SIZE;
    type ItemType            = ItemType;
    type DerivedItemType     = Arc<ItemType>;
}
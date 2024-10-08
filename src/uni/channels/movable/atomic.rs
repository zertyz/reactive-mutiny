//! Resting place for [Atomic]

use crate::{
    types::ChannelConsumer,
    ogre_std::ogre_queues::{
            atomic::atomic_move::AtomicMove,
            meta_publisher::MovePublisher,
            meta_subscriber::MoveSubscriber,
            meta_container::MoveContainer,
        },
    types::{
        ChannelCommon,
        ChannelUni,
        ChannelProducer,
        FullDuplexUniChannel,
    },
    streams_manager::StreamsManagerBase,
    mutiny_stream::MutinyStream,
};
use std::{
    time::Duration,
    fmt::Debug,
    task::Waker,
    sync::Arc,
};
use std::future::Future;
use std::marker::PhantomData;


/// A Uni channel, backed by an [AtomicMove], that may be used to create as many streams as `MAX_STREAMS` -- which must only be dropped when it is time to drop this channel
pub struct Atomic<'a, ItemType:          Send + Sync + Debug + Default,
                      const BUFFER_SIZE: usize,
                      const MAX_STREAMS: usize> {

    /// common code for dealing with streams
    streams_manager: StreamsManagerBase<MAX_STREAMS>,
    /// backing storage for events -- AKA, channels
    channel:         AtomicMove<ItemType, BUFFER_SIZE>,
    _phantom:        PhantomData<&'a ItemType>,

}

impl<'a, ItemType:          'a + Send + Sync + Debug + Default,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelCommon<ItemType, ItemType>
for Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager: StreamsManagerBase::new(name),
            channel:         AtomicMove::<ItemType, BUFFER_SIZE>::new(),
            _phantom: PhantomData,
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
        self.channel.available_elements_count() as u32
    }

    #[inline(always)]
    fn buffer_size(&self) -> u32 {
        BUFFER_SIZE as u32
    }
}

impl<'a, ItemType:          'a + Send + Sync + Debug + Default,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelUni<'a, ItemType, ItemType>
for Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn create_stream(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, ItemType>, u32) {
        let stream_id = self.streams_manager.create_stream_id();
        (MutinyStream::new(stream_id, self), stream_id)
    }
}

impl<'a, ItemType:          'a + Send + Sync + Debug + Default,
    const BUFFER_SIZE: usize,
    const MAX_STREAMS: usize>
ChannelProducer<'a, ItemType, ItemType>
for Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn send(&self, item: ItemType) -> keen_retry::RetryConsumerResult<(), ItemType, ()> {
        match self.channel.publish_movable(item) {
            (Some(len_after), _none_item) => {
                let len_after = len_after.get();
                if len_after <= MAX_STREAMS as u32 {
                    self.streams_manager.wake_stream(len_after-1)
                } else if len_after == 1 + MAX_STREAMS as u32 {
                    // the Atomic queue may enqueue at the same time it dequeues, so,
                    // on high pressure for production / consumption & low event payloads (like in our tests),
                    // the Stream might have dequeued the last element, another enqueue just finished and we triggered the wake before
                    // the Stream had returned, leaving an element stuck. This code works around this and is required only for the Atomic Queue.
                    self.streams_manager.wake_stream(len_after - 2)
                }
                keen_retry::RetryResult::Ok { reported_input: (), output: () }
            },
            (None, some_item) => {
                keen_retry::RetryResult::Transient { input: some_item.unwrap(), error: () }
            },
        }
    }

    #[inline(always)]
    fn send_with<F: FnOnce(&mut ItemType)>(&self, setter: F) -> keen_retry::RetryConsumerResult<(), F, ()> {
        let setter_option = self.channel.publish(setter, || false, |len_after| {
            if len_after <= MAX_STREAMS as u32 {
                self.streams_manager.wake_stream(len_after - 1)
            } else if len_after == 1 + MAX_STREAMS as u32 {
                // the Atomic queue may enqueue at the same time it dequeues, so,
                // on high pressure for production / consumption & low event payloads (like in our tests),
                // the Stream might have dequeued the last element, another enqueue just finished and we triggered the wake before
                // the Stream had returned, leaving an element stuck. This code works around this and is required only for the Atomic Queue.
                self.streams_manager.wake_stream(len_after - 2)
            }
        });
        // implementation note: superior branch prediction over using `.map_or_else()` directly, as `None` is more likely
        match setter_option {
            None => keen_retry::RetryResult::Ok { reported_input: (), output: () },
            Some(setter) => keen_retry::RetryResult::Transient { input: setter, error: () }
        }
    }

    #[inline(always)]
    async fn send_with_async<F:   FnOnce(&'a mut ItemType) -> Fut,
                             Fut: Future<Output=&'a mut ItemType>>
                            (&'a self,
                             setter: F) -> keen_retry::RetryConsumerResult<(), F, ()> {
        if let Some((slot, slot_id, len_before)) = self.channel.leak_slot_internal(|| false) {
            setter(slot).await;
            self.channel.publish_leaked_internal(slot_id);
            if len_before < MAX_STREAMS as u32 {
                self.streams_manager.wake_stream(len_before);
            }
            keen_retry::RetryResult::Ok { reported_input: (), output: () }
        } else {
            keen_retry::RetryResult::Transient { input: setter, error: () }
        }
    }

    #[inline(always)]
    fn reserve_slot(&self) -> Option<&mut ItemType> {
        self.channel.leak_slot_internal(|| false)
            .map(|(slot_ref, _slot_id, _len_before)| slot_ref)
    }

    #[inline(always)]
    fn try_send_reserved(&self, reserved_slot: &mut ItemType) -> bool {
        let slot_index = self.channel.slot_index_from_slot_ref(reserved_slot);
        self.channel.try_publish_leaked_internal_index(slot_index)
            .map(|len_after| {
                // wake the streams, if needed
                let len_after = len_after.get();
                if len_after <= MAX_STREAMS as u32 {
                    self.streams_manager.wake_stream(len_after % MAX_STREAMS as u32);
                }
                true
            }).unwrap_or(false)
    }

    #[inline(always)]
    fn try_cancel_slot_reserve(&self, reserved_slot: &mut ItemType) -> bool {
        let slot_index = self.channel.slot_index_from_slot_ref(reserved_slot);
        self.channel.try_unleak_slot_index_internal(slot_index)
    }


}

impl<'a, ItemType:          'a + Send + Sync + Debug + Default,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelConsumer<'a, ItemType>
for Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn consume(&self, _stream_id: u32) -> Option<ItemType> {
        self.channel.consume_movable()
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

impl <ItemType:          'static + Debug + Send + Sync + Default,
      const BUFFER_SIZE: usize,
      const MAX_STREAMS: usize>
FullDuplexUniChannel
for Atomic<'static, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    const MAX_STREAMS: usize = MAX_STREAMS;
    const BUFFER_SIZE: usize = BUFFER_SIZE;
    type ItemType            = ItemType;
    type DerivedItemType     = ItemType;

    fn name(&self) -> &str {
        self.streams_manager.name()
    }

}
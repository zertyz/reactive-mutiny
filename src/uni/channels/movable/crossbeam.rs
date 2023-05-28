use crate::{
    streams_manager::StreamsManagerBase,
    types::{
        ChannelCommon,
        ChannelUni,
        ChannelConsumer,
        ChannelProducer,
        FullDuplexUniChannel,
    },
    mutiny_stream::MutinyStream,
};
use std::{
    fmt::Debug,
    sync::Arc,
    time::Duration,
    mem::{ManuallyDrop, MaybeUninit},
    task::Waker,
};
use crossbeam_channel::{Sender, Receiver, TryRecvError};
use async_trait::async_trait;


pub struct Crossbeam<'a, ItemType,
                         const BUFFER_SIZE: usize,
                         const MAX_STREAMS: usize> {
    tx: Sender<ItemType>,
    rx: Receiver<ItemType>,
    streams_manager: Arc<StreamsManagerBase<'a, ItemType, MAX_STREAMS>>,
}

#[async_trait]      // all async functions are out of the hot path, so the `async_trait` won't impose performance penalties
impl<'a, ItemType: 'a + Send + Sync + Debug,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelCommon<'a, ItemType, ItemType>
for Crossbeam<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(streams_manager_name: IntoString) -> Arc<Self> {
        let (tx, rx) = crossbeam_channel::bounded::<ItemType>(BUFFER_SIZE);
        Arc::new(Self {
            tx,
            rx,
            streams_manager: Arc::new(StreamsManagerBase::new(streams_manager_name)),
        })
    }

    async fn flush(&self, timeout: Duration) -> u32 {
        self.streams_manager.flush(timeout, || self.tx.len() as u32).await
    }

    async fn gracefully_end_stream(&self, stream_id: u32, timeout: Duration) -> bool {
        self.streams_manager.end_stream(stream_id, timeout, || self.tx.len() as u32).await
    }

    async fn gracefully_end_all_streams(&self, timeout: Duration) -> u32 {
        self.streams_manager.end_all_streams(timeout, || self.tx.len() as u32).await
    }

    fn cancel_all_streams(&self) {
        self.streams_manager.cancel_all_streams()
    }

    fn running_streams_count(&self) -> u32 {
        self.streams_manager.running_streams_count()
    }

    #[inline(always)]
    fn pending_items_count(&self) -> u32 {
        self.tx.len() as u32
    }

    #[inline(always)]
    fn buffer_size(&self) -> u32 {
        BUFFER_SIZE as u32
    }
}

impl<'a, ItemType: 'a + Send + Sync + Debug,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
ChannelUni<'a, ItemType, ItemType>
for Crossbeam<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn create_stream(self: &Arc<Self>) -> (MutinyStream<'a, ItemType, Self, ItemType>, u32) {
        let stream_id = self.streams_manager.create_stream_id();
        (MutinyStream::new(stream_id, self), stream_id)
    }
}

impl<'a, ItemType: 'a + Send + Sync + Debug,
    const BUFFER_SIZE: usize,
    const MAX_STREAMS: usize>
ChannelProducer<'a, ItemType, ItemType>
for Crossbeam<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn try_send<F: FnOnce(&mut ItemType)>(&self, setter: F) -> bool {
        // the following value-filling sequence avoids UB for droppable types & references (like `&str`)
        let mut item = MaybeUninit::uninit();
        let mut item_ref = unsafe { &mut *item.as_mut_ptr() };
        setter(item_ref);
        self.try_send_movable(unsafe { item.assume_init() })
    }

    #[inline(always)]
    fn try_send_movable(&self, item: ItemType) -> bool {
        match self.tx.len() {
            len_before if len_before <= 2 => {
                let ret = self.tx.try_send(item).is_ok();
                self.streams_manager.wake_stream(0);
                ret
            },
            _ => self.tx.try_send(item).is_ok(),
        }
    }
}

impl<'a, ItemType: 'a + Debug,
    const BUFFER_SIZE: usize,
    const MAX_STREAMS: usize>
ChannelConsumer<'a, ItemType> for
Crossbeam<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn consume(&self, stream_id: u32) -> Option<ItemType> {
        match self.rx.try_recv() {
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
        self.streams_manager.register_stream_waker(stream_id, waker)
    }

    #[inline(always)]
    fn drop_resources(&self, stream_id: u32) {
        self.streams_manager.report_stream_dropped(stream_id)
    }
}

impl <'a, ItemType:          'a + Debug + Send + Sync,
          const BUFFER_SIZE: usize,
          const MAX_STREAMS: usize>
FullDuplexUniChannel<'a, ItemType, ItemType>
for Crossbeam<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {}
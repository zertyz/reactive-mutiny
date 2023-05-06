use std::{
    fmt::Debug,
    sync::Arc,
    time::Duration,
};
use std::mem::MaybeUninit;
use std::task::Waker;
use crossbeam_channel::{Sender, Receiver, TryRecvError};
use crate::MutinyStreamSource;
use crate::streams_manager::StreamsManagerBase;
use crate::uni::channels::uni_stream::{UniStream};
use crate::uni::channels::{UniChannelCommon, UniMovableChannel};
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
UniChannelCommon<'a, ItemType>
for Crossbeam<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn new<IntoString: Into<String>>(streams_manager_name: IntoString) -> Arc<Self> {
        let (tx, rx) = crossbeam_channel::bounded::<ItemType>(BUFFER_SIZE);
        Arc::new(Self {
            tx,
            rx,
            streams_manager: Arc::new(StreamsManagerBase::new(streams_manager_name)),
        })
    }

    fn consumer_stream(self: &Arc<Self>) -> UniStream<'a, ItemType, Self> {
        let stream_id = self.streams_manager.create_stream_id();
        UniStream::new(stream_id, self)
    }

    async fn flush(&self, timeout: Duration) -> u32 {
        self.streams_manager.flush(timeout, || self.tx.len() as u32).await
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

}

impl<'a, ItemType: 'a + Send + Sync + Debug,
    const BUFFER_SIZE: usize,
    const MAX_STREAMS: usize>
UniMovableChannel<'a, ItemType>
for Crossbeam<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn zero_copy_try_send(&self, item_builder: impl FnOnce(&mut ItemType)) -> bool {
        let mut item = unsafe { MaybeUninit::<ItemType>::uninit().assume_init() };
        item_builder(&mut item);
        self.try_send(item)
    }

    #[inline(always)]
    fn try_send(&self, item: ItemType) -> bool {
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
MutinyStreamSource<'a, ItemType> for
Crossbeam<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn provide(&self, stream_id: u32) -> Option<ItemType> {
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
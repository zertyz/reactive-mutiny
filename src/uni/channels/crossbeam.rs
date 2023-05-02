use std::{
    pin::Pin,
    fmt::Debug,
    sync::Arc,
    task::Poll,
    time::Duration,
};
use crossbeam_channel::{Sender, Receiver, TryRecvError};
use futures::{Stream, stream};
use minstant::Instant;


pub struct Crossbeam<ItemType, const BUFFER_SIZE: usize> {
    tx: Sender<ItemType>,
    rx: Receiver<ItemType>,
}

/// implementation note: Rust 1.63 does not yet support async traits. See [super::UniChannel]
impl<'a, ItemType: 'a + Debug, const BUFFER_SIZE: usize> /*UniChannel<ItemType> for*/ Crossbeam<ItemType, BUFFER_SIZE> {

    /// Instantiates
    pub fn new<IntoString: Into<String>>(_streams_manager_name: IntoString) -> Arc<Self> {
        let (tx, rx) = crossbeam_channel::bounded::<ItemType>(BUFFER_SIZE);
        Arc::new(Self { tx, rx })
    }

    pub fn consumer_stream(self: &Arc<Self>) -> impl Stream<Item=ItemType> + 'a {
        let cloned_self = self.clone();
        stream::poll_fn(move |cx| {
            match cloned_self.rx.try_recv() {
                Ok(event) => {
                    Poll::Ready(Some(event))
                },
                Err(status) => match status {
                    TryRecvError::Empty => {
                        // if this ever goes to production, optimize this -- currently, the stream will spin loop when empty
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    },
                    TryRecvError::Disconnected => Poll::Ready(None)
                },
            }
        })
    }

    pub async fn send(&self, _item: ItemType) -> bool {
        panic!("uni::channels::Crossbeam doesn't support blocking send at this time");
    }

    pub fn try_send(&self, item: ItemType) -> bool {
        self.tx.try_send(item).is_ok()
    }

    unsafe fn zero_copy_try_send(&self, _item_builder: impl FnOnce(&mut ItemType)) {
        panic!("uni::channels::Crossbeam doesn't support zero-copy send at this time");
    }

    pub async fn flush(&self, _timeout: Duration) -> u32 {
        0
    }

    pub async fn end_all_streams(&self, _timeout: Duration) -> u32 {
        0
    }

    pub fn cancel_all_streams(&self) {}

    pub fn pending_items_count(&self) -> u32 {
        self.tx.len() as u32
    }

}

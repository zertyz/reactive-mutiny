use std::{
    pin::Pin,
    fmt::Debug,
    sync::Arc,
    task::Poll,
    time::Duration,
};
use tokio::sync::{
    mpsc::{Sender, Receiver, error::TrySendError},
    Mutex,
};
use futures::{Stream, stream};
use minstant::Instant;


pub struct TokioMPSC<ItemType, const BUFFER_SIZE: usize> {
    tx: Sender<ItemType>,
    rx: Mutex<Option<Receiver<ItemType>>>,
}

/// implementation note: Rust 1.63 does not yet support async traits. See [super::UniChannel]
impl<ItemType: Debug, const BUFFER_SIZE: usize> /*UniChannel<ItemType> for*/ TokioMPSC<ItemType, BUFFER_SIZE> {

    pub fn new() -> Arc<Pin<Box<Self>>> {
        let (tx, rx) = tokio::sync::mpsc::channel::<ItemType>(BUFFER_SIZE);
        Arc::new(Box::pin(Self {
            tx,
            rx: Mutex::new(Some(rx)),
        }))
    }

    pub fn consumer_stream(self: &Arc<Pin<Box<Self>>>) -> Option<impl Stream<Item=ItemType>> {
        let rx = loop {
            let locked = self.rx.try_lock();
            if locked.is_ok() {
                break locked.unwrap()
            }
        }.take();
        const EMPTY_RETRIES: u32 = 4096;
        let mut empty_retry_count = 0;
        match rx {
            None => None,
            Some(mut rx) => Some(stream::poll_fn(move |cx| {
                let ret = rx.poll_recv(cx);
                if let Poll::Pending = &ret {
                    empty_retry_count += 1;
                    if empty_retry_count < EMPTY_RETRIES {
                        std::hint::spin_loop();
                        cx.waker().wake_by_ref();
                    }
                } else {
                    empty_retry_count = 0;
                }
                ret
            })),
        }
    }

    pub async fn send(&self, item: ItemType) {
        match self.tx.send(item).await {
            Ok(_) => (),
            Err(err) => panic!("Application Design Bug! Could not send through a TokioMPSC Uni Channel. Was the stream closed? Please chase and fix. Error: {:?}", err),
        }
    }

    pub fn try_send(&self, item: ItemType) -> bool {
        match self.tx.try_send(item) {
            Ok(_) => true,
            Err(err) => match err {
                TrySendError::Full(_) => false,
                TrySendError::Closed(err) => panic!("Application Design Bug! Could not send through a TokioMPSC Uni Channel. The stream was closed! Please chase and fix. Error: {:?}", err),
            }
        }
    }

    unsafe fn zero_copy_try_send(&self, _item_builder: impl FnOnce(&mut ItemType)) {
        panic!("tokio::mpsc::channel does not support zero-copy sending");
    }

    pub async fn flush(&self, timeout: Duration) -> u32 {
        let mut start: Option<Instant> = None;
        loop {
            let pending_count = self.pending_items_count();
            if pending_count > 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            } else {
                break 0
            }
            // enforce timeout
            if let Some(start) = start {
                if start.elapsed() > timeout {
                    break pending_count
                }
            } else {
                start = Some(Instant::now());
            }
        }
    }

    pub async fn end_all_streams(&self, timeout: Duration) -> u32 {
        if self.flush(timeout).await > 0 {
            1
        } else {
            0
        }
    }


    pub fn pending_items_count(&self) -> u32 {
        (BUFFER_SIZE - self.tx.capacity()) as u32
    }

}

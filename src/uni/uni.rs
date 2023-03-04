//! See [super]

use super::super::{
    stream_executor::StreamExecutor,
};
use std::{
    sync::Arc,
};
use std::fmt::Debug;
use std::pin::Pin;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;
use futures::{Stream};
use minstant::Instant;


/// this is the fastest [UniChannel] for general use, as revealed in performance tests
type UniChannelType<ItemType,
                    const BUFFER_SIZE: usize,
                    const MAX_STREAMS: usize> = channels::ogre_mpmc_queue::OgreMPMCQueue<ItemType, BUFFER_SIZE, MAX_STREAMS>;

/// Contains the producer-side [Uni] handle used to interact with the `uni` event
/// -- for closing the stream, requiring stats, ...
pub struct Uni<ItemType:          Unpin + Send + Sync + Debug,
               const BUFFER_SIZE: usize,
               const MAX_STREAMS: usize,
               const LOG:         bool,
               const METRICS:     bool> {
    pub uni_channel:              Arc<Pin<Box<UniChannelType<ItemType, BUFFER_SIZE, MAX_STREAMS>>>>,
    pub stream_executor:          Arc<StreamExecutor<LOG, METRICS>>,
    pub finished_executors_count: AtomicU32,
}

impl<ItemType:          Unpin + Send + Sync + Debug + 'static,
     const BUFFER_SIZE: usize,
     const MAX_STREAMS: usize,
     const LOG:         bool,
     const METRICS:     bool>
Uni<ItemType, BUFFER_SIZE, MAX_STREAMS, LOG, METRICS> {

    /// creates & returns a pair (`Uni`, `UniStream`)
    pub fn new(stream_executor: Arc<StreamExecutor<LOG, METRICS>>) -> Arc<Self> {
        Arc::new(Uni {
            uni_channel:              UniChannelType::<ItemType, BUFFER_SIZE, MAX_STREAMS>::new(),
            stream_executor,
            finished_executors_count: AtomicU32::new(0),
        })
    }

    #[inline(always)]
    pub fn try_send(&self, message: ItemType) -> bool {
        self.uni_channel.try_send(message)
    }

    pub fn consumer_stream(&self) -> Option<impl Stream<Item=ItemType>> {
        self.uni_channel.consumer_stream()
    }

    /// closes this Uni, in isolation -- flushing pending events, closing the producers,
    /// waiting for all events to be fully processed and calling the "on close" callback.\
    /// If this Uni share resources with another one (which will get dumped by the "on close"
    /// callback), most probably you want to close them atomically -- see [unis_close_async!()]
    pub async fn close(&self, timeout: Duration) -> bool{
        let start = Instant::now();
        if self.uni_channel.end_all_streams(timeout).await > 0 {
            false
        } else {
            loop {
                if self.finished_executors_count.load(Relaxed) > 0 {
                    break true;
                }
                // enforce timeout
                if timeout != Duration::ZERO && start.elapsed() > timeout {
                    break false
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    }

}


/// Macro to close, atomically, all [Uni]s passed as parameters
/// TODO the closer may receive a time limit to wait -- returning how many elements were still there after if gave up waiting
/// TODO busy wait ahead -- is it possible to get rid of that without having to poll & sleep?
///      (anyway, this function is not meant to be used in production -- unless when quitting the app... so this is not a priority)
#[macro_export]
macro_rules! unis_close_async {
    ($($producer_handle: tt),+) => {
        let timeout = Duration::ZERO;
        tokio::join!($($producer_handle.uni_channel.end_all_streams(timeout), )+);
        loop {
            let mut all_finished = true;
            $(
                if $producer_handle.finished_executors_count.load(Relaxed) == 0 {
                    all_finished = false;
                }
            )+
            if all_finished {
                break
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
}
pub use unis_close_async;
use crate::uni::channels;

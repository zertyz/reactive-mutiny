//! See [super]

use super::{
    super::{
        stream_executor::StreamExecutor,
        instruments::Instruments,
        mutiny_stream::MutinyStream,
    },
    channels::{self, UniChannelCommon, UniMovableChannel},
};
use std::{
    fmt::Debug,
    time::Duration,
    sync::{Arc, atomic::{AtomicU32, Ordering::Relaxed}},
};
use minstant::Instant;


/// this is the fastest [UniChannel] for general use, as revealed in performance tests
pub type UniChannelType<'a, ItemType,
                            const BUFFER_SIZE: usize,
                            const MAX_STREAMS: usize> = channels::movable::full_sync::FullSync<'a, ItemType, BUFFER_SIZE, MAX_STREAMS>;
pub type UniStreamType<'a, ItemType,
                           const BUFFER_SIZE: usize,
                           const MAX_STREAMS: usize> = MutinyStream<'a, ItemType, UniChannelType<'a, ItemType, BUFFER_SIZE, MAX_STREAMS>>;

/// Contains the producer-side [Uni] handle used to interact with the `uni` event
/// -- for closing the stream, requiring stats, ...
pub struct Uni<'a, ItemType:          Send + Sync + Debug,
                   const BUFFER_SIZE: usize,
                   const MAX_STREAMS: usize,
                   const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}> {
    pub uni_channel:              Arc<UniChannelType<'a, ItemType, BUFFER_SIZE, MAX_STREAMS>>,
    pub stream_executor:          Arc<StreamExecutor<INSTRUMENTS>>,
    pub finished_executors_count: AtomicU32,
}

impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize,
         const INSTRUMENTS: usize>
Uni<'a, ItemType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS> {

    /// creates & returns a pair (`Uni`, `UniStream`)
    pub fn new<IntoString: Into<String>>(uni_name: IntoString,
                                         stream_executor: Arc<StreamExecutor<INSTRUMENTS>>) -> Self {
        Uni {
            uni_channel:              UniChannelType::<'a, ItemType, BUFFER_SIZE, MAX_STREAMS>::new(uni_name),
            stream_executor,
            finished_executors_count: AtomicU32::new(0),
        }
    }

    #[inline(always)]
    pub fn zero_copy_try_send(&self, item_builder: impl Fn(&mut ItemType)) -> bool {
        self.uni_channel.zero_copy_try_send(item_builder)
    }

    #[inline(always)]
    pub fn try_send(&self, message: ItemType) -> bool {
        self.uni_channel.try_send(message)
    }

    pub fn consumer_stream(&self) -> UniStreamType<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {
        self.uni_channel.consumer_stream()
    }

    pub async fn flush(&self, duration: Duration) -> u32 {
        self.uni_channel.flush(duration).await
    }

    /// closes this Uni, in isolation -- flushing pending events, closing the producers,
    /// waiting for all events to be fully processed and calling the "on close" callback.\
    /// If this Uni share resources with another one (which will get dumped by the "on close"
    /// callback), most probably you want to close them atomically -- see [unis_close_async!()]
    pub async fn close(&self, timeout: Duration) -> bool{
        let start = Instant::now();
        if self.uni_channel.gracefully_end_all_streams(timeout).await > 0 {
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
        tokio::join!($($producer_handle.uni_channel.gracefully_end_all_streams(timeout), )+);
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

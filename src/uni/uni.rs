//! See [super]

use super::{
    super::{
        stream_executor::StreamExecutor,
        instruments::Instruments,
        mutiny_stream::MutinyStream,
        types::{ChannelCommon, ChannelProducer, ChannelConsumer, FullDuplexUniChannel},
    },
};
use std::{
    fmt::Debug,
    time::Duration,
    sync::{Arc, atomic::{AtomicU32, Ordering::Relaxed}},
};
use std::marker::PhantomData;
use minstant::Instant;


/// Contains the producer-side [Uni] handle used to interact with the `uni` event
/// -- for closing the stream, requiring stats, ...
pub struct Uni<'a, ItemType:          Send + Sync + Debug,
                   UniChannelType:    FullDuplexUniChannel<'a, ItemType, DerivedItemType>,
                   const INSTRUMENTS: usize,
                   DerivedItemType:   Debug = ItemType> {
    pub uni_channel:              Arc<UniChannelType>,
    pub stream_executor:          Arc<StreamExecutor<INSTRUMENTS>>,
    pub finished_executors_count: AtomicU32,
        _phantom:                 PhantomData<(&'a ItemType, &'a DerivedItemType)>,
}

impl<'a, ItemType:          Send + Sync + Debug + 'a,
         UniChannelType:    FullDuplexUniChannel<'a, ItemType, DerivedItemType>,
         const INSTRUMENTS: usize,
         DerivedItemType:   Debug>
Uni<'a, ItemType, UniChannelType, INSTRUMENTS, DerivedItemType> {

    /// creates & returns a pair (`Uni`, `UniStream`)
    pub fn new<IntoString: Into<String>>(uni_name: IntoString,
                                         stream_executor: Arc<StreamExecutor<INSTRUMENTS>>) -> Self {
        Uni {
            uni_channel:              UniChannelType::new(uni_name),
            stream_executor,
            finished_executors_count: AtomicU32::new(0),
            _phantom:                 PhantomData::default(),
        }
    }

    #[inline(always)]
    pub fn try_send<F: FnOnce(&mut ItemType)>(&self, setter: F) -> bool {
        self.uni_channel.try_send(setter)
    }

    #[inline(always)]
    pub fn send<F: FnOnce(&mut ItemType)>(&self, setter: F) {
        self.uni_channel.send(setter)
    }

    #[inline(always)]
    #[must_use]
    pub fn try_send_movable(&self, item: ItemType) -> bool {
        self.uni_channel.try_send_movable(item)
    }

    pub fn consumer_stream(&self) -> MutinyStream<'a, ItemType, UniChannelType, DerivedItemType> {
        let (stream, _stream_id) = self.uni_channel.create_stream();
        stream
    }

    pub async fn flush(&self, duration: Duration) -> u32 {
        self.uni_channel.flush(duration).await
    }

    /// closes this Uni, in isolation -- flushing pending events, closing the producers,
    /// waiting for all events to be fully processed and calling the "on close" callback.\
    /// If this Uni share resources with another one (which will get dumped by the "on close"
    /// callback), most probably you want to close them atomically -- see [unis_close_async!()]
    #[must_use]
    pub async fn close(&self, timeout: Duration) -> bool {
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

use crate::{
    types::MutinyStreamSource,
    ogre_std::{
        ogre_queues::{
            atomic_queues::atomic_meta::AtomicMeta,
            meta_publisher::MetaPublisher,
            meta_subscriber::MetaSubscriber,
            meta_container::MetaContainer,
        },
        ogre_sync,
    },
    uni::channels::{uni_stream::UniStream},
    Instruments,
};
use std::{
    time::Duration,
    sync::atomic::AtomicU32,
    pin::Pin,
    fmt::Debug,
    task::{Poll, Waker},
    sync::{Arc, atomic::Ordering::{Relaxed}},
    mem
};
use std::hint::spin_loop;
use std::marker::PhantomData;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ops::Deref;
use std::sync::atomic::AtomicBool;
use futures::{Stream};
use minstant::Instant;
use owning_ref::ArcRef;
use crate::streams_manager::StreamsManagerBase;
use crate::uni::channels::uni_stream;


/// A Uni channel, backed by an [AtomicMeta], that may be used to create as many streams as `MAX_STREAMS` -- which must only be dropped when it is time to drop this channel
pub struct Atomic<'a, ItemType:          Send + Sync + Debug,
                      const BUFFER_SIZE: usize,
                      const MAX_STREAMS: usize> {

    /// common code for dealing with streams
    streams_manager: StreamsManagerBase<'a, ItemType, MAX_STREAMS>,
    /// backing storage for events -- AKA, channels
    channel:       Pin<Box<AtomicMeta<ItemType, BUFFER_SIZE>>>,

}

impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    /// Returns as many consumer streams as requested, provided the specified limit [MAX_STREAMS] is respected
    /// -- events will be split for each generated stream, so no two streams  will see the same payload.\
    /// Be aware of the special semantics for dropping streams: no stream should stop consuming elements (don't drop it!) until
    /// your are ready to drop the whole channel.\
    /// DEVELOPMENT NOTE: OUT-OF-TRAIT [UniChannel] implementation. Once Rust allows trait functions to return `impls`, this should be moved there
    pub fn consumer_stream(self: &Arc<Self>) -> UniStream<'a, ItemType, Self> {
        let stream_id = self.streams_manager.create_stream_id();
        UniStream::new(stream_id, self)
    }

}

//#[async_trait]
/// implementation note: Rust 1.63 does not yet support async traits. See [super::UniChannel]
impl<'a, ItemType:          'a + Send + Sync + Debug,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
/*UniChannel<ItemType>
for*/ Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    /// Instantiates
    pub fn new<IntoString: Into<String>>(streams_manager_name: IntoString) -> Arc<Self> {
        Arc::new(Self {
            streams_manager: StreamsManagerBase::new(streams_manager_name),
            channel:       Box::pin(AtomicMeta::<ItemType, BUFFER_SIZE>::new()),
        })
    }

    #[inline(always)]
    pub unsafe fn zero_copy_try_send(&self, item_builder: impl Fn(&mut ItemType)) -> bool {
        self.channel.publish(item_builder,
                             || {
                                   self.streams_manager.wake_all_streams();
                                   false
                               },
                             |len| if len <= 1 {
                                                                        self.streams_manager.wake_stream(0)
                                                                    } else if len <= MAX_STREAMS as u32 {
                                                                        self.streams_manager.wake_stream(len-1)
                                                                    })
    }

    #[inline(always)]
    pub fn try_send(&self, item: ItemType) -> bool {
        let item = ManuallyDrop::new(item);       // ensure it won't be dropped when this function ends, since it will be "moved"
        unsafe {
            self.zero_copy_try_send(|slot| {
                // move `item` to `slot`
                std::ptr::copy_nonoverlapping(item.deref() as *const ItemType, (slot as *const ItemType) as *mut ItemType, 1);
            })
        }
    }

    pub async fn flush(&self, timeout: Duration) -> u32 {
        self.streams_manager.flush(timeout, || self.pending_items_count()).await
    }

    pub async fn end_all_streams(&self, timeout: Duration) -> u32 {
        self.streams_manager.end_all_streams(timeout, || self.pending_items_count()).await
    }

    pub fn cancel_all_streams(&self) {
        self.streams_manager.cancel_all_streams();
    }

    pub fn running_streams_count(&self) -> u32 {
        self.streams_manager.running_streams_count()
    }

    #[inline(always)]
    pub fn pending_items_count(&self) -> u32 {
        self.channel.available_elements_count() as u32
    }

}

impl<'a, ItemType:          'a + Send + Sync + Debug,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
MutinyStreamSource<'a, ItemType>
for Atomic<'a, ItemType, BUFFER_SIZE, MAX_STREAMS> {

    #[inline(always)]
    fn provide(&self, _stream_id: u32) -> Option<ItemType> {
        self.channel.consume(|item| {
                                 let mut moved_value = MaybeUninit::<ItemType>::uninit();
                                 unsafe { std::ptr::copy_nonoverlapping(item as *const ItemType, moved_value.as_mut_ptr(), 1) }
                                 unsafe { moved_value.assume_init() }
                             },
                             || false,
                             |_| {})
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

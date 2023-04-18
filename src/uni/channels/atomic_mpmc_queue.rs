use crate::{
    ogre_std::ogre_queues::{
        atomic_queues::atomic_meta::AtomicMeta,
        meta_publisher::MetaPublisher,
        meta_subscriber::MetaSubscriber,
        meta_queue::MetaQueue,
    },
    uni::channels::{StreamsManager, uni_stream::UniStream},
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
use std::sync::atomic::AtomicBool;
use futures::{Stream};
use minstant::Instant;
use owning_ref::ArcRef;


/// A channel backed by an [AtomicMeta], that may be used to create as many streams as `MAX_STREAMS` -- which must only be dropped when it is time to drop this channel
#[repr(C,align(64))]      // aligned to cache line sizes for a careful control over false-sharing performance degradation
pub struct AtomicMPMCQueue<ItemType:          Send + Sync + Debug,
                           const BUFFER_SIZE: usize,
                           const MAX_STREAMS: usize> {
    /// General atomic & non-blocking queue allowing to report back on the number of retained elements after enqueueing,
    /// so to work optimally with our round-robin stream-awaking algorithm
    queue:                  AtomicMeta<ItemType, BUFFER_SIZE>,
    wakers:                 [Option<Waker>; MAX_STREAMS],
    created_streams_count:  AtomicU32,
    finished_streams_count: AtomicU32,
    keep_streams_running:   bool,
    /// coordinates writes to the wakers
    wakers_lock:            AtomicBool,

}


impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
StreamsManager<'a, ItemType, AtomicMeta<ItemType, BUFFER_SIZE>> for
AtomicMPMCQueue<ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn backing_subscriber(self: &Arc<Self>) -> ArcRef<Self, AtomicMeta<ItemType, BUFFER_SIZE>> {
        ArcRef::from(self.clone())
            .map(|this| &this.queue)
    }

    #[inline(always)]
    fn keep_streams_running(&self) -> bool {
        self.keep_streams_running
    }

    #[inline(always)]
    fn register_stream_waker(&self, stream_id: u32, waker: &Waker) {
        // share the waker the first time this runs, so producers may wake this task up when an item is ready
        if let None = &self.wakers[stream_id as usize] {
            lock(&self.wakers_lock);
            if let None = &self.wakers[stream_id as usize] {
                let waker = waker.clone();
                let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
                let _ = mutable_self.wakers[stream_id as usize].insert(waker);
            }
            unlock(&self.wakers_lock);
        }
    }

    fn report_stream_dropped(&self, stream_id: u32) {
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        lock(&self.wakers_lock);
        mutable_self.keep_streams_running = false;
        mutable_self.wakers[stream_id as usize] = None;
        unlock(&self.wakers_lock);
        self.finished_streams_count.fetch_add(1, Relaxed);
    }
}


impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
AtomicMPMCQueue<ItemType, BUFFER_SIZE, MAX_STREAMS> {

    /// Returns as many consumer streams as requested, provided the specified limit [MAX_STREAMS] is respected
    /// -- events will be split for each generated stream, so no two streams  will see the same payload.\
    /// Be aware of the special semantics for dropping streams: no stream should stop consuming elements (don't drop it!) until
    /// your are ready to drop the whole channel.\
    /// DEVELOPMENT NOTE: OUT-OF-TRAIT [UniChannel] implementation. Once Rust allows trait functions to return `impls`, this should be moved there
    pub fn consumer_stream(self: &Arc<Self>) -> Option<UniStream<'a, ItemType, Self, AtomicMeta<ItemType, BUFFER_SIZE>>> {
        let stream_id = self.created_streams_count.fetch_add(1, Relaxed);
        if stream_id >= MAX_STREAMS as u32 {
            //return None;
            panic!("AtomicMPMCQueue: This Uni channel has a MAX_STREAMS of {MAX_STREAMS} -- which just got exhausted: program called `consumer_stream()` {stream_id} times! Please, increase the limit or fix any LOGIC BUGs!");
        }
        Some(UniStream::new(stream_id, self))
    }

    // TODO 20220924: this method imposes a Copy -- therefore we should refactor our queues into its constituint operations so we may do whatever is
    //                necessary to acquire a slot, then move the value just once, then proceed to the rest of the operation...
    //                or rename the methods to try_enqueue() (returning bool) and enqueue() (waiting for a free slot)
    /// development note: OUT-OF-TRAIT [UniChannel] implementation. Once Rust allows zero-cost async traits, this should be moved there
    // #[inline(always)]
    // pub async fn send(&self, item: ItemType) {
    //     loop {
    //         if self.try_send(item) {
    //             break;
    //         } else {
    //             self.wake_all_streams();
    //             tokio::task::yield_now().await;
    //         }
    //     }
    // }

    #[inline(always)]
    fn wake_all_streams(&self) {
        for stream_id in 0..self.created_streams_count.load(Relaxed) {
            self.wake_stream(stream_id);
        }
    }

    #[inline(always)]
    fn wake_stream(&self, stream_id: u32) {
        if stream_id < MAX_STREAMS as u32 {
            match &self.wakers[stream_id as usize] {
                Some(waker) => waker.wake_by_ref(),
                None => {
                    // here we assume streams will finish when we're about to close
                    // (keep streams running is false)... so we won't take any remedy for now
                }
            }
        }
    }

}

//#[async_trait]
/// implementation note: Rust 1.63 does not yet support async traits. See [super::UniChannel]
impl<'a, ItemType:          'a + Copy + Send + Sync + Debug,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
/*UniChannel<ItemType>
for*/ AtomicMPMCQueue<ItemType, BUFFER_SIZE, MAX_STREAMS> {

    pub fn new() -> Arc<Self> {
        let queue = AtomicMeta::<ItemType, BUFFER_SIZE>::new();
        Arc::new(Self {
            queue,
            wakers:                 (0..MAX_STREAMS).map(|_| Option::<Waker>::None).collect::<Vec<_>>().try_into().unwrap(),
            created_streams_count: AtomicU32::new(0),
            finished_streams_count: AtomicU32::new(0),
            keep_streams_running:   true,
            wakers_lock:            AtomicBool::new(false),
        })
    }

    #[inline(always)]
    pub unsafe fn zero_copy_try_send(&self, item_builder: impl Fn(&mut ItemType)) -> bool {
        self.queue.publish(item_builder,
                           || false,
                           |len| self.wake_stream(len-1))
    }

    #[inline(always)]
    pub fn try_send(&self, item: ItemType) -> bool {
        unsafe {
            self.zero_copy_try_send(|slot| *slot = item)
        }
    }

    pub async fn flush(&self, timeout: Duration) -> u32 {
        let mut start: Option<Instant> = None;
        loop {
            let pending_count = self.pending_items_count();
            if pending_count > 0 {
                self.wake_all_streams();
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
        self.flush(timeout).await;
        {
            let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
            mutable_self.keep_streams_running = false;
        }
        self.flush(timeout).await;
        self.wake_all_streams();
        let running_streams_count = self.created_streams_count.load(Relaxed) - self.finished_streams_count.load(Relaxed);
        running_streams_count
    }

    #[inline(always)]
    pub fn pending_items_count(&self) -> u32 {
        self.queue.available_elements() as u32
    }

}

#[inline(always)]
fn lock(flag: &AtomicBool) {
    while flag.compare_exchange_weak(false, true, Relaxed, Relaxed).is_err() {
        spin_loop();
    }
}

#[inline(always)]
fn unlock(flag: &AtomicBool) {
    flag.store(false, Relaxed);
}

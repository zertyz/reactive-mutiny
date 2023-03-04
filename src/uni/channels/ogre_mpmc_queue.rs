use crate::ogre_std::ogre_queues::{
    full_sync_queues::full_sync_base::FullSyncBase,
};
use std::{
    time::Duration,
    sync::atomic::AtomicU32,
    pin::Pin,
    fmt::Debug,
    task::{Poll, Waker},
    sync::Arc,
    mem
};
use std::hint::spin_loop;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ops::Deref;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use futures::{Stream};
use minstant::Instant;
use crate::ogre_std::ogre_queues::meta_queue::MetaQueue;


#[repr(C,align(64))]      // aligned to cache line sizes for a careful control over false-sharing performance degradation
pub struct OgreMPMCQueue<ItemType:           Unpin + Send + Sync + Debug,
                          const BUFFER_SIZE: usize,
                          const MAX_STREAMS: usize> {
    /// General non-blocking full sync queue allowing to report back on the number of retained elements after enqueueing,
    /// so to work optimally with our round-robin stream-awaking algorithm
    queue:                  FullSyncBase<ItemType, BUFFER_SIZE>,
    wakers:                 [Option<Waker>; MAX_STREAMS],
    created_streams_count:  AtomicU32,
    finished_streams_count: AtomicU32,
    keep_streams_running:   bool,
    /// coordinates writes to the wakers
    wakers_lock:            AtomicBool,
}

impl<ItemType:          Unpin + Send + Sync + Debug + 'static,
     const BUFFER_SIZE: usize,
     const MAX_STREAMS: usize>
OgreMPMCQueue<ItemType, BUFFER_SIZE, MAX_STREAMS> {

    /// Returns as many consumer streams as requested, provided the specified limit [MAX_STREAMS] is respected.
    /// -- events will be split for each generated stream, so no two streams  will see the same payload.\
    /// Be aware of the special semantics for dropping streams: no stream should stop consuming elements (don't drop it!) until
    /// your are ready to drop the whole channel.\
    /// This is because of the algorithm to wake Streams based on the number of queue.
    /// So, if a stream is created and dropped, the `send` functions will still try to awake it -- and not any other stream.\
    /// This is specially bad if the first stream is dropped:
    ///   1) the channel gets, eventually empty -- all streams will sleep
    ///   2) An item is sent -- the logic will wake the first stream: since the stream is no longer there, that element won't be consumed!
    ///   3) Eventually, a second item is sent: now the queue has length 2 and the send logic will wake consumer 2
    ///   4) Consumer #2, since it was not dropped, will be awaken and will run until the channel is empty again.
    /// development note: OUT-OF-TRAIT [UniChannel] implementation. Once Rust allows trait functions to return `impls`, this should be moved there
    pub fn consumer_stream(self: &Arc<Pin<Box<Self>>>) -> Option<impl Stream<Item=ItemType>> {
        // safety: Unwraps the Arc to return the required Pin<Self> to Stream::next(),
        //         which is guaranteed either:
        //           - not to mutate Self (the backing queue don't require it)
        //           - or to do it in a safe manner (using AtomicPtrs to Wakers)
        let stream_id = self.created_streams_count.fetch_add(1, Relaxed);
        if stream_id >= MAX_STREAMS as u32 {
            //return None;
            panic!("OgreMPMCQueue: This uni channel has a MAX_STREAMS of {MAX_STREAMS} -- which just got exhausted: program called `consumer_stream()` {stream_id} times! Please, increase the limit or fix any LOGIC BUGs!");
        }
        let cloned_self = self.clone();
        let pin_ptr = Arc::as_ptr(&cloned_self);
        let pinned = unsafe { core::ptr::read(pin_ptr) };
        let boxed = unsafe { Pin::into_inner_unchecked(pinned) };
        let original_self = Box::into_raw(boxed);
        let mutable_self_for_drop = unsafe {&mut *((original_self as *const Self) as *mut Self)};
        let mutable_self = unsafe {&mut *((original_self as *const Self) as *mut Self)};
        mem::forget(original_self); // this is managed by the Arc
        Some(super::super::super::stream::custom_drop_poll_fn::custom_drop_poll_fn(
            move || {
                lock(&mutable_self_for_drop.wakers_lock);
                mutable_self_for_drop.wakers[stream_id as usize] = None;
                unlock(&mutable_self_for_drop.wakers_lock);
                mutable_self_for_drop.finished_streams_count.fetch_add(1, Relaxed);
                drop(cloned_self);     // forces the Arc to be moved & dropped here instead of on the `consumer_stream()`, so `mutable_self` is guaranteed to be valid
            },
            move |cx| {
                let element = mutable_self.queue.dequeue(|item| {
                                                                        let mut moved_value = MaybeUninit::<ItemType>::uninit();
                                                                        unsafe { std::ptr::copy_nonoverlapping(item as *const ItemType, moved_value.as_mut_ptr(), 1) }
                                                                        unsafe { moved_value.assume_init() }
                                                                    },
                                                                    || false,
                                                                    |_| {});
                let next = match element {
                    Some(item) => Poll::Ready(Some(item)),
                    None => {
                        if mutable_self.keep_streams_running {
                            // share the waker the first time this runs, so producers may wake this task up when an item is ready
                            if let None = &mutable_self.wakers[stream_id as usize] {
                                // try again, syncing
                                lock(&mutable_self.wakers_lock);
                                if let None = &mutable_self.wakers[stream_id as usize] {
                                    let waker = cx.waker().clone();
                                    let _ = mutable_self.wakers[stream_id as usize].insert(waker);
                                }
                                unlock(&mutable_self.wakers_lock);
                            }
                            Poll::Pending
                        } else {
                            Poll::Ready(None)
                        }
                    },
                };
                next
            }
        ))
    }

    // TODO 20220924: this method imposes a Copy -- therefore we should refactor our queues into its constituint operations so we may do whatever is
    //                necessary to acquire a slot, then move the value just once, then proceed to the rest of the operation...
    //                or rename the methods to try_enqueue() (returning bool) and enqueue() (waiting for a free slot)
    /// development note: OUT-OF-TRAIT [UniChannel] implementation. Once Rust allows zero-cost async traits, this should be moved there
    // #[inline(always)]
    // pub async fn send(&self, item: ItemType) {
    //     let closure = |slot: &mut ItemType| *slot = item;
    //     loop {
    //         if unsafe { self.zero_copy_try_send(closure) } {
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
                    // try again, syncing
                    lock(&self.wakers_lock);
                    if let Some(waker) = &self.wakers[stream_id as usize] {
                        waker.wake_by_ref();
                    }
                    unlock(&self.wakers_lock);
                }
            }
        }
    }

}

//#[async_trait]
/// implementation note: Rust 1.63 does not yet support async traits. See [super::UniChannel]
impl<ItemType:          Unpin + Send + Sync + Debug + 'static,
     const BUFFER_SIZE: usize,
     const MAX_STREAMS: usize>
/*UniChannel<ItemType>
for*/ OgreMPMCQueue<ItemType, BUFFER_SIZE, MAX_STREAMS> {

    pub fn new() -> Arc<Pin<Box<Self>>> {
        let queue = FullSyncBase::<ItemType, BUFFER_SIZE>::new();
        Arc::new(Box::pin(Self {
            queue,
            wakers:                 (0..MAX_STREAMS).map(|_| Option::<Waker>::None).collect::<Vec<_>>().try_into().unwrap(),
            created_streams_count:  AtomicU32::new(0),
            finished_streams_count: AtomicU32::new(0),
            keep_streams_running:   true,
            wakers_lock:            AtomicBool::new(false),
        }))
    }

    #[inline(always)]
    pub unsafe fn zero_copy_try_send(&self, item_builder: impl Fn(&mut ItemType)) -> bool {
        self.queue.enqueue(item_builder,
                           || false,
                           |len| self.wake_stream(len-1))
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
            if timeout != Duration::ZERO {
                if let Some(start) = start {
                    if start.elapsed() > timeout {
                        break pending_count
                    }
                } else {
                    start = Some(Instant::now());
                }
            }
        }
    }

    pub async fn end_all_streams(&self, timeout: Duration) -> u32 {
        let start = Instant::now();
        self.flush(timeout).await;
        {
            let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
            mutable_self.keep_streams_running = false;
        }
        self.flush(timeout).await;
        self.wake_all_streams();
        loop {
            let running_streams = self.running_streams_count();
            tokio::time::sleep(Duration::from_millis(1)).await;
            if timeout != Duration::ZERO && start.elapsed() > timeout {
                break running_streams
            }
            if running_streams == 0 {
                break running_streams
            }
        }
    }

    pub fn running_streams_count(&self) -> u32 {
        self.created_streams_count.load(Relaxed) - self.finished_streams_count.load(Relaxed)
    }

    pub fn pending_items_count(&self) -> u32 {
        self.queue.len() as u32
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


/// Tests & enforces the requisites & expose good practices & exercises the API of of the [uni](self) module
#[cfg(any(test, feature = "dox"))]
mod tests {
    use super::*;


}

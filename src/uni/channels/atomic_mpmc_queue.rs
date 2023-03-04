use crate::ogre_std::ogre_queues::{
    meta_queue::MetaQueue,
    atomic_queues::atomic_meta::AtomicMeta,
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
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use futures::{Stream};
use minstant::Instant;


#[repr(C,align(64))]      // aligned to cache line sizes for a careful control over false-sharing performance degradation
pub struct AtomicMPMCQueue<ItemType:          Clone + Send + Sync + Debug,
                           const BUFFER_SIZE: usize,
                           const MAX_STREAMS: usize> {
    /// General atomic & non-blocking queue allowing to report back on the number of retained elements after enqueueing,
    /// so to work optimally with our round-robin stream-awaking algorithm
    queue:                  AtomicMeta<ItemType, BUFFER_SIZE>,
    created_streams_count:  AtomicU32,
    wakers:                 [Option<Waker>; MAX_STREAMS],
    keep_streams_running:   bool,
    finished_streams_count: AtomicU32,
}

impl<ItemType:          Clone + Send + Sync + Debug + 'static,
     const BUFFER_SIZE: usize,
     const MAX_STREAMS: usize>
AtomicMPMCQueue<ItemType, BUFFER_SIZE, MAX_STREAMS> {

    /// Returns as many consumer streams as requested, provided the specified limit [MAX_STREAMS] is respected
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
        let stream_number = self.created_streams_count.fetch_add(1, Relaxed);
        if stream_number >= MAX_STREAMS as u32 {
            //return None;
            panic!("AtomicMPMCQueue: This uni channel has a MAX_STREAMS of {MAX_STREAMS} -- which just got exhausted: program called `consumer_stream()` {stream_number} times! Please, increase the limit or fix any LOGIC BUGs!");
        }
        let cloned_self = self.clone();
        let pin_ptr = Arc::as_ptr(&cloned_self);
        let pinned = unsafe { core::ptr::read(pin_ptr) };
        let boxed = unsafe { Pin::into_inner_unchecked(pinned) };
        let original_self = Box::into_raw(boxed);
        let mutable_self_for_drop = unsafe {&mut *((original_self as *const Self) as *mut Self)};
        let mutable_self = unsafe {&mut *((original_self as *const Self) as *mut Self)};
        mem::forget(original_self); // this is managed by the Arc
        const EMPTY_RETRIES: u32 = 8192;
        let mut empty_retry_count = 0;
        Some(super::super::super::stream::custom_drop_poll_fn::custom_drop_poll_fn(
            move || {
                mutable_self_for_drop.wakers[stream_number as usize] = None;
                mutable_self_for_drop.finished_streams_count.fetch_add(1, Relaxed);
                drop(cloned_self);     // forces the Arc to be moved & dropped here instead of on the `consumer_stream()`, so `mutable_self` is guaranteed to be valid
            },
            move |cx| {
                let item = mutable_self.queue.dequeue(|item| item.clone(), || false, |_| {});
                let next = match item {
                    Some(item) => {
                        empty_retry_count = 0;
                        Poll::Ready(Some(item))
                    },
                    None => {
                        if mutable_self.keep_streams_running {
                            // share the waker the first time this runs, so producers may wake this task up when an item is ready
                            let _ = mutable_self.wakers[stream_number as usize].insert(cx.waker().clone());
                            std::sync::atomic::fence(Release);
                            empty_retry_count += 1;
                            if empty_retry_count < EMPTY_RETRIES {
                                std::hint::spin_loop();
                                cx.waker().wake_by_ref();
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
            std::sync::atomic::fence(Acquire);
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
impl<ItemType:          Copy+Send+Sync+Debug + 'static,
     const BUFFER_SIZE: usize,
     const MAX_STREAMS: usize>
/*UniChannel<ItemType>
for*/ AtomicMPMCQueue<ItemType, BUFFER_SIZE, MAX_STREAMS> {

    pub fn new() -> Arc<Pin<Box<Self>>> {
        let queue = AtomicMeta::<ItemType, BUFFER_SIZE>::new();
        Arc::new(Box::pin(Self {
            queue,
            created_streams_count: AtomicU32::new(0),
            wakers:                 (0..MAX_STREAMS).map(|_| Option::<Waker>::None).collect::<Vec<_>>().try_into().unwrap(),
            keep_streams_running:   true,
            finished_streams_count: AtomicU32::new(0)
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

    pub fn pending_items_count(&self) -> u32 {
        self.queue.len() as u32
    }

}


/// Tests & enforces the requisites & expose good practices & exercises the API of of the [uni](self) module
#[cfg(any(test, feature = "dox"))]
mod tests {
    use super::*;


}

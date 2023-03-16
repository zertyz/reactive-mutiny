use crate::{
    ogre_std::{
        ogre_queues::{
            meta_queue::MetaQueue,
            full_sync_queues::NonBlockingQueue,
        },
        reference_counted_buffer_allocator::{ReferenceCountedBufferAllocator, ReferenceCountedNonBlockingCustomQueueAllocator},
    },
    ogre_std::ogre_queues::{
        OgreQueue,
        full_sync_queues::full_sync_meta::FullSyncMeta,
    },
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
use std::marker::PhantomData;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ops::Deref;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::{Relaxed};
use futures::{Stream};
use minstant::Instant;


pub type OgreMPMCQueue<ItemType,
                       const BUFFER_SIZE: usize = 1024,
                       const MAX_STREAMS: usize = 16>
        = InternalOgreMPMCQueue<ItemType,
                                AllocatorType<ItemType, BUFFER_SIZE>,
                                BUFFER_SIZE,
                                MAX_STREAMS>;


type AllocatorType<ItemType, const BUFFER_SIZE: usize> = ReferenceCountedNonBlockingCustomQueueAllocator<ItemType, NonBlockingQueue<u32, BUFFER_SIZE>, BUFFER_SIZE>;

#[repr(C,align(64))]      // aligned to cache line sizes for a careful control over false-sharing performance degradation
pub struct InternalOgreMPMCQueue<ItemType:          Clone + Unpin + Send + Sync + Debug,
                                 AllocatorType:     ReferenceCountedBufferAllocator<ItemType, u32, u32>,
                                 const BUFFER_SIZE: usize,
                                 const MAX_STREAMS: usize> {
    pub (crate) buffer:     AllocatorType,
    /// General non-blocking full sync queues -- one for each Multi
    queues:                 [FullSyncMeta<u32, BUFFER_SIZE>; MAX_STREAMS],
    /// simple metrics
    created_streams_count:  AtomicU32,
    /// simple metrics
    finished_streams_count: AtomicU32,
    /// used to coordinate syncing between `vacant_streams` and `used_streams`
    streams_lock:           AtomicBool,
    /// the id # of streams that can be created are stored here.\
    /// synced with `used_streams`, so creating & dropping streams occurs as fast as possible, as well as enqueueing elements
    vacant_streams:         Pin<Box<NonBlockingQueue<u32, MAX_STREAMS>>>,
    /// this is synced with `vacant_streams` to be its counter-part -- so enqueueing is optimized:
    /// it holds the stream ids that are active, `u32::MAX` being used to indicate a premature end of the list
    used_streams:           [u32; MAX_STREAMS],
    /// coordinates writes to the wakers
    wakers_lock:            AtomicBool,
    /// holds wakers for each stream id #
    wakers:                 [Option<Waker>; MAX_STREAMS],
    /// signals for Streams to end or to continue waiting for elements
    keep_stream_running:    [bool; MAX_STREAMS],
    // phantoms
    _item_type:             PhantomData<ItemType>,
}

impl<ItemType:          Clone + Unpin + Send + Sync + Debug + 'static,
     const BUFFER_SIZE: usize,
     const MAX_STREAMS: usize>
InternalOgreMPMCQueue<ItemType, AllocatorType<ItemType, BUFFER_SIZE>, BUFFER_SIZE, MAX_STREAMS> {

    /// Returns a `Stream` -- may create as many streams as requested, provided the specified limit [MAX_STREAMS] is respected.\
    /// Each stream will see all payloads sent through this channel.
    pub fn listener_stream(self: &Arc<Pin<Box<Self>>>) -> (impl Stream<Item=ItemType>, u32) {
        // safety: Unwraps the Arc to return the required Pin<Self> to Stream::next(),
        //         which is guaranteed not to mutate Self (our queue don't require it) or to do it
        //         in a safe manner using AtomicPtrs to Wakers
        self.created_streams_count.fetch_add(1, Relaxed);
        let stream_id = match self.vacant_streams.dequeue() {
            Some(stream_id) => stream_id,
            None => panic!("OgreMPMCQueue: This multi channel has a MAX_STREAMS of {MAX_STREAMS} -- which just got exhausted: program called `listener_stream()` {} times & {} of these where dropped. Please, increase the limit or fix the LOGIC BUG!",
                           self.created_streams_count.load(Relaxed), self.finished_streams_count.load(Relaxed)),
        };
        self.sync_vacant_and_used_streams();
        let cloned_self = self.clone();
        let pin_ptr = Arc::as_ptr(&cloned_self);
        let pinned = unsafe { core::ptr::read(pin_ptr) };
        let boxed = unsafe { Pin::into_inner_unchecked(pinned) };
        let original_self = Box::into_raw(boxed);
        let mutable_self_for_drop = unsafe {&mut *((original_self as *const Self) as *mut Self)};
        let mutable_self = unsafe {&mut *((original_self as *const Self) as *mut Self)};
        mutable_self.keep_stream_running[stream_id as usize] = true;
        mem::forget(original_self); // this is managed by the Arc
        (
            super::super::super::stream::custom_drop_poll_fn::custom_drop_poll_fn(
                move || {
                    lock(&mutable_self_for_drop.wakers_lock);
                    mutable_self_for_drop.wakers[stream_id as usize] = None;
                    unlock(&mutable_self_for_drop.wakers_lock);
                    mutable_self_for_drop.finished_streams_count.fetch_add(1, Relaxed);
                    mutable_self_for_drop.vacant_streams.enqueue(stream_id);
                    mutable_self_for_drop.sync_vacant_and_used_streams();
                    drop(cloned_self);     // forces the Arc to be moved & dropped here instead of on the upper method `listener_stream()`, so `mutable_self` is guaranteed to be valid while the stream executes
                },
                move |cx| {
                    let slot_id = mutable_self.queues[stream_id as usize].dequeue(|item| *item,
                                                                                             || false,
                                                                                             |_| {});

                    let next = match slot_id {
                        Some(slot_id) => {
                            // TODO optimization: Stream<Item=&...> should be returned by this function instead, to allow the optimization of avoiding
                            //      copying the value over and over again. For this to work, the stream executor should accept a `post_item()` closure,
                            //      in which I'd `.ref_dec()` the reference
                            // TODO 20221010: the above attempt wasn't fully tryed. An alternative, using Arc, has shitty performance. New solution:
                            //                1) A new type will be created: `MultiPayload<ItemType>`, which will have a custom `Drop()`
                            //                2) That drop will simply `.ref_dec()` from the slot, making it free
                            //                3) It will implement `Deref` -- giving a reference to the buffer, as well as an unsafe `copy_unchecked()` method -- retuning a droppable copy of the underlying type
                            //                4) Even if `copy_unchecked()` was called, the custom drop must still call a manual `std::mem::drop()` on the last reference
                            //                5) Test that with an Arc and its internal reference counts
                            // TODO 20221010: the above attempt is shitty -- the payload is increased a lot (see MultyPayload, which should be deleted). Lets get back to adding a `post_item()` closure
                            //let item = MultiPayload::new(slot_id, Arc::clone(self));
                            if std::mem::needs_drop::<ItemType>() {
                                let slot = mutable_self.buffer.slot_from_slot_id(slot_id);
                                let opaque_slot = match mutable_self.buffer.ref_dec_callback(slot, |slot| {
                                    // move the last element out of the allocator, so it may be dropped (if necessary)
                                    let mut moved = MaybeUninit::<ItemType>::uninit();
                                    unsafe { std::ptr::copy_nonoverlapping(slot as *const ItemType, moved.as_mut_ptr(), 1) };
                                    let moved = unsafe { moved.assume_init() };
                                    Some(moved)
                                }) {
                                    None                 => slot.clone(),     // other streams will still take this element... clone it
                                    Some(slot) => slot,             // this is the last stream -- return the original item
                                };
                                Poll::Ready(Some(opaque_slot))
                            } else {
                                let slot = mutable_self.buffer.slot_from_slot_id(slot_id);
                                let cloned = slot.clone();
                                mutable_self.buffer.ref_dec(slot);
                                Poll::Ready(Some(cloned))
                            }
                        },
                        None => {
                            if mutable_self.keep_stream_running[stream_id as usize] {
                                // share the waker the first time this runs, so producers may wake this task up (when an item is ready)
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
            ),
            stream_id
        )
    }

    #[inline(always)]
    fn wake_all_streams(&self) {
        for stream_id in self.used_streams.iter() {
            if *stream_id == u32::MAX {
                break
            }
            self.wake_stream(*stream_id);
        }
    }

    #[inline(always)]
    fn wake_stream(&self, stream_id: u32) {
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

    /// rebuilds the `used_streams` list based of the current `vacant_items`
    fn sync_vacant_and_used_streams(&self) {
        lock(&self.streams_lock);
        let mut vacant = unsafe { self.vacant_streams.peek_all().concat() };
        vacant.sort_unstable();
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        let mut vacant_iter = vacant.iter();
        let mut i = 0;
        let mut last_used_stream_id = -1;
        while i < MAX_STREAMS as u32 {
            match vacant_iter.next() {
                Some(next_vacant_stream_id) => {
                    for used_stream_id in i .. *next_vacant_stream_id {
                        last_used_stream_id += 1;
                        mutable_self.used_streams[last_used_stream_id as usize] = used_stream_id;
                        i += 1;
                    }
                    i += 1;
                }
                None => {
                    last_used_stream_id += 1;
                    mutable_self.used_streams[last_used_stream_id as usize] = i;
                    i += 1;
                }
            }
        }
        for i in (last_used_stream_id + 1) as usize .. MAX_STREAMS {
            mutable_self.used_streams[i] = u32::MAX;
        }
        unlock(&self.streams_lock);
    }

}

//#[async_trait]
/// implementation note: Rust 1.63 does not yet support async traits. See [super::MultiChannel]
impl<ItemType:          Clone + Unpin + Send + Sync + Debug + 'static,
     const BUFFER_SIZE: usize,
     const MAX_STREAMS: usize>
/*MultiChannel<ItemType>
for*/ InternalOgreMPMCQueue<ItemType, AllocatorType<ItemType, BUFFER_SIZE>, BUFFER_SIZE, MAX_STREAMS> {

    pub fn new() -> Arc<Pin<Box<Self>>> {
        let instance = Arc::new(Box::pin(Self {
            buffer:                 AllocatorType::<ItemType, BUFFER_SIZE>::new(),
            queues:                 {
                                         let mut queues: [FullSyncMeta<u32, BUFFER_SIZE>; MAX_STREAMS] = unsafe { MaybeUninit::zeroed().assume_init() };
                                         for i in 0..queues.len() {
                                             queues[i] = FullSyncMeta::<u32, BUFFER_SIZE>::new();
                                         }
                                         queues
                                     },
            created_streams_count:  AtomicU32::new(0),
            vacant_streams:         {
                                        let vacant_streams = NonBlockingQueue::<u32, MAX_STREAMS>::new("vacant streams for an OgreMPMCQueue Multi channel");
                                        for stream_id in 0..MAX_STREAMS as u32 {
                                            vacant_streams.enqueue(stream_id);
                                        }
                                        vacant_streams
                                    },
            used_streams:           [u32::MAX; MAX_STREAMS],
            wakers:                 (0..MAX_STREAMS).map(|_| Option::<Waker>::None).collect::<Vec<_>>().try_into().unwrap(),
            finished_streams_count: AtomicU32::new(0),
            keep_stream_running:    [false; MAX_STREAMS],
            streams_lock:           AtomicBool::new(false),
            wakers_lock:            AtomicBool::new(false),
            _item_type:             PhantomData::default(),
        }));
        instance
    }

    #[inline(always)]
    pub async unsafe fn zero_copy_send(&self, item_builder: impl Fn(&mut ItemType)) {
        // time to sleep between retry attempts, if the channel is full
        const FULL_WAIT_TIME: Duration = Duration::from_millis(1);
        // loop until there is an allocation
        let slot = loop {
            let used_streams_count = (MAX_STREAMS - self.vacant_streams.len()) as u32;
            match self.buffer.allocate(used_streams_count) {
                Some(slot) => break slot,
                None                     => tokio::time::sleep(FULL_WAIT_TIME).await,
            }
        };
        item_builder(slot);
        let slot_id = self.buffer.slot_id_from_slot(slot);
        for stream_id in self.used_streams.iter() {
            if *stream_id == u32::MAX {
                break
            }
            self.queues[*stream_id as usize].enqueue(|e| *e = slot_id,
                                                     || panic!("Multi Channel's OgreMPMCQueue BUG! Stream (#{stream_id}) queue is full, but it should never be ahead of the Allocator, which was not full"),
                                                     |len| if len <= 1 { self.wake_stream(*stream_id) });
        }
    }

    #[inline(always)]
    pub async fn send(&self, item: ItemType) {
        let item = ManuallyDrop::new(item);       // ensure it won't be dropped when this function ends, since it will be "moved"
        unsafe {
            self.zero_copy_send(|slot| {
                // move `item` to `slot`
                std::ptr::copy_nonoverlapping(item.deref() as *const ItemType, (slot as *const ItemType) as *mut ItemType, 1);
            }).await
        }
    }

    #[inline(always)]
    pub unsafe fn zero_copy_try_send(&self, item_builder: impl Fn(&mut ItemType)) -> bool {
        let used_streams_count = (MAX_STREAMS - self.vacant_streams.len()) as u32;
        match self.buffer.allocate(used_streams_count) {
            Some(slot) => {
                item_builder(slot);
                let slot_id = self.buffer.slot_id_from_slot(slot);
                for stream_id in self.used_streams.iter() {
                    if *stream_id == u32::MAX {
                        break
                    }
                    self.queues[*stream_id as usize].enqueue(|e| *e = slot_id,
                                                             || panic!("Multi Channel's OgreMPMCQueue BUG! Stream (#{stream_id}) queue is full (but it shouldn't be ahead of the Allocator, which was not full)"),
                                                             |len| if len <= 1 { self.wake_stream(*stream_id) });
                }
                true
            },
            None => false
        }
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

    pub async fn end_stream(&self, stream_id: u32, timeout: Duration) -> bool {

        // true if `stream_id` is not running
        let is_vacant = || unsafe { self.vacant_streams.peek_all().iter() }
            .flat_map(|&slice| slice)
            .find(|&vacant_stream_id| *vacant_stream_id == stream_id)
            .is_some();

        debug_assert_eq!(false, is_vacant(), "Mutiny's Multi OgreMPMCQueue Channel @ end_stream(): BUG! stream_id {stream_id} is not running! Running ones are {:?}",
                                             self.used_streams.iter().filter(|&id| *id != u32::MAX).collect::<Vec<&u32>>());

        let start = Instant::now();
        self.flush(timeout).await;
        {
            let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
            mutable_self.keep_stream_running[stream_id as usize] = false;
        }
        self.flush(timeout).await;
        self.wake_stream(stream_id);
        loop {
            tokio::time::sleep(Duration::from_millis(1)).await;
            if is_vacant() {
                break true
            } else if timeout != Duration::ZERO && start.elapsed() > timeout {
                break false
            }
        }
    }

    pub async fn end_all_streams(&self, timeout: Duration) -> u32 {
        let start = Instant::now();
        self.flush(timeout).await;
        {
            let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
            mutable_self.keep_stream_running.iter_mut()
                .for_each(|e| *e = false);
        }
        self.flush(timeout).await;
        self.wake_all_streams();
        loop {
            let running_streams = self.running_streams_count();
            tokio::time::sleep(Duration::from_millis(1)).await;
            if running_streams == 0 {
                break running_streams
            } else if timeout != Duration::ZERO && start.elapsed() > timeout {
                break running_streams
            }
        }
    }

    pub fn running_streams_count(&self) -> u32 {
        (MAX_STREAMS - self.vacant_streams.len()) as u32
    }

    pub fn pending_items_count(&self) -> u32 {
        self.buffer.used_slots_count() as u32
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
    use futures::{stream, StreamExt};


    /// exercises the code present on the documentation for $uni_channel_type
    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn doc_test() {
        let channel = OgreMPMCQueue::<&str>::new();
        let (mut stream, _stream_id) = channel.listener_stream();
        channel.try_send("a");
        println!("received: {}", stream.next().await.unwrap());
    }

    /// guarantees no unsafe code is preventing proper dropping of the created channels and the returned streams
    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn stream_and_channel_dropping() {
        {
            print!("Dropping the channel before the stream consumes the element: ");
            let channel = OgreMPMCQueue::<&str>::new();
            assert_eq!(Arc::strong_count(&channel), 1, "Sanity check on reference counting");
            let (mut stream, _stream_id) = channel.listener_stream();
            assert_eq!(Arc::strong_count(&channel), 2, "Creating a stream should increase the ref count by 1");
            channel.try_send("a");
            drop(channel);  // dropping the channel will decrease the Arc reference count by 1
            println!("received: {}", stream.next().await.unwrap());
        }
        {
            print!("Dropping the stream before the channel produces something, then another stream is created to consume the element: ");
            let channel = OgreMPMCQueue::<&str>::new();
            let stream = channel.listener_stream();
            assert_eq!(Arc::strong_count(&channel), 2, "`channel` + `stream`: reference count should be 2");
            drop(stream);
            assert_eq!(Arc::strong_count(&channel), 1, "Dropping a stream should decrease the ref count by 1");
            let (mut stream, _stream_id) = channel.listener_stream();
            assert_eq!(Arc::strong_count(&channel), 2, "1 `channel` + 1 `stream` again, at this point: reference count should be 2");
            channel.try_send("a");
            drop(channel);  // dropping the channel will decrease the Arc reference count by 1
            println!("received: {}", stream.next().await.unwrap());
        }
        // print!("Lazy check with stupid amount of creations and destructions... watch out the process for memory: ");
        // for i in 0..1234567 {
        //     let channel = OgreMPMCQueue::<&str>::new();
        //     let mut stream = channel.consumer_stream();
        //     drop(stream);
        //     let mut stream = channel.consumer_stream();
        //     channel.try_send("a");
        //     drop(channel);
        //     stream.next().await.unwrap();
        // }
        // println!("Done. Sleeping for 30 seconds. Go watch the heap!");
        // tokio::time::sleep(Duration::from_secs(30)).await;
    }

    /// Multis are designed to allow on-the-fly additions and removal of streams.
    /// This test guarantees and stresses that
    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn on_the_fly_streams() {
        let channel = OgreMPMCQueue::<String>::new();

        println!("1) streams 1 & 2 about to receive a message");
        let message = "to `stream1` and `stream2`".to_string();
        let (stream1, _stream1_id) = channel.listener_stream();
        let (stream2, _stream2_id) = channel.listener_stream();
        channel.try_send(message.clone());
        assert_received(stream1, message.clone(), "`stream1` didn't receive the right message").await;
        assert_received(stream2, message.clone(), "`stream2` didn't receive the right message").await;

        println!("2) stream3 should timeout");
        let (stream3, _stream3_id) = channel.listener_stream();
        assert_no_message(stream3, "`stream3` simply should timeout (no message will ever be sent through it)").await;

        println!("3) several creations and removals");
        for i in 0..10240 {
            let message = format!("gremling #{}", i);
            let (gremlin_stream, _gremlin_stream_id) = channel.listener_stream();
            channel.try_send(message.clone());
            assert_received(gremlin_stream, message, "`gremling_stream` didn't receive the right message").await;
        }

        async fn assert_received(stream: impl Stream<Item=String>, expected_message: String, failure_explanation: &str) {
            match tokio::time::timeout(Duration::from_millis(10), Box::pin(stream).next()).await {
                Ok(left)  => assert_eq!(left.unwrap(), expected_message, "{}", failure_explanation),
                Err(_)                 => panic!("{} -- Timed out!", failure_explanation),
            }
        }

        async fn assert_no_message(stream: impl Stream<Item=String>, failure_explanation: &str) {
            match tokio::time::timeout(Duration::from_millis(10), Box::pin(stream).next()).await {
                Ok(left)  => panic!("{} -- A timeout was expected, but message '{}' was received", failure_explanation, left.unwrap()),
                Err(_)                 => {},
            }
        }

    }

    /// guarantees implementors allows copying messages over several streams
    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn multiple_streams() {
        const PARALLEL_STREAMS: usize = 100;     // total test time will be this number / 100 (in seconds)

        let channel = OgreMPMCQueue::<u32, 100, PARALLEL_STREAMS>::new();

        // collect the streams
        let mut streams = stream::iter(0..PARALLEL_STREAMS)
            .then(|_i| {
                let channel = channel.clone();
                async move {
                    let (stream, _stream_id) = channel.listener_stream();
                    stream
                }
            })
            .collect::<Vec<_>>().await;

        // send
        for i in 0..PARALLEL_STREAMS as u32 {
            while !channel.try_send(i) {
                spin_loop();
            };
        }

        // check each stream gets all elements
        for s in 0..PARALLEL_STREAMS as u32 {
            for i in 0..PARALLEL_STREAMS as u32 {
                let item = streams[s as usize].next().await;
                assert_eq!(item, Some(i), "Stream #{s} didn't produce item {i}")
            }
        }

        // checks each stream has no more items
        for i in 0..PARALLEL_STREAMS {
            match tokio::time::timeout(Duration::from_millis(10), streams[i as usize].next()).await {
                Ok(item)  => panic!("All items should have timed out now, because there are no more elements on the stream -- but stream #{i} produced item {:?}", item),
                Err(_)    => {},
            }
        }
    }

    /// stresses on-demand stopping streams
    /// -- related to [dropping()], but here the "dropping" is active (controlled via a function call)
    ///    rather then structural (stream object getting out of scope)
    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn end_streams() {
        const WAIT_TIME: Duration = Duration::from_millis(100);
        {
            println!("Creating & ending a single stream: ");
            let channel = OgreMPMCQueue::<&str>::new();
            let (mut stream, stream_id) = channel.listener_stream();
            tokio::spawn(async move {
                tokio::time::sleep(WAIT_TIME/2).await;
                channel.end_stream(stream_id, WAIT_TIME).await;
            });
            match tokio::time::timeout(WAIT_TIME, stream.next()).await {
                Ok(none_item)     => assert_eq!(none_item, None, "Ending a stream should make it return immediately from the waiting state with `None`"),
                Err(_timeout_err) => panic!("{:?} elapsed but `stream.next().await` didn't return after `.end_stream()` was requested", WAIT_TIME),
            }
        }
        {
            println!("Creating & ending two streams: ");
            let channel = OgreMPMCQueue::<&str>::new();
            let (mut first, first_id) = channel.listener_stream();
            let (mut second, second_id) = channel.listener_stream();
            let first_ended = Arc::new(AtomicBool::new(false));
            let second_ended = Arc::new(AtomicBool::new(false));
            {
                let first_ended = Arc::clone(&first_ended);
                tokio::spawn(async move {
                    match tokio::time::timeout(WAIT_TIME, first.next()).await {
                        Ok(none_item)     => {
                            assert_eq!(none_item, None, "Ending the 'first' stream should make it return immediately from the waiting state with `None`");
                            first_ended.store(true, Relaxed);
                        },
                        Err(_timeout_err) => panic!("{:?} elapsed but `first.next().await` didn't return after `.end_stream()` was requested", WAIT_TIME),
                    }
                });
                let second_ended = Arc::clone(&second_ended);
                tokio::spawn(async move {
                    match tokio::time::timeout(WAIT_TIME, second.next()).await {
                        Ok(none_item)     => {
                            assert_eq!(none_item, None, "Ending the 'second' stream should make it return immediately from the waiting state with `None`");
                            second_ended.store(true, Relaxed);
                        },
                        Err(_timeout_err) => panic!("{:?} elapsed but `second.next().await` didn't return after `.end_stream()` was requested", WAIT_TIME),
                    }
                });
            }
            tokio::time::sleep(WAIT_TIME/2).await;
            channel.end_stream(first_id, WAIT_TIME).await;
            channel.end_stream(second_id, WAIT_TIME).await;
            assert_eq!(first_ended.load(Relaxed), true, "`first` stream didn't end in time. Inspect the test output for hints");
            assert_eq!(second_ended.load(Relaxed), true, "`second` stream didn't end in time. Inspect the test output for hints");
        }
    }

    /// guarantees no unsafe code is preventing proper dropping & releasing of payload resources
    #[cfg_attr(not(feature = "dox"), tokio::test)]
    async fn payload_dropping() {
        const PAYLOAD_TEXT: &str = "A shareable playload";
        let channel = OgreMPMCQueue::<Option<Arc<String>>>::new();
        let (mut stream, _stream_id) = channel.listener_stream();
        channel.try_send(Some(Arc::new(String::from(PAYLOAD_TEXT))));
        let payload = stream.next().await.unwrap();
        assert_eq!(payload.as_ref().unwrap().as_str(), PAYLOAD_TEXT, "sanity check failed: wrong payload received");
        assert_eq!(Arc::strong_count(&payload.unwrap()), 1, "A payload doing the round trip (being sent by the channel and received by the stream) should have been cloned just once -- so, when dropped, all resources would be freed");
    }

        /// assures performance won't be degraded when we make changes
    #[cfg_attr(not(feature = "dox"), tokio::test(flavor = "multi_thread"))]
    async fn performance_measurements() {
        #[cfg(not(debug_assertions))]
        const FACTOR: u32 = 1024;
        #[cfg(debug_assertions)]
        const FACTOR: u32 = 20;

        macro_rules! profile_same_task_same_thread_channel {
            ($channel: expr, $profiling_name: literal, $count: expr) => {
                let channel = $channel;
                let profiling_name = $profiling_name;
                let count = $count;
                let (mut stream, _stream_id) = channel.listener_stream();
                let start = Instant::now();
                for e in 0..count {
                    channel.try_send(e);
                    if let Some(consumed_e) = stream.next().await {
                        assert_eq!(consumed_e, e, "{profiling_name}: produced and consumed items differ");
                    } else {
                        panic!("{profiling_name}: Stream didn't provide expected item {e}");
                    }
                }
                let elapsed = start.elapsed();
                println!("{} (same task / same thread): {:10.2}/s -- {} items processed in {:?}",
                         profiling_name,
                         count as f64 / elapsed.as_secs_f64(),
                         count,
                         elapsed);
            }
        }

        macro_rules! profile_different_task_same_thread_channel {
            ($channel: expr, $profiling_name: literal, $count: expr) => {
                let channel = $channel;
                let profiling_name = $profiling_name;
                let count = $count;
                let (mut stream, _stream_id) = channel.listener_stream();
                let start = Instant::now();

                let sender_future = async {
                    for e in 0..count {
                        while !channel.try_send(e) {
                            tokio::task::yield_now().await; tokio::task::yield_now().await; tokio::task::yield_now().await; tokio::task::yield_now().await; tokio::task::yield_now().await;
                            tokio::task::yield_now().await; tokio::task::yield_now().await; tokio::task::yield_now().await; tokio::task::yield_now().await; tokio::task::yield_now().await;
                            tokio::task::yield_now().await; tokio::task::yield_now().await; tokio::task::yield_now().await; tokio::task::yield_now().await; tokio::task::yield_now().await;
                            tokio::task::yield_now().await; tokio::task::yield_now().await; tokio::task::yield_now().await; tokio::task::yield_now().await; tokio::task::yield_now().await;
                            tokio::task::yield_now().await; tokio::task::yield_now().await; tokio::task::yield_now().await; tokio::task::yield_now().await; tokio::task::yield_now().await;
                        }
                    }
                    channel.end_all_streams(Duration::from_secs(1)).await;
                };

                let receiver_future = async {
                    let mut counter = 0;
                    while let Some(e) = stream.next().await {
                        assert_eq!(e, counter, "Wrong event consumed");
                        counter += 1;
                    }
                };

                tokio::join!(sender_future, receiver_future);

                let elapsed = start.elapsed();
                println!("{} (different task / same thread): {:10.2}/s -- {} items processed in {:?}",
                         profiling_name,
                         count as f64 / elapsed.as_secs_f64(),
                         count,
                         elapsed);
            }
        }

        macro_rules! profile_different_task_different_thread_channel {
            ($channel: expr, $profiling_name: literal, $count: expr) => {
                let channel = $channel;
                let profiling_name = $profiling_name;
                let count = $count;
                let (mut stream, _stream_id) = channel.listener_stream();
                let start = Instant::now();

                let sender_task = tokio::spawn(async move {
                    for e in 0..count {
                        while !channel.try_send(e) {
                            std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop();
                            std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop();
                            std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop();
                            std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop();
                            std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop();
                            tokio::task::yield_now().await;     // when running in dev mode with --test-threads 1, this is needed or else we'll hang here
                        }
                    }
                    channel.end_all_streams(Duration::from_secs(5)).await;
                });

                let receiver_task = tokio::spawn(async move {
                    let mut counter = 0;
                    while let Some(e) = stream.next().await {
                        assert_eq!(e, counter, "Wrong event consumed");
                        counter += 1;
                    }
                });

                let (sender_result, receiver_result) = tokio::join!(sender_task, receiver_task);
                receiver_result.expect("receiver task");
                sender_result.expect("sender task");

                let elapsed = start.elapsed();
                println!("{} (different task / different thread): {:10.2}/s -- {} items processed in {:?}",
                         profiling_name,
                         count as f64 / elapsed.as_secs_f64(),
                         count,
                         elapsed);
            }
        }

        println!();
        profile_same_task_same_thread_channel!(OgreMPMCQueue::<u32, 10240, 1>::new(), "OgreMPMCQueue ", 10240*FACTOR);
        profile_different_task_same_thread_channel!(OgreMPMCQueue::<u32, 10240, 1>::new(), "OgreMPMCQueue ", 10240*FACTOR);
        profile_different_task_different_thread_channel!(OgreMPMCQueue::<u32, 10240, 1>::new(), "OgreMPMCQueue ", 10240*FACTOR);
    }

}

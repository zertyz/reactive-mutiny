//! Resting place for the [OgreFullSyncMPMCQueue] Multi Channel

use crate::{
    ogre_std::{
        ogre_queues::{
            OgreQueue,
            full_sync_queues::{
                full_sync_meta::FullSyncMeta,
                NonBlockingQueue,
            },
            meta_publisher::MetaPublisher,
            meta_subscriber::MetaSubscriber,
            meta_queue::MetaQueue,
        },
        ogre_sync,
    },
    StreamsManager,
};
use std::{
    time::Duration,
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicBool, Ordering::{Relaxed}},
    },
    pin::Pin,
    fmt::Debug,
    task::{Poll, Waker},
    mem::{self, MaybeUninit},
    hint::spin_loop,
};
use futures::{Stream};
use minstant::Instant;
use log::{warn};
use owning_ref::ArcRef;
use crate::multi::channels::multi_stream::MultiStream;


/// [InternalOgreFullSyncMPMCQueue] with defaults.\
/// Refresher: the backing queue requires "BUFFER_SIZE" to be a power of 2
pub type OgreFullSyncMPMCQueue<ItemType,
                       const BUFFER_SIZE: usize = 1024,
                       const MAX_STREAMS: usize = 16>
        = InternalOgreFullSyncMPMCQueue<ItemType,
                                BUFFER_SIZE,
                                MAX_STREAMS>;


/// This channel uses the fastest of the queues [FullSyncMeta], which are the fastest for general purpose use and for most hardware but requires that elements are copied, due to the full sync characteristics
/// of the backing queue, which doesn't allow enqueueing to happen independently of dequeueing.\
/// Due to that, this channel requires that `ItemType`s are `Clone`, since they will have to be moved around during dequeueing (as there is no way to keep the queue slot allocated during processing),
/// making this channel a typical best fit for small & trivial types.\
/// Please, measure your `Multi`s using all available channels [OgreFullSyncMPMCQueue], [OgreAtomicQueue] and, possibly, even [OgreMmapLog].\
/// See also [multi::channels::ogre_full_sync_mpmc_queue].\
/// Refresher: the backing queue requires `BUFFER_SIZE` to be a power of 2 -- the same applies to `MAX_STREAMS`, which will also have its own queue
#[repr(C,align(64))]      // aligned to cache line sizes for a careful control over false-sharing performance degradation
pub struct InternalOgreFullSyncMPMCQueue<ItemType:          Send + Sync + Debug,
                                         const BUFFER_SIZE: usize,
                                         const MAX_STREAMS: usize> {
    /// General non-blocking full sync queues -- one for each Multi
    queues:                 [FullSyncMeta<Option<Arc<ItemType>>, BUFFER_SIZE>; MAX_STREAMS],
    /// simple metrics
    created_streams_count:  AtomicU32,
    /// simple metrics
    finished_streams_count: AtomicU32,
    /// used to coordinate syncing between `vacant_streams` and `used_streams`
    streams_lock:           AtomicBool,
    /// the id # of streams that can be created are stored here.\
    /// synced with `used_streams`, so creating & dropping streams occurs as fast as possible, as well as enqueueing elements
    vacant_streams:         NonBlockingQueue<u32, MAX_STREAMS>,
    /// this is synced with `vacant_streams` to be its counter-part -- so enqueueing is optimized:
    /// it holds the stream ids that are active, `u32::MAX` being used to indicate a premature end of the list
    used_streams:           [u32; MAX_STREAMS],
    /// coordinates writes to the wakers
    wakers_lock:            AtomicBool,
    /// holds wakers for each stream id #
    wakers:                 [Option<Waker>; MAX_STREAMS],
    /// signals for Streams to end or to continue waiting for elements
    keep_stream_running:    [bool; MAX_STREAMS],
    /// for logs
    channel_name:           String,
}

impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
StreamsManager<'a, Arc<ItemType>, FullSyncMeta<Option<Arc<ItemType>>, BUFFER_SIZE>, Option<Arc<ItemType>>> for
InternalOgreFullSyncMPMCQueue<ItemType, BUFFER_SIZE, MAX_STREAMS> {

    fn backing_subscriber(self: &Arc<Self>, stream_id: u32) -> ArcRef<Self, FullSyncMeta<Option<Arc<ItemType>>, BUFFER_SIZE>> {
        ArcRef::from(self.clone())
            .map(|this| &this.queues[stream_id as usize])
    }

    #[inline(always)]
    fn keep_stream_running(&self, stream_id: u32) -> bool {
        self.keep_stream_running[stream_id as usize]
    }

    #[inline(always)]
    fn register_stream_waker(&self, stream_id: u32, waker: &Waker) {

        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};

        macro_rules! set {
            () => {
                let waker = waker.clone();
                ogre_sync::lock(&mutable_self.wakers_lock);
                let waker = mutable_self.wakers[stream_id as usize].insert(waker);
                ogre_sync::unlock(&mutable_self.wakers_lock);
                // the producer might have just woken the old version of the waker,
                // so the following waking up line is needed to assure the consumers won't ever hang
                // (as demonstrated by tests)
                waker.wake_by_ref();
            }
        }

        match &mut mutable_self.wakers[stream_id as usize] {
            Some(registered_waker) => {
                if !registered_waker.will_wake(waker) {
                    set!();
                }
            },
            None => {
                set!();
            },
        }
    }

    fn report_stream_dropped(&self, stream_id: u32) {
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        ogre_sync::lock(&self.wakers_lock);
        mutable_self.wakers[stream_id as usize] = None;
        ogre_sync::unlock(&self.wakers_lock);
        mutable_self.finished_streams_count.fetch_add(1, Relaxed);
        mutable_self.vacant_streams.enqueue(stream_id);
        mutable_self.sync_vacant_and_used_streams();
    }
}

impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
InternalOgreFullSyncMPMCQueue<ItemType, BUFFER_SIZE, MAX_STREAMS> {

    /// Returns a `Stream` -- as many streams as requested may be create, provided the specified limit [MAX_STREAMS] is respected.\
    /// Each stream will see all payloads sent through this channel.
    pub fn listener_stream(self: &Arc<Self>) -> (MultiStream<'a, Arc<ItemType>, Self, FullSyncMeta<Option<Arc<ItemType>>, BUFFER_SIZE>>, u32) {
        self.created_streams_count.fetch_add(1, Relaxed);
        let stream_id = match self.vacant_streams.dequeue() {
            Some(stream_id) => stream_id,
            None => panic!("OgreFullSyncMPMCQueue: This multi channel has a MAX_STREAMS of {MAX_STREAMS} -- which just got exhausted: program called `listener_stream()` {} times & {} of these where dropped. Please, increase the limit or fix the LOGIC BUG!",
                           self.created_streams_count.load(Relaxed), self.finished_streams_count.load(Relaxed)),
        };
        let mutable_self = unsafe {&mut *((self.as_ref() as *const Self) as *mut Self)};
        mutable_self.keep_stream_running[stream_id as usize] = true;
        self.sync_vacant_and_used_streams();
        (MultiStream::new(stream_id, self), stream_id)
    }

    #[inline(always)]
    fn buffer_size(&self) -> u32 {
        BUFFER_SIZE as u32
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
                ogre_sync::lock(&self.wakers_lock);
                if let Some(waker) = &self.wakers[stream_id as usize] {
                    waker.wake_by_ref();
                }
                ogre_sync::unlock(&self.wakers_lock);
            }
        }
    }

    /// rebuilds the `used_streams` list based of the current `vacant_items`
    fn sync_vacant_and_used_streams(&self) {
        ogre_sync::lock(&self.streams_lock);
        let mut vacant = unsafe { self.vacant_streams.peek_remaining().concat() };
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
        ogre_sync::unlock(&self.streams_lock);
    }

}

//#[async_trait]
/// implementation note: Rust 1.63 does not yet support async traits. See [super::MultiChannel]
impl<'a, ItemType:          Send + Sync + Debug + 'a,
         const BUFFER_SIZE: usize,
         const MAX_STREAMS: usize>
/*MultiChannel<ItemType>
for*/ InternalOgreFullSyncMPMCQueue<ItemType, BUFFER_SIZE, MAX_STREAMS> {

    pub fn new<IntoString: Into<String>>(channel_name: IntoString) -> Arc<Self> {
        let instance = Arc::new(Self {
            queues:                 {
                                         let mut queues: [FullSyncMeta<Option<Arc<ItemType>>, BUFFER_SIZE>; MAX_STREAMS] = unsafe { MaybeUninit::zeroed().assume_init() };
                                         for i in 0..queues.len() {
                                             queues[i] = FullSyncMeta::<Option<Arc<ItemType>>, BUFFER_SIZE>::new();
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
            channel_name:           channel_name.into(),
        });
        instance
    }

    #[inline(always)]
    pub fn send_arc(&self, arc_item: Arc<ItemType>) {
        for stream_id in self.used_streams.iter() {
            if *stream_id == u32::MAX {
                break
            }
            self.queues[*stream_id as usize].publish(|slot| { let _ = slot.insert(Arc::clone(&arc_item)); },
                                                     || {
                                                         warn!("Multi Channel's OgreMPMCQueue (named '{channel_name}', {used_streams_count} streams): One of the streams (#{stream_id}) is full of elements. Multi producing performance has been degraded. Increase the Multi buffer size (currently {BUFFER_SIZE}) to overcome that.",
                                                               channel_name = self.channel_name, used_streams_count = (MAX_STREAMS - self.vacant_streams.len()) as u32);
                                                         std::thread::sleep(Duration::from_millis(500));
                                                         true
                                                     },
                                                     |len| if len == 1 { self.wake_stream(*stream_id) });
        }
    }

    #[inline(always)]
    pub fn send(&self, item: ItemType) {
        let arc_item = Arc::new(item);
        self.send_arc(arc_item);
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
        let is_vacant = || unsafe { self.vacant_streams.peek_remaining().iter() }
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
        loop {
            self.wake_stream(stream_id);
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
        self.sig_stop_all_streams();
        self.flush(timeout).await;
        loop {
            self.wake_all_streams();
            let running_streams = self.running_streams_count();
            tokio::time::sleep(Duration::from_millis(1)).await;
            if running_streams == 0 {
                break running_streams
            } else if timeout != Duration::ZERO && start.elapsed() > timeout {
                break running_streams
            }
        }
    }

    pub fn sig_stop_all_streams(&self) {
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        mutable_self.keep_stream_running.iter_mut()
            .for_each(|e| *e = false);
    }

    pub fn running_streams_count(&self) -> u32 {
        (MAX_STREAMS - self.vacant_streams.len()) as u32
    }

    #[inline(always)]
    pub fn pending_items_count(&self) -> u32 {
        self.used_streams.iter()
            .filter(|&&stream_id| stream_id != u32::MAX)
            .map(|&stream_id| self.queues[stream_id as usize].available_elements())
            .sum::<usize>() as u32
    }

}


/// Tests & enforces the requisites & expose good practices & exercises the API of of the [uni](self) module
#[cfg(any(test, doc))]
mod tests {
    use super::*;
    use std::io::Write;
    use futures::{stream, StreamExt};
    use tokio::task::spawn_blocking;


    #[ctor::ctor]
    fn suite_setup() {
        simple_logger::SimpleLogger::new().with_utc_timestamps().init().unwrap_or_else(|_| eprintln!("--> LOGGER WAS ALREADY STARTED"));
    }

    /// exercises the code present on the documentation for $uni_channel_type
    #[cfg_attr(not(doc),tokio::test)]
    async fn doc_test() {
        let channel = OgreFullSyncMPMCQueue::<&str>::new("doc_test");
        let (mut stream, _stream_id) = channel.listener_stream();
        channel.send("a");
        println!("received: {}", stream.next().await.unwrap());
    }

    /// guarantees no unsafe code is preventing proper dropping of the created channels and the returned streams
    #[cfg_attr(not(doc),tokio::test)]
    async fn stream_and_channel_dropping() {
        {
            print!("Dropping the channel before the stream consumes the element: ");
            let channel = OgreFullSyncMPMCQueue::<&str>::new("stream_and_channel_dropping");
            assert_eq!(Arc::strong_count(&channel), 1, "Sanity check on reference counting");
            let (mut stream, _stream_id) = channel.listener_stream();
            assert_eq!(Arc::strong_count(&channel), 3, "Creating a stream should increase the ref count by 2");
            let (mut _stream_2, _stream_id_2) = channel.listener_stream();
            assert_eq!(Arc::strong_count(&channel), 5, "Creating a stream should increase the ref count by 2");
            channel.send("a");
            drop(channel);  // dropping the channel will decrease the Arc reference count by 1
            println!("received: {}", stream.next().await.unwrap());
        }
        {
            print!("Dropping the stream before the channel produces something, then another stream is created to consume the element: ");
            let channel = OgreFullSyncMPMCQueue::<&str>::new("stream_and_channel_dropping");
            let stream = channel.listener_stream();
            assert_eq!(Arc::strong_count(&channel), 3, "`channel` + `stream`: reference count should be 3");
            drop(stream);
            assert_eq!(Arc::strong_count(&channel), 1, "Dropping a stream should decrease the ref count by 2");
            let (mut stream, _stream_id) = channel.listener_stream();
            assert_eq!(Arc::strong_count(&channel), 3, "1 `channel` + 1 `stream` again, at this point: reference count should be 3");
            channel.send("a");
            drop(channel);  // dropping the channel will decrease the Arc reference count by 1
            println!("received: {}", stream.next().await.unwrap());
        }
        // print!("Brute-force check with stupid amount of creations and destructions... watch out the process for memory!");
        // for i in 0..1234567 {
        //     let channel = OgreMPMCQueue::<&str>::new();
        //     let mut stream = channel.consumer_stream();
        //     drop(stream);
        //     let mut stream = channel.consumer_stream();
        //     channel.try_send("a");
        //     drop(channel);
        //     stream.next().await.unwrap();
        // }
        // println!("Done. Sleeping for 30 seconds. Go watch the heap! ... or rerun this test with `/sbin/time -v ...`");
        // tokio::time::sleep(Duration::from_secs(30)).await;
    }

    /// Multis are designed to allow on-the-fly additions and removal of streams.
    /// This test guarantees and stresses that
    #[cfg_attr(not(doc),tokio::test)]
    async fn on_the_fly_streams() {
        let channel = OgreFullSyncMPMCQueue::<String>::new("on_the_fly_streams");

        println!("1) streams 1 & 2 about to receive a message");
        let message = "to `stream1` and `stream2`".to_string();
        let (stream1, _stream1_id) = channel.listener_stream();
        let (stream2, _stream2_id) = channel.listener_stream();
        channel.send(message.clone());
        assert_received(stream1, message.clone(), "`stream1` didn't receive the right message").await;
        assert_received(stream2, message.clone(), "`stream2` didn't receive the right message").await;

        println!("2) stream3 should timeout");
        let (stream3, _stream3_id) = channel.listener_stream();
        assert_no_message(stream3, "`stream3` simply should timeout (no message will ever be sent through it)").await;

        println!("3) several creations and removals");
        for i in 0..10240 {
            let message = format!("gremling #{}", i);
            let (gremlin_stream, _gremlin_stream_id) = channel.listener_stream();
            channel.send(message.clone());
            assert_received(gremlin_stream, message, "`gremling_stream` didn't receive the right message").await;
        }

        async fn assert_received(stream: impl Stream<Item=Arc<String>>, expected_message: String, failure_explanation: &str) {
            match tokio::time::timeout(Duration::from_millis(10), Box::pin(stream).next()).await {
                Ok(left) => assert_eq!(*left.unwrap(), expected_message, "{}", failure_explanation),
                Err(_)                    => panic!("{} -- Timed out!", failure_explanation),
            }
        }

        async fn assert_no_message(stream: impl Stream<Item=Arc<String>>, failure_explanation: &str) {
            match tokio::time::timeout(Duration::from_millis(10), Box::pin(stream).next()).await {
                Ok(left)  => panic!("{} -- A timeout was expected, but message '{}' was received", failure_explanation, left.unwrap()),
                Err(_)                 => {},
            }
        }

    }

    /// guarantees implementors allows copying messages over several streams
    #[cfg_attr(not(doc),tokio::test)]
    async fn multiple_streams() {
        const ELEMENTS:         usize = 100;
        const PARALLEL_STREAMS: usize = 128;

        let channel = OgreFullSyncMPMCQueue::<u32, 128, PARALLEL_STREAMS>::new("multiple_streams");

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

        // send (one copy to each stream)
        for i in 0..ELEMENTS as u32 {
            channel.send(i);
        }

        // check each stream gets all elements
        for s in 0..PARALLEL_STREAMS as u32 {
            for i in 0..ELEMENTS as u32 {
                let item = streams[s as usize].next().await;
                assert_eq!(*item.unwrap(), i, "Stream #{s} didn't produce item {i}")
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
    #[cfg_attr(not(doc),tokio::test)]
    async fn end_streams() {
        const WAIT_TIME: Duration = Duration::from_millis(100);
        {
            println!("Creating & ending a single stream: ");
            let channel = OgreFullSyncMPMCQueue::<&str>::new("end_streams");
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
            let channel = OgreFullSyncMPMCQueue::<&str>::new("end_streams");
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
    #[cfg_attr(not(doc),tokio::test)]
    async fn payload_dropping() {
        const PAYLOAD_TEXT: &str = "A shareable playload";
        let channel = OgreFullSyncMPMCQueue::<String>::new("payload_dropping");
        let (mut stream, _stream_id) = channel.listener_stream();
        channel.send(String::from(PAYLOAD_TEXT));
        let payload = stream.next().await.unwrap();
        assert_eq!(payload.as_str(), PAYLOAD_TEXT, "sanity check failed: wrong payload received");
        assert_eq!(Arc::strong_count(&payload), 1, "A payload doing the round trip (being sent by the channel and received by the stream) should have been cloned just once -- so, when dropped, all resources would be freed");
    }

    /// assures performance won't be degraded when we make changes
    #[cfg_attr(not(doc),tokio::test(flavor="multi_thread", worker_threads=2))]
    #[ignore]   // must run in a single thread for accurate measurements
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

                print!("{} (same task / same thread):           ", profiling_name);
                std::io::stdout().flush().unwrap();

                let start = Instant::now();
                for e in 0..count {
                    channel.send(e);
                    if let Some(consumed_e) = stream.next().await {
                        assert_eq!(*consumed_e, e, "{profiling_name}: produced and consumed items differ");
                    } else {
                        panic!("{profiling_name}: Stream didn't provide expected item {e}");
                    }
                }
                let elapsed = start.elapsed();

                println!("{:12.2}/s -- {} items processed in {:?}",
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

                let sender_future = async {
                    let mut e = 0;
                    while e < count {
                        let buffer_entries_left = channel.buffer_size() - channel.pending_items_count();
                        for _ in 0..buffer_entries_left {
                            channel.send(e);
                            e += 1;
                        }
                        tokio::task::yield_now().await;
                    }
                    channel.end_all_streams(Duration::from_secs(1)).await;
                };

                let receiver_future = async {
                    let mut counter = 0;
                    while let Some(e) = stream.next().await {
                        assert_eq!(*e, counter, "Wrong event consumed");
                        counter += 1;
                    }
                };

                print!("{} (different task / same thread):      ", profiling_name);
                std::io::stdout().flush().unwrap();

                let start = Instant::now();
                tokio::join!(sender_future, receiver_future);
                let elapsed = start.elapsed();

                println!("{:12.2}/s -- {} items processed in {:?}",
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

                let sender_task = tokio::task::spawn_blocking(move || {
                    let mut e = 0;
                    while e < count {
                        let buffer_entries_left = channel.buffer_size() - channel.pending_items_count();
                        for _ in 0..buffer_entries_left {
                            channel.send(e);
                            e += 1;
                        }
                        std::hint::spin_loop();
                        //tokio::task::yield_now().await;
                    }
                    //channel.end_all_streams(Duration::from_secs(5)).await;
                    channel.sig_stop_all_streams();
                });

                let receiver_task = tokio::spawn(async move {
                    let mut counter = 0;
                    while let Some(e) = stream.next().await {
                        assert_eq!(*e, counter, "Wrong event consumed");
                        counter += 1;
                    }
                });

                print!("{} (different task / different thread): ", profiling_name);
                std::io::stdout().flush().unwrap();

                let start = Instant::now();
                let (sender_result, receiver_result) = tokio::join!(sender_task, receiver_task);
                receiver_result.expect("receiver task");
                sender_result.expect("sender task");
                let elapsed = start.elapsed();

                println!("{:12.2}/s -- {} items processed in {:?}",
                         count as f64 / elapsed.as_secs_f64(),
                         count,
                         elapsed);
            }
        }

        println!();
        profile_same_task_same_thread_channel!(OgreFullSyncMPMCQueue::<u32, 16384, 1>::new("profile_same_task_same_thread_channel"), "OgreFullSyncMPMCQueue ", 16384*FACTOR);
        profile_different_task_same_thread_channel!(OgreFullSyncMPMCQueue::<u32, 16384, 1>::new("profile_different_task_same_thread_channel"), "OgreFullSyncMPMCQueue ", 16384*FACTOR);
        profile_different_task_different_thread_channel!(OgreFullSyncMPMCQueue::<u32, 16384, 1>::new("profile_different_task_different_thread_channel"), "OgreFullSyncMPMCQueue ", 16384*FACTOR);
    }

}

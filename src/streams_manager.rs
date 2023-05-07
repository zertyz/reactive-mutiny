//! Resting place for [StreamsManager]


use crate::{
    ogre_std::{
        ogre_queues::{
            OgreQueue,
            full_sync::{
                NonBlockingQueue,
            },
        },
        ogre_sync,
    },
};
use std::{
    time::Duration,
    sync::{
        atomic::{AtomicU32, AtomicBool, Ordering::{Relaxed}},
    },
    pin::Pin,
    task::{Waker},
    marker::PhantomData,
};
use minstant::Instant;


/// Basis for all `Uni`s and `Multi`s Stream Managers,
/// containing all common fields and code.
pub struct StreamsManagerBase<'a, ItemType:           'a,
                                  const MAX_STREAMS:  usize,
                                  DerivativeItemType  : 'a = ItemType> {

    /// the id # of streams that can be created are stored here.\
    /// synced with `used_streams`, so creating & dropping streams occurs as fast as possible
    vacant_streams:         Pin<Box<NonBlockingQueue<u32, MAX_STREAMS>>>,
    /// this is synced with `vacant_streams` to be its counter-part -- so enqueueing is optimized:
    /// it holds the stream ids that are active.
    /// Note: the sentinel value `u32::MAX` is used to indicate a premature end of the list
    used_streams:           Pin<Box<[u32; MAX_STREAMS]>>,
    /// used to coordinate syncing between `vacant_streams` and `used_streams`
    streams_lock:           AtomicBool,
    /// counter streams created
    created_streams_count:  AtomicU32,
    /// counter of streams cancelled
    finished_streams_count: AtomicU32,
    /// coordinates writes to the wakers -- TODO might not be needed if we use AtomicPointers on `wakers`
    wakers_lock:            AtomicBool,
    /// holds wakers for each stream id #
    wakers:                 Pin<Box<[Option<Waker>; MAX_STREAMS]>>,
    /// signals for Streams to end or to continue waiting for elements
    keep_streams_running:   Pin<Box<[bool; MAX_STREAMS]>>,
    /// for logs
    streams_manager_name:    String,

    _phanrom: PhantomData<(&'a ItemType, &'a DerivativeItemType)>,

}

impl<'a, ItemType:           'a,
         const MAX_STREAMS:  usize,
         DerivativeItemType>
StreamsManagerBase<'a, ItemType, MAX_STREAMS, DerivativeItemType> {

    pub fn new<IntoString: Into<String>>(streams_manager_name: IntoString) -> Self {
        Self {
            created_streams_count:  AtomicU32::new(0),
            vacant_streams:         {
                                        let vacant_streams = Box::pin(NonBlockingQueue::<u32, MAX_STREAMS>::new("vacant streams for an OgreMPMCQueue Multi channel"));
                                        for stream_id in 0..MAX_STREAMS as u32 {
                                            vacant_streams.enqueue(stream_id);
                                        }
                                        vacant_streams
                                    },
            used_streams:           Box::pin([u32::MAX; MAX_STREAMS]),
            wakers:                 Box::pin((0..MAX_STREAMS).map(|_| Option::<Waker>::None).collect::<Vec<_>>().try_into().unwrap()),
            finished_streams_count: AtomicU32::new(0),
            keep_streams_running:   Box::pin([false; MAX_STREAMS]),
            streams_lock:           AtomicBool::new(false),
            wakers_lock:            AtomicBool::new(false),
            streams_manager_name:   streams_manager_name.into(),
            _phanrom:               Default::default(),
        }
    }

    /// Returns this streams manager name given to this instance, at the moment of its creation
    pub fn name(&self) -> &str {
        self.streams_manager_name.as_str()
    }

    /// Creates the room for a new `Stream`, but returns only its `stream_id`, leaving the `Stream` creation per-se to the caller.
    pub fn create_stream_id(&self) -> u32 {
        self.created_streams_count.fetch_add(1, Relaxed);
        let stream_id = match self.vacant_streams.dequeue() {
            Some(stream_id) => stream_id,
            None => panic!("StreamsManager: '{}' has a MAX_STREAMS of {MAX_STREAMS} -- which just got exhausted: stats: {} streams were created; {} dropped. Please, increase the limit or fix the LOGIC BUG!",
                           self.streams_manager_name, self.created_streams_count.load(Relaxed), self.finished_streams_count.load(Relaxed)),
        };
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        mutable_self.keep_streams_running[stream_id as usize] = true;
        self.sync_vacant_and_used_streams();
        stream_id
    }

    /// Wakes the `stream_id` -- for instance, when an element arrives at an empty container.
    #[inline(always)]
    pub fn wake_stream(&self, stream_id: u32) {
        match unsafe {self.wakers.get_unchecked(stream_id as usize)} {
            Some(waker) => waker.wake_by_ref(),
            None => {
                // try again, syncing
                ogre_sync::lock(&self.wakers_lock);
                if let Some(waker) = unsafe {self.wakers.get_unchecked(stream_id as usize)} {
                    waker.wake_by_ref();
                }
                ogre_sync::unlock(&self.wakers_lock);
            }
        }
    }

    /// Wakes all streams -- suitable for EOL procedures
    pub fn wake_all_streams(&self) {
        for stream_id in 0..MAX_STREAMS as u32 {
            self.wake_stream(stream_id);
        }
    }

    /// Returns `false` if the `Stream` has been signaled to end its operations, causing it to report "out-of-elements" as soon as possible.
    #[inline(always)]
    pub fn keep_stream_running(&self, stream_id: u32) -> bool {
        unsafe { *self.keep_streams_running.get_unchecked(stream_id as usize) }
    }

    /// Signals `stream_id` to end, as soon as possible -- making it reach its end-of-life.\
    /// Also guarantees that it will be awoken to react to the command immediately
    pub fn cancel_stream(&self, stream_id: u32) {
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        mutable_self.keep_streams_running[stream_id as usize] = false;
        self.wake_stream(stream_id);
    }

    /// Signals all `Stream`s to end as soon as possible (making them reach their "out of elements" phase).\
    /// Any parked streams are awaken, so they may end as well.
    pub fn cancel_all_streams(&self) {
        for stream_id in self.used_streams.iter() {
            if *stream_id == u32::MAX {
                break
            }
            self.cancel_stream(*stream_id);
        }
    }

    #[inline(always)]
    pub fn register_stream_waker(&self, stream_id: u32, waker: &Waker) {

        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};

        macro_rules! set {
            () => {
                let waker = waker.clone();
                ogre_sync::lock(&mutable_self.wakers_lock);
                let waker = unsafe { mutable_self.wakers.get_unchecked_mut(stream_id as usize).insert(waker) };
                ogre_sync::unlock(&mutable_self.wakers_lock);
                // the producer might have just woken the old version of the waker,
                // so the following waking up line is needed to assure the consumers won't ever hang
                // (as demonstrated by tests)
                waker.wake_by_ref();
            }
        }

        match unsafe { mutable_self.wakers.get_unchecked_mut(stream_id as usize) } {
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

    /// Informs the manager that the current stream is being dropped and should be removed & cleaned from the internal list.\
    /// Dropping a single stream, as of 2023-04-15, means that the whole Channel must cease working to avoid undefined behavior.
    /// Due to the algorithm to wake Streams being based on a unique number assigned to it at the moment of creation,
    /// if a stream is created and dropped, the `send` functions will still try to awake stream #0 -- the stream number is determined
    /// based on the elements on the queue before publishing it.
    /// This is specially bad if the first stream is dropped:
    ///   1) the channel gets, eventually empty -- all streams will sleep
    ///   2) An item is sent -- the logic will wake the first stream: since the stream is no longer there, that element won't be consumed!
    ///   3) Eventually, a second item is sent: now the queue has length 2 and the send logic will wake consumer #1
    ///   4) Consumer #1, since it was not dropped, will be awaken and will run until the channel is empty again -- consuming both elements.
    pub fn report_stream_dropped(&self, stream_id: u32) {
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        ogre_sync::lock(&self.wakers_lock);
        mutable_self.wakers[stream_id as usize] = None;
        ogre_sync::unlock(&self.wakers_lock);
        mutable_self.finished_streams_count.fetch_add(1, Relaxed);
        mutable_self.vacant_streams.enqueue(stream_id);
        mutable_self.sync_vacant_and_used_streams();
    }

    /// Return the ids of the used streams.\
    /// WARNING: the sentinel value u32::MAX is used to indicate a premature end of the list
    #[inline(always)]
    pub fn used_streams(&self) -> &[u32; MAX_STREAMS] {
        &self.used_streams
    }

    /// rebuilds the `used_streams` list based of the current `vacant_items`
    #[inline(always)]
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
                        unsafe { *mutable_self.used_streams.get_unchecked_mut(last_used_stream_id as usize)  = used_stream_id };
                        i += 1;
                    }
                    i += 1;
                }
                None => {
                    last_used_stream_id += 1;
                    unsafe { *mutable_self.used_streams.get_unchecked_mut(last_used_stream_id as usize) = i };
                    i += 1;
                }
            }
        }
        for i in (last_used_stream_id + 1) as usize .. MAX_STREAMS {
            unsafe { *mutable_self.used_streams.get_unchecked_mut(i) = u32::MAX };
        }
        ogre_sync::unlock(&self.streams_lock);
    }

    pub async fn flush(&self, timeout: Duration, pending_items_count: impl Fn() -> u32) -> u32 {
        let mut start: Option<Instant> = None;
        loop {
            let pending_count = pending_items_count();
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

    pub async fn end_stream(&self, stream_id: u32, timeout: Duration, pending_items_count: impl Fn() -> u32) -> bool {

        // true if `stream_id` is not running
        let is_vacant = || unsafe { self.vacant_streams.peek_remaining().iter() }
            .flat_map(|&slice| slice)
            .find(|&vacant_stream_id| *vacant_stream_id == stream_id)
            .is_some();

        debug_assert_eq!(false, is_vacant(), "Mutiny's Multi OgreMPMCQueue Channel @ end_stream(): BUG! stream_id {stream_id} is not running! Running ones are {:?}",
                                             self.used_streams.iter().filter(|&id| *id != u32::MAX).collect::<Vec<&u32>>());

        let start = Instant::now();
        self.flush(timeout, pending_items_count).await;
        self.cancel_stream(stream_id);
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

    pub async fn end_all_streams(&self, timeout: Duration, pending_items_count: impl Fn() -> u32) -> u32 {
        let start = Instant::now();
        self.flush(timeout, &pending_items_count).await;
        self.cancel_all_streams();
        self.flush(timeout, &pending_items_count).await;
        loop {
            let running_streams = self.running_streams_count();
            if running_streams == 0 || timeout != Duration::ZERO && start.elapsed() > timeout {
                break running_streams
            }
            self.wake_all_streams();
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    #[inline(always)]
    pub fn running_streams_count(&self) -> u32 {
        (MAX_STREAMS - self.vacant_streams.len()) as u32
    }

}
//! Resting place for [StreamsManager]


use crate::ogre_std::{
        ogre_queues::{
            full_sync::full_sync_move::FullSyncMove,
            meta_container::MoveContainer,
            meta_publisher::MovePublisher,
            meta_subscriber::MoveSubscriber,
        },
        ogre_sync,
    };
use std::{
    time::Duration,
    sync::atomic::{AtomicU32, AtomicBool, Ordering::Relaxed},
    pin::Pin,
    task::Waker,
};
use std::cell::UnsafeCell;
use minstant::Instant;


/// Basis for all `Uni`s and `Multi`s Stream Managers,
/// containing all common fields and code.
pub struct StreamsManagerBase<const MAX_STREAMS:  usize> {

    /// the id # of streams that can be created are stored here.\
    /// synced with `used_streams`, so creating & dropping streams occurs as fast as possible
    vacant_streams:         FullSyncMove<u32, MAX_STREAMS>,
    /// this is synced with `vacant_streams` to be its counter-part -- so enqueueing is optimized:
    /// it holds the stream ids that are active.
    /// Note: the sentinel value `u32::MAX` is used to indicate a premature end of the list
    used_streams:           UnsafeCell<Pin<Box<[u32; MAX_STREAMS]>>>,
    /// redundant over `created_streams_count` & `finished_streams_count`
    used_streams_count:     AtomicU32,
    /// used to coordinate syncing between `vacant_streams` and `used_streams`
    streams_lock:           AtomicBool,
    /// counter streams created
    created_streams_count:  AtomicU32,
    /// counter of streams cancelled
    finished_streams_count: AtomicU32,
    /// coordinates writes to the wakers -- TODO might not be needed if we use AtomicPointers on `wakers`
    wakers_lock:            AtomicBool,
    /// holds wakers for each stream id #
    wakers:                 UnsafeCell<Pin<Box<[Option<Waker>; MAX_STREAMS]>>>,
    /// signals for Streams to end or to continue waiting for elements
    keep_streams_running:   UnsafeCell<Pin<Box<[bool; MAX_STREAMS]>>>,
    /// for logs
    streams_manager_name:    String,

}

impl<const MAX_STREAMS:  usize>
StreamsManagerBase<MAX_STREAMS> {

    pub fn new<IntoString: Into<String>>(streams_manager_name: IntoString) -> Self {
        Self {
            vacant_streams:         {
                                        let vacant_streams = FullSyncMove::<u32, MAX_STREAMS>::new();
                                        for stream_id in 0..MAX_STREAMS as u32 {
                                            vacant_streams.publish_movable(stream_id);
                                        }
                                        vacant_streams
                                    },
            used_streams:           UnsafeCell::new(Box::pin([u32::MAX; MAX_STREAMS])),
            used_streams_count:     AtomicU32::new(0),
            created_streams_count:  AtomicU32::new(0),
            finished_streams_count: AtomicU32::new(0),
            wakers:                 UnsafeCell::new(Box::pin((0..MAX_STREAMS).map(|_| Option::<Waker>::None).collect::<Vec<_>>().try_into().unwrap())),
            wakers_lock:            AtomicBool::new(false),
            keep_streams_running:   UnsafeCell::new(Box::pin([false; MAX_STREAMS])),
            streams_lock:           AtomicBool::new(false),
            streams_manager_name:   streams_manager_name.into(),
        }
    }

    /// Returns this streams manager name given to this instance, at the moment of its creation
    pub fn name(&self) -> &str {
        self.streams_manager_name.as_str()
    }

    /// Creates the room for a new `Stream`, but returns only its `stream_id`, leaving the `Stream` creation per-se to the caller.
    pub fn create_stream_id(&self) -> u32 {
        self.created_streams_count.fetch_add(1, Relaxed);
        self.used_streams_count.fetch_add(1, Relaxed);
        let stream_id = match self.vacant_streams.consume_movable() {
            Some(stream_id) => stream_id,
            None => panic!("StreamsManager: '{}' has a MAX_STREAMS of {MAX_STREAMS} -- which just got exhausted: stats: {} streams were created; {} dropped. Please, increase the limit or fix the LOGIC BUG!",
                           self.streams_manager_name, self.created_streams_count.load(Relaxed), self.finished_streams_count.load(Relaxed)),
        };
        let keep_streams_running = unsafe { &mut * self.keep_streams_running.get() };
        keep_streams_running[stream_id as usize] = true;
        self.sync_vacant_and_used_streams();
        stream_id
    }

    /// Wakes the `stream_id` -- for instance, when an element arrives at an empty container.
    #[inline(always)]
    pub fn wake_stream(&self, stream_id: u32) {
        let wakers = unsafe { &* self.wakers.get() };
        match unsafe {wakers.get_unchecked(stream_id as usize)} {
            Some(waker) => waker.wake_by_ref(),
            None => {
                // try again, syncing
                ogre_sync::lock(&self.wakers_lock);
                if let Some(waker) = unsafe {wakers.get_unchecked(stream_id as usize)} {
                    waker.wake_by_ref();
                }
                ogre_sync::unlock(&self.wakers_lock);
            }
        }
    }

    /// Wakes all streams -- suitable for EOL procedures
    pub fn wake_all_streams(&self) {
        for stream_id in 0..MAX_STREAMS as u32 {
            if stream_id == u32::MAX {
                break
            }
            self.wake_stream(stream_id);
        }
    }

    /// Returns `false` if the `Stream` has been signaled to end its operations, causing it to report "out-of-elements" as soon as possible.
    #[inline(always)]
    pub fn keep_stream_running(&self, stream_id: u32) -> bool {
        unsafe {
            let keep_streams_running = &* self.keep_streams_running.get();
            *keep_streams_running.get_unchecked(stream_id as usize)
        }
    }

    /// Returns `true` of the channel is still processing elements
    #[inline(always)]
    pub fn is_any_stream_running(&self) -> bool {
        for stream_id in 0..MAX_STREAMS as u32 {
            if stream_id == u32::MAX {
                break
            }
            if self.keep_stream_running(stream_id) {
                return true
            }
        }
        false
    }

    /// Signals `stream_id` to end, as soon as possible -- making it reach its end-of-life.\
    /// Also guarantees that it will be awoken to react to the command immediately
    pub fn cancel_stream(&self, stream_id: u32) {
        let keep_streams_running = unsafe { &mut * self.keep_streams_running.get() };
        keep_streams_running[stream_id as usize] = false;
        self.wake_stream(stream_id);
    }

    /// Signals all `Stream`s to end as soon as possible (making them reach their "out of elements" phase).\
    /// Any parked streams are awaken, so they may end as well.
    pub fn cancel_all_streams(&self) {
        let used_streams = unsafe { &* self.used_streams.get() };
        for stream_id in used_streams.iter() {
            if *stream_id == u32::MAX {
                break
            }
            self.cancel_stream(*stream_id);
        }
    }

    #[inline(always)]
    pub fn register_stream_waker(&self, stream_id: u32, waker: &Waker) {

        let wakers = unsafe { &mut * self.wakers.get() };

        macro_rules! set {
            () => {
                let waker = waker.clone();
                ogre_sync::lock(&self.wakers_lock);
                let waker = unsafe { wakers.get_unchecked_mut(stream_id as usize).insert(waker) };
                ogre_sync::unlock(&self.wakers_lock);
                // the producer might have just woken the old version of the waker,
                // so the following waking up line is needed to assure the consumers won't ever hang
                // (as demonstrated by tests)
                waker.wake_by_ref();
            }
        }

        match unsafe { wakers.get_unchecked_mut(stream_id as usize) } {
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
        let wakers = unsafe { &mut * self.wakers.get() };
        ogre_sync::lock(&self.wakers_lock);
        wakers[stream_id as usize] = None;
        ogre_sync::unlock(&self.wakers_lock);
        self.finished_streams_count.fetch_add(1, Relaxed);
        self.used_streams_count.fetch_sub(1, Relaxed);
        self.vacant_streams.publish_movable(stream_id);
        self.sync_vacant_and_used_streams();
    }

    /// Return the ids of the used streams.\
    /// WARNING: the sentinel value u32::MAX is used to indicate a premature end of the list
    #[inline(always)]
    pub fn used_streams(&self) -> &[u32; MAX_STREAMS] {
        unsafe { &* self.used_streams.get() }
    }

    /// rebuilds the `used_streams` list based of the current `vacant_items`
    #[inline(always)]
    fn sync_vacant_and_used_streams(&self) {
        let used_streams = unsafe { &mut * self.used_streams.get() };
        ogre_sync::lock(&self.streams_lock);
        let mut vacant = unsafe { self.vacant_streams.peek_remaining().concat() };
        vacant.sort_unstable();
        let mut vacant_iter = vacant.iter();
        let mut i = 0;
        let mut last_used_stream_id = -1;
        while i < MAX_STREAMS as u32 {
            match vacant_iter.next() {
                Some(next_vacant_stream_id) => {
                    for used_stream_id in i .. *next_vacant_stream_id {
                        last_used_stream_id += 1;
                        unsafe { *used_streams.get_unchecked_mut(last_used_stream_id as usize)  = used_stream_id };
                    }
                    i = *next_vacant_stream_id + 1;
                }
                None => {
                    last_used_stream_id += 1;
                    unsafe { *used_streams.get_unchecked_mut(last_used_stream_id as usize) = i };
                    i += 1;
                }
            }
        }
        for i in (last_used_stream_id + 1) as usize .. MAX_STREAMS {
            unsafe { *used_streams.get_unchecked_mut(i) = u32::MAX };
        }
        ogre_sync::unlock(&self.streams_lock);
    }

    pub async fn flush(&self, timeout: Duration, pending_items_counter: impl Fn() -> u32) -> u32 {
        let mut start: Option<Instant> = None;
        loop {
            let pending_items_count = pending_items_counter();
            if pending_items_count > 0 {
                self.wake_all_streams();
                tokio::time::sleep(Duration::from_millis(1)).await;
            } else {
                break 0
            }
            // enforce timeout
            if timeout != Duration::ZERO {
                if let Some(start) = start {
                    if start.elapsed() > timeout {
                        break pending_items_count
                    }
                } else {
                    start = Some(Instant::now());
                }
            }
        }
    }

    pub async fn end_stream(&self, stream_id: u32, timeout: Duration, pending_items_counter: impl Fn() -> u32) -> bool {

        // true if `stream_id` is not running
        let is_vacant = || unsafe { self.vacant_streams.peek_remaining().iter() }
            .flat_map(|&slice| slice)
            .any(|vacant_stream_id| *vacant_stream_id == stream_id);

        debug_assert!(!is_vacant(), "Mutiny's `StreamsManager` @ end_stream(): BUG! stream_id {stream_id} is not running! Running ones are {:?}",
                                    unsafe { &*self.used_streams.get() } .iter().filter(|&id| *id != u32::MAX).collect::<Vec<&u32>>());

        let start = Instant::now();
        self.flush(timeout, pending_items_counter).await;
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

    pub async fn end_all_streams(&self, timeout: Duration, pending_items_counter: impl Fn() -> u32) -> u32 {
        let start = Instant::now();
        if self.flush(timeout, &pending_items_counter).await > 0 {
            self.cancel_all_streams();
            self.flush(timeout, &pending_items_counter).await;
        }
        self.cancel_all_streams();
        while self.running_streams_count() > 0 {
            if timeout != Duration::ZERO && start.elapsed() > timeout {
                break
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        self.running_streams_count()
    }

    #[inline(always)]
    pub fn running_streams_count(&self) -> u32 {
        self.used_streams_count.load(Relaxed)
        // could also be: (MAX_STREAMS - self.vacant_streams.available_elements_count()) as u32
    }

}


// TODO: 2023-06-14: Needed while `SyncUnsafeCell` is still not stabilized
unsafe impl<const MAX_STREAMS:  usize>
Send for
StreamsManagerBase<MAX_STREAMS> {}

// TODO: 2023-06-14: Needed while `SyncUnsafeCell` is still not stabilized
unsafe impl<const MAX_STREAMS:  usize>
Sync for
StreamsManagerBase<MAX_STREAMS> {}


//! Basis for multiple producer / multiple consumer queues using full synchronization & async functions
//! -- Tokio's `Semaphores` are not use as they are not low level enough

use std::fmt::Debug;
use std::future::Future;
use std::mem::MaybeUninit;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;


pub struct AsyncBase<SlotType,
                     const BUFFER_SIZE: usize,
                     const METRICS:     bool = false,
                     const DEBUG:       bool = false> {

    /// locked when the queue is full, unlocked when it is no longer full
    full_guard:        AtomicBool,
    /// guards critical regions to allow concurrency
    concurrency_guard: AtomicBool,
    tail: u32,
    /// holder for the elements
    buffer: [SlotType; BUFFER_SIZE],
    head: u32,
    /// locked when the queue is empty, unlocked when it is no longer empty
    empty_guard:      AtomicBool,

}

impl<SlotType:          Copy+Debug,
     const BUFFER_SIZE: usize,
     const METRICS:     bool,
     const DEBUG:       bool>
AsyncBase<SlotType,
          BUFFER_SIZE,
          METRICS,
          DEBUG> {

    pub fn new() -> Self {
        Self {
            full_guard:        AtomicBool::new(false),
            concurrency_guard: AtomicBool::new(false),
            tail:              0,
            buffer:            unsafe { MaybeUninit::zeroed().assume_init() },
            head:              0,
            empty_guard:       AtomicBool::new(false),
        }
    }

    #[inline(always)]
    pub async fn enqueue<SetterFn:                         Fn(&mut SlotType),
                         ReportFullFn:                     Fn() -> ReportFullFnFuture,
                         ReportFullFnFuture:               Future<Output=bool>,
                         ReportLenAfterEnqueueingFn:       Fn(u32) -> ReportLenAfterEnqueueingFnFuture,
                         ReportLenAfterEnqueueingFnFuture: Future<Output=()> >
                        (&self, setter_fn:                            SetterFn,
                                report_full_async_fn:                 ReportFullFn,
                                report_len_after_enqueueing_async_fn: ReportLenAfterEnqueueingFn)
                        -> bool {

        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        let mut len_before;

        // lock & acquire a slot to set the item
        loop {
            lock(&self.concurrency_guard).await;
            len_before = self.tail.overflowing_sub(self.head).0;
            if len_before < BUFFER_SIZE as u32 {
                break
            } else {
                unlock(&self.concurrency_guard);
                let maybe_no_longer_full = report_full_async_fn().await;
                if !maybe_no_longer_full {
                    return false;
                }
            }
        }

        setter_fn(&mut mutable_self.buffer[self.tail as usize % BUFFER_SIZE]);
        mutable_self.tail = self.tail.overflowing_add(1).0;

        unlock(&self.concurrency_guard);
        report_len_after_enqueueing_async_fn(len_before+1).await;
        true
    }

    #[inline(always)]
    pub async fn dequeue<ReportEmptyFn:                    Fn() -> ReportEmptyFnFuture,
                         ReportEmptyFnFuture:              Future<Output=bool>,
                         ReportLenAfterDequeueingFn:       Fn(i32) -> ReportLenAfterDequeueingFnFuture,
                         ReportLenAfterDequeueingFnFuture: Future<Output=()> >
                        (&self, report_empty_async_fn:                ReportEmptyFn,
                                report_len_after_dequeueing_async_fn: ReportLenAfterDequeueingFn)
                        -> Option<SlotType> {

        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};

        let mut len_before;
        loop {
            lock(&self.concurrency_guard).await;
            len_before = self.tail.overflowing_sub(self.head).0 as i32;
            if len_before > 0 {
                break
            } else {
                unlock(&self.concurrency_guard);
                let maybe_no_longer_empty = report_empty_async_fn().await;
                if !maybe_no_longer_empty {
                    return None;
                }
            }
        }

        let slot_value = self.buffer[self.head as usize % BUFFER_SIZE];
        mutable_self.head = self.head.overflowing_add(1).0;

        unlock(&self.concurrency_guard);
        report_len_after_dequeueing_async_fn(len_before-1).await;
        Some(slot_value)
    }

    pub fn len(&self) -> usize {
        self.tail.overflowing_sub(self.head).0 as usize
    }

    pub fn buffer_size(&self) -> usize {
        BUFFER_SIZE
    }

}

/// Returns when the lock was acquired -- inspired by `parking-lot`
/// Unlocked: false; locked: true
#[inline(always)]
async fn lock(raw_mutex: &AtomicBool) {
    const OPTIMISTIC_SLEEP_DURATION: Duration = Duration::from_millis(1);
    const FALLBACK_SLEEP_DURATION: Duration = Duration::from_millis(100);
    // attempt to lock -- spinning for 5 times, relaxing the CPU between attempts
    if raw_mutex.compare_exchange_weak(false, true, Relaxed, Relaxed).is_ok() { return } else { std::hint::spin_loop() }
    if raw_mutex.compare_exchange_weak(false, true, Relaxed, Relaxed).is_ok() { return } else { std::hint::spin_loop() }
    if raw_mutex.compare_exchange_weak(false, true, Relaxed, Relaxed).is_ok() { return } else { std::hint::spin_loop() }
    if raw_mutex.compare_exchange_weak(false, true, Relaxed, Relaxed).is_ok() { return } else { std::hint::spin_loop() }
    if raw_mutex.compare_exchange_weak(false, true, Relaxed, Relaxed).is_ok() { return } else { std::hint::spin_loop() }
    // no deal -- lets try 5 more times busy-waiting but yielding to Tokio
    if raw_mutex.compare_exchange(false, true, Relaxed, Relaxed).is_ok() { return } else { tokio::task::yield_now().await }
    if raw_mutex.compare_exchange(false, true, Relaxed, Relaxed).is_ok() { return } else { tokio::task::yield_now().await }
    if raw_mutex.compare_exchange(false, true, Relaxed, Relaxed).is_ok() { return } else { tokio::task::yield_now().await }
    if raw_mutex.compare_exchange(false, true, Relaxed, Relaxed).is_ok() { return } else { tokio::task::yield_now().await }
    if raw_mutex.compare_exchange(false, true, Relaxed, Relaxed).is_ok() { return } else { tokio::task::yield_now().await }
    // still no good -- 5 more times, but sleeping for an optimistic duration (small) on each attempt
    if raw_mutex.compare_exchange(false, true, Relaxed, Relaxed).is_ok() { return } else { tokio::time::sleep(OPTIMISTIC_SLEEP_DURATION).await }
    if raw_mutex.compare_exchange(false, true, Relaxed, Relaxed).is_ok() { return } else { tokio::time::sleep(OPTIMISTIC_SLEEP_DURATION).await }
    if raw_mutex.compare_exchange(false, true, Relaxed, Relaxed).is_ok() { return } else { tokio::time::sleep(OPTIMISTIC_SLEEP_DURATION).await }
    if raw_mutex.compare_exchange(false, true, Relaxed, Relaxed).is_ok() { return } else { tokio::time::sleep(OPTIMISTIC_SLEEP_DURATION).await }
    if raw_mutex.compare_exchange(false, true, Relaxed, Relaxed).is_ok() { return } else { tokio::time::sleep(OPTIMISTIC_SLEEP_DURATION).await }
    // fallback -- stay here until we're able to lock
    while !raw_mutex.compare_exchange(false, true, Relaxed, Relaxed).is_ok() {
        tokio::time::sleep(FALLBACK_SLEEP_DURATION).await
    }
}

/// Releases any locks, returning immediately
#[inline(always)]
fn unlock(raw_mutex: &AtomicBool) {
    raw_mutex.store(false, Relaxed);
}
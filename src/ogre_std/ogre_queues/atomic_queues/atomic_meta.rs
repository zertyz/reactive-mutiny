//! Basis for multiple producer / multiple consumer queues using atomics for synchronization,
//! allowing enqueueing syncing to be (almost) fully detached from the dequeueing syncing

use super::super::{
    meta_queue::MetaQueue,
};
use std::{
    fmt::Debug,
    sync::atomic::{AtomicU32,Ordering::Relaxed},
    mem::MaybeUninit,
};


/// to make the most of performance, let BUFFER_SIZE be a power of 2 (so that 'i % BUFFER_SIZE' modulus will be optimized)
/// TODO 2023-03-07: this algorithm has the following problems:
///      - concurrency (see the comments on the [BlockingQueue] & [NonBlockingQueue] using this)
///      - assimetry: the enqueue/dequeue operations goes through almost two different algorithms. A redesign would use a pair of pointers for the enqueuer and another for the dequeuer (on a different cache line)
///        with our "split pointers". The rationale is that the enqueuer and dequeuer may work independent from one another until the enqueuer finds itself to be full (then it will sync with dequeues) or deququer
///        finds itself empty (and it will try to sync to enqueuers)
///      - To explore: a third pair: head and tail, where both enqueuers and dequeuers will sync after they are done; enqueuers will simply inc their tail; dequeuers will inc their heads; cmpexch will sync to the 3rd pair.
// #[repr(C,align(64))]      // users of this class, if uncertain, are advised to declare this as their first field and have this annotation to cause alignment to cache line sizes, for a careful control over false-sharing performance degradation
pub struct AtomicMeta<SlotType,
                      const BUFFER_SIZE: usize> {
    /// where to enqueue the next element -- we'll enforce this won't overlap with 'head'\
    /// -- after increasing this field, the enqueuer must still write the value (until so, it is
    ///    not yet ready for dequeue)
    pub(crate) enqueuer_tail: AtomicU32,
    /// used by enqueer to control when 'dequeuer_tail' should be set to 'enqueuer_tail'
    pub(crate) concurrent_enqueuers: AtomicU32,
    /// holder for the elements
    pub(crate) buffer: [SlotType; BUFFER_SIZE],
    /// used by the *enqueuer* to check if the queue is full;\
    /// used by the *dequeuer* to get where from to dequeue the next element -- provided it is checked against 'dequeuer_tail'
    pub(crate) head: AtomicU32,
    /// informs dequeuers up to which 'tail' is safe to dequeue
    pub(crate) dequeuer_tail: AtomicU32,
}

impl<SlotType:          Clone + Debug,
     const BUFFER_SIZE: usize>
MetaQueue<SlotType> for
AtomicMeta<SlotType,
           BUFFER_SIZE> {

    fn new() -> Self {
        Self {
            head:                 AtomicU32::new(0),
            enqueuer_tail:        AtomicU32::new(0),
            dequeuer_tail:        AtomicU32::new(0),
            concurrent_enqueuers: AtomicU32::new(0),
            buffer:               unsafe { MaybeUninit::zeroed().assume_init() },
        }
    }

    #[inline(always)]
    fn enqueue<SetterFn:                   FnOnce(&mut SlotType),
               ReportFullFn:               Fn() -> bool,
               ReportLenAfterEnqueueingFn: FnOnce(u32)>
              (&self,
               setter_fn:                      SetterFn,
               report_full_fn:                 ReportFullFn,
               report_len_after_enqueueing_fn: ReportLenAfterEnqueueingFn)
              -> bool {

        let mutable_buffer = unsafe {
            let const_ptr = self.buffer.as_ptr();
            let mut_ptr = const_ptr as *mut [SlotType; BUFFER_SIZE];
            &mut *mut_ptr
        };
        self.concurrent_enqueuers.fetch_add(1, Relaxed);
        let mut slot_id = self.enqueuer_tail.fetch_add(1, Relaxed);
        let mut len_before;
        loop {
            let head = self.head.load(Relaxed);
            len_before = slot_id.overflowing_sub(head).0;
            // is queue full?
            if len_before < BUFFER_SIZE as u32 {
                break
            } else {
// concurrency bug instrumentation
// if len_before > BUFFER_SIZE as u32 {
//     let current_enqueuer_tail = self.enqueuer_tail.load(Relaxed);
//     let current_dequeuer_tail = self.dequeuer_tail.load(Relaxed);
//     let concurrent_enqueuers = self.concurrent_enqueuers.load(Relaxed);
//     if current_enqueuer_tail - current_dequeuer_tail > concurrent_enqueuers {
//         println!("## current_enqueuer_tail: {current_enqueuer_tail}; current_dequeuer_tail: {current_dequeuer_tail}; concurrent_enqueuers: {concurrent_enqueuers}");
//     }
// }
                if self.enqueuer_tail.compare_exchange(slot_id + 1, slot_id, Relaxed, Relaxed).is_ok() {
                    self.concurrent_enqueuers.fetch_sub(1, Relaxed);
                    // report the queue is full, allowing a retry if recovered from the condition
                    if !report_full_fn() {
                        return false;
                    } else {
                        self.concurrent_enqueuers.fetch_add(1, Relaxed);
                        slot_id = self.enqueuer_tail.fetch_add(1, Relaxed);
                    }
                } else {
                    // a resync is needed -- other enqueuers ran & queue may not be full any longer for us: try again
                    std::hint::spin_loop();
                }
            }
        }

        setter_fn(&mut mutable_buffer[slot_id as usize % BUFFER_SIZE]);

        // recede 'concurrent_enqueuers' & advance 'enqueuer_tail', attempting to get 'dequeuer_tail' as close to 'enqueuer_tail' as possible,
        // if fully syncing them is not a possibility
        let remaining_concurrent_enqueuers = self.concurrent_enqueuers.fetch_sub(1, Relaxed);
        if remaining_concurrent_enqueuers == 1 {
            // we were the last concurrent enqueuer -- attempt to sync tails
            // (sync might not be complete if 'enqueuer_tail' has been increased since last load (some lines above),
            // so dequeuers will always try to sync again when empty queue is detected
            let dequeuer_tail = self.dequeuer_tail.load(Relaxed);
            if slot_id >= dequeuer_tail {
                let _ = self.dequeuer_tail.compare_exchange(dequeuer_tail, slot_id+1, Relaxed, Relaxed);
            }
        } else {
            // we were not the last enqueuer running concurrently. Anyway,
            // attempt to inform the next dequeuer, as soon as possible, that another element is ready for dequeueing
            // (even if there are still other enqueuers running).
            // this will take effect when enqueuers end their execution in the order in which they started
            let _ = self.dequeuer_tail.compare_exchange(slot_id, slot_id+1, Relaxed, Relaxed);
        }
        report_len_after_enqueueing_fn(len_before + 1);
        true
    }

    #[inline(always)]
    fn dequeue<GetterReturnType,
               GetterFn:                   Fn(&SlotType) -> GetterReturnType,
               ReportEmptyFn:              Fn() -> bool,
               ReportLenAfterDequeueingFn: FnOnce(i32)>
              (&self,
               getter_fn:                      GetterFn,
               report_empty_fn:                ReportEmptyFn,
               report_len_after_dequeueing_fn: ReportLenAfterDequeueingFn)
              -> Option<GetterReturnType> {

        let mut head = self.head.load(Relaxed);
        let mut tail = self.dequeuer_tail.load(Relaxed);
        let mut len_before;

        // loop until we get an element or are sure to be empty -- syncing along the way
        loop {
            // sync `head` & `tail`
            loop {
                len_before = tail.overflowing_sub(head).0 as i32;
                // is queue empty? Attempt to sync tails if there are no enqueuers running
                if len_before <= 0 {   // head can be > than tail if an enqueuer is currently running, found that the queue was full and had to decrease the tail
                    let enqueuer_tail_before = self.enqueuer_tail.load(Relaxed);
                    if self.concurrent_enqueuers.load(Relaxed) == 0 {
                        let enqueuer_tail_after = self.enqueuer_tail.load(Relaxed);
                        if enqueuer_tail_before == enqueuer_tail_after {    // assures no enqueuer ran between the atomic loads (or else we won't know on which one to trust)
                            let enqueuer_tail = enqueuer_tail_before;
                            if enqueuer_tail != tail {
                                match self.dequeuer_tail.compare_exchange(tail, enqueuer_tail, Relaxed, Relaxed) {
                                    Ok(_) => tail = enqueuer_tail,
                                    Err(reloaded_val) => tail = reloaded_val,
                                };
                                std::hint::spin_loop();
                                head = self.head.load(Relaxed);
                                continue;
                            }
                        }
                    }
                    // report the queue is empty, allowing a retry if recovered from the condition
                    if !report_empty_fn() {
                        return None;
                    } else {
                        continue;
                    }
                }
                break;
            }

            // return the value if we were the one to increase 'head'
            let slot_value = &self.buffer[head as usize % BUFFER_SIZE];
            let ret_val = getter_fn(slot_value);
            match self.head.compare_exchange(head, head+1, Relaxed, Relaxed) {
                Ok(_) => {
                    report_len_after_dequeueing_fn(len_before-1);
                    return Some(ret_val);
                },
                Err(reloaded_val) => {
                    head = reloaded_val;
                    tail = self.dequeuer_tail.load(Relaxed);
                    // somebody dequeued our head -- redo the dequeueing attempt from scratch
                }
            }
        }
    }

    fn len(&self) -> usize {
        self.enqueuer_tail.load(Relaxed).overflowing_sub(self.head.load(Relaxed)).0 as usize
    }

    fn max_size(&self) -> usize {
        BUFFER_SIZE
    }

    unsafe fn peek_all(&self) -> [&[SlotType]; 2] {
        let head_index = self.head.load(Relaxed) as usize % BUFFER_SIZE;
        let tail_index = self.dequeuer_tail.load(Relaxed) as usize % BUFFER_SIZE;
        if self.head.load(Relaxed) == self.dequeuer_tail.load(Relaxed) {
            [&[],&[]]
        } else if head_index < tail_index {
            unsafe {
                let const_ptr = self.buffer.as_ptr();
                let ptr = const_ptr as *const [SlotType; BUFFER_SIZE];
                let array = &*ptr;
                [&array[head_index ..tail_index], &[]]
            }
        } else {
            unsafe {
                let const_ptr = self.buffer.as_ptr();
                let ptr = const_ptr as *const [SlotType; BUFFER_SIZE];
                let array = &*ptr;
                [&array[head_index..BUFFER_SIZE], &array[0..tail_index]]
            }
        }
    }

    fn debug_info(&self) -> String {
        let Self {enqueuer_tail, concurrent_enqueuers, buffer: _, head, dequeuer_tail} = self;
        let enqueuer_tail = enqueuer_tail.load(Relaxed);
        let head = head.load(Relaxed);
        let dequeuer_tail = dequeuer_tail.load(Relaxed);
        let concurrent_enqueuers = concurrent_enqueuers.load(Relaxed);
        format!("ogre_queues::atomic_meta's state: {{head: {head}, enqueuer_tail: {enqueuer_tail}, dequeuer_tail: {dequeuer_tail}, (len: {}), concurrent_enqueuers: {concurrent_enqueuers}, elements: {{{}}}'}}",
                self.len(),
                unsafe {self.peek_all()}.iter().flat_map(|&slice| slice).fold(String::new(), |mut acc, e| {
                    acc.push_str(&format!("'{:?}',", e));
                    acc
                }))
    }
}

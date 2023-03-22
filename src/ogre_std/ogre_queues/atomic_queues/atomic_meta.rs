//! Basis for multiple producer / multiple consumer queues using atomics for synchronization,
//! allowing enqueueing syncing to be (almost) fully detached from the dequeueing syncing

use super::super::{
    meta_publisher::MetaPublisher,
    meta_subscriber::MetaSubscriber,
    meta_queue::MetaQueue,
};
use std::{
    fmt::Debug,
    sync::atomic::{AtomicU32,Ordering::Relaxed},
    mem::MaybeUninit,
};


/// BUFFER_SIZE mut be a power of 2
// #[repr(C,align(64))]      // users of this class, if uncertain, are advised to declare this as their first field and have this annotation to cause alignment to cache line sizes, for a careful control over false-sharing performance degradation
pub struct AtomicMeta<SlotType,
                      const BUFFER_SIZE: usize> {
    /// marks the first element of the queue, ready for dequeue -- increasing when dequeues are complete
    pub(crate) head: AtomicU32,
    /// increase-before-load field, marking where the next element should be retrieved from
    /// -- may receed if it gets ahead of the published `tail`
    pub(crate) dequeuer_head: AtomicU32,
    /// holder for the queue elements
    pub(crate) buffer: [SlotType; BUFFER_SIZE],
    /// marks the last element of the queue, ready for dequeue -- increasing when enqueues are complete
    pub(crate) tail: AtomicU32,
    /// increase-before-load field, marking where the next element should be written to
    /// -- may receed if exceeded the buffer capacity
    pub(crate) enqueuer_tail: AtomicU32,
}

impl<SlotType:          Clone + Debug,
     const BUFFER_SIZE: usize>
MetaQueue<SlotType> for
AtomicMeta<SlotType, BUFFER_SIZE> {

    fn new() -> Self {
        Self {
            head:                 AtomicU32::new(0),
            tail:                 AtomicU32::new(0),
            dequeuer_head:        AtomicU32::new(0),
            enqueuer_tail:        AtomicU32::new(0),
            buffer:               unsafe { MaybeUninit::zeroed().assume_init() },
        }
    }
}

impl<SlotType:          Clone + Debug,
     const BUFFER_SIZE: usize>
MetaPublisher<SlotType> for
AtomicMeta<SlotType, BUFFER_SIZE> {

    #[inline(always)]
    fn publish<SetterFn:                   FnOnce(&mut SlotType),
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
                // queue is full: restabilish the correct `enqueuer_tail` (receeding it to its original value)
                match self.enqueuer_tail.compare_exchange(slot_id + 1, slot_id, Relaxed, Relaxed) {
                    Ok(_) => {
                        // report the queue is full, allowing a retry, if the method says we recovered from the condition
                        if !report_full_fn() {
                            return false;
                        } else {
                            slot_id = self.enqueuer_tail.fetch_add(1, Relaxed);
                        }
                    },
                    Err(reloaded_enqueuer_tail) => {
                        if reloaded_enqueuer_tail > slot_id {
                            // a new cmp_exch will be attempted due to concurrent modification of the variable
                            // we'll relax the cpu proportional to our position in the "concurrent modification order"
                            relaxed_wait::<SlotType>(reloaded_enqueuer_tail - slot_id);
                        } else {
                            panic!(" SHOULD NOT HAPPEN ");
                        }
                    },
                }
            }
        }

        setter_fn(&mut mutable_buffer[slot_id as usize % BUFFER_SIZE]);

        // strong sync tail
        loop {
            match self.tail.compare_exchange_weak(slot_id, slot_id + 1, Relaxed, Relaxed) {
                Ok(_) => break,
                Err(reloaded_tail) => {
                    if slot_id > reloaded_tail {
                        relaxed_wait::<SlotType>(slot_id - reloaded_tail)
                    }
                },
            }
        }
        report_len_after_enqueueing_fn(len_before+1);
        true
    }

    #[inline(always)]
    fn available_elements(&self) -> usize {
        self.tail.load(Relaxed).overflowing_sub(self.head.load(Relaxed)).0 as usize
    }

    fn max_size(&self) -> usize {
        BUFFER_SIZE
    }

    fn debug_info(&self) -> String {
        let Self {head, tail, dequeuer_head, enqueuer_tail, buffer: _} = self;
        let head = head.load(Relaxed);
        let tail = tail.load(Relaxed);
        let dequeuer_head = dequeuer_head.load(Relaxed);
        let enqueuer_tail = enqueuer_tail.load(Relaxed);
        format!("ogre_queues::atomic_meta's state: {{head: {head}, tail: {tail}, dequeuer_head: {dequeuer_head}, enqueuer_tail: {enqueuer_tail}, (len: {}), elements: {{{}}}'}}",
                self.available_elements(),
                unsafe {self.peek_remaining()}.iter().flat_map(|&slice| slice).fold(String::new(), |mut acc, e| {
                    acc.push_str(&format!("'{:?}',", e));
                    acc
                }))
    }
}

impl<SlotType:          Clone + Debug,
     const BUFFER_SIZE: usize>
MetaSubscriber<SlotType> for
AtomicMeta<SlotType, BUFFER_SIZE> {

    #[inline(always)]
    fn consume<GetterReturnType,
               GetterFn:                   Fn(&mut SlotType) -> GetterReturnType,
               ReportEmptyFn:              Fn() -> bool,
               ReportLenAfterDequeueingFn: FnOnce(i32)>
              (&self,
               getter_fn:                      GetterFn,
               report_empty_fn:                ReportEmptyFn,
               report_len_after_dequeueing_fn: ReportLenAfterDequeueingFn)
              -> Option<GetterReturnType> {

        let mutable_buffer = unsafe {
            let const_ptr = self.buffer.as_ptr();
            let mut_ptr = const_ptr as *mut [SlotType; BUFFER_SIZE];
            &mut *mut_ptr
        };
        let mut slot_id = self.dequeuer_head.fetch_add(1, Relaxed);
        let mut len_before;

        loop {
            let tail = self.tail.load(Relaxed);
            len_before = tail.overflowing_sub(slot_id).0 as i32;
            // is queue empty?
            if len_before > 0 {
                break
            } else {
                // queue is empty: restabilish the correct `dequeuer_head` (receeding it to its original value)
                match self.dequeuer_head.compare_exchange(slot_id + 1, slot_id, Relaxed, Relaxed) {
                    Ok(_) => {
                        if !report_empty_fn() {
                            return None;
                        } else {
                            slot_id = self.dequeuer_head.fetch_add(1, Relaxed);
                        }
                    },
                    Err(reloaded_dequeuer_head) => {
                        if reloaded_dequeuer_head > slot_id {
                            relaxed_wait::<SlotType>(reloaded_dequeuer_head - slot_id);
                        }
                    }
                }
            }
        }

        let slot_value = &mut mutable_buffer[slot_id as usize % BUFFER_SIZE];
        let ret_val = getter_fn(slot_value);

        // strong sync head
        loop {
            match self.head.compare_exchange_weak(slot_id, slot_id + 1, Relaxed, Relaxed) {
                Ok(_) => break,
                Err(reloaded_head) => {
                    if slot_id > reloaded_head {
                        relaxed_wait::<SlotType>(slot_id - reloaded_head);
                    }
                }
            }
        }
        report_len_after_dequeueing_fn(len_before-1);
        Some(ret_val)
    }

    unsafe fn peek_remaining(&self) -> [&[SlotType]; 2] {
        let head_index = self.head.load(Relaxed) as usize % BUFFER_SIZE;
        let tail_index = self.tail.load(Relaxed) as usize % BUFFER_SIZE;
        if self.head.load(Relaxed) == self.tail.load(Relaxed) {
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

}

// NOTE: spin_loop instruction disabled for now, as Tokio is faster without it (frees the CPU to do other tasks)
// /// relax the cpu for a time proportional to how much we are expected to wait for the concurrent CAS operation to succeed
// /// (5x * number of bytes to be set * wait_factor)
#[inline(always)]
fn relaxed_wait<SlotType>(_wait_factor: u32) {
    // for _ in 0..wait_factor {
    //     // subject to loop unrolling optimization
    //     for _ in 0..std::mem::size_of::<SlotType>() {
    //         std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop();
    //     }
    // }
}

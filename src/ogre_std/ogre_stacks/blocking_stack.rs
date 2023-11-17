//! multiple producer / multiple consumer blocking stack with a full locking synchronization --
//! only a single push (or pop) may execute the critical region at a time.\
//! Blocks (with parking-lot mutex) on a push if the stack is full and also on a pop, if its empty.

use super::super::ogre_stacks::OgreStack;
use std::{
    fmt::Debug,
    mem::MaybeUninit,
    time::Duration,
};
use parking_lot::lock_api::{RawMutex as RawMutex_api, RawMutexTimed};
use parking_lot::RawMutex;


#[repr(C,align(64))]      // aligned to cache line sizes to avoid false-sharing performance degradation
pub struct Stack<SlotType, const BUFFER_SIZE: usize, const METRICS: bool, const DEBUG: bool, const LOCK_TIMEOUT_MILLIS: usize> {
    buffer:            [SlotType; BUFFER_SIZE],
    /// locked when the stack is empty, unlocked when it is no longer empty
    empty_guard:       RawMutex,
    /// locked when the stack is full, unlocked when it is no longer full
    full_guard:        RawMutex,
    /// guards the code's critical regions to allow concurrency
    concurrency_guard: RawMutex,
    head:       u32,
    // for use when metrics are enabled
    push_count:      u64,
    pop_count:       u64,
    push_collisions: u64,
    pop_collisions:  u64,
    push_full_count: u64,
    pop_empty_count: u64,
    /// for use when debug is enabled
    stack_name: String,
}
impl<SlotType: Copy+Debug, const BUFFER_SIZE: usize, const METRICS: bool, const DEBUG: bool, const LOCK_TIMEOUT_MILLIS: usize>
Stack<SlotType, BUFFER_SIZE, METRICS, DEBUG, LOCK_TIMEOUT_MILLIS> {

    const TRY_LOCK_DURATION: Duration = Duration::from_millis(LOCK_TIMEOUT_MILLIS as u64);

    /// called when the stack is empty: waits for a slot to filled in for up to `LOCK_TIMEOUT_MILLIS`\
    /// -- returns true if we recovered from the empty condition; false otherwise
    #[inline(always)]
    fn report_empty(&mut self) -> bool {
        if METRICS {
            self.pop_empty_count += 1;
        }
        unsafe {self.concurrency_guard.unlock()};

        // lock 'empty_guard' with a timeout (if applicable)
        if LOCK_TIMEOUT_MILLIS == 0 {
            if DEBUG {
                eprintln!("### STACK '{}' is empty. Waiting for a new element (indefinitely) so POP may proceed...", self.stack_name);
            }
            self.empty_guard.lock();
        } else {
            if DEBUG {
                eprintln!("### STACK '{}' is empty. Waiting for a new element (for up to {}ms) so POP may proceed...", self.stack_name, LOCK_TIMEOUT_MILLIS);
            }
            if !self.empty_guard.try_lock_for(Self::TRY_LOCK_DURATION) {
                if DEBUG {
                    eprintln!("Blocking stack '{}', said to be empty, waited too much for an element to be available so it could be popped. Bailing out after waiting for ~{}ms", self.stack_name, LOCK_TIMEOUT_MILLIS);
                }
                return false;
            }
        }
        unsafe {self.empty_guard.unlock()};

        self.concurrency_guard.lock();
        true
    }

    #[inline(always)]
    fn report_no_longer_empty(&self) {
        unsafe { self.empty_guard.unlock() }
    }

    /// called when the stack is full: waits for a slot to be freed up to `LOCK_TIMEOUT_MILLIS`\
    /// -- returns true if we recovered from the full condition; false otherwise
    #[inline(always)]
    fn report_full(&mut self) -> bool {
        if METRICS {
            self.push_full_count += 1;
        }
        unsafe {self.concurrency_guard.unlock()};

        // lock 'full_guard' with a timeout (if applicable)
        if LOCK_TIMEOUT_MILLIS == 0 {
            if DEBUG {
                eprintln!("### STACK '{}' is full. Waiting for a free slot (indefinitely) so PUSH may proceed...", self.stack_name);
            }
            self.full_guard.lock();
        } else {
            if DEBUG {
                eprintln!("### STACK '{}' is full. Waiting for a free slot (for up to {}ms) so PUSH may proceed...", self.stack_name, LOCK_TIMEOUT_MILLIS);
            }
            if !self.full_guard.try_lock_for(Self::TRY_LOCK_DURATION) {
                if DEBUG {
                    eprintln!("Blocking stack '{}', said to be full, waited too much for a free slot to push a new element. Bailing out after waiting for ~{}ms", self.stack_name, LOCK_TIMEOUT_MILLIS);
                }
                return false
            }
        }
        unsafe {self.full_guard.unlock()};

        self.concurrency_guard.lock();
        true
    }

    #[inline(always)]
    fn report_no_longer_full(&self) {
        unsafe { self.full_guard.unlock() }
    }
}
impl<SlotType: Copy+Debug, const BUFFER_SIZE: usize, const METRICS: bool, const DEBUG: bool, const LOCK_TIMEOUT_MILLIS: usize>
OgreStack<SlotType>
for Stack<SlotType, BUFFER_SIZE, METRICS, DEBUG, LOCK_TIMEOUT_MILLIS> {

    fn new(stack_name: String) -> Self {
        let instance = Self {
            buffer:            unsafe { MaybeUninit::zeroed().assume_init() },
            empty_guard:       RawMutex::INIT,
            full_guard:        RawMutex::INIT,
            concurrency_guard: RawMutex::INIT,
            head:              0,
            push_count:        0,
            pop_count:         0,
            push_collisions:   0,
            pop_collisions:    0,
            push_full_count:   0,
            pop_empty_count:   0,
            stack_name,
        };
        instance.empty_guard.lock();
        instance.full_guard.lock();
        instance
    }

    #[inline(always)]
    fn push(&self, element: SlotType) -> bool {
        let mutable_self = unsafe { &mut *(*(self as *const Self as *const std::cell::UnsafeCell<Self>)).get() };
        self.concurrency_guard.lock();
        while self.head >= BUFFER_SIZE as u32 {
            let no_longer_full = mutable_self.report_full();
            if !no_longer_full {
                return false;
            }
        }
        mutable_self.buffer[self.head as usize] = element;
        mutable_self.head += 1;
        if mutable_self.head == 1 {
            self.report_no_longer_empty();
        }
        if METRICS {
            mutable_self.push_count += 1;
        }
        unsafe {self.concurrency_guard.unlock()}
        if DEBUG {
            eprintln!("### PUSH: [#{}] = {:?} -- '{}'", self.head-1, element, self.stack_name);
        }
        true
    }

    #[inline(always)]
    fn pop(&self) -> Option<SlotType> {
        let mutable_self = unsafe { &mut *(*(self as *const Self as *const std::cell::UnsafeCell<Self>)).get() };
        self.concurrency_guard.lock();
        while self.head == 0 {
            let no_longer_empty = mutable_self.report_empty();
            if !no_longer_empty {
                return None;
            }
        }
        let element = self.buffer[self.head as usize - 1];
        mutable_self.head -= 1;
        if self.head == BUFFER_SIZE as u32 - 1 {
            self.report_no_longer_full();
        }
        if METRICS {
            mutable_self.pop_count += 1;
        }
        unsafe {self.concurrency_guard.unlock()};
        if DEBUG {
            eprintln!("### POP: [#{}] == {:?} -- '{}'", self.head, element, self.stack_name);
        }
        Some(element)
    }

    #[inline(always)]
    fn len(&self) -> usize {
        self.head as usize
    }

    fn is_empty(&self) -> bool {
        self.head == 0
    }

    fn buffer_size(&self) -> usize {
        BUFFER_SIZE
    }

    fn debug_enabled(&self) -> bool {
        DEBUG
    }

    fn metrics_enabled(&self) -> bool {
        METRICS
    }

    fn stack_name(&self) -> &str {
        self.stack_name.as_ref()
    }

    fn implementation_name(&self) -> &str {
        "non_blocking_full_sync_stack"
    }

}


#[cfg(any(test,doc))]
mod tests {

    //! Unit tests for [stack](super) module

    use super::*;
    use super::super::super::test_commons::{self,ContainerKind,Blocking};


    #[cfg_attr(not(doc),test)]
    fn basic_stack_use_cases() {
        let stack = Stack::<i32, {1<<4}, false, false, 1024>::new("'basic_use_cases' test stack".to_string());
        test_commons::basic_container_use_cases(stack.stack_name(), ContainerKind::Stack, Blocking::Blocking, stack.buffer_size(),
                                                |e| stack.push(e), || stack.pop(), || stack.len());
    }

    #[cfg_attr(not(doc),test)]
    fn single_producer_multiple_consumers() {
        let stack = Stack::<u32, {1<<16}, false, false, 1024>::new("single_producer_multiple_consumers test stack".to_string());
        test_commons::container_single_producer_multiple_consumers(stack.stack_name(), |e| stack.push(e), || stack.pop());
    }

    #[cfg_attr(not(doc),test)]
    fn multiple_producers_single_consumer() {
        let stack = Stack::<u32, {1<<16}, false, false, 1024>::new("multiple_producers_single_consumer test stack".to_string());
        test_commons::container_multiple_producers_single_consumer(stack.stack_name(), |e| stack.push(e), || stack.pop());
    }

    #[cfg_attr(not(doc),test)]
    fn multiple_producers_and_consumers_all_in_and_out() {
        let stack = Stack::<u32, {1<<16}, false, false, 1024>::new("multiple_producers_and_consumers_all_in_and_out test stack".to_string());
        test_commons::container_multiple_producers_and_consumers_all_in_and_out(stack.stack_name(), Blocking::Blocking, stack.buffer_size(), |e| stack.push(e), || stack.pop());
    }

    #[cfg_attr(not(doc),test)]
    fn multiple_producers_and_consumers_single_in_and_out() {
        let stack = Stack::<u32, {1<<16}, false, false, 1024>::new("concurrency_single_in_and_out test stack".to_string());
        test_commons::container_multiple_producers_and_consumers_single_in_and_out(stack.stack_name(), |e| stack.push(e), || stack.pop());
    }

}
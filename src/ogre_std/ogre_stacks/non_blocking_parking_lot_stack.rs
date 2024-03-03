//! multiple producer / multiple consumer non-blocking stack with a full locking synchronization --
//! only a single push (or pop) may execute the critical region at a time

use super::super::ogre_stacks::OgreStack;
use std::{
    fmt::Debug,
    mem::MaybeUninit,
};
use parking_lot::lock_api::RawMutex as RawMutex_api;
use parking_lot::RawMutex;


#[repr(C,align(64))]      // aligned to cache line sizes to avoid false-sharing performance degradation
pub struct Stack<SlotType, const BUFFER_SIZE: usize, const METRICS: bool, const DEBUG: bool> {
    buffer:     [SlotType; BUFFER_SIZE],
    head:       u32,
    /// guards the code's critical regions to allow concurrency
    concurrency_guard: RawMutex,
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
impl<SlotType: Copy+Debug, const BUFFER_SIZE: usize, const METRICS: bool, const DEBUG: bool>
    OgreStack<SlotType> for Stack<SlotType, BUFFER_SIZE, METRICS, DEBUG> {

    fn new(stack_name: String) -> Self {
        Self {
            buffer:            unsafe {MaybeUninit::zeroed().assume_init()},
            concurrency_guard: RawMutex::INIT,
            head:              0,
            push_count:        0,
            pop_count:         0,
            push_collisions:   0,
            pop_collisions:    0,
            push_full_count:   0,
            pop_empty_count:   0,
            stack_name,
        }
    }

    #[inline(always)]
    fn push(&self, element: SlotType) -> bool {
        let mutable_self = unsafe { &mut *(*(self as *const Self as *const std::cell::UnsafeCell<Self>)).get() };
        self.concurrency_guard.lock();
        if self.head >= BUFFER_SIZE as u32 {
            if METRICS {
                mutable_self.push_full_count += 1;
            }
            unsafe {self.concurrency_guard.unlock()};
            return false;
        }
        mutable_self.buffer[self.head as usize] = element;
        mutable_self.head += 1;
        if METRICS {
            mutable_self.push_count += 1;
        }
        unsafe {self.concurrency_guard.unlock()};
        if DEBUG {
            eprintln!("### PUSH: [#{}] = {:?} -- '{}'", self.head-1, element, self.stack_name);
        }
        true
    }

    #[inline(always)]
    fn pop(&self) -> Option<SlotType> {
        let mutable_self = unsafe { &mut *(*(self as *const Self as *const std::cell::UnsafeCell<Self>)).get() };
        self.concurrency_guard.lock();
        if self.head == 0 {
            if METRICS {
                mutable_self.pop_empty_count += 1;
            }
            unsafe {self.concurrency_guard.unlock()};
            return None;
        }
        let element = self.buffer[self.head as usize - 1];
        mutable_self.head -= 1;
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
        let stack = Stack::<i32, {1<<4}, false, false>::new("'basic_use_cases' test stack".to_string());
        test_commons::basic_container_use_cases(stack.stack_name(), ContainerKind::Stack, Blocking::NonBlocking, stack.buffer_size(),
                                                |e| stack.push(e), || stack.pop(), || stack.len());
    }

    #[cfg_attr(not(doc),test)]
    fn single_producer_multiple_consumers() {
        let stack = Stack::<u32, {1<<16}, false, false>::new("single_producer_multiple_consumers test stack".to_string());
        test_commons::container_single_producer_multiple_consumers(stack.stack_name(), |e| stack.push(e), || stack.pop());
    }

    #[cfg_attr(not(doc),test)]
    fn multiple_producers_single_consumer() {
        let stack = Stack::<u32, {1<<16}, false, false>::new("multiple_producers_single_consumer test stack".to_string());
        test_commons::container_multiple_producers_single_consumer(stack.stack_name(), |e| stack.push(e), || stack.pop());
    }

    #[cfg_attr(not(doc),test)]
    fn multiple_producers_and_consumers_all_in_and_out() {
        let stack = Stack::<u32, {1<<16}, false, false>::new("multiple_producers_and_consumers_all_in_and_out test stack".to_string());
        test_commons::container_multiple_producers_and_consumers_all_in_and_out(stack.stack_name(), Blocking::NonBlocking, stack.buffer_size(), |e| stack.push(e), || stack.pop());
    }

    #[cfg_attr(not(doc),test)]
    fn multiple_producers_and_consumers_single_in_and_out() {
        let stack = Stack::<u32, {1<<16}, false, false>::new("concurrency_single_in_and_out test stack".to_string());
        test_commons::container_multiple_producers_and_consumers_single_in_and_out(stack.stack_name(), |e| stack.push(e), || stack.pop());
    }

}
use crate::ogre_std::{
    ogre_stacks::OgreStack,
    ogre_queues::OgreQueue,
};
use std::{
    fmt::Debug,
    sync::atomic::{AtomicU16,AtomicU32,Ordering},
    mem::{MaybeUninit, ManuallyDrop},
    pin::Pin,
};
use log::error;

/// the basis for a "reference counted buffered allocator"
pub trait ReferenceCountedBufferAllocator<SlotType, USlots:Copy+Debug+OgreFrom<usize>+OgreInto<usize>, URefs> {

    fn new() -> Self;

    /// allocates a slot from 'buffer', returning a reference to it, informing it needs to be
    /// [ref_dec()]'ed 'ref_count' times before it can be really freed (i.e., set for reuse).
    /// Returns 'None' if there are no more free slots
    fn allocate(&self, ref_count: URefs) -> Option<&mut SlotType>;

    /// decreases the reference counter for a 'slot' previously allocated by this trait.
    /// When the counter associated with that 'slot' reaches zero, the 'slot' will be marked as
    /// "reusable" (i.e., freed) and this method will return true
    fn ref_dec(&self, slot: &SlotType) -> bool;

    /// similar to [ref_dec()], but calls `on_free()` if and as soon as it was determined that the slot
    /// will be set to be reused -- `on_free()` is called while the `slot` reference is still valid,
    /// so any custom `drop()` methods may be called for `&slot`
    fn ref_dec_callback(&self, slot: &SlotType, on_free: impl FnMut(&SlotType) -> Option<SlotType>) -> Option<SlotType>;

    /// mark the slot as reusable (or free) regardless of the reference_counter
    fn deallocate_immediately(&self, slot: &SlotType) -> URefs;

    /// returns the number of slots waiting for the right number of calls to [ref_dec()]
    fn used_slots_count(&self) -> usize;

    /// returns the number of slots available for [allocate()]
    fn free_slots_count(&self) -> usize;

    fn slot_id_from_slot(&self, slot: &SlotType) -> USlots;

    fn slot_from_slot_id(&self, slot_id: USlots) -> &SlotType;
}

/// this is the default buffer allocator -- uses the fastest (benchmarked) stack or queue implementation -- either blocking or non-blocking
pub type ReferenceCountedAllocator<SlotType, USlots, const STACK_SIZE: usize> = ReferenceCountedNonBlockingCustomStackAllocator<SlotType, super::ogre_stacks::non_blocking_atomic_stack::Stack<USlots, STACK_SIZE, false, false>, STACK_SIZE>;

/// this is the default blocking buffer allocator -- uses the fastest (benchmarked) stack or queue implementation
pub type ReferenceCountedBlockingAllocator<SlotType, USlots, const STACK_SIZE: usize> = ReferenceCountedNonBlockingCustomStackAllocator<SlotType, super::ogre_stacks::blocking_stack::Stack<USlots, STACK_SIZE, false, false, 0>, STACK_SIZE>;


/// Implements a [ReferenceCountedBufferAllocator] using a non-blocking queue as the backend for the buffer.\
/// `USlots` is the shortest numeric type to accomodate `BUFFER_SIZE`
pub struct ReferenceCountedNonBlockingCustomQueueAllocator<SlotType, Queue, const BUFFER_SIZE: usize> {
    /// convention: all free slots will be on this queue
    free_slots: Pin<Box<Queue>>,
    buffer:     [ManuallyDrop<SlotType>; BUFFER_SIZE],
    counters:   [AtomicU32; BUFFER_SIZE],
}
impl<'a, _T1, _T2, const _C1: usize> ReferenceCountedNonBlockingCustomQueueAllocator<_T1, _T2, _C1> {
    #[cfg(test)]
    const DEBUG: bool = true;
    #[cfg(not(test))]
    const DEBUG: bool = false;
}
impl<SlotType,
     USlots:            Copy + Debug + OgreFrom<usize> + OgreInto<usize>,
     Queue:             OgreQueue<USlots>,
     const BUFFER_SIZE: usize>
ReferenceCountedBufferAllocator<SlotType, USlots, u32>
for ReferenceCountedNonBlockingCustomQueueAllocator<SlotType, Queue, BUFFER_SIZE> where usize: OgreFrom<USlots> {

    fn new() -> Self {
        let instance = Self {
            free_slots: Queue::new("ReferenceCountedNonBlockingCustomQueueAllocator's pool queue".to_owned()),
            buffer:     unsafe { MaybeUninit::zeroed().assume_init() },
            counters:   unsafe { MaybeUninit::zeroed().assume_init() },
        };
        for free_slot_id in 0..BUFFER_SIZE {
            instance.free_slots.enqueue(free_slot_id._into());
        }
        instance
    }

    #[inline(always)]
    fn allocate(&self, ref_count: u32) -> Option<&mut SlotType> {
        match self.free_slots.dequeue() {
            None => None,
            Some(slot_id) => {
                let mutable_buffer = unsafe {
                    let const_ptr = self.buffer.as_ptr();
                    let mut_ptr = const_ptr as *mut [SlotType; BUFFER_SIZE];
                    &mut *mut_ptr
                };
                self.counters[slot_id._into()].store(ref_count, Ordering::Relaxed);
                Some(&mut mutable_buffer[slot_id._into()])
            },
        }
    }

    fn ref_dec(&self, slot: &SlotType) -> bool {
        let mut freed = false;
        self.ref_dec_callback(slot, |_| {
            freed = true;
            None
        });
        freed
    }

    #[inline(always)]
    fn ref_dec_callback(&self, slot: &SlotType, mut on_free: impl FnMut(&SlotType) -> Option<SlotType>) -> Option<SlotType> {
        let slot_id = self.slot_id_from_slot(slot);
        let previous_ref_count = self.counters[slot_id._into()].fetch_sub(1, Ordering::Relaxed);
        if previous_ref_count == 1 {
            let opaque_slot = on_free(slot);
            self.free_slots.enqueue(slot_id);
            opaque_slot
        } else if Self::DEBUG && previous_ref_count == 0 {
            let message = "Calls to reference counted buffer allocator's ref_dec() superseded the expected amount";
            error!("{}", message);
            panic!("{}", message);
        } else {
            None
        }
    }

    fn deallocate_immediately(&self, slot: &SlotType) -> u32 {
        let slot_id = self.slot_id_from_slot(slot);
        let previous_ref_count = self.counters[slot_id._into()].swap(0, Ordering::Relaxed);
        if previous_ref_count > 0 {
            self.free_slots.enqueue(slot_id);
        }
        previous_ref_count
    }

    fn used_slots_count(&self) -> usize {
        BUFFER_SIZE - self.free_slots.len()
    }

    fn free_slots_count(&self) -> usize {
        self.free_slots.len()
    }

    fn slot_id_from_slot(&self, slot: &SlotType) -> USlots {
        (( unsafe { (slot as *const SlotType).offset_from(self.buffer.as_ptr() as *const SlotType) } ) as usize)._into()
    }

    fn slot_from_slot_id(&self, slot_id: USlots) -> &SlotType {
        &self.buffer[slot_id._into()]
    }
}


/// Implements a [ReferenceCountedBufferAllocator] using a Non-blocking stack as the backend for the buffer
/// caveat: USlots should be big enough to hold STACK_SIZE
pub struct ReferenceCountedNonBlockingCustomStackAllocator<SlotType, Stack, const STACK_SIZE: usize> {
    /// convention: all free 'buffer' positions will be on this stack
    free_slots:  Stack,
    buffer:      [SlotType; STACK_SIZE],
    counters:    [AtomicU16; STACK_SIZE],
}
impl<'a, _T1, _T2, const _C1: usize> ReferenceCountedNonBlockingCustomStackAllocator<_T1, _T2, _C1> {
    #[cfg(test)]
    const DEBUG: bool = true;
    #[cfg(not(test))]
    const DEBUG: bool = false;
}
impl<SlotType, USlots:Copy+Debug+OgreFrom<usize>+OgreInto<usize>, Stack: OgreStack<USlots>, const STACK_SIZE: usize>
ReferenceCountedBufferAllocator<SlotType, USlots, u16>
for ReferenceCountedNonBlockingCustomStackAllocator<SlotType, Stack, STACK_SIZE> where usize: OgreFrom<USlots> {

    fn new() -> Self {
        let instance = Self {
            free_slots: Stack::new("ReferenceCountedStackAllocator's pool stack".to_owned()),
            buffer:     unsafe {MaybeUninit::zeroed().assume_init()},
            counters:   unsafe {MaybeUninit::zeroed().assume_init()},
        };
        for free_slot_id in (0..STACK_SIZE).rev() {     // reversed order here so the first the allocation produce slots in increasing RAM address order
            instance.free_slots.push(free_slot_id._into());
        }
        instance
    }

    fn allocate(&self, ref_count: u16) -> Option<&mut SlotType> {
        match self.free_slots.pop() {
            None => None,
            Some(slot_id) => {
                let mutable_buffer = unsafe {
                    let const_ptr = self.buffer.as_ptr();
                    let mut_ptr = const_ptr as *mut [SlotType; STACK_SIZE];
                    &mut *mut_ptr
                };
                self.counters[slot_id._into()].store(ref_count, Ordering::Relaxed);
                Some(&mut mutable_buffer[slot_id._into()])
            },
        }
    }

    fn ref_dec(&self, slot: &SlotType) -> bool {
        let mut freed = false;
        self.ref_dec_callback(slot, |_| {
            freed = true;
            None
        });
        freed
    }

    fn ref_dec_callback(&self, slot: &SlotType, mut on_free: impl FnMut(&SlotType) -> Option<SlotType>) -> Option<SlotType> {
        let slot_id = self.slot_id_from_slot(slot);
        let previous_ref_count = self.counters[slot_id._into()].fetch_sub(1, Ordering::Relaxed);
        if previous_ref_count == 1 {
            let opaque_slot = on_free(slot);
            self.free_slots.push(slot_id);
            opaque_slot
        } else if Self::DEBUG && previous_ref_count == 0 {
            let message = "Calls to reference counted buffer allocator's ref_dec() superseded the expected amount";
            error!("{}", message);
            panic!("{}", message);
        } else {
            None
        }
    }

    fn deallocate_immediately(&self, slot: &SlotType) -> u16 {
        let slot_id = self.slot_id_from_slot(slot);
        let previous_ref_count = self.counters[slot_id._into()].swap(0, Ordering::Relaxed);
        if previous_ref_count > 0 {
            self.free_slots.push(slot_id);
        }
        previous_ref_count
    }

    fn used_slots_count(&self) -> usize {
        STACK_SIZE - self.free_slots.len()
    }

    fn free_slots_count(&self) -> usize {
        self.free_slots.len()
    }

    fn slot_id_from_slot(&self, slot: &SlotType) -> USlots {
        (( unsafe { (slot as *const SlotType).offset_from(self.buffer.as_ptr() as *const SlotType) } ) as usize)._into()
    }

    fn slot_from_slot_id(&self, slot_id: USlots) -> &SlotType {
        &self.buffer[slot_id._into()]
    }
}

/// naive: might traverse all the array to allocate an element -- kept around by now in the hope it might be useful for debugging
pub struct RingBufferAllocator<SlotType, const BUFFER_SIZE: usize> {
    buffer:     [SlotType; BUFFER_SIZE],
    counters:   [AtomicU16; BUFFER_SIZE],
    first_free: AtomicU32,
}
impl<SlotType, const BUFFER_SIZE: usize> ReferenceCountedBufferAllocator<SlotType, u32, u16> for RingBufferAllocator<SlotType, BUFFER_SIZE>
    where [AtomicU16; BUFFER_SIZE]: Default {

    fn new() -> Self {
        Self {
            buffer:     unsafe {MaybeUninit::zeroed().assume_init()},
            counters:   Default::default(),
            first_free: AtomicU32::new(0),
        }
    }

    fn allocate(&self, ref_count: u16) -> Option<&mut SlotType> {
        let mutable_buffer = unsafe {
            let const_ptr = self.buffer.as_ptr();
            let mut_ptr = const_ptr as *mut [SlotType; BUFFER_SIZE];
            &mut *mut_ptr
        };
        for _ in 0..BUFFER_SIZE {
            let index = self.first_free.fetch_add(1, Ordering::Relaxed) % (BUFFER_SIZE as u32);
            match self.counters[index as usize].compare_exchange(0, ref_count, Ordering::Release, Ordering::Relaxed) {
                Ok(_) => return Some(&mut mutable_buffer[index as usize]),
                Err(_reloaded_val) => continue,
            }
        }
        panic!("Allocator is full");
    }

    fn ref_dec(&self, slot: &SlotType) -> bool {
        let mut freed = false;
        self.ref_dec_callback(slot, |_| {
            freed = true;
            None
        });
        freed
    }

    fn ref_dec_callback(&self, slot: &SlotType, mut on_free: impl FnMut(&SlotType) -> Option<SlotType>) -> Option<SlotType> {
        let slot_id = unsafe { (slot as *const SlotType).offset_from(self.buffer.as_ptr() as *const SlotType) };
        let previous_count = self.counters[slot_id as usize].fetch_sub(1, Ordering::Relaxed);
        if previous_count == 1 {
            let opaque_slot = on_free(slot);
            let current_first_free = self.first_free.load(Ordering::Relaxed);
            if (slot_id as u32) < current_first_free {
                self.first_free.store(slot_id as u32, Ordering::Relaxed);
            }
            opaque_slot
        } else {
            None
        }
    }

    fn deallocate_immediately(&self, slot: &SlotType) -> u16 {
        let slot_id = unsafe { (slot as *const SlotType).offset_from(self.buffer.as_ptr() as *const SlotType) };
        let previous_ref_count = self.counters[slot_id as usize].swap(0, Ordering::Relaxed);
        let current_first_free = self.first_free.load(Ordering::Relaxed);
        if (slot_id as u32) < current_first_free {
            self.first_free.store(slot_id as u32, Ordering::Relaxed);
        }
        previous_ref_count
    }

    fn used_slots_count(&self) -> usize {
        todo!()
    }

    fn free_slots_count(&self) -> usize {
        todo!()
    }

    fn slot_id_from_slot(&self, slot: &SlotType) -> u32 {
        ( unsafe { (slot as *const SlotType).offset_from(self.buffer.as_ptr() as *const SlotType) } ) as u32
    }

    fn slot_from_slot_id(&self, slot_id: u32) -> &SlotType {
        let buffer = unsafe {
            let const_ptr = self.buffer.as_ptr();
            let ptr = const_ptr as *const [SlotType; BUFFER_SIZE];
            &*ptr
        };
        &buffer[slot_id as usize]
    }
}

// TODO currently needed to avoid the non-zero cost TryFrom
pub trait OgreFrom<S> {
    fn _from(value: S) -> Self;
}
impl OgreFrom<usize> for u16 {
    // note: this method performs a zero-cost conversion, but might silently fail if usize is big enough
    fn _from(value: usize) -> Self {
        value as Self
    }
}
impl OgreFrom<usize> for u32 {
    // note: this method performs a zero-cost conversion, but might silently fail if the usize value is big enough
    fn _from(value: usize) -> Self {
        value as Self
    }
}
impl OgreFrom<u16> for usize {
    fn _from(value: u16) -> Self {
        value as Self
    }
}
impl OgreFrom<u32> for usize {
    fn _from(value: u32) -> Self {
        value as Self
    }
}
pub trait OgreInto<T>: Sized {
    fn _into(self) -> T;
}
impl<T:OgreFrom<Self>> OgreInto<T> for usize {
    fn _into(self) -> T {
        T::_from(self)
    }
}
impl<T:OgreFrom<Self>> OgreInto<T> for u16 {
    fn _into(self) -> T {
        T::_from(self)
    }
}
impl<T:OgreFrom<Self>> OgreInto<T> for u32 {
    fn _into(self) -> T {
        T::_from(self)
    }
}


#[cfg(any(test, feature = "dox"))]
mod test {

    use super::*;


    #[ctor::ctor]
    fn suite_setup() {
        simple_logger::SimpleLogger::new().with_utc_timestamps().init().unwrap_or_else(|_| eprintln!("--> LOGGER WAS ALREADY STARTED"));
    }

    #[cfg_attr(not(feature = "dox"), test)]
    #[should_panic]
    fn excessive_ref_decs() {

        let allocator = ReferenceCountedAllocator::<u32, u16, 1>::new();

        let some_slot = allocator.allocate(1);
        let none_slot = allocator.allocate(1);
        assert!(some_slot.is_some(),                   "first slot should be allocated with no problems");
        assert!(none_slot.is_none(),                   "second slot will not be allocated until the first is freed -- which it was not");
        assert!(allocator.ref_dec(some_slot.unwrap()), "the solo slot should have been freed");

        let some_other_slot = allocator.allocate(1);
        assert!(some_other_slot.is_some(),                                  "third allocation should succeed -- because a release did occur");
        assert!(allocator.ref_dec(some_other_slot.as_ref().unwrap()),  "the solo slot should have been freed, once again");
        assert!(!allocator.ref_dec(some_other_slot.as_ref().unwrap()), "the solo slot is already freed. Either an error or panic should show up");
    }

}
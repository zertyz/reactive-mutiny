//! Resting place for [OgreArrayPoolAllocator]

use crate::{
    ogre_std::{
        ogre_queues::{
            meta_container::MoveContainer,
        },
        ogre_alloc::types::OgreAllocator,
    },
};
use std::{fmt::{Debug, Formatter}, mem::{MaybeUninit}, ptr};
use std::cell::UnsafeCell;
use std::mem::ManuallyDrop;
use std::pin::Pin;


pub struct OgreArrayPoolAllocator<DataType:        Send + Sync,
                                  ContainerType:   MoveContainer<u32>,
                                  const POOL_SIZE: usize> {
    pool:      UnsafeCell<Pin<Box<[ManuallyDrop<DataType>; POOL_SIZE]>>>,
    free_list: ContainerType,
}


impl<DataType:        Debug + Send + Sync,
     ContainerType:   MoveContainer<u32>,
     const POOL_SIZE: usize>
Debug for
OgreArrayPoolAllocator<DataType, ContainerType, POOL_SIZE> {

    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "OgreArrayPoolAllocator{{used_slots_count: {}, free_slots_count: {}}}",
                  POOL_SIZE - self.free_list.available_elements_count(),
                  self.free_list.available_elements_count())
    }
}


impl<DataType:        Debug + Send + Sync,
     ContainerType:   MoveContainer<u32>,
     const POOL_SIZE: usize>
OgreAllocator<DataType> for
OgreArrayPoolAllocator<DataType, ContainerType, POOL_SIZE> {

    fn new() -> Self {
        Self {
            pool:      UnsafeCell::new(Box::pin([1; POOL_SIZE].map(|v| ManuallyDrop::new(unsafe {
                           let mut slot = MaybeUninit::<DataType>::uninit();
                           slot.as_mut_ptr().write_bytes(v, 1);  // initializing with a non-zero value makes MIRI happily state there isn't any UB involved (actually, any value is equally good)
                           slot.assume_init()
                       })))),
            free_list: {
                           let free_list = ContainerType::new();
                           for slot_id in 0..POOL_SIZE as u32 {
                               free_list.publish_movable(slot_id);
                           }
                           free_list
                       },
        }
    }

    #[inline(always)]
    fn alloc_ref(&self) -> Option<(&mut DataType, u32)> {
        if let Some(slot_id) = self.free_list.consume_movable() {
            let mutable_pool = unsafe { &mut * (self.pool.get() as *mut Box<[DataType; POOL_SIZE]>) };
            let slot_ref = unsafe { mutable_pool.get_unchecked_mut(slot_id as usize) };
            return Some( ( slot_ref, slot_id) );
        }
        None
    }

    #[inline(always)]
    fn alloc_with<F: FnOnce(&mut DataType)>
                 (&self, setter: F)
                 -> Option<(/*ref:*/ &mut DataType, /*slot_id:*/ u32)> {
        self.alloc_ref()
            .map(|(slot_ref, slot_id)| {
                setter(slot_ref);
                (slot_ref, slot_id)
            })
    }

    #[inline(always)]
    fn dealloc_ref(&self, slot: &DataType) {
        let slot_id = self.id_from_ref(slot);
        self.dealloc_id(slot_id)
    }

    #[inline(always)]
    fn dealloc_id(&self, slot_id: u32) {
        if std::mem::needs_drop::<DataType>() {
            unsafe {
                let mut pool = &mut *(self.pool.get() as *mut Box<[DataType; POOL_SIZE]>);
                let mut slot = pool.get_unchecked_mut(slot_id as usize);
                ptr::drop_in_place(slot);
            }
        }
        self.free_list.publish_movable(slot_id);
    }

    #[inline(always)]
    fn id_from_ref(&self, slot: &DataType) -> u32 {
        unsafe {
            let pool = &*(self.pool.get() as *const Box<[DataType; POOL_SIZE]>);
            (slot as *const DataType).offset_from(pool.get_unchecked(0)) as u32
        }
    }

    #[inline(always)]
    fn ref_from_id(&self, slot_id: u32) -> &mut DataType {
        let mutable_pool = unsafe { &mut * (self.pool.get() as *mut Box<[DataType; POOL_SIZE]>) };
        unsafe { mutable_pool.get_unchecked_mut(slot_id as usize % POOL_SIZE) }
    }
}

// TODO: 2023-06-14: Needed while `SyncUnsafeCell` is still not stabilized
unsafe impl<DataType:        Debug + Send + Sync,
           ContainerType:   MoveContainer<u32>,
           const POOL_SIZE: usize>
Send for
OgreArrayPoolAllocator<DataType, ContainerType, POOL_SIZE> {}

// TODO: 2023-06-14: Needed while `SyncUnsafeCell` is still not stabilized
unsafe impl<DataType:        Debug + Send + Sync,
           ContainerType:   MoveContainer<u32>,
           const POOL_SIZE: usize>
Sync for
OgreArrayPoolAllocator<DataType, ContainerType, POOL_SIZE> {}


#[cfg(any(test,doc))]
mod tests {
    //! Unit tests for [ogre_array_pool_allocator](super) module

    use super::*;
    use crate::prelude::advanced::AllocatorAtomicArray;


    #[cfg_attr(not(doc), test)]
    fn refs_and_ids() {
        let allocator = AllocatorAtomicArray::<u128, 128>::new();
        for expected_id in 0..127 {
            let (reference, observed_id1) = match allocator.alloc_ref() {
                Some((reference, slot_id)) => (reference, slot_id),
                None =>                           panic!("Prematurely ran out of elements @ {expected_id}/128"),
            };
            *reference = expected_id as u128;
            assert_eq!(observed_id1, expected_id, "The `slot_id` returned by `.alloc_ref()` doesn't match");
            let observed_id2 = allocator.id_from_ref(reference);
            assert_eq!(observed_id2, expected_id, "The `slot_id` returned by `.id_from_ref()` doesn't match");
            let observed_reference = allocator.ref_from_id(expected_id);
            assert_eq!(*observed_reference, expected_id as u128, "The reference returned by `.ref_from_id()` doesn't match");
        }
    }
}
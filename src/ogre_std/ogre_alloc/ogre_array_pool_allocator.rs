//! Resting place for [OgreArrayPoolAllocator]

use std::fmt::{Debug, Formatter};
use crate::{
    ogre_std::ogre_queues::{
        meta_container::MoveContainer,
        meta_publisher::MovePublisher,
        meta_subscriber::MoveSubscriber,
        full_sync::full_sync_move::FullSyncMove,
    },
};
use std::mem::{ManuallyDrop, MaybeUninit};
use std::pin::Pin;
use crate::ogre_std::ogre_alloc::types::OgreAllocator;


pub struct OgreArrayPoolAllocator<DataType:        Send + Sync,
                                  const POOL_SIZE: usize> {
    pool:      Pin<Box<[ManuallyDrop<DataType>; POOL_SIZE]>>,
    free_list: FullSyncMove<u32, POOL_SIZE>,
}

impl<DataType:        Send + Sync,
     const POOL_SIZE: usize>
OgreArrayPoolAllocator<DataType, POOL_SIZE> {

}


impl<DataType:        Debug + Send + Sync,
     const POOL_SIZE: usize>
Debug
for OgreArrayPoolAllocator<DataType, POOL_SIZE> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "OgreArrayPoolAllocator{{used_slots_count: {}, free_slots_count: {}}}",
                  POOL_SIZE - self.free_list.available_elements_count(),
                  self.free_list.available_elements_count())
    }
}


impl<DataType:        Debug + Send + Sync,
     const POOL_SIZE: usize>
OgreAllocator<DataType>
for OgreArrayPoolAllocator<DataType, POOL_SIZE> {

    fn new() -> Self {
        Self {
            pool:      Box::pin(unsafe { MaybeUninit::zeroed().assume_init() }),
            free_list: {
                           let free_list = FullSyncMove::new();
                           for slot_id in 0..POOL_SIZE as u32 {
                               free_list.publish_movable(slot_id);
                           }
                           free_list
                       },
        }
    }

    #[inline(always)]
    fn alloc(&self) -> Option<(/*ref:*/ &mut DataType, /*slot_id:*/ u32)> {
        let mutable_pool = unsafe {
            let const_ptr = self.pool.as_ptr();
            let mut_ptr = const_ptr as *mut [DataType; POOL_SIZE];
            &mut *mut_ptr
        };
        match self.free_list.consume_movable() {
            Some(slot_id) => {
                Some( ( unsafe{mutable_pool.get_unchecked_mut(slot_id as usize)}, slot_id) )
            },
            None => None,
        }
    }

    #[inline(always)]
    fn dealloc_ref(&self, slot: &mut DataType) {
        let slot_id = self.id_from_ref(slot);
        self.dealloc_id(slot_id)
    }

    #[inline(always)]
    fn dealloc_id(&self, slot_id: u32) {
        self.free_list.publish_movable(slot_id);
    }

    #[inline(always)]
    fn id_from_ref(&self, slot: &DataType) -> u32 {
        (( unsafe { (slot as *const DataType).offset_from(self.pool.as_ptr() as *const DataType) } ) as usize) as u32
    }

    #[inline(always)]
    fn ref_from_id(&self, slot_id: u32) -> &mut DataType {
        let mutable_pool = unsafe {
            let const_ptr = self.pool.as_ptr();
            let mut_ptr = const_ptr as *mut [DataType; POOL_SIZE];
            &mut *mut_ptr
        };
        unsafe { mutable_pool.get_unchecked_mut(slot_id as usize % POOL_SIZE) }
    }
}
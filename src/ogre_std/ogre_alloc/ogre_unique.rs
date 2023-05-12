//! Resting place for [OgreUnique<>]

use super::types::OgreAllocator;
use std::{sync::{
    Arc,
    atomic::AtomicU32,
}, ops::{Deref, DerefMut}, ptr};
use std::fmt::{Debug, Display, Formatter};
use std::marker::{PhantomData};
use std::ptr::NonNull;
use std::sync::atomic;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};


/// Wrapper type for data that requires a custom Drop to be called (through an [OgreAllocator]).
/// Similar to C++'s `unique_ptr`
pub struct OgreUnique<DataType:          Debug + Send + Sync + 'static,
                      OgreAllocatorType: OgreAllocator<DataType> + Send + Sync + 'static> {
    allocator:        &'static OgreAllocatorType,
    data_ref:         &'static DataType,
}


impl<DataType:          Debug + Send + Sync + 'static,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
OgreUnique<DataType, OgreAllocatorType> {

    #[inline(always)]
    pub fn new(data: DataType, allocator: &OgreAllocatorType) -> Option<Self> {
        match allocator.alloc() {
            Some( (slot_ref, _slot_id) ) => {
                unsafe { std::ptr::write(slot_ref, data); }
                Some(Self::from_allocated(slot_ref, allocator))
            }
            None => None
        }
    }

    #[inline(always)]
    pub fn from_allocated(data_ref: &DataType, allocator: &OgreAllocatorType) -> Self {
        Self {
            allocator: unsafe { &*(allocator as *const OgreAllocatorType) },
            data_ref:  unsafe { &*(data_ref as *const DataType) },
        }
    }
}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
Deref
for OgreUnique<DataType, OgreAllocatorType> {

    type Target = DataType;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.data_ref
    }
}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
DerefMut
for OgreUnique<DataType, OgreAllocatorType> {

    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        let mut_data_ref = unsafe {&mut *((self.data_ref as *const DataType) as *mut DataType)};
        mut_data_ref
    }
}


impl<DataType:          Debug + Send + Sync + Display,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
Display
for OgreUnique<DataType, OgreAllocatorType> {

    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.deref(), f)
    }
}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
Debug
for OgreUnique<DataType, OgreAllocatorType> {

    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "OgreUnique {{allocator: {:?}, data: #{}:{:?}}}",
                  self.allocator, self.allocator.id_from_ref(self.data_ref), self.data_ref)
    }
}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
PartialEq<DataType>
for OgreUnique<DataType, OgreAllocatorType>
where DataType: PartialEq {

    fn eq(&self, other: &DataType) -> bool {
        self.deref().eq(other)
    }
}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
Drop
for OgreUnique<DataType, OgreAllocatorType> {

    #[inline(always)]
    fn drop(&mut self) {
        let mut_data_ref = unsafe {&mut *((self.data_ref as *const DataType) as *mut DataType)};
        self.allocator.dealloc_ref(mut_data_ref);
    }
}


unsafe impl<DataType:          Debug + Send + Sync,
            OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
Send
for OgreUnique<DataType, OgreAllocatorType> {}

unsafe impl<DataType:          Debug + Send + Sync,
            OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
Sync
for OgreUnique<DataType, OgreAllocatorType> {}


#[cfg(any(test,doc))]
mod tests {
    //! Unit tests for [ogre_arc](super) module

    use crate::ogre_std::ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator;
    use super::*;


    #[cfg_attr(not(doc),test)]
    pub fn ssas() {
        let allocator = OgreArrayPoolAllocator::<u128, 128>::new();
        let unique = OgreUnique::new(128, &allocator).expect("Allocation should have been done");
        println!("unique is {unique} -- {:?}", unique);
        assert_eq!(*unique.deref(), 128, "Value doesn't match");
        drop(unique);
        // TODO assert that allocator have all elements free (change the trait) and do a similar assertion in `OgreArc`
        println!("{:?}", allocator);
        println!("all is free");
    }

}

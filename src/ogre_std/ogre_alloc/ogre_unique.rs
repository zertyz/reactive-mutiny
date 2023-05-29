//! Resting place for [OgreUnique<>]

use crate::{
    ogre_std::ogre_alloc::{
        ogre_arc::OgreArc,
        OgreAllocator,
    },
};
use std::{
    ops::{Deref, DerefMut},
    fmt::{Debug, Display, Formatter},
};


/// Wrapper type for data that requires a custom Drop to be called (through an [OgreAllocator]).
/// Similar to C++'s `unique_ptr`
pub struct OgreUnique<DataType:          Debug + Send + Sync + 'static,
                      OgreAllocatorType: OgreAllocator<DataType> + Send + Sync + 'static> {
    allocator: &'static OgreAllocatorType,
    data_ref:  &'static DataType,
}


impl<DataType:          Debug + Send + Sync + 'static,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
OgreUnique<DataType, OgreAllocatorType> {

    #[inline(always)]
    pub fn new<F: FnOnce(&mut DataType)>(setter: F, allocator: &OgreAllocatorType) -> Option<Self> {
        allocator.alloc_ref()
            .map(|(slot_ref, _slot_id)| {
                setter(slot_ref);
                Self::from_allocated_ref(slot_ref, allocator)
            })
    }

    #[inline(always)]
    pub fn from_allocated_id(data_id: u32, allocator: &OgreAllocatorType) -> Self {
        let data_ref = allocator.ref_from_id(data_id);
        Self::from_allocated_ref(data_ref, allocator)
    }

    #[inline(always)]
    pub fn from_allocated_ref(data_ref: &DataType, allocator: &OgreAllocatorType) -> Self {
        Self {
            allocator: unsafe { &*(allocator as *const OgreAllocatorType) },
            data_ref:  unsafe { &*(data_ref as *const DataType) },
        }
    }

    #[inline(always)]
    pub fn into_ogre_arc(self) -> OgreArc<DataType, OgreAllocatorType> {
        let undroppable_self = std::mem::ManuallyDrop::new(self);
        OgreArc::from_allocated(undroppable_self.allocator.id_from_ref(undroppable_self.data_ref), undroppable_self.allocator)
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

    #[inline(always)]
    fn eq(&self, other: &DataType) -> bool {
        self.deref().eq(other)
    }
}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
Into<OgreArc<DataType, OgreAllocatorType>>
for OgreUnique<DataType, OgreAllocatorType> {

    #[inline(always)]
    fn into(self) -> OgreArc<DataType, OgreAllocatorType> {
        self.into_ogre_arc()
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

    use super::*;
    use crate::{
        prelude::advanced::AllocatorAtomicArray,
    };


    #[cfg_attr(not(doc),test)]
    pub fn ssas() {
        let allocator = AllocatorAtomicArray::<u128, 128>::new();
        let unique = OgreUnique::new(|slot| *slot = 128, &allocator).expect("Allocation should have been done");
        println!("unique is {unique} -- {:?}", unique);
        assert_eq!(*unique.deref(), 128, "Value doesn't match");
        drop(unique);
        // TODO assert that allocator have all elements free (change the trait) and do a similar assertion in `OgreArc`
        println!("all is free:");
        println!("{:?}", allocator);
    }

    #[cfg_attr(not(doc),test)]
    pub fn into_ogrearc() {
        let allocator = AllocatorAtomicArray::<u128, 128>::new();
        let unique = OgreUnique::new(|slot| *slot = 128, &allocator).expect("Allocation should have been done");
        let ogre_arc = unique.into_ogre_arc();
        println!("arc is {ogre_arc} -- {:?}", ogre_arc);
        assert_eq!((*ogre_arc.deref(), ogre_arc.references_count()), (128, 1), "Starting Value and Reference Counts don't match for the converted `ogre_arc`");
        drop(ogre_arc);
        // TODO assert that allocator have all elements free (change the trait) and do a similar assertion in `OgreArc`
        println!("all is free:");
        println!("{:?}", allocator);
    }

}

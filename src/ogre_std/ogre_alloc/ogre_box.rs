//! Resting place for [OgreBox<>]

use std::ops::{Deref, DerefMut};
use super::types::OgreAllocator;


/// Wrapping type for data to be allocated / freed from the given [OgreAllocator]
pub struct OgreBox<'a, DataType,
                       OgreAllocatorType: OgreAllocator<DataType>> {
    pub data:      &'a mut DataType,
    pub allocator: &'a OgreAllocatorType,
}


impl<'a, DataType: 'static,
         OgreAllocatorType: OgreAllocator<DataType>>
OgreBox<'a, DataType, OgreAllocatorType> {

    /// Zero-copy the `data` into one of the slots provided by `allocator` -- which will be used to deallocate it when the time comes
    /// --  zero-copying will be enforced (if compiled in Release mode) due to this method being inlined in the caller.\
    /// `None` will be returned if there are, currently, no space available for the requested allocation.\
    /// A possible usage pattern for use cases that don't care if we're out of space is:
    /// ```nocompile
    ///         let allocator = <something from ogre_alloc::*>;
    ///         let data = <build your data here>;
    ///         let allocated_data = loop {
    ///             match OgreBox::new(data, allocator) {
    ///                 Some(instance) => break instance,
    ///                 None => <<out_of_elements_code>>,   // sleep, warning, etc...
    ///             }
    ///         }
    #[inline(always)]
    fn new(data: DataType, allocator: &'a OgreAllocatorType) -> Option<Self> {
        match allocator.alloc() {
            Some( (slot_ref, _slot_id) ) => {
                unsafe { std::ptr::write(slot_ref, data); }
                Some(Self {
                    data: slot_ref,
                    allocator,
                })
            }
            None => None
        }
    }

}


impl<'a, DataType,
         OgreAllocatorType: OgreAllocator<DataType>>
Deref
for OgreBox<'a, DataType, OgreAllocatorType> {

    type Target = DataType;

    fn deref(&self) -> &Self::Target {
        self.data
    }
}


impl<'a, DataType,
         OgreAllocatorType: OgreAllocator<DataType>>
DerefMut
for OgreBox<'a, DataType, OgreAllocatorType> {

    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data
    }
}


impl<DataType,
     OgreAllocatorType: OgreAllocator<DataType>>
Drop
for OgreBox<'_, DataType, OgreAllocatorType> {

    fn drop(&mut self) {
        self.allocator.dealloc_ref(self.data);
    }
}
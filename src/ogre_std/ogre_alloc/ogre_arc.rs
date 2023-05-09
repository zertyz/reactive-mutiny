//! Resting place for [OgreArc<>]

use super::types::OgreAllocator;
use std::{
    sync::{
        Arc,
        atomic::AtomicU32,
    },
    ops::{Deref, DerefMut}
};
use std::fmt::{Debug, Display, Formatter};
use std::marker::{PhantomData};
use std::ptr::NonNull;
use std::sync::atomic::Ordering::Relaxed;


/// Wrapper type for data providing an atomic reference counter for dropping control, similar to `Arc`,
/// but allowing a custom allocator to be used -- [OgreAllocator].
/// providing reference counting similar to Arc
pub struct OgreArc<DataType:          Debug,
                   OgreAllocatorType: OgreAllocator<DataType>> {
    inner:    NonNull<InnerOgreArc<DataType, OgreAllocatorType>>,
    _phantom: PhantomData<DataType>,
}


impl<DataType:          Debug + 'static,
     OgreAllocatorType: OgreAllocator<DataType>>
OgreArc<DataType, OgreAllocatorType> {

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
    pub fn new(data: DataType, allocator: &Arc<OgreAllocatorType>) -> Option<Self> {
        match allocator.alloc() {
            Some( (slot_ref, slot_id) ) => {
                unsafe { std::ptr::write(slot_ref, data); }
                Some(Self::from_allocated(slot_id, Arc::clone(allocator)))
            }
            None => None
        }
    }

    /// Wraps `data` with our struct, so it will be properly deallocated when dropped
    /// -- `data` must have been previously allocated by the provided `allocator`
    #[inline(always)]
    pub fn from_allocated(data_id: u32, allocator: Arc<OgreAllocatorType>) -> Self {
        let inner = Box::new(InnerOgreArc {     // TODO: after all is set, replace Box for one of our allocators to check the performance gains
            allocator,
            data_id,
            references_count: AtomicU32::new(1),
            _phantom:         PhantomData::default(),
        });
        Self {
            inner: Box::leak(inner).into(),
            _phantom: Default::default(),
        }
    }

    /// Returns how many `OgreBox<>` copies references the same data as `self` does
    pub fn references_count(&self) -> u32 {
        let inner = unsafe { self.inner.as_ref() };
        inner.references_count.load(Relaxed)
    }

}


impl<DataType:          Debug,
     OgreAllocatorType: OgreAllocator<DataType>>
Deref
for OgreArc<DataType, OgreAllocatorType> {

    type Target = DataType;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        let inner = unsafe { self.inner.as_ref() };
        inner.allocator.ref_from_id(inner.data_id)
    }
}


impl<DataType:          Debug,
     OgreAllocatorType: OgreAllocator<DataType>>
DerefMut
for OgreArc<DataType, OgreAllocatorType> {

    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        let inner = unsafe { self.inner.as_ref() };
        inner.allocator.ref_from_id(inner.data_id)
    }
}


impl<DataType:         Debug,
    OgreAllocatorType: OgreAllocator<DataType>>
Clone
for OgreArc<DataType, OgreAllocatorType> {

    #[inline(always)]
    fn clone(&self) -> Self {
        let inner = unsafe { self.inner.as_ref() };
        inner.references_count.fetch_add(1, Relaxed);
        Self {
            inner: self.inner,
            _phantom: Default::default(),
        }
    }
}


impl<DataType:          Debug + Display,
     OgreAllocatorType: OgreAllocator<DataType>>
Display
for OgreArc<DataType, OgreAllocatorType> {

    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.deref(), f)
    }
}


impl<DataType:          Debug,
     OgreAllocatorType: OgreAllocator<DataType>>
Debug
for OgreArc<DataType, OgreAllocatorType> {

    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let inner = unsafe { self.inner.as_ref() };
        inner.fmt(f)
    }
}


impl<DataType:          Debug,
     OgreAllocatorType: OgreAllocator<DataType>>
Drop
for OgreArc<DataType, OgreAllocatorType> {

    #[inline(always)]
    fn drop(&mut self) {
        let inner = unsafe { self.inner.as_mut() };
        let references = inner.references_count.fetch_sub(1, Relaxed);
        if references <= 1 {
            inner.allocator.dealloc_id(inner.data_id);
            let boxed = unsafe { Box::from_raw(inner) };
            drop(boxed);
        }
    }
}


/// This is the Unique part that cloned [OgreArc]s reference to
struct InnerOgreArc<DataType:          Debug,
                    OgreAllocatorType: OgreAllocator<DataType>> {
    allocator:        Arc<OgreAllocatorType>,
    data_id:          u32,
    references_count: AtomicU32,
    _phantom:         PhantomData<DataType>,
}


impl<DataType:         Debug,
    OgreAllocatorType: OgreAllocator<DataType>>
Debug
for InnerOgreArc<DataType, OgreAllocatorType> {

    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "InnerOgreArc {{allocator: {:?}, data_id: #{}:{:?}, references_count: {}}}",
               self.allocator, self.data_id, self.allocator.ref_from_id(self.data_id), self.references_count.load(Relaxed))
    }
}

#[cfg(any(test,doc))]
mod tests {
    //! Unit tests for [ogre_arc](super) module

    use crate::ogre_std::ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator;
    use super::*;


    #[cfg_attr(not(doc),test)]
    pub fn ssas() {
        let allocator = Arc::new(OgreArrayPoolAllocator::<u128, 128>::new());
        let arc1 = OgreArc::new(128, &allocator).expect("Allocation should have been done");
        println!("arc1 is {arc1} -- {:?}", arc1);
        assert_eq!((*arc1.deref(), arc1.references_count()), (128, 1), "Starting Value and Reference Counts don't match for `arc1`");
        let arc2 = arc1.clone();
        println!("arc2 is {arc2} -- {:?}", arc2);
        assert_eq!((*arc2.deref(), arc2.references_count()), (128, 2), "Cloned Value and Reference Counts don't match for `arc2`");
        drop(arc1);
        println!("arc2 is still {arc2} -- {:?}", arc2);
        assert_eq!((*arc2.deref(), arc2.references_count()), (128, 1), "Value and Reference Counts don't match for `arc2` after dropping `arc1`");
        drop(arc2);
        println!("all is free");
    }

}

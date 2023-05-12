//! Resting place for [OgreArc<>]

use super::types::OgreAllocator;
use std::{sync::{
    Arc,
    atomic::AtomicU32,
}, ops::{Deref, DerefMut}, ptr};
use std::ffi::c_void;
use std::fmt::{Debug, Display, Formatter};
use std::marker::{PhantomData};
use std::ptr::NonNull;
use std::sync::atomic;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use crate::ogre_std::ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator;


/// Wrapper type for data providing an atomic reference counter for dropping control, similar to `Arc`,
/// but allowing a custom allocator to be used -- [OgreAllocator].
/// providing reference counting similar to Arc
pub struct OgreArc<DataType:          Debug + Send + Sync + 'static,
                   OgreAllocatorType: OgreAllocator<DataType> + Send + Sync + 'static> {
    inner:    NonNull<InnerOgreArc<DataType, OgreAllocatorType>>,
}


impl<DataType:          Debug + Send + Sync + 'static,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
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
    pub fn new(data: DataType, allocator: &OgreAllocatorType) -> Option<Self> {
        match allocator.alloc() {
            Some( (slot_ref, slot_id) ) => {
                unsafe { std::ptr::write(slot_ref, data); }
                Some(Self::from_allocated(slot_id, allocator))
            }
            None => None
        }
    }

    /// Wraps `data` with our struct, so it will be properly deallocated when dropped
    /// -- `data` must have been previously allocated by the provided `allocator`
    #[inline(always)]
    pub fn from_allocated(data_id: u32, allocator: &OgreAllocatorType) -> Self {
        let inner = Box::new(InnerOgreArc {     // Using static pool allocators here proved to be slower (a runtime lookup was needed + null check), so we'll stick with Box for now
            allocator: unsafe { &*(allocator as *const OgreAllocatorType) },
            data_id,
            references_count: AtomicU32::new(1),
            _phantom:         PhantomData::default(),
        });
        Self {
            inner: Box::leak(inner).into(),
        }
    }

    /// Returns how many `OgreBox<>` copies references the same data as `self` does
    #[inline(always)]
    pub fn references_count(&self) -> u32 {
        let inner = unsafe { self.inner.as_ref() };
        inner.references_count.load(Relaxed)
    }

}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
Deref
for OgreArc<DataType, OgreAllocatorType> {

    type Target = DataType;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        let inner = unsafe { self.inner.as_ref() };
        inner.allocator.ref_from_id(inner.data_id)
    }
}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
DerefMut
for OgreArc<DataType, OgreAllocatorType> {

    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        let inner = unsafe { self.inner.as_ref() };
        inner.allocator.ref_from_id(inner.data_id)
    }
}


impl<DataType:         Debug + Send + Sync,
    OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
Clone
for OgreArc<DataType, OgreAllocatorType> {

    #[inline(always)]
    fn clone(&self) -> Self {
        let inner = unsafe { self.inner.as_ref() };
        inner.references_count.fetch_add(1, Relaxed);
        Self {
            inner: self.inner,
        }
    }
}


impl<DataType:          Debug + Send + Sync + Display,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
Display
for OgreArc<DataType, OgreAllocatorType> {

    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.deref(), f)
    }
}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
Debug
for OgreArc<DataType, OgreAllocatorType> {

    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let inner = unsafe { self.inner.as_ref() };
        inner.fmt(f)
    }
}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
PartialEq<DataType>
for OgreArc<DataType, OgreAllocatorType>
where DataType: PartialEq {

    fn eq(&self, other: &DataType) -> bool {
        self.deref().eq(other)
    }
}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
Drop
for OgreArc<DataType, OgreAllocatorType> {

    #[inline(always)]
    fn drop(&mut self) {
        let inner = unsafe { self.inner.as_mut() };
        let references = inner.references_count.fetch_sub(1, Release);
        if references != 1 {
            return;
        }
        atomic::fence(Acquire);
        //unsafe { ptr::drop_in_place(Self::get_mut_unchecked(self)) };     // does this belong here on on the allocator? Do a test (using string) to prove its necessity: no leaks should occur
        inner.allocator.dealloc_id(inner.data_id);
        let boxed = unsafe { Box::from_raw(inner) };
        drop(boxed);
    }
}


unsafe impl<DataType:          Debug + Send + Sync,
            OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
Send
for OgreArc<DataType, OgreAllocatorType> {}

unsafe impl<DataType:          Debug + Send + Sync,
            OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
Sync
for OgreArc<DataType, OgreAllocatorType> {}


/// This is the Unique part that cloned [OgreArc]s reference to
struct InnerOgreArc<DataType:          Debug + Send + Sync,
                    OgreAllocatorType: OgreAllocator<DataType> + Send + Sync + 'static> {
    allocator:        &'static OgreAllocatorType,
    data_id:          u32,
    references_count: AtomicU32,
    _phantom:         PhantomData<DataType>,
}


impl<DataType:         Debug + Send + Sync,
    OgreAllocatorType: OgreAllocator<DataType> + Send + Sync>
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
        let allocator = OgreArrayPoolAllocator::<u128, 128>::new();
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
        // TODO assert that allocator have all elements free (change the trait) and do a similar assertion in `OgreUnique`
        println!("{:?}", allocator);
        println!("all is free");
    }

}

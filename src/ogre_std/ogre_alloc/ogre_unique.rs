//! Resting place for [OgreUnique<>]

use crate::ogre_std::ogre_alloc::{
    ogre_arc::OgreArc,
    BoundedOgreAllocator,
    };
use std::{
    ops::Deref,
    fmt::{Debug, Display, Formatter},
};
use std::borrow::Borrow;


/// Wrapper type for data that requires a custom Drop to be called (through an [BoundedOgreAllocator]).
/// Similar to C++'s `unique_ptr`
pub struct OgreUnique<DataType:          Debug + Send + Sync + 'static,
                      OgreAllocatorType: BoundedOgreAllocator<DataType> + Send + Sync + 'static> {
    allocator: &'static OgreAllocatorType,
    data_ref:  &'static DataType,
}


impl<DataType:          Debug + Send + Sync + 'static,
     OgreAllocatorType: BoundedOgreAllocator<DataType> + Send + Sync>
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
     OgreAllocatorType: BoundedOgreAllocator<DataType> + Send + Sync>
Deref for
OgreUnique<DataType, OgreAllocatorType> {

    type Target = DataType;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.data_ref
    }
}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: BoundedOgreAllocator<DataType> + Send + Sync>
AsRef<DataType> for
OgreUnique<DataType, OgreAllocatorType> {

    #[inline(always)]
    fn as_ref(&self) -> &DataType {
        self.data_ref
    }

}

impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: BoundedOgreAllocator<DataType> + Send + Sync>
Borrow<DataType> for
OgreUnique<DataType, OgreAllocatorType> {

    #[inline(always)]
    fn borrow(&self) -> &DataType {
        self.data_ref
    }
}

impl<DataType:          Debug + Send + Sync + Display,
     OgreAllocatorType: BoundedOgreAllocator<DataType> + Send + Sync>
Display for
OgreUnique<DataType, OgreAllocatorType> {

    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.deref(), f)
    }
}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: BoundedOgreAllocator<DataType> + Send + Sync>
Debug for
OgreUnique<DataType, OgreAllocatorType> {

    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "OgreUnique {{allocator: {:?}, data: #{}:{:?}}}",
                  self.allocator, self.allocator.id_from_ref(self.data_ref), self.data_ref)
    }
}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: BoundedOgreAllocator<DataType> + Send + Sync>
PartialEq<DataType> for
OgreUnique<DataType, OgreAllocatorType>
where DataType: PartialEq {

    #[inline(always)]
    fn eq(&self, other: &DataType) -> bool {
        self.deref().eq(other)
    }
}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: BoundedOgreAllocator<DataType> + Send + Sync>
From<OgreUnique<DataType, OgreAllocatorType>> for
OgreArc<DataType, OgreAllocatorType> {

    #[inline(always)]
    fn from(ogre_unique: OgreUnique<DataType, OgreAllocatorType>) -> OgreArc<DataType, OgreAllocatorType> {
        ogre_unique.into_ogre_arc()
    }
}


impl<DataType:          Debug + Send + Sync,
     OgreAllocatorType: BoundedOgreAllocator<DataType> + Send + Sync>
Drop for
OgreUnique<DataType, OgreAllocatorType> {

    #[inline(always)]
    fn drop(&mut self) {
        self.allocator.dealloc_ref(self.data_ref);
    }
}


unsafe impl<DataType:          Debug + Send + Sync,
            OgreAllocatorType: BoundedOgreAllocator<DataType> + Send + Sync>
Send for
OgreUnique<DataType, OgreAllocatorType> {}

unsafe impl<DataType:          Debug + Send + Sync,
            OgreAllocatorType: BoundedOgreAllocator<DataType> + Send + Sync>
Sync for
OgreUnique<DataType, OgreAllocatorType> {}


#[cfg(any(test,doc))]
mod tests {
    //! Unit tests for [ogre_arc](super) module

    use super::*;
    use crate::prelude::advanced::AllocatorAtomicArray;


    #[cfg_attr(not(doc),test)]
    pub fn happy_path_usage() {
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

    #[cfg_attr(not(doc),test)]
    pub fn comparisons() {
        // &str
        let expected: &str = "&str to compare";
        let allocator = AllocatorAtomicArray::<&str, 128>::new();
        let observed = OgreUnique::new(|slot| *slot = expected, &allocator).expect("Allocation should have been done");
        assert_eq!(observed,  expected,  "Comparison of pristine types failed for `&str`");
        assert_eq!(&observed, &expected, "Comparison of references failed for `&str`");
        assert_eq!(*observed, expected,  "Comparison of `Deref` failed for `&str`");

        // String -- notice that, although possible, there is no nice API for "movable values", like `Arc` or `String`: OgreAllocator were not designed for those, therefore, OgreArc, also isn't.
        let expected: String = String::from("String to compare");
        let allocator = AllocatorAtomicArray::<String, 128>::new();
        let observed = OgreUnique::new(|slot| unsafe { std::ptr::write(slot, expected.clone()); }, &allocator).expect("Allocation should have been done");
        assert_eq!(observed,  expected,  "Comparison of pristine types failed for `String`");
        assert_eq!(&observed, &expected, "Comparison of references failed for `String`");
        assert_eq!(*observed, expected,  "Comparison of `Deref` failed for `String`");

        // Wrapped in an `Option<>` (not yet supported as I cannot implement PartialEq for Option<MyType>...)
        // unwrap and compare instead
        // let payload = String::from("Wrapped");
        // let expected = Some(payload.clone());
        // let allocator = AllocatorAtomicArray::<String, 128>::new();
        // let observed = Some(OgreUnique::new(|slot| unsafe { std::ptr::write(slot, payload); }, &allocator).expect("Allocation should have been done"));
        // assert_eq!(observed,  expected,  "Comparison of wrapped types failed for `Option<String>`");
        // assert_eq!(&observed, &expected, "Comparison of references failed for `Option<String>`");
        // assert_eq!(*observed, expected,  "Comparison of `Deref` failed for `Option<String>`");

    }

}

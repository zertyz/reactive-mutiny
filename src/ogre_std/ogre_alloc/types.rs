//! Types used across this module and, possibly, externally (see if exports are done in mod.rs)


use std::fmt::Debug;

/// Dictates how data slots should be acquired and returned for future reuse.\
/// Two APIs are available:
///   - by ref: offers mutable references upon allocation and requiring them for deallocation
///   - by id:  offers u32 ids, which should be translated to mutable references before usage... from there, freeing might either be done from the id or the mut ref
pub trait OgreAllocator<SlotType: Debug>: Debug {

    /// Returns a mutable reference to the newly allocated slot or `None` if the allocator is, currently, out of space
    /// -- in which case, a [dealloc_ref()] would remedy the situation.\
    /// IMPLEMENTORS: #[inline(always)]
    fn alloc(&self) -> Option<(/*ref:*/ &mut SlotType, /*slot_id:*/ u32)>;

    /// Returns the slot for reuse by a subsequent call of [alloc_ref()]\
    /// IMPLEMENTORS: #[inline(always)]
    fn dealloc_ref(&self, slot: &mut SlotType);

    /// IMPLEMENTORS: #[inline(always)]
    fn dealloc_id(&self, slot_id: u32);

    /// returns the position (within the allocator's pool) that the given `slot` reference occupies\
    /// IMPLEMENTORS: #[inline(always)]
    fn id_from_ref(&self, slot: &SlotType) -> u32;

    /// returns a reference to the slot position pointed to by `slot_id`
    /// IMPLEMENTORS: #[inline(always)]
    fn ref_from_id(&self, slot_id: u32) -> &mut SlotType;
}

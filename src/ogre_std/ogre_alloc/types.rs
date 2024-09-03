//! Types used across this module and, possibly, externally (see if exports are done in mod.rs)


use std::fmt::Debug;
use std::future::Future;

/// Dictates how data slots -- in a bounded allocator -- should be acquired and returned for future reuse.\
/// A bounded allocator differs from a tradicional allocator (like Box) in these fundamental aspects:
///   1) Their allocations are set for a specific type -- `SlotType`
///   2) Their allocated data are made to not exceed certain limits -- hence, "Bounded". The `id` parameters
///      in several methods defined here emphasize this idea.
///   3) They may be implemented backed by Box/malloc or by Arrays -- which offers better allocation performance
///      and less "false sharing" performance degradation).
/// 
/// Four APIs are available:
///   - functional / async: see [Self::alloc_with()] & [Self::alloc_with_async()]. Consider these as the "high level" API usage.
///   - [Self::register_external_alloc()] -- allows some use cases that requires allocations to be done externally
/// 
/// The other usages of the API are to be considered "low level":
///   - by ref: offers mutable references upon allocation and requiring them for deallocation
///   - by id:  offers u32 ids, which should be translated to mutable references before usage... from there, freeing might either be done from the id or the mut ref.
/// 
/// Regarding comparisons, the `PartialEq` trait should be implemented so any wrapper types using the allocator can be compared: two allocators are only equal if they
/// share the same exact address.
pub trait BoundedOgreAllocator<SlotType: Debug>
                              : Debug + PartialEq {
    
    type OwnedSlotType;

    /// Instantiates a new allocator
    fn new() -> Self;

    /// Returns an `AllocatedSlotType` to the newly allocated slot or `None` if the allocator is, currently, out of space
    /// -- in which case, a [dealloc_ref()] would remedy the situation.\
    /// IMPLEMENTORS: #[inline(always)]
    fn alloc_ref(&self) -> Option<(/*ref:*/ &mut SlotType, /*slot_id:*/ u32)>;

    /// Allocates & sets the data with the provided `setter` callback, which receives a `BorrowedSlotType` to fill with data.\
    /// Returns the `OwnedSlotType` to the newly allocated slot or `None` if the allocator is, currently, out of space
    /// -- in which case, a [dealloc()] would, typically, remedy the situation.\
    /// IMPLEMENTORS: #[inline(always)]
    fn alloc_with(&self,
                  setter: impl FnOnce(&mut SlotType))
                 -> Option<(/*ref:*/ &mut SlotType, /*slot_id:*/ u32)>;

    /// Same as [Self::alloc_with()], but accepts an `async` closure.
    /// IMPLEMENTORS: #[inline(always)]
    async fn alloc_with_async<'r, Fut: Future<Output=(/*ref:*/ &'r mut SlotType, /*slot_id:*/ u32)>>
                             (&'r self,
                              setter: impl FnOnce(/*ref:*/ &'r mut SlotType, /*slot_id:*/ u32) -> Fut)
                             -> Option<(/*ref:*/ &'r mut SlotType, /*slot_id:*/ u32)>
                             where SlotType: 'r;
    
    /// Registers an externally allocated entry.\
    /// `f()` is guaranteed to be called only if there are available slots.
    /// Note: implementors should only override the default method if they can
    ///       perform the intended behavior with zero-copy.
    ///       Array Allocators are know not to be able to implement this method,
    ///       but Box<> allocators do.
    /// IMPLEMENTORS: #[inline(always)]
    fn with_external_alloc(&self,
                           _f: impl FnOnce() -> Self::OwnedSlotType)
                          -> Option<(/*ref:*/ &mut SlotType, /*slot_id:*/ u32)> {
        panic!("This method is not available to {}, as this couldn't be made zero-copy",
               std::any::type_name::<Self>());
    }

    /// Releases the slot resources, which may be reused upon a subsequent call of [alloc()].\
    /// IMPLEMENTORS: #[inline(always)]
    fn dealloc_ref(&self, slot: &SlotType);

    /// IMPLEMENTORS: #[inline(always)]
    fn dealloc_id(&self, slot_id: u32);

    /// returns the position (within the allocator's pool) that the given `slot` reference occupies\
    /// IMPLEMENTORS: #[inline(always)]
    fn id_from_ref(&self, slot: &SlotType) -> u32;

    /// returns a reference to the slot position pointed to by `slot_id`
    /// IMPLEMENTORS: #[inline(always)]
    #[allow(clippy::mut_from_ref)]
    fn ref_from_id(&self, slot_id: u32) -> &mut SlotType;
}

//! Defines some allocators designed for performance, with a flexible trait.\
//! The motivation for these is use cases where a `Box::new()` (& Drop) would be called too often, for
//! sizes small enough to cause heap fragmentation (knowing that big ones will use `mmap`, which is not subject
//! to such fragmentation).
//!
//! The allocators present here offers a "new arena", if you will, for those usage patterns -- with some nice
//! additions, as the possibility of sharing them with other processes or offloading fat ones to the disk, to avoid swapping.

mod types;
pub use types::{OgreAllocator, OgreBox};
mod types_impls;

pub mod ogre_box;
pub mod ogre_array_pool_allocator;



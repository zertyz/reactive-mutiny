//! Taken from ogre-std while it is not made open-source

pub mod reference_counted_buffer_allocator;
pub mod ogre_queues;
pub mod ogre_stacks;

mod benchmarks;
mod instruments;

#[cfg(any(test, feature = "dox"))]
mod test_commons;

//! Taken from ogre-std while it is not made open-source

pub mod ogre_alloc;
pub mod ogre_queues;
pub mod ogre_stacks;
pub mod ogre_sync;

mod benchmarks;
mod instruments;

#[cfg(any(test,doc))]
mod test_commons;

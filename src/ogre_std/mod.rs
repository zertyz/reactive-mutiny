//! Taken from ogre-std while it is not made open-source

pub mod ogre_queues;
pub mod ogre_stacks;
pub mod ogre_sync;

mod benchmarks;
mod instruments;

#[cfg(any(test, feature = "dox"))]
mod test_commons;

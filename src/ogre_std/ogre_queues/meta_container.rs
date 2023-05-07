//! Resting place for the (rather internal) [MetaContainer] trait

use super::{
    meta_publisher::*,
    meta_subscriber::*,
};

/// Dictates the API for "meta" containers and how they should work.\
/// Meta containers are not proper containers yet: they become "concrete" containers when turned into [blocking_queue::Queue], [non_blocking_queue::Queue] or blocking/non-blocking stacks.\
/// This trait, therefore, exists to allow sharing common code between the concrete implementations.
pub trait MetaContainer<'a, SlotType: 'a>: MetaPublisher<'a, SlotType> + MetaSubscriber<'a, SlotType> {

    /// Instantiates the meta container
    fn new() -> Self;

}

/// Dictates the API for "move" containers and how they should work.\
/// Move containers are the ones that accepts that their payloads are moved around -- in opposition to zero-copy.\
/// See [ZeroCopyContainer]
pub trait MoveContainer<SlotType>: MovePublisher<SlotType> + MoveSubscriber<SlotType> {

    /// Instantiates the container of movable data
    fn new() -> Self;

}
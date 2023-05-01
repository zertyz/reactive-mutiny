//! Resting place for the (rather internal) [MetaContainer] trait

use super::{
    meta_publisher::MetaPublisher,
    meta_subscriber::MetaSubscriber,
};

/// Dictates the API for "meta" containers and how they should work.\
/// Meta containers are not proper containers yet: they become "concrete" containers when turned into [blocking_queue::Queue], [non_blocking_queue::Queue] or blocking/non-blocking stacks.\
/// This trait, therefore, exists to allow sharing common code between the concrete implementations.
pub trait MetaContainer<'a, SlotType: 'a>: MetaPublisher<'a, SlotType> + MetaSubscriber<'a, SlotType> {

    /// Instantiates the meta container
    fn new() -> Self;

}
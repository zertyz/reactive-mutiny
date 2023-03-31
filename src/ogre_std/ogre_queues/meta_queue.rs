//! Resting place for the (rather internal) [MetaQueue] trait

use super::{
    meta_publisher::MetaPublisher,
    meta_subscriber::MetaSubscriber,
};

/// Dictates the API for "meta" queues and how they should work.\
/// Meta queues are not proper queues yet: they become "concrete" queues when turned into [blocking_queue::Queue] or [non_blocking_queue::Queue].\
/// This trait, therefore, exists to allow sharing common code between the mentioned concrete queue implementations.
pub trait MetaQueue<'a, SlotType: 'a>: MetaPublisher<'a, SlotType> + MetaSubscriber<'a, SlotType> {

    /// Instantiates the meta queue
    fn new() -> Self;

}
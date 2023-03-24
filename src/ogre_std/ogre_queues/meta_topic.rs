//! Resting place for the (rather internal) [MetaTopic] trait

use super::{
    meta_publisher::MetaPublisher,
    meta_subscriber::MetaSubscriber,
};

/// Dictates the API for "meta" topics and how they should work.\
/// Topics are like queues but allow multiple & independent consumers -- like consumer-groups in a Kafka queue topic: each "consumer group" will see all available events.\
/// Meta topics are not proper topics yet: they become "concrete" topics when turned into [log_topic::Topic] or [ring_buffer_topic::Topic].
/// This trait, therefore, exists to allow sharing common code between the mentioned concrete topic implementations.
pub trait MetaTopic<SlotType>: MetaPublisher<SlotType> {

    /// Instantiates the meta topic using `mmap_file_path` as the backing storage
    /// and `growing_step_size` as the incrment in the elements it can handle, once creating new space is needed.
    fn new<IntoString: Into<String>>(mmap_file_path: IntoString, growing_step_size: usize);

    // /// Creates a consumer (aka, a consumer group) able to consume elements in parallel with other consumers returned by this method -- each one receiving its own reference to each element.\
    // /// When considering a single consumer returned by this function, multiple callers (from different threads) may consume from it through [MetaSubscriber::consume()] -- but, this time,
    // /// a single element will be given to each caller.\
    // /// Implementors are free to choose if elements will be available from the point this function is called on, or if each subscriber will see all elements ever created (like done in [log_topics::mmap_meta])
    // fn subscribe(&self) -> impl MetaSubscriber<SlotType>; 
    // TODO 2023-03-23: NOT ALLOWED AS OF RUST 1.68: error[E0562]: `impl Trait` only allowed in function and inherent method return types, not in trait method return

}
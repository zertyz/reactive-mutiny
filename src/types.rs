//! Common types across this module

use crate::{
    Instruments,
    stream_executor::StreamExecutor,
    multi,
    uni,
    ogre_std::{ogre_queues, ogre_alloc},
};
use std::{
    task::{Waker},
    fmt::Debug,
};
use std::future::Future;
use std::sync::Arc;


// allocators
pub type AllocatorAtomicArray<InType, const BUFFER_SIZE: usize> = ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator<InType, ogre_queues::atomic::atomic_move::AtomicMove<u32, BUFFER_SIZE>, BUFFER_SIZE>;

// Uni channels
pub type UniAtomicMoveChannel    <InType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = uni::channels::movable::atomic::Atomic      <'static, InType, BUFFER_SIZE, MAX_STREAMS>;
pub type UniCrossbeamMoveChannel <InType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = uni::channels::movable::crossbeam::Crossbeam<'static, InType, BUFFER_SIZE, MAX_STREAMS>;
pub type UniFullSyncMoveChannel  <InType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = uni::channels::movable::full_sync::FullSync <'static, InType, BUFFER_SIZE, MAX_STREAMS>;
pub type UniAtomicZeroCopyChannel<InType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = uni::channels::zero_copy::atomic::Atomic    <'static, InType, AllocatorAtomicArray<InType, BUFFER_SIZE>, BUFFER_SIZE, MAX_STREAMS>;

/// Default 'UniBuilder' for "zero-copying" data that will be shared around
pub type UniZeroCopy<InType,
                     const BUFFER_SIZE: usize,
                     const MAX_STREAMS: usize,
                     const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = uni::UniBuilder<InType,
                      UniAtomicZeroCopyChannel<InType, BUFFER_SIZE, MAX_STREAMS>,
                      INSTRUMENTS,
                      InType>;

/// Default `UniBuilder` for "moving" data around
pub type UniMove<InType,
                 const BUFFER_SIZE: usize,
                 const MAX_STREAMS: usize,
                 const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = uni::UniBuilder<InType,
                      UniFullSyncMoveChannel<InType, BUFFER_SIZE, MAX_STREAMS>,
                      INSTRUMENTS,
                      InType>;

/// Default `Multi`, for ease of use -- the other options can be derived from this one
pub type MultiCrossbeamArcChannel<ItemType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = multi::channels::movable::crossbeam::Crossbeam<'static, ItemType, BUFFER_SIZE, MAX_STREAMS>;
pub type MultiCrossbeamArc<ItemType,
                           const BUFFER_SIZE: usize,
                           const MAX_STREAMS: usize,
                           const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = multi::Multi<'static, ItemType,
                            MultiCrossbeamArcChannel<ItemType, BUFFER_SIZE, MAX_STREAMS>,
                            INSTRUMENTS,
                            Arc<ItemType>>;
pub type MultiArc<ItemType,
                  const BUFFER_SIZE: usize,
                  const MAX_STREAMS: usize,
                  const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}> = MultiCrossbeamArc<ItemType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>;


/// Source of events for [MutinyStream].
pub trait ChannelConsumer<'a, DerivedItemType: 'a + Debug> {

    /// Delivers the next event, whenever the Stream wants it.\
    /// IMPLEMENTORS: use #[inline(always)]
    fn consume(&self, stream_id: u32) -> Option<DerivedItemType>;

    /// Returns `false` if the `Stream` has been signaled to end its operations, causing it to report "out-of-elements" as soon as possible.\
    /// IMPLEMENTORS: use #[inline(always)]
    fn keep_stream_running(&self, stream_id: u32) -> bool;

    /// Shares, to implementors concern, how `stream_id` may be awaken.\
    /// IMPLEMENTORS: use #[inline(always)]
    fn register_stream_waker(&self, stream_id: u32, waker: &Waker);

        /// Reports no more elements will be required through [provide()].\
    /// IMPLEMENTORS: use #[inline(always)]
    fn drop_resources(&self, stream_id: u32);
}


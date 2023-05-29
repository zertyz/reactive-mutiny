//! Extra type aliases & internal types re-exports to allow advanced usage of the `reactive-mutiny` library.\
//! For simple usage, see [crate::prelude].
//!
//! See `all-channels` example.

pub use crate::{
    prelude::*,
    types::{FullDuplexUniChannel, FullDuplexMultiChannel},
    ogre_std::{
        ogre_alloc::{
            OgreAllocator,
            ogre_arc::OgreArc,
            ogre_unique::OgreUnique,
        }
    }
};
use crate::{
    multi,
    uni,
    ogre_std::{
        ogre_queues,
        ogre_alloc,
    },
};
use std::sync::Arc;


// allocators
pub type AllocatorAtomicArray  <InType, const BUFFER_SIZE: usize> = ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator<InType, ogre_queues::atomic::atomic_move::AtomicMove        <u32, BUFFER_SIZE>, BUFFER_SIZE>;
pub type AllocatorFullSyncArray<InType, const BUFFER_SIZE: usize> = ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator<InType, ogre_queues::full_sync::full_sync_move::FullSyncMove<u32, BUFFER_SIZE>, BUFFER_SIZE>;
// TODO: pub type AllocatorBox                                             = ogre_alloc::ogre_box_allocator::OgreBoxAllocator;
//       (Simply uses the Rust's default allocator. Usually, it is slower, is subjected to false-sharing performance degradation and, when used for event payloads, several allocation/deallocations may scatter the free space a little bit,
//        but it has the advantage of not requiring any pre-allocation. So, there are legitimate use cases for this one here. For more info, look at the benchmarks).

// Uni channels
pub type ChannelUniMoveAtomic      <InType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = uni::channels::movable::atomic::Atomic       <'static, InType, BUFFER_SIZE, MAX_STREAMS>;
pub type ChannelUniMoveCrossbeam   <InType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = uni::channels::movable::crossbeam::Crossbeam <'static, InType, BUFFER_SIZE, MAX_STREAMS>;
pub type ChannelUniMoveFullSync    <InType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = uni::channels::movable::full_sync::FullSync  <'static, InType, BUFFER_SIZE, MAX_STREAMS>;
pub type ChannelUniZeroCopyAtomic  <InType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = uni::channels::zero_copy::atomic::Atomic     <'static, InType, AllocatorAtomicArray  <InType, BUFFER_SIZE>, BUFFER_SIZE, MAX_STREAMS>;
pub type ChannelUniZeroCopyFullSync<InType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = uni::channels::zero_copy::full_sync::FullSync<'static, InType, AllocatorFullSyncArray<InType, BUFFER_SIZE>, BUFFER_SIZE, MAX_STREAMS>;

// Unis
pub type UniMoveAtomic<InType,
                       const BUFFER_SIZE: usize,
                       const MAX_STREAMS: usize = 1,
                       const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = uni::UniBuilder<InType, ChannelUniMoveAtomic<InType, BUFFER_SIZE, MAX_STREAMS>, INSTRUMENTS, InType>;
pub type UniMoveCrossbeam<InType,
                          const BUFFER_SIZE: usize,
                          const MAX_STREAMS: usize = 1,
                          const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = uni::UniBuilder<InType, ChannelUniMoveCrossbeam<InType, BUFFER_SIZE, MAX_STREAMS>, INSTRUMENTS, InType>;
pub type UniMoveFullSync<InType,
                         const BUFFER_SIZE: usize,
                         const MAX_STREAMS: usize = 1,
                         const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = uni::UniBuilder<InType, ChannelUniMoveFullSync<InType, BUFFER_SIZE, MAX_STREAMS>, INSTRUMENTS, InType>;
pub type UniZeroCopyAtomic<InType,
                     const BUFFER_SIZE: usize,
                     const MAX_STREAMS: usize = 1,
                     const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = uni::UniBuilder<InType, ChannelUniZeroCopyAtomic<InType, BUFFER_SIZE, MAX_STREAMS>, INSTRUMENTS, OgreUnique<InType, AllocatorAtomicArray<InType, BUFFER_SIZE>>>;
pub type UniZeroCopyFullSync<InType,
                             const BUFFER_SIZE: usize,
                             const MAX_STREAMS: usize = 1,
                             const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = uni::UniBuilder<InType, ChannelUniZeroCopyFullSync<InType, BUFFER_SIZE, MAX_STREAMS>, INSTRUMENTS, OgreUnique<InType, AllocatorFullSyncArray<InType, BUFFER_SIZE>>>;

// Multi channels
pub type ChannelMultiArcAtomic      <ItemType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = multi::channels::arc::atomic::Atomic          <'static, ItemType, BUFFER_SIZE, MAX_STREAMS>;
pub type ChannelMultiArcCrossbeam   <ItemType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = multi::channels::arc::crossbeam::Crossbeam    <'static, ItemType, BUFFER_SIZE, MAX_STREAMS>;
pub type ChannelMultiArcFullSync    <ItemType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = multi::channels::arc::full_sync::FullSync     <'static, ItemType, BUFFER_SIZE, MAX_STREAMS>;
pub type ChannelMultiOgreArcAtomic  <ItemType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = multi::channels::ogre_arc::atomic::Atomic     <'static, ItemType, AllocatorAtomicArray<ItemType, BUFFER_SIZE>, BUFFER_SIZE, MAX_STREAMS>;
pub type ChannelMultiOgreArcFullSync<ItemType, const BUFFER_SIZE: usize, const MAX_STREAMS: usize> = multi::channels::ogre_arc::full_sync::FullSync<'static, ItemType, AllocatorFullSyncArray<ItemType, BUFFER_SIZE>, BUFFER_SIZE, MAX_STREAMS>;
pub type ChannelMultiMmapLog        <ItemType,                           const MAX_STREAMS: usize> = multi::channels::reference::mmap_log::MmapLog <'static, ItemType, MAX_STREAMS>;

// Multis
pub type MultiAtomicArc<ItemType,
                        const BUFFER_SIZE: usize,
                        const MAX_STREAMS: usize,
                        const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = multi::Multi<ItemType,
                   ChannelMultiArcAtomic<ItemType, BUFFER_SIZE, MAX_STREAMS>,
                   INSTRUMENTS,
                   Arc<ItemType>>;
pub type MultiCrossbeamArc<ItemType,
                           const BUFFER_SIZE: usize,
                           const MAX_STREAMS: usize,
                           const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = multi::Multi<ItemType,
                   ChannelMultiArcCrossbeam<ItemType, BUFFER_SIZE, MAX_STREAMS>,
                   INSTRUMENTS,
                   Arc<ItemType>>;
pub type MultiFullSyncArc<ItemType,
                           const BUFFER_SIZE: usize,
                           const MAX_STREAMS: usize,
                           const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = multi::Multi<ItemType,
                   ChannelMultiArcFullSync<ItemType, BUFFER_SIZE, MAX_STREAMS>,
                   INSTRUMENTS,
                   Arc<ItemType>>;
pub type MultiAtomicOgreArc<ItemType,
                            const BUFFER_SIZE: usize,
                            const MAX_STREAMS: usize,
                            const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = multi::Multi<ItemType,
                   ChannelMultiOgreArcAtomic<ItemType, BUFFER_SIZE, MAX_STREAMS>,
                   INSTRUMENTS,
                   OgreArc<ItemType, AllocatorAtomicArray<ItemType, BUFFER_SIZE>>>;
pub type MultiFullSyncOgreArc<ItemType,
                              const BUFFER_SIZE: usize,
                              const MAX_STREAMS: usize,
                              const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = multi::Multi<ItemType,
                   ChannelMultiOgreArcFullSync<ItemType, BUFFER_SIZE, MAX_STREAMS>,
                   INSTRUMENTS,
                   OgreArc<ItemType, AllocatorFullSyncArray<ItemType, BUFFER_SIZE>>>;
pub type MultiMmapLog<ItemType,
                      const MAX_STREAMS: usize,
                      const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = multi::Multi<ItemType,
                   ChannelMultiMmapLog<ItemType, MAX_STREAMS>,
                   INSTRUMENTS,
                   &'static ItemType>;

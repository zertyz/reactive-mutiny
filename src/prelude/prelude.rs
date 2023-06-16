//! See [super]

use super::advanced::*;
pub use crate::{
    instruments::Instruments,
    mutiny_stream::MutinyStream,
    uni::{
        Uni,
        unis_close_async,
    },
    multi::{
        Multi,
        multis_close_async,
    },
    types::*,
};


/// Default 'UniBuilder' for "zero-copying" data that will either be shared around or has a reasonably "big" payload (> 1k)
pub type UniZeroCopy<InType,
                     const BUFFER_SIZE: usize,
                     const MAX_STREAMS: usize,
                     const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = UniZeroCopyAtomic<InType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>;

/// Default `UniBuilder` for "moving" data around -- good for small payloads (< 1k) whose types don't require a custom `Drop` function
pub type UniMove<InType,
                 const BUFFER_SIZE: usize,
                 const MAX_STREAMS: usize,
                 const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}>
    = UniMoveFullSync<InType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>;

/// Default `Multi` for those who wants to use `Arc` as the wrapping type for payloads
pub type MultiArc<ItemType,
                  const BUFFER_SIZE: usize,
                  const MAX_STREAMS: usize,
                  const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}> = MultiAtomicArc<ItemType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>;

/// Default `Multi` for those who want the more performant [OgreArc] as the wrapping type for their payloads
pub type MultiOgreArc<ItemType,
                  const BUFFER_SIZE: usize,
                  const MAX_STREAMS: usize,
                  const INSTRUMENTS: usize = {Instruments::LogsWithMetrics.into()}> = MultiAtomicOgreArc<ItemType, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>;


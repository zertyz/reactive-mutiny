//! Common types across this module

use std::{
    task::{Waker},
    fmt::Debug,
};
use std::future::Future;
use std::sync::Arc;
use crate::Instruments;
use crate::stream_executor::StreamExecutor;


/// Default `UniBuilder`, for ease of use -- to be expanded into all options
pub type UniBuilder<InType,
                    const BUFFER_SIZE: usize,
                    const MAX_STREAMS: usize,
                    const INSTRUMENTS: usize,
                    OnStreamCloseFnType,
                    CloseVoidAsyncType>
    = super::uni::UniBuilder<InType,
                             super::uni::channels::movable::full_sync::FullSync<'static, InType, BUFFER_SIZE, MAX_STREAMS>,
                             INSTRUMENTS,
                             InType,
                             OnStreamCloseFnType,
                             CloseVoidAsyncType>;


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


//! Common types across this module

use std::{
    task::{Waker},
    fmt::Debug,
};


/// Source of events for [UniStream] & [MultiStream]
pub trait MutinyStreamSource<'a, ItemType:        Debug + 'a,
                                 DerivedItemType: 'a = ItemType> {

    /// Delivers the next event, whenever the Stream wants it.\
    /// IMPLEMENTORS: use #[inline(always)]
    fn provide(&self, stream_id: u32) -> Option<DerivedItemType>;

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


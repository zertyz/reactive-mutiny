//! Common types across this module

use crate::{
    uni::Uni,
    multi::Multi,
    ogre_std::ogre_queues::meta_subscriber::MetaSubscriber,
};
use std::{
    pin::Pin,
    task::{Context, Poll, Waker},
    fmt::Debug,
    mem::{MaybeUninit},
    ops::Deref,
    sync::Arc,
};
use futures::stream::Stream;
use owning_ref::ArcRef;

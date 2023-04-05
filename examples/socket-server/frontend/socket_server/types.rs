use super::protocol::{ClientMessages, ServerMessages};
use std::future::Future;
use futures::Stream;

// TODO 2022-09-09 when Rust allows, those complex Stream types might be moved here as types or traits
//                 pub type ProcessorStreamType = Stream<Item = Result<(Endpoint, FromServerMessage), (Endpoint, Box<dyn std::error::Error + Sync + Send>)>> + Send + Sync;
//                 pub trait ProcessorStreamType: Stream<Item = Result<(Endpoint, FromServerMessage), (Endpoint, Box<dyn std::error::Error + Sync + Send>)>> + Send + Sync {}
//                 -- currently, we're not allowed to use "impl" in user defined types
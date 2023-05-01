#![doc = include_str!("../README.md")]

pub mod uni;
pub mod multi;
pub mod stream_executor;

mod instruments;
pub use instruments::Instruments;

mod types;
pub use types::*;

mod incremental_averages;
mod stream;
mod streams_manager;

// pub for criterion usage
pub mod ogre_std;
#![doc = include_str!("../README.md")]

pub mod uni;
pub mod multi;
pub mod stream_executor;
pub mod mutiny_stream;
pub mod prelude;
pub mod types;

mod instruments;

mod incremental_averages;
mod streams_manager;

// pub for criterion usage
pub mod ogre_std;
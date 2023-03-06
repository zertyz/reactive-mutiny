#![doc = include_str!("../README.md")]

pub mod uni;
pub mod multi;
pub mod stream_executor;

mod types;
mod instruments;
mod incremental_averages;
mod stream;

mod ogre_std;
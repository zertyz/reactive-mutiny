//! Application-wide runtime info.
//!
//! The information defined here are created at runtime -- as soon as the program starts -- and is intended to be shared by all tasks,
//! which should be able to read and write to it.
//!
//! Some examples:
//!   * Environment info              -- such as the process executable's file path
//!   * Counters / Metrics / Reports  -- maybe for metrics you'd better use one of the dedicated crates... but you'd define your Job reports here
//!   * Controllers                   -- for instance, handlers for Telegram / Rocket to send push messages and request shutdown
//!   * Injections & globals          -- if you really want it, you may place them here

mod runtime;
pub use runtime::*;
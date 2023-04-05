//! Monitors relevant market events, keeping the last states and making them available for inquiry --
//! while also allowing subscribers for the state changes.

mod market_watcher;
pub use market_watcher::*;
//! Application-wide configuration.
//!
//! The contents of [config_model::Config] are meant to be shared between [frontend] and all business logic components.\
//! This module is able to load & save configuration values from/to a given file -- see it's usage in [main].\
//! ... and there is a contract with [crate::command_line], assuring flags specified there have a higher priority over the ones
//! specified on the configuration file.

pub mod config;
pub mod config_impls;
pub mod config_ops;
pub use config::*;


/// the application name, in case some one needs it
pub const APP_NAME: &str = "kickass-app-template";


/// are we compiled in DEBUG or RELEASE mode?
#[cfg(debug_assertions)]
pub const DEBUG: bool = true;
#[cfg(not(debug_assertions))]
pub const DEBUG: bool = false;
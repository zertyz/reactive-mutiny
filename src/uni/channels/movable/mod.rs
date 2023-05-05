//! Defines channels for Unis that will move data around (instead of [super::zero_copy]ing it).\
//! Basic tests show they are the best performants for payload sizes < 1k.\
//! The Rust Compiler is able, to some extent, to optimize some operations to zero-copy: publishing
//! is subject to this optimization, but consumption isn't.

pub mod atomic;
pub mod crossbeam;
pub mod full_sync;


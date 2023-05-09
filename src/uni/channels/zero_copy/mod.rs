//! Defines our zero-copy channels for Unis, designed to be used with payloads with sizes > 1k,
//! in which case, they will outperform the [movable] channels variants

pub mod atomic;
//pub mod full_sync;
//! Defines our zero-copy channels for Multis, designed to be used with payloads with sizes > (1k / number of Multis),
//! in which case, they will outperform the [movable] channels variants

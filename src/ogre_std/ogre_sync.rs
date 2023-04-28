//! Defines some carefully crafted locking & syncing primitives

use std::sync::atomic::{
    AtomicBool,
    Ordering::{Acquire, Release, Relaxed},
};


/// Returns when the lock was acquired -- inspired by `parking-lot`
/// Unlocked: false; locked: true
#[inline(always)]
pub fn lock(flag: &AtomicBool) {
    // attempt to lock -- spinning for 10 times, relaxing the CPU between attempts
    if flag.compare_exchange_weak(false, true, Acquire, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if flag.compare_exchange_weak(false, true, Acquire, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if flag.compare_exchange_weak(false, true, Acquire, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if flag.compare_exchange_weak(false, true, Acquire, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if flag.compare_exchange_weak(false, true, Acquire, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if flag.compare_exchange_weak(false, true, Acquire, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if flag.compare_exchange_weak(false, true, Acquire, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if flag.compare_exchange_weak(false, true, Acquire, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if flag.compare_exchange_weak(false, true, Acquire, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    if flag.compare_exchange_weak(false, true, Acquire, Relaxed).is_ok() { return } else { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
    // no deal -- fallback without using the _weak version of compare_exchange
    while !flag.compare_exchange(false, true, Acquire, Relaxed).is_ok() { std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop() }
}

/// Releases any locks, returning immediately
#[inline(always)]
pub fn unlock(flag: &AtomicBool) {
    flag.store(false, Release);
}

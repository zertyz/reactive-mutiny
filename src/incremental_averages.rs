//! Provides real-time-ish & lock-free algorithms for computing metrics -- currently, counters and incremental averages.\
//! NOTE regarding real-time-ishness: in all practical cases, real-time is granted, but there is a compare-exchange loop
//!                                   that gets exercised in case of a -- nearly impossible in real workloads -- update collision.

use std::{
    sync::atomic::{
            AtomicU64,
            Ordering::{self,Relaxed},
        },
    mem::ManuallyDrop,
    fmt::{Debug, Formatter},
};


/// Although not exposed, this struct defines the fields and types we compute.\
/// Sadly, we're limited to 32 bits per variable as, as of Rust 1.63, AtomicU128 is not available in x86_64
/// and we're denied from using 'cmpxchg16b' in stable. Things may soon change, 'though:
/// -- see https://github.com/rust-lang/rust/issues/98253
///        https://www.reddit.com/r/rust/comments/s7b009/comment/htbf5fj/?utm_source=share&utm_medium=web2x&context=3
struct IncrementalAveragePair32 {
    counter: u32,
    average: f32,
}

/// Main type for computing a lock-free counter + incremental floating point average -- 32 bits each
pub union AtomicIncrementalAverage64 {
    split:  ManuallyDrop<IncrementalAveragePair32>,
    joined: ManuallyDrop<AtomicU64>,
}

impl AtomicIncrementalAverage64 {

    /// starts a new metrics object
    pub fn new() -> Self {
        Self {
            split: ManuallyDrop::new(IncrementalAveragePair32 {
                counter: 0,
                average: 0.0,
            })
        }
    }

    /// increments the counter and includes 'measurement' in the computation
    /// of the internal incremental average associated with the counter
    /// NOTE: if the counter exceeds the type limit, it will be reset keeping the average
    ///       (currently, the number of resets are not tracked)
    pub fn inc(&self, measurement: f32) {
        self.atomic_compute(Relaxed, Relaxed, |mut counter, average| {
            if counter == u32::MAX {
                // reset the counter to prevent blowing up the average
                // (or panicking, if running in debug mode), while
                // keeping a reasonable "weight", so the average won't fluctuate much
                counter = 100;
            }
            ( ( counter + 1 ),
              ( ( counter as f32 / (1.0 + counter as f32) ) * average ) + ( measurement / (1.0 + counter as f32) )
            )
        });
    }

    /// gets the latest, synchronized `counter` and `average` values\
    /// -- most likely you don't need these -- slightly heavier -- synchronization guarantees: consider using [lightweight_probe()]
    pub fn probe(&self) -> (u32, f32) {
        AtomicIncrementalAverage64::split_joined(unsafe {&self.joined}.load(Relaxed))
    }

    /// gets either the latest (or the ones before last) `counter` and `average` values -- which may come out of sync
    /// (either the `counter` may be the last or before last as well as the `average` may be the last or the one before that)\
    /// -- see [probe()] if getting the synchronized last entry is really important (which is a little bit heavier operation)
    pub fn lightweight_probe(&self) -> (u32, f32) {
        unsafe {
            (self.split.counter, self.split.average)
        }
    }

    /// atomically resets the counter, preserving the existing averages
    /// with the given `weight` (which is unchecked, but should be >= 1).\
    /// this should be used before the counter is about to turn over
    /// (even if the turn over increment doesn't panic, it would ruin the incremental averages calculations,
    /// so this method should be called even for Release compilation)
    fn reset(&self, weight: u32) {
        self.atomic_compute(Relaxed, Relaxed, |_counter, average| (weight, average) );
    }

    /// accessor for the split variant
    fn split(&self) -> &IncrementalAveragePair32 {
        unsafe { &self.split }
    }

    /// accessor for the joined variant
    fn atomic(&self) -> &AtomicU64 {
        unsafe { &self.joined }
    }

    /// take a joined u64 and return its two u32s: (counter,average)
    fn split_joined(joined: u64) -> (u32, f32) {
        ( (joined & ((1<<32)-1)) as u32, f32::from_bits((joined >> 32) as u32) )
    }

    /// take the two u32s (counter,average) and return the joined u64
    fn join_split(counter: u32, average: f32) -> u64 {
        (counter as u64) | ((f32::to_bits(average) as u64) << 32)
    }

    /// atomically compute new values for the two u32s in this union.\
    /// `computation(counter, average)` is called for the current values as many times as needed for the compare-exchange operation to succeed
    /// -- and the new values it returns will replace the ones passed as parameters
    fn atomic_compute(&self, load_ordering: Ordering, store_ordering: Ordering, computation: impl Fn(u32, f32) -> (u32, f32)) {
        unsafe {
            let mut current_joined = self.joined.load(load_ordering);
            loop {
                let (current_counter, current_average) = AtomicIncrementalAverage64::split_joined(current_joined);
                let (new_counter, new_average) = computation(current_counter, current_average);
                let new_joined = AtomicIncrementalAverage64::join_split(new_counter, new_average);
                match self.joined.compare_exchange(current_joined, new_joined, store_ordering, load_ordering) {
                    Err(reloaded_current_joined) => current_joined = reloaded_current_joined,
                    Ok(_) => break,
                }
            }
        }
    }
}

impl Debug for AtomicIncrementalAverage64 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let (counter, average) = self.probe();
        write!(f, "AtomicIncrementalAverage64 {{counter: {}, average: {:.5}}}", counter, average)
    }
}

/// Unit tests the [stream_executors](self) module
#[cfg(any(test,doc))]
mod tests {
    use super::*;

    /// Rudimentarily tests [AtomicIncrementalAverage64]:
    /// counts from 0 to `ELEMENTS` and compares the incremental averages to the theoretical one
    #[cfg_attr(not(doc),test)]
    fn incremental_averages() {
        const ELEMENTS: u32 = 12345679;
        let expected_average = |elements| (elements as f32 - 1.0) / 2.0;

        let rolling_avg = AtomicIncrementalAverage64::new();
        for i in 0..ELEMENTS {
            rolling_avg.inc(i as f32);
            let (observed_count, observed_avg) = rolling_avg.lightweight_probe();
            // do checks along the way... from this, we get an idea of the minimum precision to expect from this module, with the feed in values -- at around ~ 1e-1
            assert_eq!(observed_count, i+1, "count is wrong");
            let delta = (observed_avg-expected_average(i+1)).abs();
            assert!(delta <= 0.207, "incremental average, probed along the way, is wrong (within ~ 10^-1 precision) at element #{i} -- observed: {}; expected: {} / delta: {delta}", observed_avg, expected_average(i+1));
        }
        // end of journey averages check -- 10^-4 precision -- show the rolling averages formula is not divergent, even if other precisions were measured on the way (surely due to f32 precision)
        let (observed_count, observed_avg) = rolling_avg.probe();
        assert_eq!(observed_count, ELEMENTS, "count is wrong");
        assert!((observed_avg-expected_average(ELEMENTS)).abs() <= 1e-4, "average is wrong -- observed: {}; expected: {}", observed_avg, expected_average(ELEMENTS));
    }

}
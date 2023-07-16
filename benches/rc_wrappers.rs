//! Compares the performance of `Arc` with our `OgreArc` and `OgreUnique`.
//!
//! These types are the basis for `Zero-Copy` channels, where the data has to be filled in just 1 time throughout the life of the object.
//!
//! # Analysis 2023-05-xx
//!
//!

use std::{
    sync::Arc,
    hint::black_box,
};
use reactive_mutiny::prelude::advanced::*;
use criterion::{
    criterion_group,
    criterion_main,
    Criterion,
};


// The size of 1 diminishes the effects of CPU caches. On these tests, we simply alloc/dealloc, so
// this allows a fair comparison between `Arc` and the `OgreAllocator` -- be it Stack or Queue based
const BUFFER_SIZE: usize = 1;


/// Measures the creation and dropping times for all of our wrappers of interest
fn bench_creation_and_dropping(criterion: &mut Criterion) {

    let mut group = criterion.benchmark_group("Creation & Dropping");

    let bench_id = format!("Arc");
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        let arc = Arc::new(9758usize);
        drop(arc);
    })));

    let bench_id = format!("OgreArc");
    let allocator = AllocatorFullSyncArray::<usize, BUFFER_SIZE>::new();
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        let ogre_arc = unsafe { OgreArc::new(|slot| *slot = 9758usize, &allocator).unwrap_unchecked() };
        drop(ogre_arc);
    })));

    let bench_id = format!("OgreUnique");
    let allocator = AllocatorFullSyncArray::<usize, BUFFER_SIZE>::new();
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        let ogre_unique = unsafe { OgreUnique::new(|slot| *slot = 9758usize, &allocator).unwrap_unchecked() };
        drop(ogre_unique);
    })));

    group.finish();
}

/// Measures the access times for all of our wrappers of interest
fn bench_deref(criterion: &mut Criterion) {

    let mut group = criterion.benchmark_group("Dereferencing");

    let bench_id = format!("Arc");
    let arc = Arc::new(1usize);
    let mut count = 0;
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc;   // x10
        count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc;   // x10
        count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc;   // x10
        count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc;   // x10
        count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc;   // x10
        count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc;   // x10
        count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc;   // x10
        count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc;   // x10
        count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc;   // x10
        count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc; count += *arc;   // x10
        // x100
    })));
    println!("\n==> Arc: {count}\n");

    let bench_id = format!("OgreArc");
    let allocator = AllocatorFullSyncArray::<usize, BUFFER_SIZE>::new();
    let ogre_arc = unsafe {
        OgreArc::new(|slot| *slot = 1usize, &allocator).unwrap_unchecked()
    };
    let mut count = 0;
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; // x 10
        count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; // x 10
        count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; // x 10
        count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; // x 10
        count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; // x 10
        count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; // x 10
        count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; // x 10
        count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; // x 10
        count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; // x 10
        count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; count += *ogre_arc; // x 10
        // x100
    })));
    println!("\n==> OgreArc: {count}\n");

    let bench_id = format!("OgreUnique");
    let allocator = AllocatorFullSyncArray::<usize, BUFFER_SIZE>::new();
    let ogre_unique = unsafe {
        OgreUnique::new(|slot| *slot = 1usize, &allocator).unwrap_unchecked()
    };
    let mut count = 0;
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique;   // x 10
        count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique;   // x 10
        count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique;   // x 10
        count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique;   // x 10
        count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique;   // x 10
        count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique;   // x 10
        count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique;   // x 10
        count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique;   // x 10
        count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique;   // x 10
        count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique; count += *ogre_unique;   // x 10
        // x100
    })));
    println!("\n==> OgreUnique: {count}\n");

    group.finish();
}

/// Measures the cloning & clone dropping times for all of our wrappers of interest.
/// `OgreUnique` times are not included as they should be made an `OgreArc` before cloning.
fn bench_cloning(criterion: &mut Criterion) {

    let mut group = criterion.benchmark_group("Cloning & Dropping");

    let bench_id = format!("Arc");
    let arc = Arc::new(1usize);
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        drop(arc.clone());
    })));

    let bench_id = format!("OgreArc");
    let allocator = AllocatorFullSyncArray::<usize, BUFFER_SIZE>::new();
    let ogre_arc = unsafe {
        OgreArc::new(|slot| *slot = 1usize, &allocator).unwrap_unchecked()
    };
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        drop(ogre_arc.clone());
    })));

    group.finish();
}

/// Measures any overhead (that shouldn't be there) for sized payloads.\
/// The `Baseline` measurement measures the time to init the data in the payload.\
/// Wrappers are expected to have a small ns addition to it when building, reading the dropping.
fn bench_sized_payloads(criterion: &mut Criterion) {

    let mut group = criterion.benchmark_group("Sized payloads");

    #[derive(Debug)]
    #[repr(align(128))]     // for a stable CPU cache management
    struct SizedPayload {
        value:   u32,
        payload: [u8; 32764],
    }

    let bench_id = format!("Mem-filling Baseline");
    let payload_array = [SizedPayload { value: 0, payload: [0; 32764] }; 1];
    let mut sum = 0;
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        let mutable_array = unsafe {
            let const_ptr = payload_array.as_ptr();
            let mut_ptr = const_ptr as *mut [SizedPayload; 1];
            &mut *mut_ptr
        };
        let slot_ref = unsafe { mutable_array.get_unchecked_mut(0) };
        *slot_ref = SizedPayload {
            value: sum,
            payload: [127; 32764],
        };
        sum += slot_ref.value;
    })));
    println!("Mem-filling Baseline's sum: {sum}");

    let bench_id = format!("Allocator-ref Baseline");
    let allocator = AllocatorFullSyncArray::<SizedPayload, BUFFER_SIZE>::new();
    let mut sum = 0;
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        if let Some((slot_ref, slot_id)) = allocator.alloc_ref() {
            *slot_ref = SizedPayload {
                value: sum,
                payload: [127; 32764],
            };
            sum += slot_ref.value;
            allocator.dealloc_id(slot_id);
        }
    })));
    println!("Allocator-ref Baseline's sum: {sum}");

    let bench_id = format!("Allocator-callback Baseline");
    let allocator = AllocatorFullSyncArray::<SizedPayload, BUFFER_SIZE>::new();
    let mut sum = 0;
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        allocator.alloc_with(|slot| *slot = SizedPayload {
            value: sum,
            payload: [127; 32764],
        }).map(|(slot_ref, slot_id)| {
            sum += slot_ref.value;
            allocator.dealloc_id(slot_id);
        });
    })));
    println!("Allocator-callback Baseline's sum: {sum}");

    let bench_id = format!("Arc");
    let mut sum = 0;
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        let arc = Arc::new(SizedPayload {
            value: sum,
            payload: [127; 32764],
        });
        sum += arc.value;
        drop(arc);
    })));
    println!("Arc's sum: {sum}");

    let bench_id = format!("OgreArc");
    let allocator = AllocatorFullSyncArray::<SizedPayload, BUFFER_SIZE>::new();
    let mut sum = 0;
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box(unsafe {
        let ogre_arc = OgreArc::new(|slot| *slot = SizedPayload {
            value: sum,
            payload: [127; 32764],
        }, &allocator).unwrap_unchecked();
        sum += ogre_arc.value;
        drop(ogre_arc);
    })));
    println!("OgreArc's sum: {sum}");

    let bench_id = format!("OgreUnique");
    let allocator = AllocatorFullSyncArray::<SizedPayload, BUFFER_SIZE>::new();
    let mut sum = 0;
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box(unsafe {
        let ogre_unique = OgreUnique::new(|slot| *slot = SizedPayload {
                value: sum,
                payload: [127; 32764],
        }, &allocator).unwrap_unchecked();
        sum += ogre_unique.value;
        drop(ogre_unique);
    })));
    println!("OgreUnique's sum: {sum}");

    group.finish();
}



criterion_group!(benches, bench_creation_and_dropping, bench_deref, bench_cloning, bench_sized_payloads);
criterion_main!(benches);

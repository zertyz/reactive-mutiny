//! Compares the performance of `Arc` with our `OgreArc` and `OgreUnique`.
//!
//! These types are the basis for `Zero-Copy` channels, where the data has to be filled in just 1 time throughout the life of the object.
//!
//! # Analysis 2023-05-11
//!
//!   Our Atomic is the winner on all tests, for a variety of Intel, AMD and ARM cpus, followed by our FullSync, then Crossbeam.
//!   For Intel(R) Core(TM) i5-10500H CPU, we have the following figures:
//!     - Inter-thread LATENCY:
//!       - baseline:  62ns
//!       - Atomic:    302ns
//!       - FullSync:  378ns
//!       - Crossbeam: 611ns
//!     - Inter-thread THROUGHPUT:
//!       - Atomic:    100µs
//!       - FullSync:  107µs
//!       - Crossbeam: 135µs
//!     - The Same-thread measurements follow the same tendency
//!

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8};
use std::sync::atomic::Ordering::Relaxed;
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkGroup};
use criterion::measurement::WallTime;
use ogre_std::ogre_queues::{
    full_sync::full_sync_move::FullSyncMove,
    atomic::atomic_move::AtomicMove,
};
use reactive_mutiny::{uni, multi, ogre_std};
use futures::{Stream, stream};
use reactive_mutiny::ogre_std::ogre_alloc::ogre_arc::OgreArc;
use reactive_mutiny::ogre_std::ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator;
use reactive_mutiny::ogre_std::ogre_alloc::ogre_unique::OgreUnique;
use reactive_mutiny::ogre_std::ogre_alloc::OgreAllocator;
use reactive_mutiny::ogre_std::ogre_queues::meta_publisher::{MetaPublisher, MovePublisher};
use reactive_mutiny::ogre_std::ogre_queues::meta_container::{MetaContainer, MoveContainer};
use reactive_mutiny::ogre_std::ogre_queues::meta_subscriber::{MetaSubscriber, MoveSubscriber};


const BUFFER_SIZE: usize = 1024;


/// Measures the creation and dropping times for all of our wrappers of interest
fn bench_creation_and_dropping(criterion: &mut Criterion) {

    let mut group = criterion.benchmark_group("Creation & Dropping");

    let bench_id = format!("Arc");
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        let arc = Arc::new(9758usize);
        drop(arc);
    }));

    let bench_id = format!("OgreArc");
    let allocator = OgreArrayPoolAllocator::<usize, BUFFER_SIZE>::new();
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        let ogre_arc = OgreArc::new(9758usize, &allocator);
        drop(ogre_arc);
    }));

    let bench_id = format!("OgreUnique");
    let allocator = OgreArrayPoolAllocator::<usize, BUFFER_SIZE>::new();
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        let ogre_unique = OgreUnique::new(9758usize, &allocator);
        drop(ogre_unique);
    }));

    group.finish();
}

/// Measures the access times for all of our wrappers of interest
fn bench_deref(criterion: &mut Criterion) {

    let mut group = criterion.benchmark_group("Dereferencing");

    let bench_id = format!("Arc");
    let arc = Arc::new(1usize);
    let mut count = 0;
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
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
    }));
    println!("\n==> Arc: {count}\n");

    let bench_id = format!("OgreArc");
    let allocator = OgreArrayPoolAllocator::<usize, BUFFER_SIZE>::new();
    let ogre_arc = OgreArc::new(1usize, &allocator).expect("OgreArc Should have allocated 1 item");
    let mut count = 0;
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
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
    }));
    println!("\n==> OgreArc: {count}\n");

    let bench_id = format!("OgreUnique");
    let allocator = OgreArrayPoolAllocator::<usize, BUFFER_SIZE>::new();
    let ogre_unique = OgreUnique::new(1usize, &allocator).expect("OgreUnique Should have allocated 1 item");
    let mut count = 0;
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
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
    }));
    println!("\n==> OgreUnique: {count}\n");

    group.finish();
}

/// Measures the cloning & clone dropping times for all of our wrappers of interest
fn bench_cloning(criterion: &mut Criterion) {

    let mut group = criterion.benchmark_group("Cloning & Dropping");

    let bench_id = format!("Arc");
    let arc = Arc::new(1usize);
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        drop(arc.clone());
    }));

    let bench_id = format!("OgreArc");
    let allocator = OgreArrayPoolAllocator::<usize, BUFFER_SIZE>::new();
    let ogre_arc = OgreArc::new(1usize, &allocator).expect("OgreArc Should have allocated 1 item");
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        drop(ogre_arc.clone());
    }));

    group.finish();
}


criterion_group!(benches, bench_creation_and_dropping, bench_deref, bench_cloning);
criterion_main!(benches);
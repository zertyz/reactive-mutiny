//! Compares the performance of Rust's community-provided channels with containers from `ogre-std`'s queues & stacks
//!
//! Additionally (and for the sake of mere curiosity), `Multi` channels [multi::channels] are also benchmarked, although they are expected to be
//! slower (for they provide extra functionalities). Anyway, here we'll know "how slow".
//!
//! # Analysis 2023-04-30
//!
//!   - Crossbeam is the winner for Intel(R) Core(TM) i5-10500H CPU, tied with Std & Full Sync
//!
//!

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8};
use std::sync::atomic::Ordering::Relaxed;
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkGroup};
use criterion::measurement::WallTime;
use ogre_std::ogre_queues::{
    full_sync_queues::full_sync_meta::FullSyncMeta,
    atomic_queues::atomic_meta::AtomicMeta,
};
use reactive_mutiny::{uni, multi, ogre_std};
use futures::{Stream, stream};
use reactive_mutiny::ogre_std::ogre_queues::meta_publisher::MetaPublisher;
use reactive_mutiny::ogre_std::ogre_queues::meta_queue::MetaQueue;
use reactive_mutiny::ogre_std::ogre_queues::meta_subscriber::MetaSubscriber;


/// Benchmarks the same-thread latency of our containers against Std, Tokio, Futures & Crossbeam channels.\
/// Latency is measured by the time it takes to send a single element + time to receive that element
fn bench_same_thread_latency(criterion: &mut Criterion) {

    type ItemType = u32;
    const BUFFER_SIZE: usize = 1024;

    let mut group = criterion.benchmark_group("Same-thread LATENCY");

    let full_sync_channel = Arc::new(FullSyncMeta::<ItemType, BUFFER_SIZE>::new());
    let (full_sync_sender, full_sync_receiver) = (full_sync_channel.clone(), full_sync_channel);
    let atomic_channel = Arc::new(AtomicMeta::<ItemType, BUFFER_SIZE>::new());
    let (atomic_sender, atomic_receiver) = (atomic_channel.clone(), atomic_channel);

    let (std_sender, std_receiver) = std::sync::mpsc::sync_channel::<ItemType>(BUFFER_SIZE);
    let (tokio_sender, mut tokio_receiver) = tokio::sync::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (mut futures_sender, mut futures_receiver) = futures::channel::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (crossbeam_sender, crossbeam_receiver) = crossbeam_channel::bounded::<ItemType>(BUFFER_SIZE);

    let bench_id = format!("ogre_std's FullSync queue");
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        while !full_sync_sender.publish(|slot| *slot = 1, || false, |_| {}) {};
        while full_sync_receiver.consume(|slot| *slot, || false, |_| {}).is_none() {};
    }));

    let bench_id = format!("ogre_std's Atomic queue");
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        while !atomic_sender.publish(|slot| *slot = 1, || false, |_| {}) {};
        while atomic_receiver.consume(|slot| *slot, || false, |_| {}).is_none() {};
    }));

    let bench_id = format!("Std MPSC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        while std_sender.try_send(1).is_err() {};
        while std_receiver.try_recv().is_err() {};
    }));

    let bench_id = format!("Tokio MPSC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        while tokio_sender.try_send(1).is_err() {};
        while tokio_receiver.try_recv().is_err() {};
    }));

    let bench_id = format!("Futures MPSC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        while futures_sender.try_send(1).is_err() {};
        while futures_receiver.try_next().is_err() {};
    }));

    let bench_id = format!("Crossbeam MPMC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        while crossbeam_sender.try_send(1).is_err() {};
        while crossbeam_receiver.try_recv().is_err() {};
    }));

    group.finish();
}

/// Benchmarks the inter-thread latency of our containers against Std, Tokio, Futures & Crossbeam channels.\
/// Latency is measured by the receiver thread, which signals the sender thread to produce an item, then
/// waits for the element. At any given time, there are only 2 threads running and the measured times are:\
///   - the time it takes for a thread to signal another one (this is the same for everybody)
///   - + the time for the first thread to receive the element
fn bench_inter_thread_latency(criterion: &mut Criterion) {

    type ItemType = u32;
    const BUFFER_SIZE: usize = 1024;

    let mut group = criterion.benchmark_group("Inter-thread LATENCY");

    let full_sync_channel = Arc::new(FullSyncMeta::<ItemType, BUFFER_SIZE>::new());
    let (full_sync_sender, full_sync_receiver) = (full_sync_channel.clone(), full_sync_channel);
    let atomic_channel = Arc::new(AtomicMeta::<ItemType, BUFFER_SIZE>::new());
    let (atomic_sender, atomic_receiver) = (atomic_channel.clone(), atomic_channel);

    let (std_sender, std_receiver) = std::sync::mpsc::sync_channel::<ItemType>(BUFFER_SIZE);
    let (tokio_sender, mut tokio_receiver) = tokio::sync::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (mut futures_sender, mut futures_receiver) = futures::channel::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (crossbeam_sender, crossbeam_receiver) = crossbeam_channel::bounded::<ItemType>(BUFFER_SIZE);

    baseline_it(&mut group);

    bench_it(&mut group,
             format!("ogre_std's FullSync queue"),
             || while !full_sync_sender.publish(|slot| *slot = 1, || false, |_| {}) {},
             || while full_sync_receiver.consume(|slot| *slot, || false, |_| {}).is_none() {std::hint::spin_loop()});

    bench_it(&mut group,
             format!("ogre_std's Atomic queue"),
             || while !atomic_sender.publish(|slot| *slot = 1, || false, |_| {}) {},
             || while atomic_receiver.consume(|slot| *slot, || false, |_| {}).is_none() {std::hint::spin_loop()});

    bench_it(&mut group,
             format!("Std MPSC Channel"),
             || while std_sender.try_send(1).is_err() {},
             || while std_receiver.try_recv().is_err() {std::hint::spin_loop()});

    bench_it(&mut group,
             format!("Tokio MPSC Channel"),
             || while tokio_sender.try_send(1).is_err() {},
             || while tokio_receiver.try_recv().is_err() {std::hint::spin_loop()});

    bench_it(&mut group,
             format!("Futures MPSC Channel"),
             || while futures_sender.try_send(1).is_err() {},
             || while futures_receiver.try_next().is_err() {std::hint::spin_loop()});

    bench_it(&mut group,
             format!("Crossbeam MPMC Channel"),
             || while crossbeam_sender.try_send(1).is_err() {},
             || while crossbeam_receiver.try_recv().is_err() {std::hint::spin_loop()});

    group.finish();
}

/// Benchmarks the same-thread throughput of our containers against Std, Tokio, Futures & Crossbeam channels.\
/// Throughput is measured by the time it takes to fill the backing buffer with elements + the time to receive all of them
fn bench_same_thread_throughput(criterion: &mut Criterion) {

    type ItemType = u32;
    const BUFFER_SIZE: usize = 1<<18;

    let mut group = criterion.benchmark_group("Same-thread THROUGHPUT");

    let full_sync_channel = Arc::new(FullSyncMeta::<ItemType, BUFFER_SIZE>::new());
    let (full_sync_sender, full_sync_receiver) = (full_sync_channel.clone(), full_sync_channel);
    let atomic_channel = Arc::new(AtomicMeta::<ItemType, BUFFER_SIZE>::new());
    let (atomic_sender, atomic_receiver) = (atomic_channel.clone(), atomic_channel);

    let (std_sender, std_receiver) = std::sync::mpsc::sync_channel::<ItemType>(BUFFER_SIZE);
    let (tokio_sender, mut tokio_receiver) = tokio::sync::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (mut futures_sender, mut futures_receiver) = futures::channel::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (crossbeam_sender, crossbeam_receiver) = crossbeam_channel::bounded::<ItemType>(BUFFER_SIZE);

    let bench_id = format!("ogre_std's FullSync queue");
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        for _ in 0..BUFFER_SIZE {
            while !full_sync_sender.publish(|slot| *slot = 1, || false, |_| {}) {};
        }
        for _ in 0..BUFFER_SIZE {
            while full_sync_receiver.consume(|slot| *slot, || false, |_| {}).is_none() {};
        }
    }));

    let bench_id = format!("ogre_std's Atomic queue");
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        for _ in 0..BUFFER_SIZE {
            while !atomic_sender.publish(|slot| *slot = 1, || false, |_| {}) {};
        }
        for _ in 0..BUFFER_SIZE {
            while atomic_receiver.consume(|slot| *slot, || false, |_| {}).is_none() {};
        }
    }));

    let bench_id = format!("Std MPSC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        for _ in 0..BUFFER_SIZE {
            while std_sender.try_send(1).is_err() {};
        }
        for _ in 0..BUFFER_SIZE {
            while std_receiver.try_recv().is_err() {};
        }
    }));

    let bench_id = format!("Tokio MPSC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        for _ in 0..BUFFER_SIZE {
            while tokio_sender.try_send(1).is_err() {};
        }
        for _ in 0..BUFFER_SIZE {
            while tokio_receiver.try_recv().is_err() {};
        }
    }));

    let bench_id = format!("Futures MPSC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        for _ in 0..BUFFER_SIZE {
            while futures_sender.try_send(1).is_err() {};
        }
        for _ in 0..BUFFER_SIZE {
            while futures_receiver.try_next().is_err() {};
        }
    }));

    let bench_id = format!("Crossbeam MPMC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        for _ in 0..BUFFER_SIZE {
            while crossbeam_sender.try_send(1).is_err() {};
        }
        for _ in 0..BUFFER_SIZE {
            while crossbeam_receiver.try_recv().is_err() {};
        }
    }));

    group.finish();
}

/// Benchmarks the inter-thread throughput of our containers against Std, Tokio, Futures & Crossbeam channels.\
/// Throughput is measured by the receiver thread, which signals the sender thread to produce a batch of items,
/// then waits for the elements. At any given time, there are only 2 threads running and the measured times are:\
///   - the time it takes for a thread to signal another one (this is the same for everybody)
///   - + the time it takes to fill the backing buffer with elements + the time to receive all of them
fn bench_inter_thread_throughput(criterion: &mut Criterion) {

    type ItemType = u32;
    const BUFFER_SIZE: usize = 1<<18;

    let mut group = criterion.benchmark_group("Inter-thread THROUGHPUT");

    let full_sync_channel = Arc::new(FullSyncMeta::<ItemType, BUFFER_SIZE>::new());
    let (full_sync_sender, full_sync_receiver) = (full_sync_channel.clone(), full_sync_channel);
    let atomic_channel = Arc::new(AtomicMeta::<ItemType, BUFFER_SIZE>::new());
    let (atomic_sender, atomic_receiver) = (atomic_channel.clone(), atomic_channel);

    let (std_sender, std_receiver) = std::sync::mpsc::sync_channel::<ItemType>(BUFFER_SIZE);
    let (tokio_sender, mut tokio_receiver) = tokio::sync::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (mut futures_sender, mut futures_receiver) = futures::channel::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (crossbeam_sender, crossbeam_receiver) = crossbeam_channel::bounded::<ItemType>(BUFFER_SIZE);

    bench_it(&mut group,
             format!("ogre_std's FullSync queue"),
             || for _ in 0..BUFFER_SIZE {
                            while !full_sync_sender.publish(|slot| *slot = 1, || false, |_| {}) {}
                        },
             || for _ in 0..BUFFER_SIZE {
                              while full_sync_receiver.consume(|slot| *slot, || false, |_| {}).is_none() {std::hint::spin_loop()}
                          });

    bench_it(&mut group,
             format!("ogre_std's Atomic queue"),
             || for _ in 0..BUFFER_SIZE {
                            while !atomic_sender.publish(|slot| *slot = 1, || false, |_| {}) {}
                        },
             || for _ in 0..BUFFER_SIZE {
                              while atomic_receiver.consume(|slot| *slot, || false, |_| {}).is_none() {std::hint::spin_loop()}
                          });

    bench_it(&mut group,
             format!("Std MPSC Channel"),
             || for _ in 0..BUFFER_SIZE {
                            while std_sender.try_send(1).is_err() {};
                        },
             || for _ in 0..BUFFER_SIZE {
                              while std_receiver.try_recv().is_err() {std::hint::spin_loop()};
                          });

    bench_it(&mut group,
             format!("Tokio MPSC Channel"),
             || for _ in 0..BUFFER_SIZE {
                            while tokio_sender.try_send(1).is_err() {};
                        },
             || for _ in 0..BUFFER_SIZE {
                              while tokio_receiver.try_recv().is_err() {std::hint::spin_loop()};
                          });

    bench_it(&mut group,
             format!("Futures MPSC Channel"),
             || for _ in 0..BUFFER_SIZE {
                            while futures_sender.try_send(1).is_err() {};
                        },
             || for _ in 0..BUFFER_SIZE {
                              while futures_receiver.try_next().is_err() {std::hint::spin_loop()};
                          });

    bench_it(&mut group,
             format!("Crossbeam MPMC Channel"),
             || for _ in 0..BUFFER_SIZE {
                            while crossbeam_sender.try_send(1).is_err() {};
                        },
             || for _ in 0..BUFFER_SIZE {
                              while crossbeam_receiver.try_recv().is_err() {std::hint::spin_loop()};
                          });

    group.finish();
}

/// For inter-thread measurements: measures the time for one thread to send a signal to another
fn baseline_it(group: &mut BenchmarkGroup<WallTime>) {
    let bench_id = format!("Baseline -- added signaling time");
    crossbeam::scope(|scope| {
        let keep_running = Arc::new(AtomicBool::new(true));
        let keep_running_ref = keep_running.clone();
        let counter = Arc::new(AtomicU8::new(0));
        let counter_ref = counter.clone();
        scope.spawn(move |_| {
            while keep_running.load(Relaxed) {
                counter.fetch_add(1, Relaxed);
            }
        });
        group.bench_function(bench_id, |bencher| bencher.iter(|| {
            let mut last_count = counter_ref.load(Relaxed);
            loop {
                let current_count = counter_ref.load(Relaxed);
                if current_count != last_count {
                    break;
                }
                std::hint::spin_loop();
            }
        }));
        keep_running_ref.store(false, Relaxed);
    }).expect("Spawn baseline threads");
}

/// For inter-thread measurements: measures the time for one thread to send info to another + the time for the other to receive it
fn bench_it(group:          &mut BenchmarkGroup<WallTime>,
            bench_id:       String,
            mut send_fn:    impl FnMut() + Send,
            mut receive_fn: impl FnMut()) {
    crossbeam::scope(move |scope| {
        let keep_running = Arc::new(AtomicBool::new(true));
        let keep_running_ref = keep_running.clone();
        let send = Arc::new(AtomicBool::new(false));
        let produce_ref = send.clone();
        scope.spawn(move |_| {
            while keep_running.load(Relaxed) {
                while !send.swap(false, Relaxed) {}
                send_fn();
            }
        });
        group.bench_function(bench_id, |bencher| bencher.iter(|| {
            produce_ref.store(true, Relaxed);
            receive_fn();
        }));
        keep_running_ref.store(false, Relaxed);
        produce_ref.store(true, Relaxed);
    }).expect("Spawn benchmarking threads");
}


criterion_group!(benches, /*bench_same_thread_latency, bench_same_thread_throughput, bench_inter_thread_latency, */bench_inter_thread_throughput);
criterion_main!(benches);
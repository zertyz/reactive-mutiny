//! Compares the performance of Rust's community-provided channels with containers from `ogre-std`'s queues & stacks
//!
//! `Atomic`, `Crossbeam` and `FullSync` are, currently, used as the base for `Uni` and `Multi` channels.
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

use std::hint::black_box;
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
use reactive_mutiny::ogre_std::ogre_queues::meta_publisher::{MetaPublisher, MovePublisher};
use reactive_mutiny::ogre_std::ogre_queues::meta_container::{MetaContainer, MoveContainer};
use reactive_mutiny::ogre_std::ogre_queues::meta_subscriber::{MetaSubscriber, MoveSubscriber};


/// Represents a reasonably sized event, similar to production needs
#[derive(Debug)]
struct MessageType {
    _data:  [u8; 2048],
}
impl Default for MessageType {
    fn default() -> Self {
        MessageType { _data: [0; 2048] }
    }
}

type ItemType = MessageType;
const BUFFER_SIZE: usize = 1<<14;


/// Benchmarks the same-thread latency of our containers against Std, Tokio, Futures & Crossbeam channels.\
/// Latency is measured by the time it takes to send a single element + time to receive that element
fn bench_same_thread_latency(criterion: &mut Criterion) {

    let mut group = criterion.benchmark_group("Same-thread LATENCY");

    let full_sync_channel = Arc::new(FullSyncMove::<ItemType, BUFFER_SIZE>::new());
    let (full_sync_sender, full_sync_receiver) = (full_sync_channel.clone(), full_sync_channel);
    let atomic_channel = Arc::new(AtomicMove::<ItemType, BUFFER_SIZE>::new());
    let (atomic_sender, atomic_receiver) = (atomic_channel.clone(), atomic_channel);

    let (std_sender, std_receiver) = std::sync::mpsc::sync_channel::<ItemType>(BUFFER_SIZE);
    let (tokio_sender, mut tokio_receiver) = tokio::sync::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (mut futures_sender, mut futures_receiver) = futures::channel::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (crossbeam_sender, crossbeam_receiver) = crossbeam_channel::bounded::<ItemType>(BUFFER_SIZE);

    let bench_id = format!("ogre_std's FullSync queue");
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        while full_sync_sender.publish_movable(ItemType::default()).is_none() {};
        while full_sync_receiver.consume_movable().is_none() {};
    })));

    let bench_id = format!("ogre_std's Atomic queue");
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        while atomic_sender.publish_movable(ItemType::default()).is_none() {};
        while atomic_receiver.consume_movable().is_none() {};
    })));

    let bench_id = format!("Std MPSC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        while std_sender.try_send(ItemType::default()).is_err() {};
        while std_receiver.try_recv().is_err() {};
    })));

    let bench_id = format!("Tokio MPSC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        while tokio_sender.try_send(ItemType::default()).is_err() {};
        while tokio_receiver.try_recv().is_err() {};
    })));

    let bench_id = format!("Futures MPSC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        while futures_sender.try_send(ItemType::default()).is_err() {};
        while futures_receiver.try_next().is_err() {};
    })));

    let bench_id = format!("Crossbeam MPMC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        while crossbeam_sender.try_send(ItemType::default()).is_err() {};
        while crossbeam_receiver.try_recv().is_err() {};
    })));

    group.finish();
}

/// Benchmarks the inter-thread latency of our containers against Std, Tokio, Futures & Crossbeam channels.\
/// Latency is measured by the receiver thread, which signals the sender thread to produce an item, then
/// waits for the element. At any given time, there are only 2 threads running and the measured times are:\
///   - the time it takes for a thread to signal another one (this is the same for everybody)
///   - + the time for the first thread to receive the element.
/// The "Baseline" measurement is an attempt to determine how much time is spent signaling a thread -- so
/// the real latency would be the measured values - Baseline
fn bench_inter_thread_latency(criterion: &mut Criterion) {

    let mut group = criterion.benchmark_group("Inter-thread LATENCY");

    let full_sync_channel = Arc::new(FullSyncMove::<ItemType, BUFFER_SIZE>::new());
    let (full_sync_sender, full_sync_receiver) = (full_sync_channel.clone(), full_sync_channel);
    let atomic_channel = Arc::new(AtomicMove::<ItemType, BUFFER_SIZE>::new());
    let (atomic_sender, atomic_receiver) = (atomic_channel.clone(), atomic_channel);

    let (std_sender, std_receiver) = std::sync::mpsc::sync_channel::<ItemType>(BUFFER_SIZE);
    let (tokio_sender, mut tokio_receiver) = tokio::sync::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (mut futures_sender, mut futures_receiver) = futures::channel::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (crossbeam_sender, crossbeam_receiver) = crossbeam_channel::bounded::<ItemType>(BUFFER_SIZE);

    fn baseline_it(group: &mut BenchmarkGroup<WallTime>) {
        let bench_id = format!("Baseline -- thread signaling time");
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
            group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
                let last_count = counter_ref.load(Relaxed);
                loop {
                    let current_count = counter_ref.load(Relaxed);
                    if current_count != last_count {
                        break;
                    }
                    std::hint::spin_loop();
                }
            })));
            keep_running_ref.store(false, Relaxed);
        }).expect("Spawn baseline threads");
    }

    fn bench_it(group:          &mut BenchmarkGroup<WallTime>,
                bench_id:       String,
                mut send_fn:    impl FnMut() + Send,
                mut receive_fn: impl FnMut()) {
        crossbeam::scope(move |scope| {
            let keep_running = Arc::new(AtomicBool::new(true));
            let keep_running_ref = keep_running.clone();
            let send = Arc::new(AtomicBool::new(false));
            let send_ref = send.clone();
            scope.spawn(move |_| {
                while keep_running.load(Relaxed) {
                    while !send.swap(false, Relaxed) {}
                    send_fn();
                }
            });
            group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
                send_ref.store(true, Relaxed);
                receive_fn();
            })));
            keep_running_ref.store(false, Relaxed);
            send_ref.store(true, Relaxed);
        }).expect("Spawn benchmarking threads");
    }

    baseline_it(&mut group);

    bench_it(&mut group,
             format!("ogre_std's FullSync queue"),
             || while full_sync_sender.publish_movable(ItemType::default()).is_none() {std::hint::spin_loop()},
             || while full_sync_receiver.consume_movable().is_none() {std::hint::spin_loop()});

    bench_it(&mut group,
             format!("ogre_std's Atomic queue"),
             || while atomic_sender.publish_movable(ItemType::default()).is_none() {std::hint::spin_loop()},
             || while atomic_receiver.consume_movable().is_none() {std::hint::spin_loop()});

    bench_it(&mut group,
             format!("Std MPSC Channel"),
             || while std_sender.try_send(ItemType::default()).is_err() {std::hint::spin_loop()},
             || while std_receiver.try_recv().is_err() {std::hint::spin_loop()});

    bench_it(&mut group,
             format!("Tokio MPSC Channel"),
             || while tokio_sender.try_send(ItemType::default()).is_err() {std::hint::spin_loop()},
             || while tokio_receiver.try_recv().is_err() {std::hint::spin_loop()});

    bench_it(&mut group,
             format!("Futures MPSC Channel"),
             || while futures_sender.try_send(ItemType::default()).is_err() {std::hint::spin_loop()},
             || while futures_receiver.try_next().is_err() {std::hint::spin_loop()});

    bench_it(&mut group,
             format!("Crossbeam MPMC Channel"),
             || while crossbeam_sender.try_send(ItemType::default()).is_err() {std::hint::spin_loop()},
             || while crossbeam_receiver.try_recv().is_err() {std::hint::spin_loop()});

    group.finish();
}

/// Benchmarks the same-thread throughput of our containers against Std, Tokio, Futures & Crossbeam channels.\
/// Throughput is measured by the time it takes to fill the backing buffer with elements + the time to receive all of them
fn bench_same_thread_throughput(criterion: &mut Criterion) {

    let mut group = criterion.benchmark_group("Same-thread THROUGHPUT");

    let full_sync_channel = Arc::new(FullSyncMove::<ItemType, BUFFER_SIZE>::new());
    let (full_sync_sender, full_sync_receiver) = (full_sync_channel.clone(), full_sync_channel);
    let atomic_channel = Arc::new(AtomicMove::<ItemType, BUFFER_SIZE>::new());
    let (atomic_sender, atomic_receiver) = (atomic_channel.clone(), atomic_channel);

    let (std_sender, std_receiver) = std::sync::mpsc::sync_channel::<ItemType>(BUFFER_SIZE);
    let (tokio_sender, mut tokio_receiver) = tokio::sync::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (mut futures_sender, mut futures_receiver) = futures::channel::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (crossbeam_sender, crossbeam_receiver) = crossbeam_channel::bounded::<ItemType>(BUFFER_SIZE);

    let bench_id = format!("ogre_std's FullSync queue");
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        for _ in 0..BUFFER_SIZE {
            while full_sync_sender.publish_movable(ItemType::default()).is_none() {};
        }
        for _ in 0..BUFFER_SIZE {
            while full_sync_receiver.consume_movable().is_none() {};
        }
    })));

    let bench_id = format!("ogre_std's Atomic queue");
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        for _ in 0..BUFFER_SIZE {
            while atomic_sender.publish_movable(ItemType::default()).is_none() {};
        }
        for _ in 0..BUFFER_SIZE {
            while atomic_receiver.consume_movable().is_none() {};
        }
    })));

    let bench_id = format!("Std MPSC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        for _ in 0..BUFFER_SIZE {
            while std_sender.try_send(ItemType::default()).is_err() {};
        }
        for _ in 0..BUFFER_SIZE {
            while std_receiver.try_recv().is_err() {};
        }
    })));

    let bench_id = format!("Tokio MPSC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        for _ in 0..BUFFER_SIZE {
            while tokio_sender.try_send(ItemType::default()).is_err() {};
        }
        for _ in 0..BUFFER_SIZE {
            while tokio_receiver.try_recv().is_err() {};
        }
    })));

    let bench_id = format!("Futures MPSC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        for _ in 0..BUFFER_SIZE {
            while futures_sender.try_send(ItemType::default()).is_err() {};
        }
        for _ in 0..BUFFER_SIZE {
            while futures_receiver.try_next().is_err() {};
        }
    })));

    let bench_id = format!("Crossbeam MPMC Channel");
    group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
        for _ in 0..BUFFER_SIZE {
            while crossbeam_sender.try_send(ItemType::default()).is_err() {};
        }
        for _ in 0..BUFFER_SIZE {
            while crossbeam_receiver.try_recv().is_err() {};
        }
    })));

    group.finish();
}

/// Benchmarks the inter-thread throughput of our containers against Std, Tokio, Futures & Crossbeam channels.\
/// Throughput is measured by the receiver thread, which consumes the events that are produced -- non-stop --
/// by the producer thread, simulating a high contention scenario.
/// The measured times are inversely proportional to the number of elements consumed.
fn bench_inter_thread_throughput(criterion: &mut Criterion) {

    let mut group = criterion.benchmark_group("Inter-thread THROUGHPUT");

    let full_sync_channel = Arc::new(FullSyncMove::<ItemType, BUFFER_SIZE>::new());
    let (full_sync_sender, full_sync_receiver) = (full_sync_channel.clone(), full_sync_channel);
    let atomic_channel = Arc::new(AtomicMove::<ItemType, BUFFER_SIZE>::new());
    let (atomic_sender, atomic_receiver) = (atomic_channel.clone(), atomic_channel);

    let (std_sender, std_receiver) = std::sync::mpsc::sync_channel::<ItemType>(BUFFER_SIZE);
    let (tokio_sender, mut tokio_receiver) = tokio::sync::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (mut futures_sender, mut futures_receiver) = futures::channel::mpsc::channel::<ItemType>(BUFFER_SIZE);
    let (crossbeam_sender, crossbeam_receiver) = crossbeam_channel::bounded::<ItemType>(BUFFER_SIZE);

    fn bench_it(group:          &mut BenchmarkGroup<WallTime>,
                bench_id:       String,
                mut send_fn:    impl FnMut() + Send,
                mut receive_fn: impl FnMut()) {
        crossbeam::scope(move |scope| {
            let keep_running = Arc::new(AtomicBool::new(true));
            let keep_running_ref = keep_running.clone();
            scope.spawn(move |_| {
                while keep_running.load(Relaxed) {
                    send_fn();
                }
            });
            group.bench_function(bench_id, |bencher| bencher.iter(|| black_box({
                receive_fn();
            })));
            keep_running_ref.store(false, Relaxed);
        }).expect("Spawn benchmarking threads");
    }

    bench_it(&mut group,
             format!("ogre_std's FullSync queue"),
             || for _ in 0..BUFFER_SIZE {
                            if full_sync_sender.publish_movable(ItemType::default()).is_none() {std::hint::spin_loop();std::hint::spin_loop();std::hint::spin_loop()}
                        },
             || for _ in 0..(BUFFER_SIZE>>5) { while full_sync_receiver.consume_movable().is_none() {std::hint::spin_loop()} });

    bench_it(&mut group,
             format!("ogre_std's Atomic queue"),
             || for _ in 0..BUFFER_SIZE {
                            if atomic_sender.publish_movable(ItemType::default()).is_none() {std::hint::spin_loop();std::hint::spin_loop();std::hint::spin_loop()}
                        },
             || for _ in 0..(BUFFER_SIZE>>5) { while atomic_receiver.consume_movable().is_none() {std::hint::spin_loop()} });

    bench_it(&mut group,
             format!("Std MPSC Channel"),
             || for _ in 0..BUFFER_SIZE {
                            if std_sender.try_send(ItemType::default()).is_err() {std::hint::spin_loop();std::hint::spin_loop();std::hint::spin_loop()};
                        },
             || for _ in 0..(BUFFER_SIZE>>5) { while std_receiver.try_recv().is_err() {std::hint::spin_loop()} });

    bench_it(&mut group,
             format!("Tokio MPSC Channel"),
             || for _ in 0..BUFFER_SIZE {
                            if tokio_sender.try_send(ItemType::default()).is_err() {std::hint::spin_loop();std::hint::spin_loop();std::hint::spin_loop()};
                        },
             || for _ in 0..(BUFFER_SIZE>>5) { while tokio_receiver.try_recv().is_err() {std::hint::spin_loop()} });

    bench_it(&mut group,
             format!("Futures MPSC Channel"),
             || for _ in 0..BUFFER_SIZE {
                            if futures_sender.try_send(ItemType::default()).is_err() {std::hint::spin_loop();std::hint::spin_loop();std::hint::spin_loop()};
                        },
             || for _ in 0..(BUFFER_SIZE>>5) { while futures_receiver.try_next().is_err() {std::hint::spin_loop()} });

    bench_it(&mut group,
             format!("Crossbeam MPMC Channel"),
             || for _ in 0..BUFFER_SIZE {
                            if crossbeam_sender.try_send(ItemType::default()).is_err() {std::hint::spin_loop();std::hint::spin_loop();std::hint::spin_loop()};
                        },
             || for _ in 0..(BUFFER_SIZE>>5) { while crossbeam_receiver.try_recv().is_err() {std::hint::spin_loop()} });

    group.finish();
}

criterion_group!(benches, bench_same_thread_latency, bench_same_thread_throughput, bench_inter_thread_latency, bench_inter_thread_throughput);
criterion_main!(benches);

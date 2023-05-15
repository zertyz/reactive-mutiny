//! Demonstrates how to work with all the channel options, for performance tuning of [Uni]s & [Multi]s.

#[path = "../common/mod.rs"] mod common;

use common::{ExchangeEvent};
use std::{
    future, sync::{
        Arc,
        atomic::AtomicU32,
    },
    time::Duration,
    io::Write,
    fmt::Debug,
    future::Future,
    ops::Deref,
    sync::atomic::{
        AtomicU64, Ordering::Relaxed,
    },
    time::Instant,
};
use reactive_mutiny::{ogre_std::ogre_alloc::ogre_unique::OgreUnique, stream_executor::StreamExecutor, uni::{
    Uni,
    channels::{
        ChannelCommon,
        ChannelProducer,
        FullDuplexChannel,
    },
}, ChannelConsumer, Instruments, UniZeroCopyAtomic, UniZeroCopyFullSync, UniMoveAtomic, UniMoveCrossbeam, UniMoveFullSync, MultiAtomicArc, MultiCrossbeamArc, MultiFullSyncArc};
use futures::{SinkExt, Stream, StreamExt};


const BUFFER_SIZE: usize = 1<<17;
const MAX_STREAMS: usize = 1;
const INSTRUMENTS: usize = {Instruments::NoInstruments.into()};


async fn uni_builder_benchmark<DerivedEventType:    'static + Debug + Send + Sync + Deref<Target = ExchangeEvent>,
                               UniChannelType:      FullDuplexChannel<'static, ExchangeEvent, DerivedEventType> + Sync + Send + 'static>
                              (ident: &str,
                               name: &str,
                               uni_builder: reactive_mutiny::uni::UniBuilder<ExchangeEvent, UniChannelType, INSTRUMENTS, DerivedEventType>) {

    #[cfg(not(debug_assertions))]
    const ITERATIONS: u32 = 1<<24;
    #[cfg(debug_assertions)]
    const ITERATIONS: u32 = 1<<20;

    let mut sum = Arc::new(AtomicU64::new(0));

    let uni = uni_builder
        .spawn_non_futures_non_fallible_executor(name, |stream| {
            let sum = Arc::clone(&sum);
            stream.map(move |exchange_event| {
                let val = match *exchange_event {
                    ExchangeEvent::TradeEvent { unitary_value, quantity } => quantity,
                    _ => 0,
                };
                sum.fetch_add(val, Relaxed)
            })
        },
        |_| async {});

    print!("{ident}{name}: "); std::io::stdout().flush().unwrap();

    let start = Instant::now();
    for e in 1..=ITERATIONS {
        while !uni.try_send(ExchangeEvent::TradeEvent { unitary_value: 10.05, quantity: e as u64 }) {
            std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop();
        }
    }
    uni.close(Duration::from_secs(5)).await;
    let elapsed = start.elapsed();
    let observed = sum.load(Relaxed);
    let expected = (1 + ITERATIONS) as u64 * (ITERATIONS / 2) as u64;
    println!("{:10.2}/s {}",
             ITERATIONS as f64 / elapsed.as_secs_f64(),
             if observed == expected { format!("✓") } else { format!("∅ -- SUM of the first {ITERATIONS} natural numbers differ! Expected: {expected}; Observed: {observed}") });
}

async fn multi_builder_benchmark<DerivedEventType: Debug + Send + Sync + Deref<Target = ExchangeEvent>,
                                 MultiChannelType: FullDuplexChannel<'static, ExchangeEvent, DerivedEventType> + Sync + Send + 'static>
                                (ident: &str,
                                 name: &str,
                                 multi: reactive_mutiny::multi::Multi<'static, ExchangeEvent, MultiChannelType, INSTRUMENTS, DerivedEventType>)
                                -> Result<(), Box<dyn std::error::Error>> {

    #[cfg(not(debug_assertions))]
    const ITERATIONS: u32 = 1<<24;
    #[cfg(debug_assertions)]
    const ITERATIONS: u32 = 1<<20;

    let mut sum = Arc::new(AtomicU64::new(0));

    multi
        .spawn_non_futures_non_fallible_executor_ref(1, name, |stream| {
            let sum = Arc::clone(&sum);
            stream.map(move |exchange_event| {
                let val = match *exchange_event {
                    ExchangeEvent::TradeEvent { unitary_value, quantity } => quantity,
                    _ => 0,
                };
                sum.fetch_add(val, Relaxed)
            })
        }, |_| async {}).await
        .map_err(|err| format!("Couldn't spawn a new executor for Multi {}", multi.multi_name))?;

    print!("{ident}{name}: "); std::io::stdout().flush().unwrap();

    let start = Instant::now();
    let mut e = 0;
    'done: loop {
        for _ in 0..(multi.buffer_size() - multi.pending_items_count()) {
            if e < ITERATIONS {
                e += 1;
                multi.send(ExchangeEvent::TradeEvent { unitary_value: 10.05, quantity: e as u64 });
            } else {
                break 'done;
            }
        }
        std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop();
    }
    multi.close(Duration::from_secs(5)).await;
    let elapsed = start.elapsed();
    let observed = sum.load(Relaxed);
    let expected = (1 + ITERATIONS) as u64 * (ITERATIONS / 2) as u64;
    println!("{:10.2}/s {}",
             ITERATIONS as f64 / elapsed.as_secs_f64(),
             if observed == expected { format!("✓") } else { format!("∅ -- SUM of the first {ITERATIONS} natural numbers differ! Expected: {expected}; Observed: {observed}") });
    Ok(())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("On this code, you may see how to build `Uni`s and `Multi`s using all the available channels");
    println!("-- each providing tradeoffs between features and performance.");
    println!("Performance characteristics of passing our `ExchangeEvent` through all the different channels:");
    println!();
    println!("Uni:");
    println!("    Move:");
    uni_builder_benchmark("        ", "Atomic    ", UniMoveAtomic::   <ExchangeEvent, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>::new()).await;
    uni_builder_benchmark("        ", "Crossbeam ", UniMoveCrossbeam::<ExchangeEvent, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>::new()).await;
    uni_builder_benchmark("        ", "Full Sync ", UniMoveFullSync:: <ExchangeEvent, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>::new()).await;
    println!("    Zero-Copy:");
    uni_builder_benchmark("        ", "Atomic    ", UniZeroCopyAtomic::  <ExchangeEvent, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>::new()).await;
    uni_builder_benchmark("        ", "Full Sync ", UniZeroCopyFullSync::<ExchangeEvent, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>::new()).await;
    println!();
    println!("Multi:");
    println!("    Arc:");
    multi_builder_benchmark("        ", "Atomic    ", MultiAtomicArc::<ExchangeEvent, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>::new("profiling multi")).await?;
    multi_builder_benchmark("        ", "Crossbeam ", MultiCrossbeamArc::<ExchangeEvent, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>::new("profiling multi")).await?;
    multi_builder_benchmark("        ", "FullSync  ", MultiFullSyncArc::<ExchangeEvent, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>::new("profiling multi")).await?;
    println!();

    Ok(())
}
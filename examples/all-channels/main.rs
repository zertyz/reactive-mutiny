//! Demonstrates how to work with all the channel options, for performance tuning of [Uni]s & [Multi]s.

#[path = "../common/mod.rs"] mod common;

use common::ExchangeEvent;
use std::{
    time::Duration,
    io::Write,
    fmt::Debug,
    ops::Deref,
    sync::{
        Arc,
        atomic::{
            AtomicU64, Ordering::Relaxed,
        }
    },
    time::Instant,
};
use reactive_mutiny::prelude::advanced::*;
use futures::StreamExt;


const BUFFER_SIZE: usize = 1<<12;
const MAX_STREAMS: usize = 1;
const INSTRUMENTS: usize = Instruments::NoInstruments.into();


async fn uni_builder_benchmark<DerivedEventType:    'static + Debug + Send + Sync + Deref<Target = ExchangeEvent>,
                               UniChannelType:      FullDuplexUniChannel<ItemType=ExchangeEvent, DerivedItemType=DerivedEventType> + Sync + Send + 'static>
                              (ident: &str,
                               name: &str,
                               uni: Uni<ExchangeEvent, UniChannelType, INSTRUMENTS, DerivedEventType>) {

    #[cfg(not(debug_assertions))]
    const ITERATIONS: u32 = 1<<24;
    #[cfg(debug_assertions)]
    const ITERATIONS: u32 = 1<<20;

    let sum = Arc::new(AtomicU64::new(0));

    let uni = uni
        .spawn_non_futures_non_fallibles_executors(1,
                                                   |stream| {
                                                      let sum = Arc::clone(&sum);
                                                      stream.map(move |exchange_event| {
                                                          if let ExchangeEvent::TradeEvent { quantity, .. } = *exchange_event {
                                                              sum.fetch_add(quantity as u64, Relaxed);
                                                          }
                                                      })
                                                  },
                                                   |_| async {});

    print!("{ident}{name}: "); std::io::stdout().flush().unwrap();

    let start = Instant::now();
    for e in 1..=ITERATIONS {
        uni.send_with(|slot| *slot = ExchangeEvent::TradeEvent { unitary_value: 10.05, quantity: e as u128, time: e as u64 })
            .retry_with(|setter| uni.send_with(setter))
            .spinning_forever();
    }
    debug_assert!(uni.close(Duration::from_secs(5)).await, "Uni wasn't properly closed");
    std::thread::sleep(Duration::from_millis(10));
    let elapsed = start.elapsed();
    let observed = sum.load(Relaxed);
    let expected = (1 + ITERATIONS) as u64 * (ITERATIONS / 2) as u64;
    println!("{:10.2}/s {}",
             ITERATIONS as f64 / elapsed.as_secs_f64(),
             if observed == expected {
                 format!("✓")
             } else {
                 // if the observed iteractions is zero, remember to enable metrics in the `INSTRUMENTS` config at the top of this file
                 format!("∅ -- SUM of the first {ITERATIONS} natural numbers differ! Expected: {expected}; Observed: {observed} (observed iteractions: {}; expected: {})",
                         uni.stream_executors[0].ok_events_avg_future_duration.probe().0, ITERATIONS)
             });
}

async fn multi_builder_benchmark<DerivedEventType: Debug + Send + Sync + Deref<Target = ExchangeEvent>,
                                 MultiChannelType: FullDuplexMultiChannel<ItemType=ExchangeEvent, DerivedItemType=DerivedEventType> + Sync + Send + 'static>
                                (ident:               &str,
                                 name:                &str,
                                 number_of_listeners: u32,
                                 multi:               Multi<ExchangeEvent, MultiChannelType, INSTRUMENTS, DerivedEventType>)
                                -> Result<(), Box<dyn std::error::Error>> {

    #[cfg(not(debug_assertions))]
    const ITERATIONS: u32 = 1<<24;
    #[cfg(debug_assertions)]
    const ITERATIONS: u32 = 1<<20;

    let mut counters = Vec::with_capacity(8 * number_of_listeners as usize);  // we will use counters spaced by 64 bytes, to avoid false sharing of CPU caches
    (0..(8 * number_of_listeners)).for_each(|_| counters.push(AtomicU64::new(0)));
    let counters = Arc::new(counters);
    let listeners_count = AtomicU64::new(0);

    for listener_number in 0..number_of_listeners {
        let listener_name = format!("#{}: {} for multi {}", listener_number, name, multi.multi_name);
        let counters_for_listener = Arc::clone(&counters);
        multi
            .spawn_non_futures_non_fallible_executor(1, listener_name, |stream| {
                let listener_id = listeners_count.fetch_add(1, Relaxed);
                stream.map(move |exchange_event| {
                    if let ExchangeEvent::TradeEvent { quantity, .. } = *exchange_event {
                        let counter = counters_for_listener.get(8 * listener_id as usize).unwrap();
                        counter.fetch_add(quantity as u64, Relaxed);
                    }
                })
            }, |_| async {}).await
            .map_err(|err| format!("Couldn't spawn a new executor for Multi named '{}', listener #{}: {}", multi.multi_name, listener_number, err))?;
    }

    print!("{ident}{name}: "); std::io::stdout().flush().unwrap();

    let start = Instant::now();
    let mut e = 0;
    'done: loop {
        for _ in 0..(multi.buffer_size() - multi.pending_items_count()) {
            if e < ITERATIONS {
                if multi.send_with(|slot| *slot = ExchangeEvent::TradeEvent { unitary_value: 10.05, quantity: e as u128 + 1, time: e as u64 }).is_ok() {
                    e += 1;
                }
            } else {
                break 'done;
            }
        }
        std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop();
        std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop();
        std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop();
        std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop(); std::hint::spin_loop();
    }
    multi.close(Duration::from_secs(5)).await;
    let elapsed = start.elapsed();
    let observed = counters.iter().map(|n| n.load(Relaxed)).sum::<u64>();
    let expected = number_of_listeners as u64 * ( (1 + ITERATIONS) as u64 * (ITERATIONS / 2) as u64 );
    println!("{:10.2}/s {}",
             ITERATIONS as f64 / elapsed.as_secs_f64(),
             if observed == expected { format!("✓") } else { format!("∅ -- SUM of the first {ITERATIONS} natural numbers differ! Expected: {expected}; Observed: {observed}") });
    Ok(())
}

#[tokio::main(flavor = "multi_thread"/*, worker_threads = 12*/)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("On this code, you may see how to build `Uni`s and `Multi`s using all the available channels");
    println!("-- each providing tradeoffs between features and performance.");
    println!("So, here are performance characteristics of passing our `ExchangeEvent` through all the different channels -- for all `Uni`s and `Multi`s:");
    println!();
    simple_logger::SimpleLogger::new().with_utc_timestamps().init().unwrap_or_else(|_| eprintln!("--> LOGGER WAS ALREADY STARTED"));
    println!("Uni:");
    println!("    Move:");
    uni_builder_benchmark("        ", "Atomic    ", UniMoveAtomic::   <ExchangeEvent, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>::new("Movable Atomic profiling Uni")).await;
    uni_builder_benchmark("        ", "Crossbeam ", UniMoveCrossbeam::<ExchangeEvent, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>::new("Movable Crossbeam profiling Uni")).await;
    uni_builder_benchmark("        ", "Full Sync ", UniMoveFullSync:: <ExchangeEvent, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>::new("Movable Full Sync profiling Uni")).await;
    println!("    Zero-Copy:");
    uni_builder_benchmark("        ", "Atomic    ", UniZeroCopyAtomic::  <ExchangeEvent, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>::new("Zero-Copy Atomic profiling Uni")).await;
    uni_builder_benchmark("        ", "Full Sync ", UniZeroCopyFullSync::<ExchangeEvent, BUFFER_SIZE, MAX_STREAMS, INSTRUMENTS>::new("Zero-Copy Full Sync profiling Uni")).await;
    println!();
    println!("Multi:");
    const LISTENERS: usize = 4;
    println!("    Arc:");
    multi_builder_benchmark("        ", "Atomic    ", LISTENERS as u32, MultiAtomicArc::<ExchangeEvent, BUFFER_SIZE, LISTENERS, INSTRUMENTS>::new("Arc Atomic profiling Multi")).await?;
    multi_builder_benchmark("        ", "Crossbeam ", LISTENERS as u32, MultiCrossbeamArc::<ExchangeEvent, BUFFER_SIZE, LISTENERS, INSTRUMENTS>::new("Arc Crossbeam profiling Multi")).await?;
    multi_builder_benchmark("        ", "FullSync  ", LISTENERS as u32, MultiFullSyncArc::<ExchangeEvent, BUFFER_SIZE, LISTENERS, INSTRUMENTS>::new("Arc Full Sync profiling Multi")).await?;
    println!("    OgreArc:");
    multi_builder_benchmark("        ", "Atomic    ", LISTENERS as u32, MultiAtomicOgreArc::<ExchangeEvent, BUFFER_SIZE, LISTENERS, INSTRUMENTS>::new("OgreArc Atomic profiling Multi")).await?;
    multi_builder_benchmark("        ", "FullSync  ", LISTENERS as u32, MultiFullSyncOgreArc::<ExchangeEvent, BUFFER_SIZE, LISTENERS, INSTRUMENTS>::new("OgreArc Full Sync profiling Multi")).await?;
    println!("    Reference:");
    multi_builder_benchmark("        ", "MmapLog   ", LISTENERS as u32, MultiMmapLog::<ExchangeEvent, LISTENERS, INSTRUMENTS>::new("Mmap Log profiling Multi")).await?;
    println!();

    Ok(())
}
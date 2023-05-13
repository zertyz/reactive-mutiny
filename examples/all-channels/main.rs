//! Demonstrates how to work with all the channel options, for performance tuning of [Uni]s & [Multi]s.

#[path = "../common/mod.rs"] mod common;

use common::{ExchangeEvent};
use reactive_mutiny::{
    uni::{
        Uni,
        channels::{
            ChannelCommon,
            ChannelProducer,
        },
    },
    ChannelConsumer,
    Instruments,
};
use std::{
    future, sync::{
        Arc,
        atomic::AtomicU32,
    },
    time::Duration,
    io::Write,
};
use std::fmt::Debug;
use std::future::Future;
use std::ops::Deref;
use std::sync::atomic::{
    AtomicU64, Ordering::Relaxed,
};
use std::time::Instant;
use futures::{SinkExt, Stream, StreamExt};
use reactive_mutiny::ogre_std::ogre_alloc::ogre_arc::OgreArc;
use reactive_mutiny::stream_executor::StreamExecutor;
use reactive_mutiny::uni::channels::FullDuplexChannel;


const BUFFER_SIZE: usize = 1<<17;
const MAX_STREAMS: usize = 1;
const INSTRUMENTS: usize = {Instruments::NoInstruments.into()};

// allocators
type OgreArrayPoolAllocator = reactive_mutiny::ogre_std::ogre_alloc::ogre_array_pool_allocator::OgreArrayPoolAllocator<ExchangeEvent, BUFFER_SIZE>;

// Unis
///////

type AtomicMoveUniBuilder<OnStreamCloseFnType, CloseVoidAsyncType> = reactive_mutiny::uni::UniBuilder<ExchangeEvent,
                                                                                                      reactive_mutiny::uni::channels::movable::atomic::Atomic<'static, ExchangeEvent, BUFFER_SIZE, MAX_STREAMS>,
                                                                                                      INSTRUMENTS, ExchangeEvent, OnStreamCloseFnType, CloseVoidAsyncType>;
type CrossbeamMoveUniBuilder<OnStreamCloseFnType, CloseVoidAsyncType> = reactive_mutiny::uni::UniBuilder<ExchangeEvent,
                                                                                                         reactive_mutiny::uni::channels::movable::crossbeam::Crossbeam<'static, ExchangeEvent, BUFFER_SIZE, MAX_STREAMS>,
                                                                                                         INSTRUMENTS, ExchangeEvent, OnStreamCloseFnType, CloseVoidAsyncType>;
type FullSyncMoveUniBuilder<OnStreamCloseFnType, CloseVoidAsyncType> = reactive_mutiny::uni::UniBuilder<ExchangeEvent,
                                                                                                        reactive_mutiny::uni::channels::movable::full_sync::FullSync<'static, ExchangeEvent, BUFFER_SIZE, MAX_STREAMS>,
                                                                                                        INSTRUMENTS, ExchangeEvent, OnStreamCloseFnType, CloseVoidAsyncType>;

type AtomicZeroCopyUniBuilder<OnStreamCloseFnType, CloseVoidAsyncType> = reactive_mutiny::uni::UniBuilder<ExchangeEvent,
                                                                                                          reactive_mutiny::uni::channels::zero_copy::atomic::Atomic<'static, ExchangeEvent, OgreArrayPoolAllocator, BUFFER_SIZE, MAX_STREAMS>,
                                                                                                          INSTRUMENTS, OgreArc<ExchangeEvent, OgreArrayPoolAllocator>, OnStreamCloseFnType, CloseVoidAsyncType>;

// Multis
/////////

type AtomicArcMulti = reactive_mutiny::multi::Multi<'static, ExchangeEvent,
                                                             reactive_mutiny::multi::channels::movable::atomic::Atomic<'static, ExchangeEvent, BUFFER_SIZE, MAX_STREAMS>,
                                                             INSTRUMENTS, Arc<ExchangeEvent>>;
type CrossbeamArcMulti = reactive_mutiny::multi::Multi<'static, ExchangeEvent,
                                                                reactive_mutiny::multi::channels::movable::crossbeam::Crossbeam<'static, ExchangeEvent, BUFFER_SIZE, MAX_STREAMS>,
                                                                INSTRUMENTS, Arc<ExchangeEvent>>;
type FullSyncArcMulti = reactive_mutiny::multi::Multi<'static, ExchangeEvent,
                                                               reactive_mutiny::multi::channels::movable::full_sync::FullSync<'static, ExchangeEvent, BUFFER_SIZE, MAX_STREAMS>,
                                                               INSTRUMENTS, Arc<ExchangeEvent>>;


async fn uni_builder_benchmark<DerivedEventType:    'static + Debug + Send + Sync + Deref<Target = ExchangeEvent>,
                               UniChannelType:      FullDuplexChannel<'static, ExchangeEvent, DerivedEventType> + Sync + Send + 'static,
                               OnStreamCloseFnType: Fn(Arc<StreamExecutor<INSTRUMENTS>>) -> CloseVoidAsyncType + Send + Sync + 'static,
                               CloseVoidAsyncType:  Future<Output=()> + Send + 'static>
                              (ident: &str,
                               name: &str,
                               uni_builder: reactive_mutiny::uni::UniBuilder<ExchangeEvent, UniChannelType, INSTRUMENTS, DerivedEventType, OnStreamCloseFnType, CloseVoidAsyncType>) {

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
        });

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
async fn main() {
    println!("On this code, you may see how to build `Uni`s and `Multi`s using all the available channels");
    println!("-- each providing tradeoffs between features and performance.");
    println!("Performance characteristics of passing our `ExchangeEvent` through all the different channels:");
    println!();
    println!("Uni:");
    println!("    Move:");
    uni_builder_benchmark("        ", "Atomic    ", AtomicMoveUniBuilder::new().on_stream_close(&|_| async {})).await;
    uni_builder_benchmark("        ", "Crossbeam ", CrossbeamMoveUniBuilder::new().on_stream_close(&|_| async {})).await;
    uni_builder_benchmark("        ", "Full Sync ", FullSyncMoveUniBuilder::new().on_stream_close(&|_| async {})).await;
    println!("    Zero-Copy:");
    uni_builder_benchmark("        ", "Atomic    ", AtomicZeroCopyUniBuilder::new().on_stream_close(&|_| async {})).await;
    println!();
    println!("Multi:");
    println!("    Arc:");
    multi_builder_benchmark("        ", "Atomic    ", AtomicArcMulti::new("profiling multi")).await;
    multi_builder_benchmark("        ", "Crossbeam ", CrossbeamArcMulti::new("profiling multi")).await;
    multi_builder_benchmark("        ", "FullSync  ", FullSyncArcMulti::new("profiling multi")).await;
    println!();
}
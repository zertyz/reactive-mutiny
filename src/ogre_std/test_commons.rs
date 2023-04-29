//! Common unit tests for [OgreStack] implementations
//!
//! Nice way of running the tests here:\
//! sudo sync; clear; pkill -f -stop firefox; pkill -f -stop chrome; pkill -f -stop java; RUSTFLAGS="-C target-cpu=native" cargo test --release -p ogre-std -- -Z unstable-options --report-time --test-threads=1; pkill -f -cont firefox; pkill -f -cont chrome; pkill -f -cont java
#![allow(warnings, unused)]

use super::{
    ogre_stacks::{
        OgreBlockingStack, OgreStack,
    },
    ogre_queues::OgreQueue,
    benchmarks::multi_threaded_iterate,
};

use std::{
    fmt::Debug,
    io::Write,
    sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed},
    time::{Duration, SystemTime},
};
use std::sync::Arc;
use parking_lot::{
    Mutex, RawMutex,
    lock_api::{
        RawMutex as _RawMutex,
        RawMutexTimed,
    }
};


#[derive(PartialEq)]
pub enum ContainerKind {
    Stack,
    Queue,
}

#[derive(PartialEq)]
pub enum Blocking {
    Blocking,
    NonBlocking,
}

pub fn basic_container_use_cases(container_kind: ContainerKind,
                                 blocking:       Blocking,
                                 container_size: usize,
                                 mut publish:    impl FnMut(i32) -> bool,
                                 mut consume:    impl FnMut() -> Option<i32>,
                                 mut len:        impl FnMut() -> usize) {

    macro_rules! consume_from_empty_if_non_blocking {
        () => {
            if blocking == Blocking::NonBlocking {
                assert_eq!(len(), 0, "data structure should be empty at this point");
                match consume() {
                    None               => (),   // test passed
                    Some(element) => panic!("Something was consumed when noting should have been: {:?}", element),
                }
                assert_eq!(len(), 0, "data structure should remain empty after consuming from empty");
            }
        }
    }

    macro_rules! publish_to_full_if_non_blocking {
        () => {
            if blocking == Blocking::NonBlocking {
                let published = publish(999);
                assert!(!published, "Data structure should be full already. A new element should have not been accepted");
            }
        }
    }

    macro_rules! publish_and_consume_a_single_element {
        () => {
            let expected = 123;
            let length_before = len();
            publish(expected);
            let length_after_publishing = len();
            assert_eq!(length_after_publishing, length_before+1, "Publishing an element didn't increase the data structure length");
            match consume() {
                None          => panic!("No element was consumed, even when {} has just been published", expected),
                Some(element) => assert_eq!(element, expected, "Wrong element consumed"),
            }
            consume_from_empty_if_non_blocking!();
        }
    }

    macro_rules! publish_to_exhaustion_and_consume_to_emptiness {
        () => {
            // publish on non-full data structure
            for i in 0..container_size as i32 {
                let published = publish(i);
                assert!(published, "Data structure was reported as FULL prematurely -- could publish only {} elements where a total of {} should fit there", i, container_size);
            }
            publish_to_full_if_non_blocking!();
            // consume all
            let consume_and_assert = |expected_element| match consume() {
                    None          => panic!("Data structure was reported as EMPTY prematurely -- could consume only {} elements where a total of {} should be there",
                                            container_size as i32 - expected_element - 1,
                                            container_size),
                    Some(element) => assert_eq!(element, expected_element, "Wrong element consumed"),
                };
            if container_kind == ContainerKind::Stack {
                (0..container_size as i32).rev().for_each(consume_and_assert);
            } else {
                (0..container_size as i32).for_each(consume_and_assert);
            };
            consume_from_empty_if_non_blocking!();
        }
    }

    consume_from_empty_if_non_blocking!();
    publish_and_consume_a_single_element!();
    publish_to_exhaustion_and_consume_to_emptiness!();

    if container_kind == ContainerKind::Queue {
        // test the full trip wrapping around the buffer, ending with head%QUEUE_SIZE & tail%QUEUE_SIZE = 0
        for _ in 1..container_size {
            publish_and_consume_a_single_element!();
            publish_to_exhaustion_and_consume_to_emptiness!();
        }
        // // continue on wrapping around the buffer until head and tail wraps around all the u32 values, ending with head & tail = 0 as when the queue was first created
        // // UNCOMMENT IF YOU ARE PREPARED FOR A 5+ MINUTES TEST, provided you run it in release mode and disable debugging output (see queue::new() clause)
        // #[cfg(debug_assertions)]
        // return eprintln!("NOT RUNNING EXTENDED QUEUE TEST: it runs only when compiled in Release mode (long runner test)");
        // for i in container_size..u32::MAX as usize {
        //     publish_and_consume_a_single_element!();
        //     // if i % 1048576000 == 0 {
        //     //     eprintln!("### another {} elements added. i={}; u32::MAX={}", 104857600, i, u32::MAX);
        //     // }
        // }
        // publish_to_exhaustion_and_consume_to_emptiness!();
    }

}


/// WARNING: runs only when compiled in Release mode (long runner test)
pub fn container_single_producer_multiple_consumers(produce: impl Fn(u32) -> bool        + Send + Sync,
                                                    consume: impl Fn()    -> Option<u32> + Send + Sync) {

    const DEBUG_PUBLISHMENTS: bool = false;
    const DEBUG_CONSUMPTIONS: bool = false;

    #[cfg(debug_assertions)]
    return eprintln!("TEST DID NOT RUN: runs only when compiled in Release mode (long runner test)");

    let consumer_threads = 2-1;
    let start = 0;
    let finish = 4096000;

    let expected_sum                           = (start + (finish-1)) * ( (finish - start) / 2 );
    let expected_successful_productions        = finish - start;
    let expected_successful_consumptions       = finish - start;
    let observed_sum                      = AtomicU64::new(0);
    let observed_productions              = AtomicU64::new(0);
    let observed_successful_productions   = AtomicU64::new(0);
    let observed_consumptions             = AtomicU64::new(0);
    let observed_successful_consumptions  = AtomicU64::new(0);
    let sanity_check_sum                  = AtomicU64::new(0);


    crossbeam::scope(|scope| {
        // start the multiple consumers
        let consumer_join_handlers: Vec<_> = (0..consumer_threads)
            .into_iter()
            .map(|_| scope.spawn(|_| {
                loop {
                    let i = observed_successful_consumptions.fetch_add(1, Relaxed);
                    if i >= expected_successful_consumptions {
                        observed_successful_consumptions.fetch_sub(1, Relaxed);
                        break;
                    }
                    if DEBUG_CONSUMPTIONS {
                        if i % (expected_successful_consumptions / 100) == 0 {
                            eprint!("c");
                        }
                        if i % (expected_successful_consumptions / 10) == 0 {
                            eprint!("(c{}%)", 10 * (i / (expected_successful_consumptions / 10)));
                        }
                    }
                    let mut result;
                    loop {
                        result = consume();
                        observed_consumptions.fetch_add(1, Relaxed);
                        if result.is_some() {
                            break;
                        }
                        if observed_successful_productions.load(Relaxed) == expected_successful_productions {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            let observed_successful_productions = observed_successful_productions.load(Relaxed);
                            let observed_successful_consumptions = observed_successful_consumptions.load(Relaxed);
                            if observed_successful_productions == expected_successful_productions && observed_successful_consumptions != expected_successful_consumptions {
                                eprintln!("Production already stopped but we are no longer consuming anything. So far, {}; wanted: {}", observed_successful_consumptions, expected_successful_consumptions)
                            }
                        }
                        std::hint::spin_loop();
                    }
                    let element = result.unwrap();
                    observed_sum.fetch_add(element as u64, Relaxed);
                }
                if DEBUG_PUBLISHMENTS {
                    eprintln!("(c100%)");
                }
            })).collect();

        // start the single producer
        let producer_join_handlers: Vec<_> = (0..1)
            .into_iter()
            .map(|_| scope.spawn(|_| {
                let mut i = 0u32;
                while observed_successful_productions.load(Relaxed) < expected_successful_productions {
                    if DEBUG_PUBLISHMENTS {
                        if i % (expected_successful_productions as u32 / 100) == 0 {
                            eprint!("p");
                        }
                        if i % (expected_successful_productions as u32 / 10) == 0 {
                            eprint!("(p{}%)", 10 * (i / (expected_successful_productions as u32 / 10)));
                        }
                    }
                    if produce(i) {
                        sanity_check_sum.fetch_add(i as u64, Relaxed);
                        observed_successful_productions.fetch_add(1, Relaxed);
                        i += 1;
                    } else if observed_successful_consumptions.load(Relaxed) == expected_successful_consumptions {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        let observed_successful_productions = observed_successful_productions.load(Relaxed);
                        let observed_successful_consumptions = observed_successful_consumptions.load(Relaxed);
                        if observed_successful_consumptions == expected_successful_consumptions && observed_successful_productions != expected_successful_productions {
                            eprintln!("Consumption already stopped but we are no longer producing anything. So far, {}; wanted: {}", observed_successful_productions, expected_successful_productions)
                        }
                    }
                    observed_productions.fetch_add(1, Relaxed);
                }
                if DEBUG_PUBLISHMENTS {
                    eprintln!("(p100%)");
                }
            })).collect();

        // // uncomment if this test hangs -- will bring some light into the queue's internal state
        // scope.spawn(|_| {
        //     for _ in 0..15 {
        //         std::thread::sleep(std::time::Duration::from_secs(5));
        //         println!("###Not done yet????");
        //         println!("    PRODUCTION:   {:12} successful, {:12} reported queue was full",
        //                  observed_successful_productions.load(Ordering::Relaxed),
        //                  observed_productions.load(Ordering::Relaxed) - observed_successful_productions.load(Ordering::Relaxed));
        //         println!("                              Sanity check SUM: {:12} (expected: {:12})", sanity_check_sum.load(Ordering::Relaxed), expected_sum);
        //         println!("    CONSUMPTION:  {:12} successful, {:12} reported queue was empty",
        //                  observed_successful_consumptions.load(Ordering::Relaxed),
        //                  observed_consumptions.load(Ordering::Relaxed) - observed_successful_consumptions.load(Ordering::Relaxed));
        //         println!("                                           SUM: {:12} (expected: {:12})", observed_sum.load(Ordering::Relaxed), expected_sum);
        //         //queue.debug();
        //     }
        // });

        consumer_join_handlers.into_iter()
            .for_each(|h| h.join()
                .map_err(|err| format!("Error in consumer thread: {:?}", err))
                .unwrap());
        producer_join_handlers.into_iter()
            .for_each(|h| h.join()
                .map_err(|err| format!("Error in producer thread: {:?}", err))
                .unwrap());
    }).unwrap();

    if observed_successful_consumptions.load(Relaxed) > expected_successful_consumptions {
        eprintln!("BUG!! Thread detected that more elements were consumed than what were published: Expected: {}; Observed: {}",
                  expected_successful_consumptions, observed_successful_consumptions.load(Relaxed));
    }

    println!("'container_single_producer_multiple_consumers' test concluded with:");
    println!("    PUBLISHMENTS:   {:12} successful, {:12} reports of 'full container'",
             observed_successful_productions.load(Relaxed),
             observed_productions.load(Relaxed)- observed_successful_productions.load(Relaxed));
    println!("    CONSUMPTIONS:   {:12} successful, {:12} reports of 'empty container'",
             observed_successful_consumptions.load(Relaxed),
             observed_consumptions.load(Relaxed) - observed_successful_consumptions.load(Relaxed));

    // check
    assert_eq!(sanity_check_sum.load(Relaxed), expected_sum, "Sanity check failed -- most probably an error in the test itself");
    assert_eq!(observed_sum.load(Relaxed),     expected_sum, "Sum failed -- container is, currently, not fully concurrent");
}

/// WARNING: runs only when compiled in Release mode (long runner test)
pub fn container_multiple_producers_single_consumer(produce: impl Fn(u32) -> bool        + Send + Sync,
                                                    consume: impl Fn()    -> Option<u32> + Send + Sync) {
    #[cfg(debug_assertions)]
    return eprintln!("TEST DID NOT RUN: runs only when compiled in Release mode (long runner test)");

    let start = 0;
    let finish = 4096000;
    let producer_threads = 2-1;

    let expected_sum                           = (start + (finish-1)) * ( (finish - start) / 2 );
    let expected_successful_productions        = finish - start;
    let expected_successful_consumptions       = finish - start;
    let observed_sum                      = AtomicU64::new(0);
    let observed_productions              = AtomicU64::new(0);
    let observed_successful_productions   = AtomicU64::new(0);
    let observed_consumptions             = AtomicU64::new(0);
    let observed_successful_consumptions  = AtomicU64::new(0);
    let sanity_check_sum                  = AtomicU64::new(0);


    crossbeam::scope(|scope| {
        // start the single consumer
        scope.spawn(|_| {
            while observed_successful_consumptions.load(Relaxed) < expected_successful_consumptions {
                match consume() {
                    None => if observed_successful_productions.load(Relaxed) == expected_successful_productions {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        let observed_successful_productions = observed_successful_productions.load(Relaxed);
                        let observed_successful_consumptions = observed_successful_consumptions.load(Relaxed);
                        if observed_successful_productions == expected_successful_productions && observed_successful_consumptions != expected_successful_consumptions {
                            eprintln!("Producing already completed, but we are no longer consuming anything. So far, {}; wanted: {}", observed_successful_consumptions, expected_successful_consumptions)
                        }
                    },
                    Some(element) => {
                        observed_sum.fetch_add(element as u64, Relaxed);
                        observed_successful_consumptions.fetch_add(1, Relaxed);
                    }
                }
                observed_consumptions.fetch_add(1, Relaxed);
            }
            if observed_successful_consumptions.load(Relaxed) > expected_successful_consumptions {
                eprintln!("BUG!! Thread detected that more elements were consumed than what were produced: Expected: (at most, at this point) {}; Observed: {}",
                          observed_successful_consumptions.load(Relaxed), expected_successful_consumptions);
            }
        });
        // start the multiple producers
        for _ in 0..producer_threads {
            scope.spawn(|_| {
                loop {
                    let i = observed_successful_productions.fetch_add(1, Relaxed);
                    if i >= expected_successful_productions {
                        observed_successful_productions.fetch_sub(1, Relaxed);
                        break;
                    }
                    while !produce(i as u32) {
                        std::hint::spin_loop();
                        observed_productions.fetch_add(1, Relaxed);
//                        std::thread::sleep(Duration::from_secs(1));
                    }
                    observed_productions.fetch_add(1, Relaxed);
                    sanity_check_sum.fetch_add(i as u64, Relaxed);
                }
            });
        }
        // // uncomment if this test hangs -- will bring some light into the queue's internal state
        // scope.spawn(|_| {
        //     for _ in 0..5 {
        //         std::thread::sleep(std::time::Duration::from_secs(2));
        //         println!("###Not done yet????");
        //         println!("    PRODUCTION:   {:12} successful, {:12} reported container was full",
        //                  observed_successful_productions.load(Relaxed),
        //                  observed_productions.load(Relaxed) - observed_successful_productions.load(Relaxed));
        //         println!("    CONSUMPTION:  {:12} successful, {:12} reported container was empty",
        //                  observed_successful_consumptions.load(Relaxed),
        //                  observed_consumptions.load(Relaxed) - observed_successful_consumptions.load(Relaxed));
        //         //queue.debug();
        //     }
        // });
    }).unwrap();


    println!("'container_multiple_producers_single_consumer' test concluded with:");
    println!("    PRODUCTION:   {:12} successful, {:12} reported container was full",
             observed_successful_productions.load(Relaxed),
             observed_productions.load(Relaxed)- observed_successful_productions.load(Relaxed));
    println!("    CONSUMPTION:  {:12} successful, {:12} reported container was empty",
             observed_successful_consumptions.load(Relaxed),
             observed_consumptions.load(Relaxed)- observed_successful_consumptions.load(Relaxed));

    // check
    assert_eq!(sanity_check_sum.load(Relaxed), expected_sum, "Sanity check failed -- most probably an error in the test itself");
    assert_eq!(observed_sum.load(Relaxed),     expected_sum, "Sum failed -- stack is, currently, not fully concurrent");
}

/// uses varying number of threads for both produce & consume operations in all-in / all-out mode -- produces everybody and then consumes everybody
/// -- asserting the consumed elements sum is correct.\
/// WARNING: runs only when compiled in Release mode (long runner test)
pub fn container_multiple_producers_and_consumers_all_in_and_out(blocking:       Blocking,
                                                                 container_size: usize,
                                                                 produce:        impl Fn(u32) -> bool        + Sync,
                                                                 consume:        impl Fn()    -> Option<u32> + Sync) {
    const MINIMUM_CONTAINER_SIZE: usize = 1024*64;
    const N_THREADS:              usize = 2;    // might as well be num_cpus::get();

    let loops = 320;

    assert!(container_size >= MINIMUM_CONTAINER_SIZE, "Please provide a container with a minimum size of {}", MINIMUM_CONTAINER_SIZE);

    // #[cfg(debug_assertions)]
    // return eprintln!("TEST DID NOT RUN: runs only when compiled in Release mode (long runner test)");

    for _ in 0..loops {
        for threads in N_THREADS ..= N_THREADS {
            let start = 0;
            let finish = container_size as u64;

            let expected_sum = (finish - 1) * (finish - start) / 2;
            let observed_sum = AtomicU64::new(0);

            // all-in (populate)
            multi_threaded_iterate(start as usize, finish as usize, threads, |i| assert!(produce(i), "Container filled up prematurely"));

            // assert fullness, if applicable
            if blocking == Blocking::NonBlocking {
                assert!(!produce(999), "Container should be filled-up, therefore it shouldn't have accepted another element");
            }

            // all-out (consume)
            multi_threaded_iterate(start as usize, finish as usize, threads, |_| match consume() {
                Some(element) => { observed_sum.fetch_add(element as u64, Relaxed); },
                None => panic!("Container ran out of elements prematurely"),
            });

            // assert emptiness, if applicable
            if blocking == Blocking::NonBlocking {
                match consume() {
                    Some(element) => panic!("Container should be empty, therefore it shouldn't have popped an element: {}", element),
                    None => (),    // all good here
                }
            }

            // check
            assert_eq!(observed_sum.load(Relaxed), expected_sum, "Error in all-in / all-out multi-threaded test (with {} threads)", threads);
        }
    }
}

/// uses varying number of threads for both produce & consume operations in single-in / single-out test -- each thread will produce / consume a single element at a time
/// -- asserting the consumed elements sum is correct.\
/// WARNING: runs only when compiled in Release mode (long runner test)
pub fn container_multiple_producers_and_consumers_single_in_and_out(produce: impl Fn(u32) -> bool        + Sync,
                                                                    consume: impl Fn()    -> Option<u32> + Sync) {

    const N_THREADS: usize = 2;    // might as well be num_cpus::get();

    #[cfg(debug_assertions)]
    return eprintln!("TEST DID NOT RUN: runs only when compiled in Release mode (long runner test)");

    let start: u64 = 0;
    let finish: u64 = 4096000;

    let expected_sum                 = (start + (finish-1)) * ( (finish - start) / 2 );
    let expected_callback_calls      = finish - start;
    let observed_callback_calls = AtomicU64::new(0);
    let observed_sum            = AtomicU64::new(0);
    let sanity_check_sum        = AtomicU64::new(0);

    multi_threaded_iterate(start as usize, finish as usize, N_THREADS, |i| {

        observed_callback_calls.fetch_add(1, Relaxed);
        // single-in
        assert!(produce(i), "Container filled up prematurely");

        sanity_check_sum.fetch_add(i as u64, Relaxed);
        // single-out
        let mut consecutive_consumption_failures = 0;
        loop {
            match consume() {
                Some(element) => {
                    observed_sum.fetch_add(element as u64, Relaxed);
                    break;
                },
                None => {
                    // allows container's consume() to fail if bursts of produce() are going on
                    consecutive_consumption_failures += 1;
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    if consecutive_consumption_failures > 100 {
                        let msg = format!("Container ran out of elements prematurely -- i: {}; sequential counter: {}", i, observed_callback_calls.load(Relaxed));
                        eprintln!("{}", msg);
                        panic!("{}", msg);
                    }
                },
            }
        }

    });

    // check
    assert_eq!(observed_callback_calls.load(Relaxed), expected_callback_calls, "¿Wrong number of callback calls?");
    assert_eq!(sanity_check_sum.load(Relaxed),        expected_sum,            "Sanity check failed for single-in / single-out multi-threaded test (with {} threads)", N_THREADS);
    assert_eq!(observed_sum.load(Relaxed),            expected_sum,            "Error in single-in / single-out multi-threaded test (with {} threads)", N_THREADS);

}

/// makes sure the queue waits on a mutex when appropriate -- dequeueing from empty, enqueueing when full --
/// and doesn't wait when not needed -- dequeueing an existing element, enqueueing when there are free slots available
pub fn blocking_behavior(queue_size:    usize,
                         produce:       impl Fn(usize) -> bool          + Send + Sync,
                         consume:       impl Fn()      -> Option<usize> + Send + Sync,
                         try_produce:   impl Fn(usize) -> bool          + Send + Sync,
                         try_consume:   impl Fn()      -> Option<usize> + Send + Sync,
                         interruptable: bool,
                         interrupt:     impl Fn() + Send + Sync) {
    const TIMEOUT_MILLIS:   usize = 100;
    const TOLERANCE_MILLIS: usize = 10;
    // asserts in several passes, so we're sure blocking side effects on mutexes are fine
    for pass in ["virgin", "non-virgin", "promiscuous"] {
        println!("  Asserting pass '{}'", pass);
        assert_block_and_give_up(|| consume(), None, &format!("  Blocking on empty (from a {} container)", pass));
        assert_block_and_give_up(|| consume(), None, "  Blocking on empty (again)");
        assert_non_blocking(|| try_consume(), "  Non-Blocking 'try_consume()'");

        assert_block_and_succeed(|| consume(), Some(50135), || { produce(50135); }, "  Waiting to dequeue");

        assert_non_blocking(|| for i in 0..queue_size {
            produce(i);
        }, "  Blocking 'produce()' (won't block as there are free slots)");

        assert_block_and_give_up(|| produce(queue_size), false, "  Blocking on full");
        assert_block_and_give_up(|| produce(queue_size), false, "  Blocking on full (again)");
        assert_non_blocking(|| try_produce(queue_size), "  Non-Blocking 'try_produce()'");

        assert_block_and_succeed(|| produce(50135), true, || { consume(); }, "  Waiting to 'produce()'");

        assert_non_blocking(|| for i in 0..queue_size {
            consume();
        }, "  Blocking 'consume()' (won't block as there are elements)");
    }

    if interruptable {
        assert_block_and_succeed(|| consume(), None, || { interrupt(); }, "Interrupted consumption");
    }

    /// asserts the value issued by the `blocking_operation()` (which should give up blocking after `TIMEOUT_MILLIS`)
    /// matches `expected_give_up_value`.\
    /// `assertion_name` is used for diagnostic messages.
    fn assert_block_and_give_up<R: PartialEq+Debug>(blocking_operation: impl Fn() -> R, expected_give_up_value: R, assertion_name: &str) {
        print!("{}:  ", assertion_name); std::io::stdout().flush().unwrap();
        let start = SystemTime::now();
        let observed_give_up_value = blocking_operation();
        let elapsed = start.elapsed().unwrap();
        println!("{:?}", elapsed);
        assert!((elapsed.as_millis() as i32 - TIMEOUT_MILLIS as i32).abs() < TOLERANCE_MILLIS as i32,
                "Time spent in the blocked state exceeded the tolerance of {}ms: observed (blocking?) time: {}ms; expected: {}ms",
                TOLERANCE_MILLIS, elapsed.as_millis(), TIMEOUT_MILLIS);
        assert_eq!(observed_give_up_value, expected_give_up_value,
                   "Even if the queue is blocking, it should have given-up waiting -- behaving as non-blocking after the timeout exceeds");
    }

    /// asserts that `blocking_operation()` hangs until `unblock_operation()` completes -- which is fired in another thread, midway to `TIMEOUT_MILLIS`.\
    /// If all goes well, `expected_value` is compared against what `blocking_operation()` returns and `assertion_name` is used for diagnostic messages.
    fn assert_block_and_succeed<R: PartialEq+Debug>(blocking_operation: impl Fn() -> R,
                                                    expected_value:     R,
                                                    unblock_operation:  impl Fn() + Send,
                                                    assertion_name:     &str) {
        print!("{}:  ", assertion_name); std::io::stdout().flush().unwrap();
        let start = SystemTime::now();
        let observed_value = crossbeam::scope(|scope| {
            scope.spawn(move |_| {
                std::thread::sleep(Duration::from_millis(TIMEOUT_MILLIS as u64 / 2));
                unblock_operation();
            });
            blocking_operation()
        }).expect("Crossbeam failed");
        let elapsed = start.elapsed().unwrap();
        println!("{:?}", elapsed);
        assert!((elapsed.as_millis() as i32 - (TIMEOUT_MILLIS as i32 / 2)).abs() < TOLERANCE_MILLIS as i32,
                "Time spent in the blocked state exceeded the tolerance of {}ms: observed (blocking?) time: {}ms; expected: {}ms",
                TOLERANCE_MILLIS, elapsed.as_millis(), TIMEOUT_MILLIS / 2);
        assert_eq!(observed_value, expected_value,
                   "Wrong recovered from blocking operation result for '{}'", assertion_name);
    }

    /// asserts `operation()` is a non-blocking operation (which should return before `TOLLERANCE_MILLIS`)
    fn assert_non_blocking<R: PartialEq+Debug>(operation: impl Fn() -> R, msg: &str) {
        print!("{}:  ", msg); std::io::stdout().flush().unwrap();
        let start = SystemTime::now();
        let _result = operation();
        let elapsed = start.elapsed().unwrap();
        println!("{:?}", elapsed);
        assert!((elapsed.as_millis() as i32).abs() < TOLERANCE_MILLIS as i32,
                "Non-blocking operations should not take considerable time: tolerance of {}ms; observed (non-blocking?) time: {}ms; expected: 0ms ",
                TOLERANCE_MILLIS, elapsed.as_millis());
    }
}

/// measures the independency of producers/consumers, returning:
/// ```no_compile
///   let (independent_productions_count, dependent_productions_count, independent_consumptions_count, dependent_consumptions_count) = measure_syncing_independency(...)
/// ```
/// where `independent_productions_count` & `independent_consumptions_count` are the count of operations done while the counter-part operation was "busy".\
/// Having all counts in those two properties (and none in the "dependent_*" versions, means the operations are independent.\
/// IMPLEMENTATION NOTE: due to the complex thread relations used in this test, lots of `Arc`s are used -- it would be wonderful, for sake of readability, to get rid of them.
pub fn measure_syncing_independency(produce: impl Fn(u32,        &dyn Fn()) -> bool       + Sync + Send,
                                    consume: impl Fn(&AtomicU32, &dyn Fn()) -> Option<()> + Sync + Send)
                                   -> (/*independent_productions_count: */u32, /*dependent_productions_count: */u32, /*independent_consumptions_count: */u32, /*dependent_productions_count: */u32) {

    const N_OPERATIONS: usize = 16;
    const MAX_THREAD_START_MILLIS: Duration = Duration::from_millis(100);

    let independent_productions_count  = Arc::new(AtomicU32::new(0));
    let dependent_productions_count    = Arc::new(AtomicU32::new(0));
    let independent_consumptions_count = Arc::new(AtomicU32::new(0));
    let dependent_consumptions_count   = Arc::new(AtomicU32::new(0));

    let independent_productions_count_ref  = Arc::clone(&independent_productions_count);
    let dependent_productions_count_ref    = Arc::clone(&dependent_productions_count);
    let independent_consumptions_count_ref = Arc::clone(&independent_consumptions_count);
    let dependent_consumptions_count_ref   = Arc::clone(&dependent_consumptions_count);

    crossbeam::scope(|scope| {

        // locked: not allowed to release the current producer
        let release_producer_signal = Arc::new(unsafe { RawMutex::INIT });
        // locked: not allowed to release the current consumer
        let release_consumer_signal = Arc::new(unsafe { RawMutex::INIT });
        // locked: event not produced yet
        let produced_signal = Arc::new(unsafe { RawMutex::INIT });
        // locked: event not consumed yet
        let consumed_signal = Arc::new(unsafe { RawMutex::INIT });

        let release_producer_signal_ref = Arc::clone(&release_producer_signal);
        let release_consumer_signal_ref = Arc::clone(&release_consumer_signal);
        let produced_signal_ref = Arc::clone(&produced_signal);
        let consumed_signal_ref = Arc::clone(&consumed_signal);
        let produced_signal_ref2 = Arc::clone(&produced_signal);
        let consumed_signal_ref2 = Arc::clone(&consumed_signal);

        let wait_for_producer_release = Arc::new(move || release_producer_signal_ref.lock());
        let wait_for_consumer_release = Arc::new(move || release_consumer_signal_ref.lock());
        let inform_production_completed = Arc::new(move || unsafe { produced_signal_ref.unlock() });
        let inform_consumption_completed = Arc::new(move || unsafe { consumed_signal_ref.unlock() });
        let is_production_complete = move || produced_signal_ref2.try_lock_for(MAX_THREAD_START_MILLIS);
        let is_consumption_complete = move || consumed_signal_ref2.try_lock_for(MAX_THREAD_START_MILLIS);
        let release_previous_producer = move || unsafe {
            release_producer_signal.unlock();
            produced_signal.try_lock();
            std::thread::sleep(Duration::from_millis(10));
            release_producer_signal.try_lock();
        };
        let release_previous_consumer = move || unsafe {
            release_consumer_signal.unlock();
            consumed_signal.try_lock();
            std::thread::sleep(Duration::from_millis(10));
            release_consumer_signal.try_lock();
        };

        let produce = Arc::new(produce);
        let consume = Arc::new(consume);

        // signaler
        scope.spawn(move |scope2| {
            for i in 0..N_OPERATIONS as u32 {

                release_previous_producer();
                //produce_next(i);
                let inform_production_completed_ref = Arc::clone(&inform_production_completed);
                let wait_for_producer_release_ref = Arc::clone(&wait_for_producer_release);
                let produce_ref = Arc::clone(&produce);
                let independent_productions_count_ref  = Arc::clone(&independent_productions_count_ref);
                let dependent_productions_count_ref    = Arc::clone(&dependent_productions_count_ref);
                scope2.spawn(move |_| {
                    let post_produce_callback = || {
                        inform_production_completed_ref();
                        wait_for_producer_release_ref();
                    };
                    loop {
                        if produce_ref(i+1, &post_produce_callback) {
                            break
                        } else {
                            std::hint::spin_loop();
                        }
                    }
                });
                if is_production_complete() {
                    independent_productions_count_ref.fetch_add(1, Relaxed);
                } else {
                    dependent_productions_count_ref.fetch_add(1, Relaxed);
                }

                release_previous_consumer();
                //consume_next(i);
                let inform_consumption_completed_ref = Arc::clone(&inform_consumption_completed);
                let wait_for_consumer_release_ref = Arc::clone(&wait_for_consumer_release);
                let consume_ref = Arc::clone(&consume);
                let independent_consumptions_count_ref = Arc::clone(&independent_consumptions_count_ref);
                let dependent_consumptions_count_ref   = Arc::clone(&dependent_consumptions_count_ref);
                scope2.spawn(move |_| {
                    let post_consume_callback = || {
                        inform_consumption_completed_ref();
                        wait_for_consumer_release_ref();
                    };
                    let observed = AtomicU32::new(0);
                    loop {
                        consume_ref(&observed, &post_consume_callback);
                        let observed = observed.load(Relaxed);
                        if observed > 0 {
                            //assert_eq!(observed, i+1, "Consumed a wrong element");
                            break;
                        } else {
                            std::hint::spin_loop();
                        }
                    }
                });
                if is_consumption_complete() {
                    independent_consumptions_count_ref.fetch_add(1, Relaxed);
                } else {
                    dependent_consumptions_count_ref.fetch_add(1, Relaxed);
                }
            }

            release_previous_producer();
            release_previous_consumer();

        });

    }).unwrap();

    ( independent_productions_count.load(Relaxed), dependent_productions_count.load(Relaxed), independent_consumptions_count.load(Relaxed), dependent_consumptions_count.load(Relaxed) )

}
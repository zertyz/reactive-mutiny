//! provides common code used by tests which are also useful for determining the best stack & queue to use at runtime

use std::{
    time::{SystemTime,Duration},
    sync::atomic::{AtomicBool, Ordering},
    io::Write,
};
use std::hint::spin_loop;


/// wraps queues and stacks in a single trait so we can have the same benchmarking code for both
pub trait BenchmarkableContainer<SlotType> {
    fn max_size(&self) -> usize;
    fn add(&self, item: SlotType) -> bool;
    fn remove(&self) -> Option<SlotType>;
    fn implementation_name(&self) -> &str;
}


/// Given a `container' and a `deadline`, recruits the specified number of 'threads' to perform add & remove operations in all-in / all-out mode --
/// pushes everybody and then pops everybody -- repeating the process until 'deadline' exceeds. Returns the projected (or average) number of
/// operations per second (each individual push or pop is considered an operation)
pub fn all_in_and_out_benchmark<ContainerType: BenchmarkableContainer<usize> + Sync>
                               (container: ContainerType, threads: usize, deadline: Duration) -> f64 {

    let container_size = container.max_size();

    let ops = compute_operations_per_second(format!("all_in_and_out_benchmark {}", container.implementation_name()),
                                                  &deadline,
                                                  59,
                                                  || {
                                                      // all-in (populate)
                                                      multi_threaded_iterate(0, container_size, threads, |i| { container.add(i as usize);});
                                                      // all-out (consume)
                                                      multi_threaded_iterate(0, container_size, threads, |_| { container.remove();});
                                                      container_size as u64
                                                  });
    println!("pc"); std::io::stdout().flush().unwrap();
    ops
}

/// Given a `container` and a `deadline`, recruits the specified number of 'threads' to perform add & remove operations in single-in / single-out mode --
/// each thread will push / pop a single element at a time -- repeating the process until 'deadline' exceeds. Returns the projected (or average)
/// number of operations per second (each individual push or pop is considered an operation)
pub fn single_in_and_out_benchmark<ContainerType: BenchmarkableContainer<usize> + Sync>
                                  (container: ContainerType, threads: usize, deadline: Duration) -> f64 {

    let loops_per_iteration = 1<<16;
    let ops = compute_operations_per_second(format!("single_in_and_out_benchmark {}", container.implementation_name()),
                                                 &deadline,
                                                 62,
                                                 || {
                                                     multi_threaded_iterate(0, loops_per_iteration, threads, |i| {
                                                         container.add(i as usize);
                                                         container.remove();
                                                     });
                                                     loops_per_iteration as u64
                                                 });
    println!("pc"); std::io::stdout().flush().unwrap();
    ops
}

/// Given a `container` and a `deadline`, recruits the specified number of 'threads' to perform add & remove operations in single producer / multi consumer mode --
/// a single thread will push and 'threads'-1 will pop -- repeating the process until 'deadline' exceeds. Returns the projected (or average) number of
/// operations per second (each individual push or pop is considered an operation, as long as they succeed)
pub fn single_producer_multiple_consumers_benchmark<ContainerType: BenchmarkableContainer<usize> + Sync>
                                                   (container: ContainerType, threads: usize, deadline: Duration) -> f64 {

    let container_size = container.max_size();
    let consumer_threads = threads-1;

    let keep_running = AtomicBool::new(true);

    let mut operations_per_second = -1.0;

    crossbeam::scope(|scope| {
        // start the multiple consumers
        for _ in 0..consumer_threads {
            scope.spawn(|_| {
                while keep_running.load(Ordering::Relaxed) {
                    for _ in 0..container_size {
                        if container.remove().is_none() {
                            spin_loop();
                            break;
                        }
                    }
                }
                print!("c"); std::io::stdout().flush().unwrap();
            });
        }
        // start the single producer, looping until 'deadline' elapses
        scope.spawn(|_| {
            operations_per_second = compute_operations_per_second(format!("single_producer_multiple_consumers_benchmark {}", container.implementation_name()),
                                                                 &deadline,
                                                                 79,
                                                                 || {
                                                                     let mut additions = 0u64;
                                                                     for i in 0..(container_size*2) {
                                                                         if container.add(i) {
                                                                             additions += 1;
                                                                         } else {
                                                                             spin_loop();
                                                                         }
                                                                     }
                                                                     additions
                                                                 });
            keep_running.store(false, Ordering::Relaxed);
            print!("p"); std::io::stdout().flush().unwrap();
        });
    }).unwrap();
    println!();

    return operations_per_second;
}

/// Given a `container` and a `deadline`, recruits the specified number of 'threads' to perform add & remove operations in multi producer / single consumer mode --
/// a single thread will pop and 'threads'-1 will push -- repeating the process until 'deadline' exceeds. Returns the projected (or average) number of
/// operations per second (each individual push or pop is considered an operation, as long as they succeed)
pub fn multiple_producers_single_consumer_benchmark<ContainerType: BenchmarkableContainer<usize> + Sync>
                                                   (container: ContainerType, threads: usize, deadline: Duration) -> f64 {

    let container_size = container.max_size();
    let producer_threads = threads-1;

    let keep_running = AtomicBool::new(true);

    let mut operations_per_second = -1.0;

    crossbeam::scope(|scope| {
        // start the single consumer, looping until 'deadline' elapses
        scope.spawn(|_| {
            operations_per_second = compute_operations_per_second(format!("multiple_producers_single_consumer_benchmark {}", container.implementation_name()),
                                                                  &deadline,
                                                                  79,
                                                                  || {
                                                                      let mut consumption = 0u64;
                                                                      for _ in 0..container_size*2 {
                                                                          if container.remove().is_some() {
                                                                              consumption += 1;
                                                                          } else {
                                                                              spin_loop()
                                                                          }
                                                                      }
                                                                      consumption
                                                                  });
            keep_running.store(false, Ordering::Relaxed);
            print!("c"); std::io::stdout().flush().unwrap();
        });
        // start the multiple producers
        for _ in 0..producer_threads {
            scope.spawn(|_| {
                let mut i = 0usize;
                while keep_running.load(Ordering::Relaxed) {
                    for _ in 0..container_size {
                        if container.add(i) {
                            i += 1;
                        } else {
                            spin_loop();
                            break;
                        }
                    }
                }
                print!("p"); std::io::stdout().flush().unwrap();
            });
        }
    }).unwrap();
    println!();

    return operations_per_second;
}


// auxiliary functions
//////////////////////

fn compute_operations_per_second(benchmark_name:      String,
                                 deadline:            &Duration,
                                 padding:             usize,
                                 mut algorithm:       impl FnMut() -> u64)
                                -> f64 {
    print!("    ... running {:1$} ", format!("{}:", benchmark_name), padding); std::io::stdout().flush().unwrap();
    let mut total_operations: u64 = 0;
    let start_time = SystemTime::now();
    // loop until 'deadline' elapses
    loop {
        let operations_on_last_run = algorithm();
        if let Some(operations_per_second) = keep_looping(&start_time, &mut total_operations, &deadline, operations_on_last_run) {
            print!("{:12.2} ops/s: ", operations_per_second); std::io::stdout().flush().unwrap();
            return operations_per_second
        }
    }
}

/// benchmarking function... to exceute until the `deadline`.
/// Returns Some(operations_per_second) when it is time to stop looping
fn keep_looping(start_time: &SystemTime, total_operations: &mut u64, deadline: &Duration, operations_on_last_run: u64) -> Option<f64> {
    const MICROS_IN_ONE_SECOND: f64 = 1e6;
    *total_operations += operations_on_last_run;
    let elapsed = start_time.elapsed().unwrap();
    if elapsed >= *deadline {
        let elapsed_micros = elapsed.as_micros();
        let operations_per_second = MICROS_IN_ONE_SECOND * (*total_operations as f64 / elapsed_micros as f64);
        return Some(operations_per_second);
    } else {
        None
    }
}

/// iterate from 'start' to 'finish', dividing the work among the given number of 'threads', calling 'callback' on each iteration
pub fn multi_threaded_iterate(start: usize, finish: usize, threads: usize, callback: impl Fn(u32) -> () + std::marker::Sync) {
    crossbeam::scope(|scope| {
        let cb = &callback;
        let join_handlers: Vec<crossbeam::thread::ScopedJoinHandle<()>> = (start..start+threads).into_iter()
            .map(|thread_number| scope.spawn(move |_| iterate(thread_number, finish, threads, &cb)))
            .collect();
        for join_handler in join_handlers {
            join_handler.join().unwrap();
        }
    }).unwrap();
}

/// iterate from 'start' to 'finish' with the given 'step' size and calls 'callback' on each iteration
fn iterate(start: usize, finish: usize, step: usize, callback: impl Fn(u32) -> () + std::marker::Sync) {
    for i in (start..finish).step_by(step) {
        callback(i as u32);
    }
}

#[cfg(any(test,doc))]
mod benchmark_stacks {
    //! Benchmarks all known stacks

    use super::*;
    use super::super::ogre_stacks::OgreStack;
    use std::fmt::Debug;


    // implementation note: doing something like bellow for both queues & stacks is not an option,
    // as of Rust 1.63 due to conflicting implementations -- we cannot exclude traits yet
    // impl<StackType, SlotType> BenchmarkableContainer<SlotType>
    // for StackType where StackType: OgreStack<SlotType> {
    //     ...
    // }

    macro_rules! impl_benchmarkable_container_for {
        ($stack_type: ty) => {
            impl<SlotType: Copy+Debug, const BUFFER_SIZE: usize> BenchmarkableContainer<SlotType> for $stack_type {
                fn max_size(&self) -> usize {
                    OgreStack::<SlotType>::buffer_size(self)
                }
                fn add(&self, item: SlotType) -> bool {
                    self.push(item)
                }
                fn remove(&self) -> Option<SlotType> {
                    self.pop()
                }
                fn implementation_name(&self) -> &str {
                    OgreStack::<SlotType>::implementation_name(self)
                }
            }
        }
    }

    impl_benchmarkable_container_for!(super::super::ogre_stacks::non_blocking_atomic_stack::Stack::<SlotType, BUFFER_SIZE, false, false>);

    #[cfg_attr(not(doc),test)]
    #[ignore]   // must run in a single thread for accurate measurements
    fn all_in_and_out_benchmarks() {
        println!();
        for n_threads in [1, 2, 4] {
            println!("{n_threads} threads:");
            all_in_and_out_benchmark(super::super::ogre_stacks::non_blocking_atomic_stack::Stack::<usize, 65536, false, false>::new("".to_string()), n_threads, Duration::from_secs(5));
        }
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // must run in a single thread for accurate measurements
    fn single_in_and_out_benchmarks() {
        println!();
        for n_threads in [1, 2, 4] {
            println!("{n_threads} threads:");
            single_in_and_out_benchmark(super::super::ogre_stacks::non_blocking_atomic_stack::Stack::<usize, 65536, false, false>::new("".to_string()), n_threads, Duration::from_secs(5));
        }
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // must run in a single thread for accurate measurements
    fn single_producer_multiple_consumers_benchmarks() {
        println!();
        for n_threads in [2, 3, 5] {
            println!("{n_threads} threads:");
            single_producer_multiple_consumers_benchmark(super::super::ogre_stacks::non_blocking_atomic_stack::Stack::<usize, 65536, false, false>::new("".to_string()), n_threads, Duration::from_secs(5));
        }
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // must run in a single thread for accurate measurements
    fn multiple_producers_single_consumer_benchmarks() {
        println!();
        for n_threads in [2, 3, 5] {
            println!("{n_threads} threads:");
            multiple_producers_single_consumer_benchmark(super::super::ogre_stacks::non_blocking_atomic_stack::Stack::<usize, 65536, false, false>::new("".to_string()), n_threads, Duration::from_secs(5));
        }
    }

}


#[cfg(any(test,doc))]
mod benchmark_queues {
    //! Benchmarks all known queues
    //! TODO: as of 2023-05-08, changes to the queue's publishers & subscribers brought problems to this test. More specifically, NonBlocking & Blocking queues are in trouble now... should they implement the Movable or Zero-Copy? Or both?
    //!       anyway, we now have `benches`, so... are these really necessary?

    use super::*;
    use super::super::{
        instruments::Instruments,
        ogre_queues::OgreQueue
    };
    use std::fmt::Debug;

    macro_rules! impl_benchmarkable_container_for {
        ($queue_type: ty) => {
            impl<SlotType: Copy+Unpin+Debug+Send+Sync, const BUFFER_SIZE: usize> BenchmarkableContainer<SlotType> for $queue_type {
                fn max_size(&self) -> usize {
                    OgreQueue::max_size(self)
                }
                fn add(&self, item: SlotType) -> bool {
                    self.enqueue(item).is_none()
                }
                fn remove(&self) -> Option<SlotType> {
                    self.dequeue()
                }
                fn implementation_name(&self) -> &str {
                    OgreQueue::implementation_name(self)
                }
            }
        }
    }

    macro_rules! _impl_benchmarkable_container_for_blocking {
        ($queue_type: ty) => {
            impl<SlotType: Copy+Unpin+Debug+Send+Sync, const BUFFER_SIZE: usize> BenchmarkableContainer<SlotType> for $queue_type {
                fn max_size(&self) -> usize {
                    OgreQueue::max_size(self)
                }
                fn add(&self, item: SlotType) -> bool {
                    self.try_enqueue(item)
                }
                fn remove(&self) -> Option<SlotType> {
                    self.try_dequeue()
                }
                fn implementation_name(&self) -> &str {
                    OgreQueue::implementation_name(self)
                }
            }
        }
    }

    // Implementation of bookmark ability for queues
    ////////////////////////////////////////////////
    // NOTE: blocking queues should be tested without lock timeouts

    impl_benchmarkable_container_for!(super::super::ogre_queues::atomic::NonBlockingQueue::<SlotType, BUFFER_SIZE, {Instruments::NoInstruments.into()}>);
    impl_benchmarkable_container_for!(super::super::ogre_queues::atomic::BlockingQueue::<SlotType, BUFFER_SIZE, 1, {Instruments::NoInstruments.into()}>);
    impl_benchmarkable_container_for!(super::super::ogre_queues::full_sync::NonBlockingQueue::<SlotType, BUFFER_SIZE, {Instruments::NoInstruments.into()}>);

    #[cfg_attr(not(doc),test)]
    #[ignore]   // must run in a single thread for accurate measurements
    fn all_in_and_out_benchmarks() {
        println!();
        for n_threads in [1,2,4] {
            println!("{n_threads} threads:");
            all_in_and_out_benchmark(super::super::ogre_queues::atomic::NonBlockingQueue::<usize, 32768, {Instruments::NoInstruments.into()}>::new("".to_string()), n_threads, Duration::from_secs(5));
            all_in_and_out_benchmark(super::super::ogre_queues::atomic::BlockingQueue::<usize, 32768, 1, {Instruments::NoInstruments.into()}>::new("".to_string()), n_threads, Duration::from_secs(5));
            all_in_and_out_benchmark(super::super::ogre_queues::full_sync::NonBlockingQueue::<usize, 32768, {Instruments::NoInstruments.into()}>::new("".to_string()), n_threads, Duration::from_secs(5));
        }
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // must run in a single thread for accurate measurements
    fn single_in_and_out_benchmarks() {
        println!();
        for n_threads in [1,2,4] {
            println!("{n_threads} threads:");
            single_in_and_out_benchmark(super::super::ogre_queues::atomic::NonBlockingQueue::<usize, 32768, {Instruments::NoInstruments.into()}>::new("".to_string()), n_threads, Duration::from_secs(5));
            single_in_and_out_benchmark(super::super::ogre_queues::atomic::BlockingQueue::<usize, 32768, 1, {Instruments::NoInstruments.into()}>::new("".to_string()), n_threads, Duration::from_secs(5));
            single_in_and_out_benchmark(super::super::ogre_queues::full_sync::NonBlockingQueue::<usize, 32768, {Instruments::NoInstruments.into()}>::new("".to_string()), n_threads, Duration::from_secs(5));
        }
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // must run in a single thread for accurate measurements
    fn single_producer_multiple_consumers_benchmarks() {
        println!();
        for n_threads in [2,3,5] {
            println!("{n_threads} threads:");
            single_producer_multiple_consumers_benchmark(super::super::ogre_queues::atomic::NonBlockingQueue::<usize, 32768, {Instruments::NoInstruments.into()}>::new("".to_string()), n_threads, Duration::from_secs(5));
            single_producer_multiple_consumers_benchmark(super::super::ogre_queues::atomic::BlockingQueue::<usize, 32768, 1, {Instruments::NoInstruments.into()}>::new("".to_string()), n_threads, Duration::from_secs(5));
            single_producer_multiple_consumers_benchmark(super::super::ogre_queues::full_sync::NonBlockingQueue::<usize, 32768, {Instruments::NoInstruments.into()}>::new("".to_string()), n_threads, Duration::from_secs(5));
        }
    }

    #[cfg_attr(not(doc),test)]
    #[ignore]   // must run in a single thread for accurate measurements
    fn multiple_producers_single_consumer_benchmarks() {
        println!();
        for n_threads in [2,3,5] {
            println!("{n_threads} threads:");
            multiple_producers_single_consumer_benchmark(super::super::ogre_queues::atomic::NonBlockingQueue::<usize, 32768, {Instruments::NoInstruments.into()}>::new("".to_string()), n_threads, Duration::from_secs(5));
            multiple_producers_single_consumer_benchmark(super::super::ogre_queues::atomic::BlockingQueue::<usize, 32768, 1, {Instruments::NoInstruments.into()}>::new("".to_string()), n_threads, Duration::from_secs(5));
            multiple_producers_single_consumer_benchmark(super::super::ogre_queues::full_sync::NonBlockingQueue::<usize, 32768, {Instruments::NoInstruments.into()}>::new("".to_string()), n_threads, Duration::from_secs(5));
        }
    }

}


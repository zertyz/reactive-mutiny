//! provides common code used by tests which are also useful for determining the best stack to use at runtime

use std::{
    time::{SystemTime,Duration},
    sync::atomic::{AtomicBool, Ordering},
};


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
pub fn all_in_and_out_benchmark(container: &(dyn BenchmarkableContainer<usize> + Sync), threads: usize, deadline: Duration) -> f64 {

    let container_size = container.max_size();
    let mut iterations: u64 = 0;

    // loop until 'deadline' elapses
    let start_time = SystemTime::now();
    loop {
        // all-in (populate)
        multi_threaded_iterate(0, container_size, threads, |i| { container.add(i as usize);});

        // all-out (consume)
        multi_threaded_iterate(0, container_size, threads, |_| { container.remove();});

        match keep_looping_or_compute_operations_per_second(format!("all_in_and_out_benchmark('{}')", container.implementation_name()),
                                                            &start_time,
                                                            &mut iterations,
                                                            &deadline,
                                                            container_size as u64) {
            Some(operations_per_second) => return operations_per_second,
            None => (),
        }
    }
}

/// Given a `container` and a `deadline`, recruits the specified number of 'threads' to perform add & remove operations in single-in / single-out mode --
/// each thread will push / pop a single element at a time -- repeating the process until 'deadline' exceeds. Returns the projected (or average)
/// number of operations per second (each individual push or pop is considered an operation)
pub fn single_in_and_out_benchmark(container: &(dyn BenchmarkableContainer<usize> + Sync), threads: usize, deadline: Duration) -> f64 {

    let loops_per_iteration = 1<<16;
    let mut iterations: u64 = 0;

    // loop until 'deadline' elapses
    let start_time = SystemTime::now();
    loop {
        multi_threaded_iterate(0, loops_per_iteration, threads, |i| {
            container.add(i as usize);
            container.remove();
        });
        match keep_looping_or_compute_operations_per_second(format!("single_in_and_out_benchmark(('{}')", container.implementation_name()),
                                                            &start_time,
                                                            &mut iterations,
                                                            &deadline,
                                                            loops_per_iteration as u64) {
            Some(operations_per_second) => return operations_per_second,
            None => (),
        }
    }
}

/// Given a `container` and a `deadline`, recruits the specified number of 'threads' to perform add & remove operations in single producer / multi consumer mode --
/// a single thread will push and 'threads'-1 will pop -- repeating the process until 'deadline' exceeds. Returns the projected (or average) number of
/// operations per second (each individual push or pop is considered an operation, as long as they succeed)
pub fn single_producer_multiple_consumers_benchmark(container: &(dyn BenchmarkableContainer<usize> + Sync), threads: usize, deadline: Duration) -> f64 {

    let container_size = container.max_size();
    let consumer_threads = threads-1;

    let keep_running = AtomicBool::new(true);

    let mut operations_per_second: f64 = 0.0;

    let start_time = SystemTime::now();

    crossbeam::scope(|scope| {
        // start the multiple consumers
        for _ in 0..consumer_threads {
            scope.spawn(|_| {
                while keep_running.load(Ordering::Relaxed) {
                    if container.remove().is_none() {
                        std::hint::spin_loop();
                    }
                }
            });
        }
        // start the single producer, looping until 'deadline' elapses
        scope.spawn(|_| {
            let mut iterations: u64 = 0;
            loop {
                for i in 0..container_size {
                    while !container.add(i) {
                        std::hint::spin_loop();
                    }
                }
                match keep_looping_or_compute_operations_per_second(format!("single_producer_multiple_consumers_benchmark(('{}')", container.implementation_name()),
                                                                    &start_time,
                                                                    &mut iterations,
                                                                    &deadline,
                                                                    container_size as u64) {
                    Some(_operations_per_second) => {
                        operations_per_second = _operations_per_second;
                        keep_running.store(false, Ordering::Relaxed);
                        break;
                    },
                    None => (),
                }
            }
        });
    }).unwrap();

    return operations_per_second;
}

/// Given a `container` and a `deadline`, recruits the specified number of 'threads' to perform add & remove operations in multi producer / single consumer mode --
/// a single thread will pop and 'threads'-1 will push -- repeating the process until 'deadline' exceeds. Returns the projected (or average) number of
/// operations per second (each individual push or pop is considered an operation, as long as they succeed)
pub fn multiple_producers_single_consumer_benchmark(container: &(dyn BenchmarkableContainer<usize> + Sync), threads: usize, deadline: Duration) -> f64 {

    let container_size = container.max_size();
    let producer_threads = threads-1;

    let keep_running = AtomicBool::new(true);

    let mut operations_per_second: f64 = 0.0;

    let start_time = SystemTime::now();

    crossbeam::scope(|scope| {
        // start the single consumer, looping until 'deadline' elapses
        scope.spawn(|_| {
            let mut iterations: u64 = 0;
            loop {
                for _ in 0..container_size {
                    if container.remove().is_none() {
                        std::hint::spin_loop();
                    }
                }
                match keep_looping_or_compute_operations_per_second(format!("multiple_producers_single_consumer_benchmark(('{}')", container.implementation_name()),
                                                                    &start_time,
                                                                    &mut iterations,
                                                                    &deadline,
                                                                    container_size as u64) {
                    Some(_operations_per_second) => {
                        operations_per_second = _operations_per_second;
                        keep_running.store(false, Ordering::Relaxed);
                        break;
                    },
                    None => (),
                }
            }
        });
        // start the multiple producers
        for _ in 0..producer_threads {
            scope.spawn(|_| {
                let mut i = 0usize;
                while keep_running.load(Ordering::Relaxed) {
                    if container.add(i) {
                        i += 1;
                    } else {
                        std::hint::spin_loop();
                    }
                }
            });
        }
    }).unwrap();

    return operations_per_second;
}


// auxiliary functions
//////////////////////

fn keep_looping_or_compute_operations_per_second(benchmark_name: String, start_time: &SystemTime, iterations: &mut u64, deadline: &Duration, loops_per_iteration: u64) -> Option<f64> {
    const MICROS_IN_ONE_SECOND: f64 = 10e6;
    *iterations += 1;
    let elapsed = start_time.elapsed().unwrap();
    if elapsed >= *deadline {
        let elapsed_micros = elapsed.as_micros();
        let operations = 2 * *iterations * loops_per_iteration;  // 2 is 1 for add + 1 for remove
        let operations_per_second = MICROS_IN_ONE_SECOND * (operations as f64 / elapsed_micros as f64);
        println!("    ... running '{}': {:12.4} ops/s", benchmark_name, operations_per_second);
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

#[cfg(any(test, feature="dox"))]
mod benchmark_stacks {
    //! Benchmarks all known stacks

    use std::fmt::Debug;
    use super::*;
    use super::super::ogre_stacks::OgreStack;


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

    #[test]
    #[ignore]
    fn all_in_and_out_benchmarks() {
        println!();
        for n_threads in 1..=4 {
            println!("{n_threads} threads:");
            all_in_and_out_benchmark(&super::super::ogre_stacks::non_blocking_atomic_stack::Stack::<usize, 65536, false, false>::new("".to_string()), n_threads, Duration::from_secs(5));
        }
    }

    #[test]
    #[ignore]
    fn single_in_and_out_benchmarks() {
        println!();
        for n_threads in 1..=4 {
            println!("{n_threads} threads:");
            single_in_and_out_benchmark(&super::super::ogre_stacks::non_blocking_atomic_stack::Stack::<usize, 65536, false, false>::new("".to_string()), n_threads, Duration::from_secs(5));
        }
    }

    #[test]
    #[ignore]
    fn single_producer_multiple_consumers_benchmarks() {
        println!();
        for n_threads in 2..=5 {
            println!("{n_threads} threads:");
            single_producer_multiple_consumers_benchmark(&super::super::ogre_stacks::non_blocking_atomic_stack::Stack::<usize, 65536, false, false>::new("".to_string()), n_threads, Duration::from_secs(5));
        }
    }

    #[test]
    #[ignore]
    fn multiple_producers_single_consumer_benchmarks() {
        println!();
        for n_threads in 2..=5 {
            println!("{n_threads} threads:");
            multiple_producers_single_consumer_benchmark(&super::super::ogre_stacks::non_blocking_atomic_stack::Stack::<usize, 65536, false, false>::new("".to_string()), n_threads, Duration::from_secs(5));
        }
    }

}


#[cfg(any(test, feature="dox"))]
mod benchmark_queues {
    //! Benchmarks all known queues

    use super::*;
    use super::super::{
        container_instruments::ContainerInstruments,
        ogre_queues::OgreQueue,
    };
    use std::{
        fmt::Debug,
        pin::Pin,
    };

    macro_rules! impl_benchmarkable_container_for {
        ($queue_type: ty) => {
            impl<SlotType: Copy+Unpin+Debug, const BUFFER_SIZE: usize> BenchmarkableContainer<SlotType> for $queue_type {
                fn max_size(&self) -> usize {
                    OgreQueue::max_size(self)
                }
                fn add(&self, item: SlotType) -> bool {
                    self.enqueue(item)
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

    impl_benchmarkable_container_for!(super::super::ogre_queues::atomic_queues::NonBlockingQueue::<SlotType, BUFFER_SIZE, {ContainerInstruments::NoInstruments.into()}>);
    impl_benchmarkable_container_for!(super::super::ogre_queues::atomic_queues::BlockingQueue::<SlotType, BUFFER_SIZE, {ContainerInstruments::NoInstruments.into()}>);
    impl_benchmarkable_container_for!(super::super::ogre_queues::blocking_queue::Queue::<'static, SlotType, BUFFER_SIZE, false, false, 1>);
    impl_benchmarkable_container_for!(super::super::ogre_queues::full_sync_queues::NonBlockingQueue::<SlotType, BUFFER_SIZE, false, false>);

    #[test]
    #[ignore]
    fn all_in_and_out_benchmarks() {
        println!();
        for n_threads in 1..=4 {
            println!("{n_threads} threads:");
            all_in_and_out_benchmark(Pin::into_inner(super::super::ogre_queues::atomic_queues::NonBlockingQueue::<usize, 65536, {ContainerInstruments::NoInstruments.into()}>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));
            all_in_and_out_benchmark(Pin::into_inner(super::super::ogre_queues::atomic_queues::BlockingQueue::<usize, 65536, {ContainerInstruments::NoInstruments.into()}>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));
            unsafe {all_in_and_out_benchmark(Pin::into_inner_unchecked(super::super::ogre_queues::blocking_queue::Queue::<'static, usize, 65536, false, false, 1>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));}
            all_in_and_out_benchmark(Pin::into_inner(super::super::ogre_queues::full_sync_queues::NonBlockingQueue::<usize, 65536, false, false>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));
        }
    }

    #[test]
    #[ignore]
    fn single_in_and_out_benchmarks() {
        println!();
        for n_threads in 1..=4 {
            println!("{n_threads} threads:");
            single_in_and_out_benchmark(Pin::into_inner(super::super::ogre_queues::atomic_queues::NonBlockingQueue::<usize, 65536, {ContainerInstruments::NoInstruments.into()}>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));
            single_in_and_out_benchmark(Pin::into_inner(super::super::ogre_queues::atomic_queues::BlockingQueue::<usize, 65536, {ContainerInstruments::NoInstruments.into()}>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));
            unsafe {single_in_and_out_benchmark(Pin::into_inner_unchecked(super::super::ogre_queues::blocking_queue::Queue::<'static, usize, 65536, false, false, 1>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));}
            single_in_and_out_benchmark(Pin::into_inner(super::super::ogre_queues::full_sync_queues::NonBlockingQueue::<usize, 65536, false, false>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));
        }
    }

    #[test]
    #[ignore]
    fn single_producer_multiple_consumers_benchmarks() {
        println!();
        for n_threads in 2..=5 {
            println!("{n_threads} threads:");
            single_producer_multiple_consumers_benchmark(Pin::into_inner(super::super::ogre_queues::atomic_queues::NonBlockingQueue::<usize, 65536, {ContainerInstruments::NoInstruments.into()}>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));
            single_producer_multiple_consumers_benchmark(Pin::into_inner(super::super::ogre_queues::atomic_queues::BlockingQueue::<usize, 65536, {ContainerInstruments::NoInstruments.into()}>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));
            unsafe {single_producer_multiple_consumers_benchmark(Pin::into_inner_unchecked(super::super::ogre_queues::blocking_queue::Queue::<'static, usize, 65536, false, false, 1>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));}
            single_producer_multiple_consumers_benchmark(Pin::into_inner(super::super::ogre_queues::full_sync_queues::NonBlockingQueue::<usize, 65536, false, false>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));
        }
    }

    #[test]
    #[ignore]
    fn multiple_producers_single_consumer_benchmarks() {
        println!();
        for n_threads in 2..=5 {
            println!("{n_threads} threads:");
            multiple_producers_single_consumer_benchmark(Pin::into_inner(super::super::ogre_queues::atomic_queues::NonBlockingQueue::<usize, 65536, {ContainerInstruments::NoInstruments.into()}>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));
            multiple_producers_single_consumer_benchmark(Pin::into_inner(super::super::ogre_queues::atomic_queues::BlockingQueue::<usize, 65536, {ContainerInstruments::NoInstruments.into()}>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));
            unsafe {multiple_producers_single_consumer_benchmark(Pin::into_inner_unchecked(super::super::ogre_queues::blocking_queue::Queue::<'static, usize, 65536, false, false, 1>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));}
            multiple_producers_single_consumer_benchmark(Pin::into_inner(super::super::ogre_queues::full_sync_queues::NonBlockingQueue::<usize, 65536, false, false>::new("".to_string())).as_ref(), n_threads, Duration::from_secs(5));
        }
    }

}


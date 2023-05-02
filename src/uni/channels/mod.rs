pub mod uni_stream;
pub mod crossbeam;
pub mod atomic_mpmc_queue;
pub mod ogre_full_sync_mpmc_queue;

use crate::ogre_std::ogre_queues::meta_subscriber::MetaSubscriber;
use std::{
    io::Write,
    pin::Pin,
    sync::Arc,
    time::Duration,
    fmt::Debug,
    sync::atomic::Ordering::Relaxed,
    task::{Context, Poll, Waker},
    mem::{self, ManuallyDrop, MaybeUninit},
};
use futures::stream::Stream;
use async_trait::async_trait;
use owning_ref::ArcRef;


/// Defines abstractions over Channels and other structures suitable for being used in [Uni]s.\
/// `UniChannel`s transfer unitary messages between the producer (who calls `send()`) and the consumer(s) `Stream`(s) and
/// implementors allow a combination of across threads & buffered transfers capabilities, as well as splitting the received messages to
/// multiple `Streams` (allowing parallel processing).\
/// As an optimization, there are no `close()` operations, since keeping a state to be consulted every time imposes a performance hit.
/// Instead, callers are encouraged to perform the "close semantics" themselves:
///   1. Stop producing new items
///   2. Call this trait's `.flush()` method
///   3. Drop all instances & Streams.
///
/// Known implementers of this trait are: [mutex_mpmc_queue], [atomic_mpmc_queue], [crossbeam].\
///
/// See also: [MultiChannel], allowing a single `send()` to reach multiple consumers (broadcast).
///
/// implementation note: Rust 1.63 does not yet support async traits and the alternative -- using the `async-trait` crate -- introduces
///                      inefficiencies, as `Box::pin()` allocation is required for every returned future. For this reason, we refrained from
///                      adding an `async fn send()` -- this hot-path should be implemented by the caller to avoid the mentioned overhead.
///                      Refer to [UniChannel::try_send()] for how to do it.
/// implementation note 2: Rust 1.63 rules out any optimizations if this trait is used -- since we're not yet allowed to return `impl` in trait
///                        functions, `consumer_stream()` callers wouldn't manage to get the compiler / linker to optimize / mangle / inline their
///                        code together if we return a `Box<dyn>` reference. For this reason, this trait is, currently, useless. Nonetheless,
///                        implementors should honor the function names and signatures defined here, as macros will be used to circumvent the absence
///                        of a trait relating all implementors together (ought!) -- see: [impl_uni_channel_for_struct!()]
#[async_trait]
pub trait UniChannel<ItemType> {

    /// Creates a new instance of this channel
    fn new() -> Arc<Pin<Box<Self>>>;

    // /// Returns a stream able to consume elements sent through this channel.\
    // /// For MPSC (multi-producer, single-consumer) channels, this method will only produce 1 stream.\
    // /// If called on a MPMC (multi-producer, multi-Consumer) channel, every call will produce a new stream
    // /// which will, concurrently, grab the next entry available in this channel -- using the consumer pattern.
    // fn consumer_stream(self: &Arc<Pin<Box<Self>>>) -> Option<impl Stream<Item=ItemType>>;
    // additional implementation note: Rust 1.63 also don't allow trait functions to return `impl` versions,
    // so this should be implemented (by implementors of this trait) out of this trait.
    // A macro will enforce users of this trait have that function available

    // disabled for now. See implementation note on this trait.
    // /// Sends `item` through this channel, asynchronously, waiting for a slot if the buffer is full
    // async fn send(&self, item: ItemType);

    /// For channels that allow optimized allocators, this version of send should be used instead, as it will enforce
    /// a zero-copy policy (which is likely to payoff for data types some dozens of bytes wide).\
    /// This is marked as unsafe for extra care should be taken when providing the `item_builder()` callback: it will reuse
    /// a previous `ItemType`, so care should be taken for old data not to leak out as if it was new.
    unsafe fn zero_copy_try_send(&self, item_builder: impl FnOnce(&mut ItemType)) -> bool;

    /// Sends `item` through this channel, synchronously, returning immediately if the buffer is full
    /// -- returns `false` if the buffer was full and the `item` wasn't sent; `true` otherwise.
    fn try_send(&self, item: ItemType) -> bool;

    /// Waits until all pending items are taken from this channel, up until `timeout` elapses.\
    /// Returns the number of still unconsumed items -- which is 0 if it was not interrupted by the timeout
    async fn flush(&self, timeout: Duration) -> u32;

    /// Flushes & signals that all streams should cease their activities when there are no more elements left
    /// to process, waiting for the operation to complete up to `timeout`.\
    /// Returns the number of un-ended streams -- which is 0 if it was not interrupted by the timeout
    async fn end_all_streams(&self, timeout: Duration) -> u32;

    /// Tells how many items are waiting to be taken out of this channel.\
    fn pending_items_count(&self) -> u32;


    // to be moved to the Uni
    /////////////////////////

    // /// Method through which an executor reports it is processing items from this channel
    // fn report_executor_started(&self);
    //
    // /// Method through which executors tells this channel they have ceased executing
    // fn report_executor_finished(&self);
    //
    // /// Tells how many executors found this channel to have come to its end and have fully processed
    // /// the last item.\
    // /// `if self.closed() && self.pending_items_count() == 0 && self.active_executors_count() == 0`
    // /// then you may safely drop this channel.
    // fn active_executors_count(&self) -> u32;

}

/// IMPORTANT: Hacky solution ahead -- see [UniChannel]'s development note 2 for the reason why this macro should exist!\
/// This macro receives an `implementer` of [UniChannel] -- such as [], [] or []
/// -- and implements all the trait functions for the given `struct`, so they can be called locally.\
/// This is a hack required by Rust 1.63 so performance won't be dropped -- see the referred implementation note!
macro_rules! impl_uni_channel_for_struct {
        ($strct: tt, $implementer_type: ty, $implementer_field: ident) => {

            impl $strct<$implementer_type> {

                pub fn yeah(&self) {
                    println!("yeah!: {}", self.$implementer_field.try_send(0));
                }

            }

        }
}


/// Tests & enforces the requisites & expose good practices & exercises the API of of the [uni/channels](self) module
/// WARNING: unusual test module ahead -- macros are used to implement test functions.\
///          this is due to the implementation notes on [UniChannel]: we can't have a trait to unify all
///          implementor's behavior (as of Rust 1.63), therefore, we use macros to allow writing test code
///          only once and get them specialized automatically for each type. This also tests that the API
///          is unified among implementors (since there is no Trait to enforce it by now).\
/// NOTE for USERS: you're likely to, unfortunately, also have to resort to the same macro strategy as of Rust 1.63
///                 -- or pick a single hard coded (but easy to replace) type as in the example bellow
///                    (which may even be configured by a "features" flag in Cargo):
/// ```no_compile
///     type UniChannelType<ItemType, BUFFER_SIZE> = mutex_mpmc_queue::OgreMPMCQueue<ItemType, BUFFER_SIZE>
#[cfg(any(test,doc))]
mod tests {
    use std::fmt::Debug;
    use std::future::Future;
    use super::{*, crossbeam};
    use crate::uni::channels::{
        crossbeam::Crossbeam,
        atomic_mpmc_queue::AtomicMPMCQueue,
        ogre_full_sync_mpmc_queue::OgreFullSyncMPMCQueue,
    };
    use futures::{stream, StreamExt};
    use minstant::Instant;


    #[ctor::ctor]
    fn suite_setup() {
        simple_logger::SimpleLogger::new().with_utc_timestamps().init().unwrap_or_else(|_| eprintln!("--> LOGGER WAS ALREADY STARTED"));
    }

    // *_doc_test for all known `UniChannel`s
    /////////////////////////////////////////

    macro_rules! doc_test {
        ($fn_name: tt, $uni_channel_type: ty) => {
            /// exercises the code present on the documentation for $uni_channel_type
            #[cfg_attr(not(doc),tokio::test)]
            async fn $fn_name() {
                let channel = <$uni_channel_type>::new("doc_test");
                let mut stream = channel.consumer_stream();
                let send_result = channel.try_send("a");
                assert!(send_result, "Even couldn't be sent!");
                exec_future(stream.next(), "receiving", 1.0, true).await;
            }
        }
    }
    doc_test!(crossbeam_channel_doc_test, crossbeam::Crossbeam<&str, 1024>);
    doc_test!(atomic_queue_doc_test,      atomic_mpmc_queue::AtomicMPMCQueue<&str, 1024, 1>);
    doc_test!(full_sync_queue_doc_test,   ogre_full_sync_mpmc_queue::OgreFullSyncMPMCQueue<&str, 1024, 1>);


    // *_dropping for known parallel stream implementors
    ////////////////////////////////////////////////////

    macro_rules! dropping {
        ($fn_name: tt, $uni_channel_type: ty) => {
            /// guarantees no unsafe code is preventing proper dropping of artifacts for implementors
            /// allowing splitting messages into several parallel streams
            #[cfg_attr(not(doc),tokio::test)]
            async fn $fn_name() {
                {
                    print!("Dropping the channel before the stream consumes the element: ");
                    let channel = <$uni_channel_type>::new("dropping");
                    let streams_manager = channel.streams_manager();    // will add 1 to the references count
                    assert_eq!(Arc::strong_count(&streams_manager), 2, "Sanity check on reference counting");
                    let mut stream_1 = channel.consumer_stream();
                    let stream_2 = channel.consumer_stream();
                    assert_eq!(Arc::strong_count(&streams_manager), 4, "Creating each stream should increase the ref count by 1");
                    channel.try_send("a");
                    exec_future(stream_1.next(), "receiving", 1.0, true).await;
                    // dropping the streams & channel will decrease the Arc reference count to 1
                    drop(stream_1);
                    drop(stream_2);
                    drop(channel);
                    assert_eq!(Arc::strong_count(&streams_manager), 1, "The internal streams manager reference counting should be 1 at this point, as we are the only holders by now");
                }
                {
                    print!("Dropping the stream before the channel produces something, then another stream is created to consume the element: ");
                    let channel = <$uni_channel_type>::new("dropping");
                    let streams_manager = channel.streams_manager();    // will add 1 to the references count
                    let stream = channel.consumer_stream();
                    assert_eq!(Arc::strong_count(&streams_manager), 3, "`channel` + `stream` + `local ref`: reference count should be 3");
                    drop(stream);
                    assert_eq!(Arc::strong_count(&streams_manager), 2, "Dropping a stream should decrease the ref count by 1");
                    let mut stream = channel.consumer_stream();
                    assert_eq!(Arc::strong_count(&streams_manager), 3, "1 `channel` + 1 `stream` + `local ref` again, at this point: reference count should be 3");
                    channel.try_send("a");
                    exec_future(stream.next(), "receiving", 1.0, true).await;
                    // dropping the stream & channel will decrease the Arc reference count to 1
                    drop(stream);
                    drop(channel);
                    assert_eq!(Arc::strong_count(&streams_manager), 1, "The internal streams manager reference counting should be 1 at this point, as we are the only holders by now");
                }
                // print!("Lazy check with stupid amount of creations and destructions... watch out the process for memory: ");
                // for i in 0..1234567 {
                //     let channel = <$uni_channel_type>::new();
                //     let mut stream = channel.consumer_stream().unwrap();
                //     drop(stream);
                //     let mut stream = channel.consumer_stream().unwrap();
                //     channel.try_send("a");
                //     drop(channel);
                //     stream.next().await.unwrap();
                // }
                // println!("Done. Sleeping for 30 seconds. Go watch the heap!");
                // tokio::time::sleep(Duration::from_secs(30)).await;
            }
        }
    }
    dropping!(atomic_queue_dropping,    atomic_mpmc_queue::AtomicMPMCQueue<&str, 1024, 2>);
    dropping!(full_sync_queue_dropping, ogre_full_sync_mpmc_queue::OgreFullSyncMPMCQueue<&str, 1024, 2>);


    // *_parallel_streams for known parallel stream implementors
    ////////////////////////////////////////////////////////////

    const PARALLEL_STREAMS: usize = 128;     // total test time will be this number / 100 (in seconds)
    macro_rules! parallel_streams {
        ($fn_name: tt, $uni_channel_type: ty) => {
            /// guarantees implementors allows splitting messages in several parallel streams
            #[cfg_attr(not(doc),tokio::test)]
            async fn $fn_name() {

                let channel = Arc::new(<$uni_channel_type>::new("parallel streams"));

                // collect the streams
                let mut streams = stream::iter(0..PARALLEL_STREAMS)
                    .then(|i| {
                        let channel = channel.clone();
                        async move {
                            channel.consumer_stream()
                        }
                    })
                    .collect::<Vec<_>>().await;

                // send
                for i in 0..PARALLEL_STREAMS as u32 {
                    while !channel.try_send(i) {
                        std::hint::spin_loop();
                    };
                }

                // check each stream gets a different item, in sequence
                for i in 0..PARALLEL_STREAMS as u32 {
                    let item = exec_future(streams[i as usize].next(), &format!("receiving on stream #{i}"), 1.0, true).await;
                    assert_eq!(item, Some(i), "Stream #{i} didn't produce item {i}")
                }

                // checks each stream has no more items
                for i in 0..PARALLEL_STREAMS {
                    match tokio::time::timeout(Duration::from_millis(10), streams[i as usize].next()).await {
                        Ok(item)  => panic!("All items should have timed out now, because there are no more elements on the stream -- but stream #{i} produced item {:?}", item),
                        Err(_)    => {},
                    }
                }
            }
        }
    }
    parallel_streams!(atomic_mpmc_queue_parallel_streams, atomic_mpmc_queue::AtomicMPMCQueue<u32, 1024, PARALLEL_STREAMS>);
    parallel_streams!(mutex_mpmc_queue_parallel_streams,  ogre_full_sync_mpmc_queue::OgreFullSyncMPMCQueue<u32, 1024, PARALLEL_STREAMS>);


    /// assures performance won't be degraded when we make changes
    #[cfg_attr(not(doc),tokio::test(flavor="multi_thread", worker_threads=2))]
    #[ignore]   // must run in a single thread for accurate measurements
    async fn performance_measurements() {
        const BUFFER_SIZE: usize = 1<<14;
        #[cfg(not(debug_assertions))]
        const FACTOR: u32 = 1024;
        #[cfg(debug_assertions)]
        const FACTOR: u32 = 20;

        macro_rules! profile_same_task_same_thread_channel {
            ($channel: expr, $profiling_name: literal, $count: expr) => {
                let channel = $channel;
                let profiling_name = $profiling_name;
                let count = $count;
                let mut stream = channel.consumer_stream();

                print!("{} (same task / same thread):           ", profiling_name);
                std::io::stdout().flush().unwrap();

                let start = Instant::now();
                for e in 0..count {
                    channel.try_send(e);
                    if let Some(consumed_e) = stream.next().await {
                        assert_eq!(consumed_e, e, "{profiling_name}: produced and consumed items differ");
                    } else {
                        panic!("{profiling_name}: Stream didn't provide expected item {e}");
                    }
                }
                let elapsed = start.elapsed();

                println!("{:12.2}/s -- {} items processed in {:?}",
                         count as f64 / elapsed.as_secs_f64(),
                         count,
                         elapsed);
            }
        }

        macro_rules! profile_different_task_same_thread_channel {
            ($channel: expr, $profiling_name: literal, $count: expr) => {
                let channel = $channel;
                let profiling_name = $profiling_name;
                let count = $count;
                let mut stream = channel.consumer_stream();
                let start = Instant::now();

                let sender_future = async {
                    for e in 0..count {
                        while !channel.try_send(e) {
                            tokio::task::yield_now().await;     // hanging prevention: since we're on the same thread, we must yield or else the other task won't execute
                        }
                    }
                    channel.end_all_streams(Duration::from_secs(1)).await;
                };

                let receiver_future = async {
                    let mut counter = 0;
                    while let Some(_e) = stream.next().await {
                        counter += 1;
// TODO remove this if once all channels implement `.end_all_streams()` -- which will cause streams to return none and end the loop
if counter == count {
    break;
}
                    }
                };

                print!("{} (different task / same thread):      ", profiling_name);
                std::io::stdout().flush().unwrap();

                let start = Instant::now();
                tokio::join!(sender_future, receiver_future);
                let elapsed = start.elapsed();

                println!("{:12.2}/s -- {} items processed in {:?}",
                         count as f64 / elapsed.as_secs_f64(),
                         count,
                         elapsed);
            }
        }

        macro_rules! profile_different_task_different_thread_channel {
            ($channel: expr, $profiling_name: literal, $count: expr) => {
                let channel = $channel;
                let profiling_name = $profiling_name;
                let count = $count;
                let mut stream = channel.consumer_stream();

                let sender_task = tokio::task::spawn_blocking(move || {
                    for e in 0..count {
                        while !channel.try_send(e) {
                            std::hint::spin_loop();
                        }
                    }
                    //channel.end_all_streams(Duration::from_secs(5)).await;
                    channel.cancel_all_streams();
                });

                let receiver_task = tokio::spawn(
                    exec_future(async move {
                        let mut counter = 0;
                        while let Some(_e) = stream.next().await {
                            counter += 1;
// TODO remove this if once all channels implement `.end_all_streams()` -- which will cause streams to return none and end the loop
if counter == count {
    break;
}
                        }
                    },
                    format!("Different Task & Thread consumer for '{}'", $profiling_name),
                    300.0,
                    false)
                );

                print!("{} (different task / different thread): ", profiling_name);
                std::io::stdout().flush().unwrap();

                let start = Instant::now();
                let (receiver_result, sender_result) = tokio::join!(receiver_task, sender_task);
                receiver_result.expect("receiver task");
                sender_result.expect("sender task");
                let elapsed = start.elapsed();

                println!("{:12.2}/s -- {} items processed in {:?}",
                         count as f64 / elapsed.as_secs_f64(),
                         count,
                         elapsed);
            }
        }

        println!();
        profile_same_task_same_thread_channel!(Crossbeam::<u32, BUFFER_SIZE>::new(""), "Crossbeam            ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_same_thread_channel!(Crossbeam::<u32, BUFFER_SIZE>::new(""), "Crossbeam            ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_different_thread_channel!(Crossbeam::<u32, BUFFER_SIZE>::new(""), "Crossbeam            ", FACTOR*BUFFER_SIZE as u32);

        profile_same_task_same_thread_channel!(OgreFullSyncMPMCQueue::<u32, BUFFER_SIZE, 1>::new(""), "OgreFullSyncMPMCQueue", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_same_thread_channel!(OgreFullSyncMPMCQueue::<u32, BUFFER_SIZE, 1>::new(""), "OgreFullSyncMPMCQueue", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_different_thread_channel!(OgreFullSyncMPMCQueue::<u32, BUFFER_SIZE, 1>::new(""), "OgreFullSyncMPMCQueue", FACTOR*BUFFER_SIZE as u32);

        profile_same_task_same_thread_channel!(AtomicMPMCQueue::<u32, BUFFER_SIZE, 1>::new(""), "AtomicMPMCQueue      ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_same_thread_channel!(AtomicMPMCQueue::<u32, BUFFER_SIZE, 1>::new(""), "AtomicMPMCQueue      ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_different_thread_channel!(AtomicMPMCQueue::<u32, BUFFER_SIZE, 1>::new(""), "AtomicMPMCQueue      ", FACTOR*BUFFER_SIZE as u32);
    }

    /// executes the given `fut`ure, tracking timeouts
    async fn exec_future<Output: Debug,
                         FutureType: Future<Output=Output>,
                         IntoString: Into<String>>
                         (fut: FutureType, operation_name: IntoString, timeout_secs: f64, verbose: bool) -> Output {
        let operation_name = operation_name.into();
        let timeout = Duration::from_secs_f64(timeout_secs);
        match tokio::time::timeout(timeout, fut).await {
            Ok(non_timed_out_result) => {
                if verbose {
                    println!("{operation_name}: {:?}", non_timed_out_result);
                }
                non_timed_out_result
            },
            Err(_time_out_err) => {
                let msg = format!("\"{operation_name}\" has TIMED OUT: more than {:?} had passed while waiting the Future to complete", timeout);
                    println!("{}", msg);
                    panic!("{}", msg);
            }
        }
    }

}

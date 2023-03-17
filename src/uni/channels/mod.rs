pub mod tokio_mpsc;
pub mod atomic_mpmc_queue;
pub mod ogre_mpmc_queue;

use std::{
    pin::Pin,
    sync::Arc,
    time::Duration,
};
use async_trait::async_trait;


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
/// Known implementers of this trait are: [mutex_mpmc_queue], [atomic_mpmc_queue], [tokio_mpsc].\
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

trait Test {
    fn performance_measurements(&self);
}

type UniChannelType = atomic_mpmc_queue::AtomicMPMCQueue<u32, 1024, 1>;

struct TestAtomic<T> {
    uni_channel: T,
}
impl_uni_channel_for_struct!(TestAtomic, UniChannelType, uni_channel);

impl Test for TestAtomic<UniChannelType> {
    fn performance_measurements(&self) {
        self.yeah();
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
#[cfg(any(test, feature = "dox"))]
mod tests {
    use std::fmt::Debug;
    use std::future::Future;
    use super::{*, tokio_mpsc};
    use crate::uni::channels::{
        tokio_mpsc::TokioMPSC,
        atomic_mpmc_queue::AtomicMPMCQueue,
        ogre_mpmc_queue::OgreMPMCQueue,
    };
    use futures::{stream, StreamExt};
    use minstant::Instant;


    // *_doc_test for all known `UniChannel`s
    /////////////////////////////////////////

    macro_rules! doc_test {
        ($fn_name: tt, $uni_channel_type: ty) => {
            /// exercises the code present on the documentation for $uni_channel_type
            #[cfg_attr(not(feature = "dox"), tokio::test)]
            async fn $fn_name() {
                let channel = <$uni_channel_type>::new();
                let mut stream = channel.consumer_stream().expect("At least one stream should be available -- regardless of the implementation");
                let send_result = channel.try_send("a");
                assert!(send_result, "Even couldn't be sent!");
                exec_future(stream.next(), "receiving").await;
            }
        }
    }
    doc_test!(tokio_mpsc_queue_doc_test,  tokio_mpsc::TokioMPSC<&str, 1024>);
    doc_test!(atomic_mpmc_queue_doc_test, atomic_mpmc_queue::AtomicMPMCQueue<&str, 1024, 1>);
    doc_test!(mutex_mpmc_queue_doc_test,  ogre_mpmc_queue::OgreMPMCQueue<&str, 1024, 1>);


    // *_dropping for known parallel stream implementors
    ////////////////////////////////////////////////////

    macro_rules! dropping {
        ($fn_name: tt, $uni_channel_type: ty) => {
            /// guarantees no unsafe code is preventing proper dropping of artifacts for implementors
            /// allowing splitting messages into several parallel streams
            #[cfg_attr(not(feature = "dox"), tokio::test)]
            async fn $fn_name() {
                {
                    print!("Dropping the channel before the stream consumes the element: ");
                    let channel = <$uni_channel_type>::new();
                    assert_eq!(Arc::strong_count(&channel), 1, "Sanity check on reference counting");
                    let mut stream = channel.consumer_stream().unwrap();
                    assert_eq!(Arc::strong_count(&channel), 2, "Creating a stream should increase the ref count by 1");
                    channel.try_send("a");
                    drop(channel);  // dropping the channel will decrease the Arc reference count by 1
                    exec_future(stream.next(), "receiving").await;
                }
                {
                    print!("Dropping the stream before the channel produces something, then another stream is created to consume the element: ");
                    let channel = <$uni_channel_type>::new();
                    let stream = channel.consumer_stream().unwrap();
                    assert_eq!(Arc::strong_count(&channel), 2, "`channel` + `stream`: reference count should be 2");
                    drop(stream);
                    assert_eq!(Arc::strong_count(&channel), 1, "Dropping a stream should decrease the ref count by 1");
                    let mut stream = channel.consumer_stream().unwrap();
                    assert_eq!(Arc::strong_count(&channel), 2, "1 `channel` + 1 `stream` again, at this point: reference count should be 2");
                    channel.try_send("a");
                    drop(channel);  // dropping the channel will decrease the Arc reference count by 1
                    exec_future(stream.next(), "receiving").await;
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
    dropping!(atomic_mpmc_queue_dropping, atomic_mpmc_queue::AtomicMPMCQueue<&str, 1024, 2>);
    dropping!(mutex_mpmc_queue_dropping,  ogre_mpmc_queue::OgreMPMCQueue<&str, 1024, 2>);


    // *_parallel_streams for known parallel stream implementors
    ////////////////////////////////////////////////////////////

    const PARALLEL_STREAMS: usize = 100;     // total test time will be this number / 100 (in seconds)
    macro_rules! parallel_streams {
        ($fn_name: tt, $uni_channel_type: ty) => {
            /// guarantees implementors allows splitting messages in several parallel streams
            #[cfg_attr(not(feature = "dox"), tokio::test)]
            async fn $fn_name() {

                let channel = <$uni_channel_type>::new();

                // collect the streams
                let mut streams = stream::iter(0..PARALLEL_STREAMS)
                    .then(|i| {
                        let channel = channel.clone();
                        async move {
                            channel.consumer_stream().expect(&format!("Stream #{i} was not conceded"))
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
                    let item = exec_future(streams[i as usize].next(), &format!("receiving on stream #{i}")).await;
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
    parallel_streams!(mutex_mpmc_queue_parallel_streams,  ogre_mpmc_queue::OgreMPMCQueue<u32, 1024, PARALLEL_STREAMS>);


    /// assures performance won't be degraded when we make changes
    #[cfg_attr(not(feature = "dox"), tokio::test(flavor = "multi_thread", worker_threads = 4))]
    async fn performance_measurements() {
        #[cfg(not(debug_assertions))]
        const FACTOR: u32 = 1024;
        #[cfg(debug_assertions)]
        const FACTOR: u32 = 20;

        macro_rules! profile_same_task_same_thread_channel {
            ($channel: expr, $profiling_name: literal, $count: expr) => {
                let channel = $channel;
                let profiling_name = $profiling_name;
                let count = $count;
                let mut stream = channel.consumer_stream().expect("Asking for the first stream");
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
                println!("{} (same task / same thread): {:10.2}/s -- {} items processed in {:?}",
                         profiling_name,
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
                let mut stream = channel.consumer_stream().expect("Asking for the first stream");
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
// TODO remove this if once all channels implement `.end_all_streams()` -- which will cause stream to return none and end the loop
if counter == count {
    break;
}
                    }
                };

                tokio::join!(sender_future, receiver_future);

                let elapsed = start.elapsed();
                println!("{} (different task / same thread): {:10.2}/s -- {} items processed in {:?}",
                         profiling_name,
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
                let mut stream = channel.consumer_stream().expect("Asking for the first stream");
                let start = Instant::now();

                let sender_task = tokio::spawn(async move {
                    for e in 0..count {
                        while !channel.try_send(e) {
                            std::hint::spin_loop();
                        }
                    }
                    channel.end_all_streams(Duration::from_secs(5)).await;
                });

                let receiver_task = tokio::spawn(async move {
                    let mut counter = 0;
                    while let Some(_e) = stream.next().await {
                        counter += 1;
// TODO remove this if once all channels implement `.end_all_streams()` -- which will cause stream to return none and end the loop
if counter == count {
    break;
}
                    }
                });

                let (sender_result, receiver_result) = tokio::join!(sender_task, receiver_task);
                receiver_result.expect("receiver task");
                sender_result.expect("sender task");

                let elapsed = start.elapsed();
                println!("{} (different task / different thread): {:10.2}/s -- {} items processed in {:?}",
                         profiling_name,
                         count as f64 / elapsed.as_secs_f64(),
                         count,
                         elapsed);
            }
        }

        println!();
        profile_same_task_same_thread_channel!(TokioMPSC::<u32, 10240>::new(), "TokioMPSC ", 10240*FACTOR);
        profile_different_task_same_thread_channel!(TokioMPSC::<u32, 10240>::new(), "TokioMPSC ", 10240*FACTOR);
        profile_different_task_different_thread_channel!(TokioMPSC::<u32, 10240>::new(), "TokioMPSC ", 10240*FACTOR);

        profile_same_task_same_thread_channel!(AtomicMPMCQueue::<u32, 10240, 1>::new(), "AtomicMPMCQueue ", 10240*FACTOR);
        profile_different_task_same_thread_channel!(AtomicMPMCQueue::<u32, 10240, 1>::new(), "AtomicMPMCQueue ", 10240*FACTOR);
        profile_different_task_different_thread_channel!(AtomicMPMCQueue::<u32, 10240, 1>::new(), "AtomicMPMCQueue ", 10240*FACTOR);

        profile_same_task_same_thread_channel!(OgreMPMCQueue::<u32, 10240, 1>::new(), "OgreMPMCQueue ", 10240*FACTOR);
        profile_different_task_same_thread_channel!(OgreMPMCQueue::<u32, 10240, 1>::new(), "OgreMPMCQueue ", 10240*FACTOR);
        profile_different_task_different_thread_channel!(OgreMPMCQueue::<u32, 10240, 1>::new(), "OgreMPMCQueue ", 10240*FACTOR);
    }

    /// executes the given `fut`ure, tracking timeouts
    async fn exec_future<Output: Debug, FutureType: Future<Output=Output>>(fut: FutureType, operation_name: &str) -> Output {
        const TIMEOUT: Duration = Duration::from_secs(1);
        match tokio::time::timeout(TIMEOUT, fut).await {
            Ok(non_timed_out_result) => {
                println!("{operation_name}: {:?}", non_timed_out_result);
                non_timed_out_result
            },
            Err(_time_out_err) => {
                let msg = format!("\"{operation_name}\" has TIMED OUT: more than {:?} had passed while waiting the Future to complete", TIMEOUT);
                    println!("{}", msg);
                    panic!("{}", msg);
            }
        }
    }


}
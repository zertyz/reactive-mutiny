//! Provides channels to be used in Unis

pub mod movable;
pub mod zero_copy;

use {
    super::{
        super::{
            types::ChannelConsumer,
            mutiny_stream::MutinyStream,
        },
    },
};
use std::{
    sync::Arc,
    time::Duration,
    fmt::Debug,
};
use async_trait::async_trait;


/// Defines common abstractions on how [Uni]s receives produced events and delivers them to `Stream`s.\
/// Implementors should also implement one of [ChannelProducer] or [UniZeroCopyChannel].
/// NOTE: all async functions are out of the hot path, so the `async_trait` won't impose performance penalties
#[async_trait]
pub trait ChannelCommon<'a, ItemType:        Debug + Send + Sync,
                            DerivedItemType: Debug> {

    /// Creates a new instance of this channel, to be referred to (in logs) as `name`
    fn new<IntoString: Into<String>>(name: IntoString) -> Arc<Self>;

    /// Returns a `Stream` able to receive elements sent through this channel.\
    /// If called more than once, the behavior differs in regard to how the events will be routed,
    /// depending on the implementor:
    ///   - [Uni]s implement the "consumer" pattern: each `Stream` will receive a different element;
    ///   - [Multi]s implement the "listener" pattern: each `Stream` will see all elements
    /// Panics if called more times than allowed by [Uni]'s `MAX_STREAMS`
    fn create_stream(self: &Arc<Self>) -> MutinyStream<'a, ItemType, Self, DerivedItemType>
                                          where Self: ChannelConsumer<'a, DerivedItemType>;

    /// Waits until all pending items are taken from this channel, up until `timeout` elapses.\
    /// Returns the number of still unconsumed items -- which is 0 if it was not interrupted by the timeout
    async fn flush(&self, timeout: Duration) -> u32;

    /// Flushes & signals that the given `stream_id` should cease its activities when there are no more elements left
    /// to process, waiting for the operation to complete for up to `timeout`.\
    /// Returns `true` if the stream ended within the given `timeout` or `false` if it is still processing elements.
    async fn gracefully_end_stream(&self, stream_id: u32, timeout: Duration) -> bool;

    /// Flushes & signals that all streams should cease their activities when there are no more elements left
    /// to process, waiting for the operation to complete for up to `timeout`.\
    /// Returns the number of un-ended streams -- which is 0 if it was not interrupted by the timeout
    async fn gracefully_end_all_streams(&self, timeout: Duration) -> u32;

    /// Sends a signal to all streams, urging them to cease their operations.\
    /// In opposition to [end_all_streams()], this method does not wait for any confirmation,
    /// nor cares if there are remaining elements to be processed.
    fn cancel_all_streams(&self);

    /// Informs the caller how many active streams are currently managed by this channel
    fn running_streams_count(&self) -> u32;

    /// Tells how many events are waiting to be taken out of this channel.\
    /// IMPLEMENTORS: #[inline(always)]
    fn pending_items_count(&self) -> u32;

    /// Tells how many events may be produced ahead of the consumers.\
    /// IMPLEMENTORS: #[inline(always)]
    fn buffer_size(&self) -> u32;
}

/// Defines how to send events (to a [Uni] or [Multi]).
pub trait ChannelProducer<'a, ItemType:         Debug + Send + Sync,
                              DerivedItemType: 'a + Debug = ItemType> {

    /// Sends `item` through this channel, synchronously, returning immediately if the buffer is full
    /// -- returns `false` if the buffer was full and the `item` wasn't sent; `true` otherwise.\
    /// IMPLEMENTORS: #[inline(always)]
    fn try_send(&self, item: ItemType) -> bool;

    /// Sends `event` through this channel, to be received by all `Streams` returned by [MultiChannelCommon::create_stream()]
    #[inline(always)]
    fn send(&self, _item: ItemType) {
        todo!("`ChannelProducer.send()` is not available for channels where it can't be implemented as zero-copy (including this one)")
    }

    /// For channels that stores the `DerivedItemType` instead of the `ItemType`, this method may be useful
    /// -- for instance: the Stream consumes Arc<String> (the derived item type) and the channel is for Strings. With this method one may send an Arc directly.\
    /// The default implementation, though, is made for types that don't have a derived item type.
    #[inline(always)]
    fn send_derived(&self, _derived_item_type: DerivedItemType) {
        todo!("`ChannelProducer.send_derived()` is only available for channels whose Streams will see different types than the produced one -- example: send(`string`) / Stream<Item=Arc<String>>")
    }

}

pub trait FullDuplexChannel<'a, ItemType:        'a + Debug + Send + Sync,
                                DerivedItemType: 'a + Debug = ItemType>: ChannelCommon<'a, ItemType, DerivedItemType> + ChannelProducer<'a, ItemType> + ChannelConsumer<'a, DerivedItemType> {}

// TODO 2023-05-10: Transform `UniMovableChannel` (& `MultiMovableChannel`) into `ChannelProducer` and place it in types,
//                  along with `MutinyStreamSource`, which should be renamed to `ChannelConsumer`.
//                  Maybe, creating a "FullDuplexChannel" trait with both in it may help with reducing the generics signatures we'll need to use everywhere
//                  (the full signature may be gathered from / replace the one in uni_builder.rs#29)
// TODO 2023-05-10: `DerivedItemType` everywhere should be renamed to `StreamItemType`, for clarity -- and `ItemType`/`InType` to `ChannelItemType`, maybe?

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
    use super::*;
    use crate::{
        ogre_std::ogre_alloc::{OgreAllocator, ogre_array_pool_allocator::OgreArrayPoolAllocator},
        uni::channels::{ movable, zero_copy },
    };
    use std::{
        fmt::Debug,
        future::Future,
        sync::Arc,
        time::Duration,
        io::Write,
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
                let mut stream = channel.create_stream();
                let send_result = channel.try_send("a");
                assert!(send_result, "Even couldn't be sent!");
                exec_future(stream.next(), "receiving", 1.0, true).await;
            }
        }
    }
    doc_test!(movable_crossbeam_channel_doc_test, movable::crossbeam::Crossbeam<&str, 1024, 1>);
    doc_test!(movable_atomic_queue_doc_test,      movable::atomic::Atomic<&str, 1024, 1>);
    doc_test!(movable_full_sync_queue_doc_test,   movable::full_sync::FullSync<&str, 1024, 1>);
    doc_test!(zero_copy_atomic_queue_doc_test,    zero_copy::atomic::Atomic<&str, OgreArrayPoolAllocator<&str, 1024>, 1024, 1>);


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
                    assert_eq!(Arc::strong_count(&channel), 1, "Sanity check on reference counting");
                    let mut stream_1 = channel.create_stream();
                    let stream_2 = channel.create_stream();
                    assert_eq!(Arc::strong_count(&channel), 3, "Creating each stream should increase the ref count by 1");
                    channel.try_send("a");
                    exec_future(stream_1.next(), "receiving", 1.0, true).await;
                    // dropping the streams & channel will decrease the Arc reference count to 1
                    drop(stream_1);
                    drop(stream_2);
                    assert_eq!(Arc::strong_count(&channel), 1, "The internal streams manager reference counting should be 1 at this point, as we are the only holders by now");
                    drop(channel);
                }
                {
                    print!("Dropping the stream before the channel produces something, then another stream is created to consume the element: ");
                    let channel = <$uni_channel_type>::new("dropping");
                    let stream = channel.create_stream();
                    assert_eq!(Arc::strong_count(&channel), 2, "`channel` + `stream` + `local ref`: reference count should be 2");
                    drop(stream);
                    assert_eq!(Arc::strong_count(&channel), 1, "Dropping a stream should decrease the ref count by 1");
                    let mut stream = channel.create_stream();
                    assert_eq!(Arc::strong_count(&channel), 2, "1 `channel` + 1 `stream` again, at this point: reference count should be 2");
                    channel.try_send("a");
                    exec_future(stream.next(), "receiving", 1.0, true).await;
                    // dropping the stream & channel will decrease the Arc reference count to 1
                    drop(stream);
                    assert_eq!(Arc::strong_count(&channel), 1, "The internal streams manager reference counting should be 1 at this point, as we are the only holders by now");
                    drop(channel);
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
    dropping!(movable_crossbeam_queue_dropping, movable::crossbeam::Crossbeam<&str, 1024, 2>);
    dropping!(movable_atomic_queue_dropping,    movable::atomic::Atomic<&str, 1024, 2>);
    dropping!(movable_full_sync_queue_dropping, movable::full_sync::FullSync<&str, 1024, 2>);
    dropping!(zero_copy_atomic_queue_dropping,  zero_copy::atomic::Atomic<&str, OgreArrayPoolAllocator<&str, 1024>, 1024, 2>);


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
                    .then(|_i| {
                        let channel = channel.clone();
                        async move {
                            channel.create_stream()
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
                    let observed = item.expect("item should not be none");
                    assert_eq!(observed, i, "Stream #{i} didn't produce item {i}")
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
    parallel_streams!(movable_crossbeam_parallel_streams, movable::crossbeam::Crossbeam<u32, 1024, PARALLEL_STREAMS>);
    parallel_streams!(movable_atomic_parallel_streams,    movable::atomic::Atomic<u32, 1024, PARALLEL_STREAMS>);
    parallel_streams!(movable_mutex_parallel_streams,     movable::full_sync::FullSync<u32, 1024, PARALLEL_STREAMS>);
    parallel_streams!(zero_copy_atomic_parallel_streams,  zero_copy::atomic::Atomic<u32, OgreArrayPoolAllocator<u32, 1024>, 1024, PARALLEL_STREAMS>);


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
                let mut stream = channel.create_stream();

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
                let mut stream = channel.create_stream();

                let sender_future = async {
                    for e in 0..count {
                        while !channel.try_send(e) {
                            tokio::task::yield_now().await;     // hanging prevention: since we're on the same thread, we must yield or else the other task won't execute
                        }
                    }
                    channel.gracefully_end_all_streams(Duration::from_secs(1)).await;
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
                let mut stream = channel.create_stream();

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

        profile_same_task_same_thread_channel!(movable::atomic::Atomic::<u32, BUFFER_SIZE, 1>::new(""), "Movable Atomic    ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_same_thread_channel!(movable::atomic::Atomic::<u32, BUFFER_SIZE, 1>::new(""), "Movable Atomic    ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_different_thread_channel!(movable::atomic::Atomic::<u32, BUFFER_SIZE, 1>::new(""), "Movable Atomic    ", FACTOR*BUFFER_SIZE as u32);

        profile_same_task_same_thread_channel!(movable::full_sync::FullSync::<u32, BUFFER_SIZE, 1>::new(""), "Movable FullSync  ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_same_thread_channel!(movable::full_sync::FullSync::<u32, BUFFER_SIZE, 1>::new(""), "Movable FullSync  ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_different_thread_channel!(movable::full_sync::FullSync::<u32, BUFFER_SIZE, 1>::new(""), "Movable FullSync  ", FACTOR*BUFFER_SIZE as u32);

        profile_same_task_same_thread_channel!(movable::crossbeam::Crossbeam::<u32, BUFFER_SIZE, 1>::new(""), "Movable Crossbeam ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_same_thread_channel!(movable::crossbeam::Crossbeam::<u32, BUFFER_SIZE, 1>::new(""), "Movable Crossbeam ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_different_thread_channel!(movable::crossbeam::Crossbeam::<u32, BUFFER_SIZE, 1>::new(""), "Movable Crossbeam ", FACTOR*BUFFER_SIZE as u32);

        type ZeroCopyAtomic<'a> = zero_copy::atomic::Atomic::<'a, u32, OgreArrayPoolAllocator<u32, BUFFER_SIZE>, BUFFER_SIZE, 1>;

        profile_same_task_same_thread_channel!(ZeroCopyAtomic::new(""), "Zero-Copy Atomic  ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_same_thread_channel!(ZeroCopyAtomic::new(""), "Zero-Copy Atomic  ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_different_thread_channel!(ZeroCopyAtomic::new(""), "Zero-Copy Atomic  ", FACTOR*BUFFER_SIZE as u32);

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

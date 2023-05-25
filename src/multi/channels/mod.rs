//! Provides channels to be used in Unis

pub mod arc;
pub mod ogre_arc;
pub mod reference;

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


/// Tests & enforces the requisites & expose good practices & exercises the API of of the [multi/channels](self) module
/// WARNING: unusual test module ahead -- macros are used to implement test functions.\
///          this is due to the implementation notes on [UniChannel]: we can't have a trait to unify all
///          implementor's behavior (as of Rust 1.63), therefore, we use macros to allow writing test code
///          only once and get them specialized automatically for each type. This also tests that the API
///          is unified among implementors (since there is no Trait to enforce it by now).\
/// NOTE for USERS: you're likely to, unfortunately, also have to resort to the same macro strategy as of Rust 1.63
///                 -- or pick a single hard coded (but easy to replace) type as in the example bellow
///                    (which may even be configured by a "features" flag in Cargo):
/// ```no_compile
///     type MultiChannelType<ItemType, BUFFER_SIZE> = atomic_queue::AtomicQueue<ItemType, BUFFER_SIZE>
#[cfg(any(test,doc))]
mod tests {
    use super::*;
    use crate::uni::channels::{ChannelCommon, ChannelProducer, FullDuplexChannel};
    use {arc, ogre_arc};
    use std::{
        sync::Arc,
        time::Duration,
        sync::atomic::{AtomicBool,Ordering::Relaxed},
        time::Instant,
        io::Write,
    };
    use futures::{stream,Stream,StreamExt};
    use crate::{AllocatorAtomicArray, AllocatorFullSyncArray};
    use crate::ogre_std::ogre_alloc;


    // *_doc_test for all known Multi Channel's
    ///////////////////////////////////////////

    macro_rules! doc_test {
        ($fn_name: tt, $multi_channel_type: ty) => {
            /// exercises the code present on the documentation for $multi_channel_type
            #[cfg_attr(not(doc),tokio::test)]
            async fn $fn_name() {
                let channel = <$multi_channel_type>::new("doc_test");
                let (mut stream, _stream_id) = channel.create_stream();
                channel.try_send_movable("a");
                println!("received: {}", stream.next().await.unwrap());
            }
        }
    }

    doc_test!(arc_atomic_queue_doc_test,         arc::atomic::Atomic<&str, 1024, 1>);
    doc_test!(arc_full_sync_queue_doc_test,      arc::full_sync::FullSync<&str, 1024, 1>);
    doc_test!(arc_crossbeam_queue_doc_test,      arc::crossbeam::Crossbeam<&str, 1024, 1>);
    doc_test!(ogre_arc_atomic_queue_doc_test,    ogre_arc::atomic::Atomic<&str, AllocatorAtomicArray<&str, 1024>, 1024, 1>);
    doc_test!(ogre_arc_full_sync_queue_doc_test, ogre_arc::full_sync::FullSync<&str, AllocatorFullSyncArray<&str, 1024>, 1024, 1>);
    doc_test!(reference_mmap_log_queue_doc_test, reference::mmap_log::MmapLog<&str, 1>);


    // *_stream_and_channel_dropping for all known Multi Channel's
    //////////////////////////////////////////////////////////////

    macro_rules! stream_and_channel_dropping {
        ($fn_name: tt, $multi_channel_type: ty) => {
            /// guarantees no unsafe code is preventing proper dropping of the created channels and the returned streams
            #[cfg_attr(not(doc),tokio::test)]
            async fn $fn_name() {
                {
                    print!("Dropping the channel before the stream consumes the element: ");
                    let channel = <$multi_channel_type>::new("stream_and_channel_dropping");
                    assert_eq!(Arc::strong_count(&channel), 1, "Sanity check on reference counting");
                    let (mut stream_1, _stream_id) = channel.create_stream();
                    assert_eq!(Arc::strong_count(&channel), 2, "Creating a stream should increase the ref count by 1");
                    let (mut stream_2, _stream_id_2) = channel.create_stream();
                    assert_eq!(Arc::strong_count(&channel), 3, "Creating a stream should increase the ref count by 1");
                    channel.try_send_movable("a");
                    println!("received: stream_1: {}; stream_2: {}", stream_1.next().await.unwrap(), stream_2.next().await.unwrap());
                    // dropping the streams & channel will decrease the Arc reference count to 1
                    drop(stream_1);
                    drop(stream_2);
                    assert_eq!(Arc::strong_count(&channel), 1, "The internal streams manager reference counting should be 1 at this point, as we are the only holders by now");
                    drop(channel);
                }
                {
                    print!("Dropping the stream before the channel produces something, then another stream is created to consume the element: ");
                    let channel = <$multi_channel_type>::new("stream_and_channel_dropping");
                    let stream = channel.create_stream();
                    assert_eq!(Arc::strong_count(&channel), 2, "`channel` + `stream`: reference count should be 3");
                    drop(stream);
                    assert_eq!(Arc::strong_count(&channel), 1, "Dropping a stream should decrease the ref count by 1");
                    let (mut stream, _stream_id) = channel.create_stream();
                    assert_eq!(Arc::strong_count(&channel), 2, "1 `channel` + 1 `stream` again, at this point: reference count should be 2");
                    channel.try_send_movable("a");
                    println!("received: {}", stream.next().await.unwrap());
                    // dropping the stream & channel will decrease the Arc reference count to 1
                    drop(stream);
                    assert_eq!(Arc::strong_count(&channel), 1, "The internal streams manager reference counting should be 1 at this point, as we are the only holders by now");
                    drop(channel);
                }
                // print!("Brute-force check with stupid amount of creations and destructions... watch out the process for memory!");
                // for i in 0..1234567 {
                //     let channel = OgreMPMCQueue::<&str>::new();
                //     let mut stream = channel.consumer_stream();
                //     drop(stream);
                //     let mut stream = channel.consumer_stream();
                //     channel.try_send_movable("a");
                //     drop(channel);
                //     stream.next().await.unwrap();
                // }
                // println!("Done. Sleeping for 30 seconds. Go watch the heap! ... or rerun this test with `/sbin/time -v ...`");
                // tokio::time::sleep(Duration::from_secs(30)).await;
            }
        }
    }

    stream_and_channel_dropping!(arc_atomic_queue_stream_and_channel_dropping,         arc::atomic::Atomic<&str>);
    stream_and_channel_dropping!(arc_full_sync_queue_stream_and_channel_dropping,      arc::full_sync::FullSync<&str>);
    stream_and_channel_dropping!(arc_crossbeam_stream_and_channel_dropping,            arc::crossbeam::Crossbeam<&str>);
    stream_and_channel_dropping!(ogre_arc_atomic_queue_stream_and_channel_dropping,    ogre_arc::atomic::Atomic<&str, AllocatorAtomicArray<&str, 1024>>);
    stream_and_channel_dropping!(ogre_arc_full_sync_queue_stream_and_channel_dropping, ogre_arc::full_sync::FullSync<&str, AllocatorFullSyncArray<&str, 1024>>);
    stream_and_channel_dropping!(reference_mmap_log_stream_and_channel_dropping,       reference::mmap_log::MmapLog<&str>);


    // *_on_the_fly_streams for all known Multi Channel's
    /////////////////////////////////////////////////////

    macro_rules! on_the_fly_streams {
        ($fn_name: tt, $multi_channel_type: ty) => {
            /// Multis are designed to allow on-the-fly additions and removal of streams.
            /// This test guarantees and stresses that
            #[cfg_attr(not(doc),tokio::test)]
            async fn $fn_name() {
                let channel = <$multi_channel_type>::new("on_the_fly_streams");

                println!("1) streams 1 & 2 about to receive a message");
                let message = "to `stream1` and `stream2`".to_string();
                let (stream1, _stream1_id) = channel.create_stream();
                let (stream2, _stream2_id) = channel.create_stream();
                channel.try_send_movable(message.clone());
                assert_received(stream1, message.clone(), "`stream1` didn't receive the right message").await;
                assert_received(stream2, message.clone(), "`stream2` didn't receive the right message").await;

                println!("2) stream3 should timeout");
                let (stream3, _stream3_id) = channel.create_stream();
                assert_no_message(stream3, "`stream3` simply should timeout (no message will ever be sent through it)").await;

                println!("3) several creations and removals");
                for i in 0..10240 {
                    let message = format!("gremling #{}", i);
                    let (gremlin_stream, _gremlin_stream_id) = channel.create_stream();
                    channel.try_send_movable(message.clone());
                    assert_received(gremlin_stream, message, "`gremling_stream` didn't receive the right message").await;
                }

                async fn assert_received(stream: impl Stream<Item=Arc<String>>, expected_message: String, failure_explanation: &str) {
                    match tokio::time::timeout(Duration::from_millis(10), Box::pin(stream).next()).await {
                        Ok(left) => assert_eq!(*left.unwrap(), expected_message, "{}", failure_explanation),
                        Err(_)                    => panic!("{} -- Timed out!", failure_explanation),
                    }
                }

                async fn assert_no_message(stream: impl Stream<Item=Arc<String>>, failure_explanation: &str) {
                    match tokio::time::timeout(Duration::from_millis(10), Box::pin(stream).next()).await {
                        Ok(left)  => panic!("{} -- A timeout was expected, but message '{}' was received", failure_explanation, left.unwrap()),
                        Err(_)                 => {},
                    }
                }
            }
        }
    }

    on_the_fly_streams!(arc_atomic_queue_on_the_fly_streams,         arc::atomic::Atomic<String>);
    on_the_fly_streams!(arc_full_sync_queue_on_the_fly_streams,      arc::full_sync::FullSync<String>);
    on_the_fly_streams!(arc_crossbeam_on_the_fly_streams,            arc::crossbeam::Crossbeam<String>);
    // on_the_fly_streams!(ogre_arc_atomic_queue_on_the_fly_streams,    ogre_arc::atomic::Atomic<String, AllocatorAtomicArray<String, 1024>>);
    // on_the_fly_streams!(ogre_arc_full_sync_queue_on_the_fly_streams, ogre_arc::full_sync::FullSync<String, AllocatorFullSyncArray<String, 1024>>);
    // on_the_fly_streams!(references_mmap_log_on_the_fly_streams,      reference::mmap_log::MmapLog<String>);


    // *_multiple_streams for all known Multi Channel's
    ///////////////////////////////////////////////////

    const ELEMENTS:         usize = 100;
    const PARALLEL_STREAMS: usize = 128;
    macro_rules! multiple_streams {
        ($fn_name: tt, $multi_channel_type: ty) => {
            /// guarantees implementors allows copying messages over several streams
            #[cfg_attr(not(doc),tokio::test)]
            async fn $fn_name() {

                let channel = Arc::new(<$multi_channel_type>::new("multiple_streams"));

                // collect the streams
                let mut streams = stream::iter(0..PARALLEL_STREAMS)
                    .then(|_i| {
                        let channel = channel.clone();
                        async move {
                            let (stream, _stream_id) = channel.create_stream();
                            stream
                        }
                    })
                    .collect::<Vec<_>>().await;

                // send (one copy to each stream)
                for i in 0..ELEMENTS as u32 {
                    channel.send(|slot| *slot = i);
                }

                // check each stream gets all elements
                for s in 0..PARALLEL_STREAMS as u32 {
                    for i in 0..ELEMENTS as u32 {
                        let item = streams[s as usize].next().await;
                        assert_eq!(*item.unwrap(), i, "Stream #{s} didn't produce item {i}")
                    }
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

    multiple_streams!(arc_atomic_queue_multiple_streams,         arc::atomic::Atomic<u32, 128, PARALLEL_STREAMS>);
    multiple_streams!(arc_full_sync_queue_multiple_streams,      arc::full_sync::FullSync<u32, 128, PARALLEL_STREAMS>);
    multiple_streams!(arc_crossbeam_multiple_streams,            arc::crossbeam::Crossbeam<u32, 128, PARALLEL_STREAMS>);
    multiple_streams!(ogre_arc_atomic_queue_multiple_streams,    ogre_arc::atomic::Atomic<u32, AllocatorAtomicArray<u32, 128>, 128, PARALLEL_STREAMS>);
    multiple_streams!(ogre_arc_full_sync_queue_multiple_streams, ogre_arc::full_sync::FullSync<u32, AllocatorFullSyncArray<u32, 128>, 128, PARALLEL_STREAMS>);
    //multiple_streams!(references_mmap_log_multiple_streams,      reference::mmap_log::MmapLog<u32, PARALLEL_STREAMS>);


    // *_end_streams for all known Multi Channel's
    //////////////////////////////////////////////

    macro_rules! end_streams {
        ($fn_name: tt, $multi_channel_type: ty) => {
            /// stresses on-demand stopping streams
            /// -- related to [dropping()], but here the "dropping" is active (controlled via a function call)
            ///    rather then structural (stream object getting out of scope)
            #[cfg_attr(not(doc),tokio::test)]
            async fn $fn_name() {
                const WAIT_TIME: Duration = Duration::from_millis(100);
                {
                    println!("Creating & ending a single stream: ");
                    let channel = <$multi_channel_type>::new("end_streams");
                    let (mut stream, stream_id) = channel.create_stream();
                    tokio::spawn(async move {
                        tokio::time::sleep(WAIT_TIME/2).await;
                        channel.gracefully_end_stream(stream_id, WAIT_TIME).await;
                    });
                    match tokio::time::timeout(WAIT_TIME, stream.next()).await {
                        Ok(none_item)     => assert_eq!(none_item, None, "Ending a stream should make it return immediately from the waiting state with `None`"),
                        Err(_timeout_err) => panic!("{:?} elapsed but `stream.next().await` didn't return after `.end_stream()` was requested", WAIT_TIME),
                    }
                }
                {
                    println!("Creating & ending two streams: ");
                    let channel = <$multi_channel_type>::new("end_streams");
                    let (mut first, first_id) = channel.create_stream();
                    let (mut second, second_id) = channel.create_stream();
                    let first_ended = Arc::new(AtomicBool::new(false));
                    let second_ended = Arc::new(AtomicBool::new(false));
                    {
                        let first_ended = Arc::clone(&first_ended);
                        tokio::spawn(async move {
                            match tokio::time::timeout(WAIT_TIME, first.next()).await {
                                Ok(none_item)     => {
                                    assert_eq!(none_item, None, "Ending the 'first' stream should make it return immediately from the waiting state with `None`");
                                    first_ended.store(true, Relaxed);
                                },
                                Err(_timeout_err) => panic!("{:?} elapsed but `first.next().await` didn't return after `.end_stream()` was requested", WAIT_TIME),
                            }
                        });
                        let second_ended = Arc::clone(&second_ended);
                        tokio::spawn(async move {
                            match tokio::time::timeout(WAIT_TIME, second.next()).await {
                                Ok(none_item)     => {
                                    assert_eq!(none_item, None, "Ending the 'second' stream should make it return immediately from the waiting state with `None`");
                                    second_ended.store(true, Relaxed);
                                },
                                Err(_timeout_err) => panic!("{:?} elapsed but `second.next().await` didn't return after `.end_stream()` was requested", WAIT_TIME),
                            }
                        });
                    }
                    tokio::time::sleep(WAIT_TIME/2).await;
                    channel.gracefully_end_stream(first_id, WAIT_TIME).await;
                    channel.gracefully_end_stream(second_id, WAIT_TIME).await;
                    assert_eq!(first_ended.load(Relaxed), true, "`first` stream didn't end in time. Inspect the test output for hints");
                    assert_eq!(second_ended.load(Relaxed), true, "`second` stream didn't end in time. Inspect the test output for hints");
                }
            }
        }
    }

    end_streams!(arc_atomic_queue_end_streams,         arc::atomic::Atomic<&str>);
    end_streams!(arc_full_sync_queue_end_streams,      arc::full_sync::FullSync<&str>);
    end_streams!(arc_crossbeam_end_streams,            arc::crossbeam::Crossbeam<&str>);
    // end_streams!(ogre_arc_atomic_queue_end_streams,    ogre_arc::atomic::Atomic<&str, AllocatorAtomicArray<&str, 1024>>);
    // end_streams!(ogre_arc_full_sync_queue_end_streams, ogre_arc::full_sync::FullSync<&str, AllocatorFullSyncArray<&str, 1024>>);
    end_streams!(reference_mmap_log_end_streams,       reference::mmap_log::MmapLog<&str>);


    // *_payload_dropping for all known Multi Channel's
    ///////////////////////////////////////////////////

    macro_rules! payload_dropping {
        ($fn_name: tt, $multi_channel_type: ty) => {
            /// guarantees no unsafe code is preventing proper dropping & releasing of payload resources
            #[cfg_attr(not(doc),tokio::test)]
            async fn $fn_name() {
                const PAYLOAD_TEXT: &str = "A shareable playload";
                let channel = <$multi_channel_type>::new("payload_dropping");
                let (mut stream_1, _stream_id) = channel.create_stream();
                let (mut stream_2, _stream_id) = channel.create_stream();
                channel.try_send_movable(String::from(PAYLOAD_TEXT));

                let payload_1 = stream_1.next().await.unwrap();
                assert_eq!(payload_1.as_str(), PAYLOAD_TEXT, "sanity check failed: wrong payload received");
                assert_eq!(Arc::strong_count(&payload_1), 2, "The payload is shared with 2 streams, therefore, the Arc count should be 2");
                let payload_2 = stream_2.next().await.unwrap();
                assert_eq!(payload_2.as_str(), PAYLOAD_TEXT, "sanity check failed: wrong payload received");
                assert_eq!(Arc::strong_count(&payload_2), 2, "The payload is shared with 2 streams, therefore, the Arc count should be 2");

                drop(payload_1);
                assert_eq!(Arc::strong_count(&payload_2), 1, "The payload was cloned 2 times -- but, by now, there should be only 1 reference to it: `payload_2`");
            }
        }
    }

    payload_dropping!(arc_atomic_queue_payload_dropping,         arc::atomic::Atomic<String>);
    payload_dropping!(arc_full_sync_queue_payload_dropping,      arc::full_sync::FullSync<String>);
    payload_dropping!(arc_crossbeam_payload_dropping,            arc::crossbeam::Crossbeam<String>);
    // payload_dropping!(ogre_arc_atomic_queue_payload_dropping,    ogre_arc::atomic::Atomic<String, AllocatorAtomicArray<String, 1024>>);
    // payload_dropping!(ogre_arc_full_sync_queue_payload_dropping, ogre_arc::full_sync::FullSync<String, AllocatorFullSyncArray<String, 1024>>);


    /// small stress test with outputs
    #[cfg_attr(not(doc),tokio::test(flavor="multi_thread", worker_threads=2))]
    #[ignore]   // must run in a single thread for accurate measurements
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
                let (mut stream, _stream_id) = channel.create_stream();

                print!("{} (same task / same thread):           ", profiling_name);
                std::io::stdout().flush().unwrap();

                let start = Instant::now();
                for e in 0..count {
                    channel.send(|slot| *slot = e);
                    if let Some(consumed_e) = stream.next().await {
                        assert_eq!(*consumed_e, e, "{profiling_name}: produced and consumed items differ");
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
                let (mut stream, _stream_id) = channel.create_stream();

                let sender_future = async {
                    let mut e = 0;
                    while e < count {
                        let buffer_entries_left = channel.buffer_size() - channel.pending_items_count();
                        for _ in 0..buffer_entries_left {
                            channel.send(|slot| *slot = e);
                            e += 1;
                        }
                        tokio::task::yield_now().await;
                    }
                    channel.gracefully_end_all_streams(Duration::from_secs(1)).await;
                };

                let receiver_future = async {
                    let mut counter = 0;
                    while let Some(e) = stream.next().await {
                        assert_eq!(*e, counter, "Wrong event consumed");
                        counter += 1;
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
                let (mut stream, _stream_id) = channel.create_stream();

                let sender_task = tokio::task::spawn_blocking(move || {
                    let mut e = 0;
                    while e < count {
                        let buffer_entries_left = channel.buffer_size() - channel.pending_items_count();
                        for _ in 0..buffer_entries_left {
                            channel.send(|slot| *slot = e);
                            e += 1;
                        }
                        std::hint::spin_loop();
                        //tokio::task::yield_now().await;
                    }
                    //channel.end_all_streams(Duration::from_secs(5)).await;
                    channel.cancel_all_streams();
                });

                let receiver_task = tokio::spawn(async move {
                    let mut counter = 0;
                    while let Some(e) = stream.next().await {
                        assert_eq!(*e, counter, "Wrong event consumed");
                        counter += 1;
                    }
                });

                print!("{} (different task / different thread): ", profiling_name);
                std::io::stdout().flush().unwrap();

                let start = Instant::now();
                let (sender_result, receiver_result) = tokio::join!(sender_task, receiver_task);
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

        profile_same_task_same_thread_channel!(arc::atomic::Atomic::<u32, 16384, 1>::new("profile_same_task_same_thread_channel"), "Arc Atomic       ", 16384*FACTOR);
        profile_different_task_same_thread_channel!(arc::atomic::Atomic::<u32, 16384, 1>::new("profile_different_task_same_thread_channel"), "Arc Atomic       ", 16384*FACTOR);
        profile_different_task_different_thread_channel!(arc::atomic::Atomic::<u32, 16384, 1>::new("profile_different_task_different_thread_channel"), "Arc Atomic       ", 16384*FACTOR);

        profile_same_task_same_thread_channel!(arc::full_sync::FullSync::<u32, 16384, 1>::new("profile_same_task_same_thread_channel"), "Arc FullSync     ", 16384*FACTOR);
        profile_different_task_same_thread_channel!(arc::full_sync::FullSync::<u32, 16384, 1>::new("profile_different_task_same_thread_channel"), "Arc FullSync     ", 16384*FACTOR);
        profile_different_task_different_thread_channel!(arc::full_sync::FullSync::<u32, 16384, 1>::new("profile_different_task_different_thread_channel"), "Arc FullSync     ", 16384*FACTOR);

        profile_same_task_same_thread_channel!(arc::crossbeam::Crossbeam::<u32, 16384, 1>::new("profile_same_task_same_thread_channel"), "Arc Crossbeam    ", 16384*FACTOR);
        profile_different_task_same_thread_channel!(arc::crossbeam::Crossbeam::<u32, 16384, 1>::new("profile_different_task_same_thread_channel"), "Arc Crossbeam    ", 16384*FACTOR);
        profile_different_task_different_thread_channel!(arc::crossbeam::Crossbeam::<u32, 16384, 1>::new("profile_different_task_different_thread_channel"), "Arc Crossbeam    ", 16384*FACTOR);

        profile_same_task_same_thread_channel!(ogre_arc::atomic::Atomic::<u32, AllocatorAtomicArray<u32, 16384>, 16384, 1>::new("profile_same_task_same_thread_channel"), "OgreArc Atomic   ", 16384*FACTOR);
        profile_different_task_same_thread_channel!(ogre_arc::atomic::Atomic::<u32, AllocatorAtomicArray<u32, 16384>, 16384, 1>::new("profile_different_task_same_thread_channel"), "OgreArc Atomic   ", 16384*FACTOR);
        profile_different_task_different_thread_channel!(ogre_arc::atomic::Atomic::<u32, AllocatorAtomicArray<u32, 16384>, 16384, 1>::new("profile_different_task_different_thread_channel"), "OgreArc Atomic   ", 16384*FACTOR);

        profile_same_task_same_thread_channel!(ogre_arc::full_sync::FullSync::<u32, AllocatorFullSyncArray<u32, 16384>, 16384, 1>::new("profile_same_task_same_thread_channel"), "OgreArc FullSync ", 16384*FACTOR);
        profile_different_task_same_thread_channel!(ogre_arc::full_sync::FullSync::<u32, AllocatorFullSyncArray<u32, 16384>, 16384, 1>::new("profile_different_task_same_thread_channel"), "OgreArc FullSync ", 16384*FACTOR);
        profile_different_task_different_thread_channel!(ogre_arc::full_sync::FullSync::<u32, AllocatorFullSyncArray<u32, 16384>, 16384, 1>::new("profile_different_task_different_thread_channel"), "OgreArc FullSync ", 16384*FACTOR);

    }

}
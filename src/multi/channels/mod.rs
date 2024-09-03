//! Provides channels to be used in Unis

pub mod arc;
pub mod ogre_arc;
pub mod reference;


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
    use crate::{
        prelude::advanced::{AllocatorAtomicArray, AllocatorFullSyncArray},
        types::{
            ChannelCommon, ChannelProducer, ChannelMulti,
        }
    };
    use {arc, ogre_arc};
    use std::{
        sync::Arc,
        time::Duration,
        sync::atomic::{AtomicBool,Ordering::Relaxed},
        time::Instant,
        io::Write,
    };
    use std::fmt::Debug;
    use std::future::Future;
    use std::sync::atomic::AtomicU32;
    use futures::{stream, Stream, StreamExt};


    // *_doc_test for all known Multi Channel's
    ///////////////////////////////////////////

    macro_rules! doc_test {
        ($fn_name: tt, $multi_channel_type: ty) => {
            /// exercises the code present on the documentation for $multi_channel_type
            #[cfg_attr(not(doc),tokio::test)]
            async fn $fn_name() {
                let channel = <$multi_channel_type>::new("doc_test");
                let (mut stream, _stream_id) = channel.create_stream_for_new_events();
                assert!(channel.send("a").is_ok(), "could not send an event");
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
                    let (mut stream_1, _stream_id) = channel.create_stream_for_new_events();
                    assert_eq!(Arc::strong_count(&channel), 2, "Creating a stream should increase the ref count by 1");
                    let (mut stream_2, _stream_id_2) = channel.create_stream_for_new_events();
                    assert_eq!(Arc::strong_count(&channel), 3, "Creating a stream should increase the ref count by 1");
                    assert!(channel.send("a").is_ok(), "could not send an event");
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
                    let stream = channel.create_stream_for_new_events();
                    assert_eq!(Arc::strong_count(&channel), 2, "`channel` + `stream`: reference count should be 3");
                    drop(stream);
                    assert_eq!(Arc::strong_count(&channel), 1, "Dropping a stream should decrease the ref count by 1");
                    let (mut stream, _stream_id) = channel.create_stream_for_new_events();
                    assert_eq!(Arc::strong_count(&channel), 2, "1 `channel` + 1 `stream` again, at this point: reference count should be 2");
                    assert!(channel.send("a").is_ok(), "could not send an event");
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
                let (stream1, _stream1_id) = channel.create_stream_for_new_events();
                let (stream2, _stream2_id) = channel.create_stream_for_new_events();
                assert!(channel.send(message.clone()).is_ok(), "could not send an event");
                assert_received(stream1, message.clone(), "`stream1` didn't receive the right message").await;
                assert_received(stream2, message.clone(), "`stream2` didn't receive the right message").await;

                println!("2) stream3 should timeout");
                let (stream3, _stream3_id) = channel.create_stream_for_new_events();
                assert_no_message(stream3, "`stream3` simply should timeout (no message will ever be sent through it)").await;

                println!("3) several creations and removals");
                for i in 0..10240 {
                    let message = format!("gremling #{}", i);
                    let (gremlin_stream, _gremlin_stream_id) = channel.create_stream_for_new_events();
                    assert!(channel.send(message.clone()).is_ok(), "could not send event");
                    assert_received(gremlin_stream, message, "`gremling_stream` didn't receive the right message").await;
                }

                async fn assert_received(stream: impl Stream<Item=Arc<String>>, expected_message: String, failure_explanation: &str) {
                    match tokio::time::timeout(Duration::from_millis(10), Box::pin(stream).next()).await {
                        Ok(left) => assert_eq!(*left.unwrap(), expected_message, "{}", failure_explanation),
                        Err(_)   => panic!("{} -- Timed out!", failure_explanation),
                    }
                }

                async fn assert_no_message(stream: impl Stream<Item=Arc<String>>, failure_explanation: &str) {
                    match tokio::time::timeout(Duration::from_millis(10), Box::pin(stream).next()).await {
                        Ok(left)  => panic!("{} -- A timeout was expected, but message '{}' was received", failure_explanation, left.unwrap()),
                        Err(_)    => {},
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
                            let (stream, _stream_id) = channel.create_stream_for_new_events();
                            stream
                        }
                    })
                    .collect::<Vec<_>>().await;

                // send (one copy to each stream)
                for i in 0..ELEMENTS as u32 {
                    channel.send_with(|slot| *slot = i).expect_ok("couldn't send");
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
                    let (mut stream, stream_id) = channel.create_stream_for_new_events();
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
                    let (mut first, first_id) = channel.create_stream_for_new_events();
                    let (mut second, second_id) = channel.create_stream_for_new_events();
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
                let (mut stream_1, _stream_id) = channel.create_stream_for_new_events();
                let (mut stream_2, _stream_id) = channel.create_stream_for_new_events();
                assert!(channel.send(String::from(PAYLOAD_TEXT)).is_ok(), "could not send");

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


    // *_async_sending for all known Multi Channels
    ///////////////////////////////////////////////

    macro_rules! async_sending {
        ($fn_name: tt, $uni_channel_type: ty) => {
            /// guarantees implementors can properly send messages with an async closure:
            /// create `BUFFER_SIZE` insertion tasks, all of them awaiting on a single mutex latch.
            /// at the end, assert on the sum of the received items.
            #[cfg_attr(not(doc),tokio::test)]
            async fn $fn_name() {

                let channel = Arc::new(<$uni_channel_type>::new("async sending"));

                let series_size = channel.buffer_size();
                let expected_sum = (1 + series_size) * (series_size / 2);
                let observed_sum = Arc::new(AtomicU32::new(0));


                // start the receiver task
                let (mut stream, _stream_id) = channel.create_stream_for_new_events();
                let cloned_observed_sum = observed_sum.clone();
                let receiver_task = tokio::spawn(async move {
                    while let Some(received_number) = exec_future(stream.next(), "receiving an item produced by `send_with_async()`", 1.0, false).await {
                        use std::borrow::Borrow;    // allows working with either `u32` and `OgreUnique`
                        cloned_observed_sum.fetch_add(*received_number.borrow(), Relaxed);
                    }
                });

                // start concurrent async tasks for all sends, which will block on `latch`
                let latch = Arc::new(tokio::sync::RwLock::new(()));
                let cloned_latch = latch.clone();
                let lock = cloned_latch.write().await;
                for i in 0..series_size {
                    let latch = Arc::clone(&latch);
                    let channel = Arc::clone(&channel);
                    tokio::spawn(async move {
                        channel.send_with_async(move |slot| {
                            async move {
                                _ = latch.read().await;
                                *slot = i+1;
                                slot
                            }
                        }).await.expect_ok("couldn't send");
                    });
                }
                // special task to close the channel -- after giving some time for all elements to be sent
                tokio::spawn(async move {
                    _ = latch.read().await;
                    let timeout = Duration::from_millis(900);
                    tokio::time::sleep(timeout).await;
                    channel.gracefully_end_all_streams(timeout).await;
                });

                assert_eq!(observed_sum.load(Relaxed), 0, "Sanity check failed: no items should have been received until the latch is released");

                // release the lock and wait for the receiver to complete
                drop(lock);
                // wait for the receiver to complete
                receiver_task.await.expect("Error waiting for the receiver task to finish");

                assert_eq!(observed_sum.load(Relaxed), expected_sum, "Async sending is not working");
            }
        }
    }
    async_sending!(arc_atomic_async_sending,         arc::atomic::Atomic<u32, 1024, 1>);
    async_sending!(arc_full_sync_async_sending,      arc::full_sync::FullSync<u32, 1024, 1>);
    async_sending!(arc_crossbeam_async_sending,      arc::crossbeam::Crossbeam<u32, 1024, 1>);
    async_sending!(ogre_arc_atomic_async_sending,    ogre_arc::atomic::Atomic<u32, AllocatorAtomicArray<u32, 1024>>);
    async_sending!(ogre_arc_full_sync_async_sending, ogre_arc::full_sync::FullSync<u32, AllocatorFullSyncArray<u32, 1024>>);


    // *_allocation_semantics_alloc_and_send for all known Multi Channels
    /////////////////////////////////////////////////////////////////////

    macro_rules! allocation_semantics_alloc_and_send {
        ($fn_name: tt, $uni_channel_type: ty) => {
            /// Pre-allocates `BUFFER_SIZE` entries, then fill in their values, then send them all.
            /// Do this twice, so we are sure every slot was correctly freed up.
            /// Assert on the expected sum on both times.
            #[cfg_attr(not(doc),tokio::test)]
            async fn $fn_name() {

                let channel = Arc::new(<$uni_channel_type>::new("allocation_semantics_alloc_and_send"));

                let series_size = channel.buffer_size();
                let expected_sum = (1 + series_size) * (series_size / 2);
                let observed_sum = Arc::new(AtomicU32::new(0));

                // start the receiver task
                let (mut stream, _stream_id) = channel.create_stream_for_new_events();
                let cloned_observed_sum = observed_sum.clone();
                tokio::spawn(async move {
                    while let Some(received_number) = exec_future(stream.next(), &format!("receiving an item produced by `.reserve_slot()` / `.send_reserved()`"), 1.0, false).await {
                        use std::borrow::Borrow;    // allows working with either `u32` and `OgreUnique`
                        cloned_observed_sum.fetch_add(*received_number.borrow(), Relaxed);
                    }
                });

                let perform_pass = |pass_n| {
                    // reserve all slots
                    let reserved_slots: Vec<&mut u32> = (0..channel.buffer_size())
                        .map(|i| channel.reserve_slot().unwrap_or_else(|| panic!("We ran out of slots at #{i}, pass {pass_n}")))
                        .collect();

                    // publish
                    for (i, slot) in reserved_slots.into_iter().enumerate() {
                        *slot = (i+1) as u32;
                        assert!(channel.try_send_reserved(slot), "couldn't send a reserved slot on the first try -- we should, as there is no concurrency in place");
                    }
                };

                let timeout = Duration::from_millis(10);

                // 1st pass
                perform_pass(1);
                tokio::time::sleep(timeout).await;
                assert_eq!(observed_sum.load(Relaxed), expected_sum, "1st pass didn't produce the expected sum of items -- meaning not all items were produced/received");

                // 2nd pass
                perform_pass(2);
                tokio::time::sleep(timeout).await;
                assert_eq!(observed_sum.load(Relaxed), 2*expected_sum, "2nd pass (+ 1st pass) didn't produce the expected sum of items -- meaning, on the 2nd pass, not all items were produced/received");
            }
        }
    }
    //allocation_semantics_alloc_and_send!(arc_atomic_allocation_semantics_alloc_and_send,         arc::atomic::Atomic<u32, 1024, 1>);
    //allocation_semantics_alloc_and_send!(arc_full_sync_allocation_semantics_alloc_and_send,      arc::full_sync::FullSync<u32, 1024, 1>);
    //allocation_semantics_alloc_and_send!(arc_crossbeam_allocation_semantics_alloc_and_send,      arc::crossbeam::Crossbeam<u32, 1024, 1>);
    allocation_semantics_alloc_and_send!(ogre_arc_atomic_allocation_semantics_alloc_and_send,    ogre_arc::atomic::Atomic<u32, AllocatorAtomicArray<u32, 1024>>);
    allocation_semantics_alloc_and_send!(ogre_arc_full_sync_allocation_semantics_alloc_and_send, ogre_arc::full_sync::FullSync<u32, AllocatorFullSyncArray<u32, 1024>>);


    // *_allocation_semantics_alloc_and_give_up for all known Multi Channels
    ////////////////////////////////////////////////////////////////////////

    macro_rules! allocation_semantics_alloc_and_give_up {
        ($fn_name: tt, $uni_channel_type: ty) => {
            /// Tests that our reserve cancellation semantics work, not leaving any leaked elements behind.
            /// For each produced element, pre-allocates n entries, publishing just one -- cancelling the others:
            ///   1. On the 1st pass, just  perform 1 extra allocation (and 1 cancellation) for each produced element -- for `BUFFER_SIZE` + 2 times
            ///   2. On the 2nd pass, pre-allocates all the `BUFFER_SIZE` elements, producing just 1 and cancelling the others -- repeating this for `BUFFER_SIZE` + 2 times.
            #[cfg_attr(not(doc),tokio::test)]
            async fn $fn_name() {

                let channel = Arc::new(<$uni_channel_type>::new("allocation_semantics_alloc_and_give_up"));

                let series_size = channel.buffer_size() + 2;
                let expected_sum = (1 + series_size) * (series_size / 2);
                let observed_sum = Arc::new(AtomicU32::new(0));

                // start the receiver task
                let (mut stream, _stream_id) = channel.create_stream_for_new_events();
                let cloned_observed_sum = observed_sum.clone();
                tokio::spawn(async move {
                    while let Some(received_number) = exec_future(stream.next(), &format!("receiving an item produced by `.reserve_slot()` / `.send_reserved()`"), 1.0, false).await {
                        use std::borrow::Borrow;    // allows working with either `u32` and `OgreUnique`
                        cloned_observed_sum.fetch_add(*received_number.borrow(), Relaxed);
                    }
                });

                let publish = |pass, n, val| {
                    debug_assert!(n >= 2, "`n` should be, at least, 2");
                    // reserve `n` slots
                    let mut reserved_slots: Vec<&mut u32> = (0..n)
                        .map(|i| channel.reserve_slot().unwrap_or_else(|| panic!("We ran out of slots at #{val} (i={i}), pass {pass}, pending_items={}", channel.pending_items_count())))
                        .collect();

                    // populate & publish the first reserved slot
                    let slot = reserved_slots.remove(0);
                    *slot = val;
                    assert!(channel.try_send_reserved(slot), "couldn't send a reserved slot on the first try -- we should, as there is no concurrency in place");

                    // cancel all the remaining `n-1` reservations -- in the reversed order (the best case for all implementations)
                    for slot in reserved_slots.into_iter().rev() {
                        assert!(channel.try_cancel_slot_reserve(slot), "couldn't cancel a slot reservation on the first try -- we should, as there is no concurrency in place & we are cancelling reservations in the reversed order");
                    }
                };


                let timeout = Duration::from_millis(10);

                // 1st pass -- publish each element always reserving 1 extra slot -- which gets cancelled after each publishing
                for i in 0..series_size {
                    publish(1, 2, i+1);
                    if i >= channel.buffer_size()-2 {
                        channel.flush(Duration::from_millis(1)).await;
                    }
                }
                assert_eq!(channel.flush(timeout).await, 0, "Couldn't flush pending items withing {timeout:?} @ pass #1");
                assert_eq!(observed_sum.load(Relaxed), expected_sum, "1st pass didn't produce the expected sum of items -- meaning not all items were produced/received");

                // 2nd pass -- publish each element always reserving all slots -- cancelling the extra reservations after each publishing
                for i in 0..series_size {
                    publish(2, channel.buffer_size(), i+1);
                    channel.flush(Duration::from_millis(1)).await;
                }
                assert_eq!(channel.flush(timeout).await, 0, "Couldn't flush pending items withing {timeout:?} @ pass #2");
                assert_eq!(observed_sum.load(Relaxed), 2*expected_sum, "2nd pass (+ 1st pass) didn't produce the expected sum of items -- meaning, on the 2nd pass, not all items were produced/received");
            }
        }
    }
    //allocation_semantics_alloc_and_give_up!(arc_atomic_allocation_semantics_alloc_and_give_up,      arc::atomic::Atomic<u32, 1024, 1>);
    //allocation_semantics_alloc_and_give_up!(arc_crossbeam_allocation_semantics_alloc_and_give_up,   arc::crossbeam::Crossbeam<u32, 1024, 1>);
    //allocation_semantics_alloc_and_give_up!(arc_full_sync_allocation_semantics_alloc_and_give_up,   arc::full_sync::FullSync<u32, 1024, 1>);
    allocation_semantics_alloc_and_give_up!(ogre_arc_atomic_allocation_semantics_alloc_and_give_up,    ogre_arc::atomic::Atomic<u32, AllocatorAtomicArray<u32, 1024>>);
    allocation_semantics_alloc_and_give_up!(ogre_arc_full_sync_allocation_semantics_alloc_and_give_up, ogre_arc::full_sync::FullSync<u32, AllocatorFullSyncArray<u32, 1024>>);


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
                let (mut stream, _stream_id) = channel.create_stream_for_new_events();

                print!("{} (same task / same thread):           ", profiling_name);
                std::io::stdout().flush().unwrap();

                let start = Instant::now();
                for e in 0..count {
                    channel.send_with(|slot| *slot = e).expect_ok("couldn't send");
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
                let (mut stream, _stream_id) = channel.create_stream_for_new_events();

                let sender_future = async {
                    let mut e = 0;
                    while e < count {
                        let buffer_entries_left = channel.buffer_size() - channel.pending_items_count();
                        for _ in 0..buffer_entries_left {
                            channel.send_with(|slot| *slot = e).expect_ok("couldn't send");
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
                let (mut stream, _stream_id) = channel.create_stream_for_new_events();

                let sender_task = tokio::task::spawn_blocking(move || {
                    let mut e = 0;
                    while e < count {
                        let buffer_entries_left = channel.buffer_size() - channel.pending_items_count();
                        for _ in 0..buffer_entries_left {
                            channel.send_with(|slot| *slot = e).expect_ok("couldn't send");
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

        profile_same_task_same_thread_channel!(reference::mmap_log::MmapLog::<u32, 16384>::new("profile_same_task_same_thread_channel"), "Ref MmapLog      ", 16384*FACTOR);
        profile_different_task_same_thread_channel!(reference::mmap_log::MmapLog::<u32, 16384>::new("profile_different_task_same_thread_channel"), "Ref MmapLog      ", 16384*FACTOR);
        profile_different_task_different_thread_channel!(reference::mmap_log::MmapLog::<u32, 16384>::new("profile_different_task_different_thread_channel"), "Ref MmapLog      ", 16384*FACTOR);

    }

    /// resolves/executes the given `fut`ure, enforcing a timeout
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
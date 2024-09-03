//! Provides channels to be used in Unis

pub mod movable;
pub mod zero_copy;


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
    //use super::*;
    use crate::{
        uni::channels::movable,
        prelude::advanced::{
            ChannelCommon,
            ChannelUni,
            ChannelProducer,
            ChannelUniZeroCopyAtomic,
            ChannelUniZeroCopyFullSync,
        },
    };
    use std::{fmt::Debug, future::Future, sync::{Arc, atomic::{AtomicU32, Ordering::Relaxed}}, time::Duration, io::Write};
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
                let (mut stream, _stream_id) = channel.create_stream();
                let send_result = channel.send("a").is_ok();
                assert!(send_result, "Event couldn't be sent!");
                exec_future(stream.next(), "receiving", 1.0, true).await;
            }
        }
    }
    doc_test!(movable_atomic_queue_doc_test,      movable::atomic::Atomic<&str, 1024, 1>);
    doc_test!(movable_crossbeam_channel_doc_test, movable::crossbeam::Crossbeam<&str, 1024, 1>);
    doc_test!(movable_full_sync_queue_doc_test,   movable::full_sync::FullSync<&str, 1024, 1>);
    doc_test!(zero_copy_atomic_queue_doc_test,    ChannelUniZeroCopyAtomic<&str, 1024, 2>);
    doc_test!(zero_copy_full_sync_queue_doc_test, ChannelUniZeroCopyFullSync<&str, 1024, 2>);


    // *_sending_droppable_types for all known `UniChannel`s
    ////////////////////////////////////////////////////////

    macro_rules! sending_droppable_types {
        ($fn_name: tt, $uni_channel_type: ty) => {
            /// exercises the code present on the documentation for $uni_channel_type
            #[cfg_attr(not(doc),tokio::test)]
            async fn $fn_name() {
                let payload = String::from("Hey, it worked!");
                let channel = <$uni_channel_type>::new("test sending droppable types");
                let (mut stream, _stream_id) = channel.create_stream();
                // send()
                let send_result = channel.send(payload.clone()).is_ok();
                assert!(send_result, "Event couldn't be `send()`");
                let observed = exec_future(stream.next(), "receiving", 1.0, true).await;
                assert_eq!(observed.unwrap(), payload, "Wrong payloads using `send_with()`");
                // send_with()
                let send_result = channel.send_with(|slot| unsafe { std::ptr::write(slot, payload.clone()); }).is_ok();
                assert!(send_result, "Event couldn't be `send_with()`");
                let observed = exec_future(stream.next(), "receiving", 1.0, true).await;
                assert_eq!(observed.unwrap(), payload, "Wrong payloads using `send_with()`");
            }
        }
    }
    sending_droppable_types!(movable_atomic_queue_sending_droppable_types,      movable::atomic::Atomic<String, 1024, 1>);
    sending_droppable_types!(movable_crossbeam_channel_sending_droppable_types, movable::crossbeam::Crossbeam<String, 1024, 1>);
    sending_droppable_types!(movable_full_sync_queue_sending_droppable_types,   movable::full_sync::FullSync<String, 1024, 1>);
    sending_droppable_types!(zero_copy_atomic_queue_sending_droppable_types,    ChannelUniZeroCopyAtomic<String, 1024, 2>);
    sending_droppable_types!(zero_copy_full_sync_queue_sending_droppable_types, ChannelUniZeroCopyFullSync<String, 1024, 2>);


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
                    let (mut stream_1, _stream_id) = channel.create_stream();
                    let (stream_2, _stream_id) = channel.create_stream();
                    assert_eq!(Arc::strong_count(&channel), 3, "Creating each stream should increase the ref count by 1");
                    channel.send_with(|slot| *slot = "a").expect_ok("couldn't send");
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
                    let (stream, _stream_id) = channel.create_stream();
                    assert_eq!(Arc::strong_count(&channel), 2, "`channel` + `stream` + `local ref`: reference count should be 2");
                    drop(stream);
                    assert_eq!(Arc::strong_count(&channel), 1, "Dropping a stream should decrease the ref count by 1");
                    let (mut stream, _stream_id) = channel.create_stream();
                    assert_eq!(Arc::strong_count(&channel), 2, "1 `channel` + 1 `stream` again, at this point: reference count should be 2");
                    channel.send_with(|slot| *slot = "a").expect_ok("couldn't send");
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
                //     channel.try_send(|slot| *slot = "a");
                //     drop(channel);
                //     stream.next().await.unwrap();
                // }
                // println!("Done. Sleeping for 30 seconds. Go watch the heap!");
                // tokio::time::sleep(Duration::from_secs(30)).await;
            }
        }
    }
    dropping!(movable_atomic_queue_dropping,      movable::atomic::Atomic<&str, 1024, 2>);
    dropping!(movable_crossbeam_queue_dropping,   movable::crossbeam::Crossbeam<&str, 1024, 2>);
    dropping!(movable_full_sync_queue_dropping,   movable::full_sync::FullSync<&str, 1024, 2>);
    dropping!(zero_copy_atomic_queue_dropping,    ChannelUniZeroCopyAtomic<&str, 1024, 2>);
    dropping!(zero_copy_full_sync_queue_dropping, ChannelUniZeroCopyFullSync<&str, 1024, 2>);


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
                            let (stream, _stream_id) = channel.create_stream();
                            stream
                        }
                    })
                    .collect::<Vec<_>>().await;

                // send
                for i in 0..PARALLEL_STREAMS as u32 {
                    channel.send_with(|slot| *slot = i)
                        .retry_with(|setter| channel.send_with(setter))
                        .spinning_forever();
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
    parallel_streams!(movable_atomic_parallel_streams,      movable::atomic::Atomic<u32, 1024, PARALLEL_STREAMS>);
    parallel_streams!(movable_crossbeam_parallel_streams,   movable::crossbeam::Crossbeam<u32, 1024, PARALLEL_STREAMS>);
    parallel_streams!(movable_full_sync_parallel_streams,   movable::full_sync::FullSync<u32, 1024, PARALLEL_STREAMS>);
    parallel_streams!(zero_copy_atomic_parallel_streams,    ChannelUniZeroCopyAtomic<u32, 1024, PARALLEL_STREAMS>);
    parallel_streams!(zero_copy_full_sync_parallel_streams, ChannelUniZeroCopyFullSync<u32, 1024, PARALLEL_STREAMS>);


    // *_async_sending for all known Uni Channels
    /////////////////////////////////////////////

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
                let (mut stream, _stream_id) = channel.create_stream();
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
    async_sending!(movable_atomic_async_sending,      movable::atomic::Atomic<u32, 1024, 1>);
    async_sending!(movable_crossbeam_async_sending,   movable::crossbeam::Crossbeam<u32, 1024, 1>);
    async_sending!(movable_full_sync_async_sending,   movable::full_sync::FullSync<u32, 1024, 1>);
    async_sending!(zero_copy_atomic_async_sending,    ChannelUniZeroCopyAtomic<u32, 1024, 1>);
    async_sending!(zero_copy_full_sync_async_sending, ChannelUniZeroCopyFullSync<u32, 1024, 1>);


    // *_allocation_semantics_alloc_and_send for all known Uni Channels
    ///////////////////////////////////////////////////////////////////

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
                let (mut stream, _stream_id) = channel.create_stream();
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
    allocation_semantics_alloc_and_send!(movable_atomic_allocation_semantics_alloc_and_send,      movable::atomic::Atomic<u32, 1024, 1>);
    //allocation_semantics_alloc_and_send!(movable_crossbeam_allocation_semantics_alloc_and_send,   movable::crossbeam::Crossbeam<u32, 1024, 1>);
    //allocation_semantics_alloc_and_send!(movable_full_sync_allocation_semantics_alloc_and_send,   movable::full_sync::FullSync<u32, 1024, 1>);
    allocation_semantics_alloc_and_send!(zero_copy_atomic_allocation_semantics_alloc_and_send,    ChannelUniZeroCopyAtomic<u32, 1024, 1>);
    allocation_semantics_alloc_and_send!(zero_copy_full_sync_allocation_semantics_alloc_and_send, ChannelUniZeroCopyFullSync<u32, 1024, 1>);


    // *_allocation_semantics_alloc_and_give_up for all known Uni Channels
    //////////////////////////////////////////////////////////////////////

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
                let (mut stream, _stream_id) = channel.create_stream();
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
    allocation_semantics_alloc_and_give_up!(movable_atomic_allocation_semantics_alloc_and_give_up,      movable::atomic::Atomic<u32, 1024, 1>);
    //allocation_semantics_alloc_and_give_up!(movable_crossbeam_allocation_semantics_alloc_and_give_up,   movable::crossbeam::Crossbeam<u32, 1024, 1>);
    //allocation_semantics_alloc_and_give_up!(movable_full_sync_allocation_semantics_alloc_and_give_up,   movable::full_sync::FullSync<u32, 1024, 1>);
    allocation_semantics_alloc_and_give_up!(zero_copy_atomic_allocation_semantics_alloc_and_give_up,    ChannelUniZeroCopyAtomic<u32, 1024, 1>);
    allocation_semantics_alloc_and_give_up!(zero_copy_full_sync_allocation_semantics_alloc_and_give_up, ChannelUniZeroCopyFullSync<u32, 1024, 1>);


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
                let (mut stream, _stream_id) = channel.create_stream();

                print!("{} (same task / same thread):           ", profiling_name);
                std::io::stdout().flush().unwrap();

                let start = Instant::now();
                for e in 0..count {
                    channel.send_with(|slot| *slot = e).expect_ok("couldn't send");
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
                let (mut stream, _stream_id) = channel.create_stream();

                let sender_future = async {
                    for e in 0..count {
                        channel.send_with(|slot| *slot = e)
                            .retry_with_async(|setter| core::future::ready(channel.send_with(setter)))
                            .yielding_forever()
                            .await;        // hanging prevention: since we're on the same thread, we must yield or else the other task won't execute
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
                let (mut stream, _stream_id) = channel.create_stream();

                let sender_task = tokio::task::spawn_blocking(move || {
                    for e in 0..count {
                        channel.send_with(|slot| *slot = e)
                            .retry_with(|setter| channel.send_with(setter))
                            .spinning_forever();
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

        profile_same_task_same_thread_channel!(movable::atomic::Atomic::<u32, BUFFER_SIZE, 1>::new(""), "Movable Atomic     ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_same_thread_channel!(movable::atomic::Atomic::<u32, BUFFER_SIZE, 1>::new(""), "Movable Atomic     ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_different_thread_channel!(movable::atomic::Atomic::<u32, BUFFER_SIZE, 1>::new(""), "Movable Atomic     ", FACTOR*BUFFER_SIZE as u32);

        profile_same_task_same_thread_channel!(movable::crossbeam::Crossbeam::<u32, BUFFER_SIZE, 1>::new(""), "Movable Crossbeam  ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_same_thread_channel!(movable::crossbeam::Crossbeam::<u32, BUFFER_SIZE, 1>::new(""), "Movable Crossbeam  ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_different_thread_channel!(movable::crossbeam::Crossbeam::<u32, BUFFER_SIZE, 1>::new(""), "Movable Crossbeam  ", FACTOR*BUFFER_SIZE as u32);

        profile_same_task_same_thread_channel!(movable::full_sync::FullSync::<u32, BUFFER_SIZE, 1>::new(""), "Movable FullSync   ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_same_thread_channel!(movable::full_sync::FullSync::<u32, BUFFER_SIZE, 1>::new(""), "Movable FullSync   ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_different_thread_channel!(movable::full_sync::FullSync::<u32, BUFFER_SIZE, 1>::new(""), "Movable FullSync   ", FACTOR*BUFFER_SIZE as u32);

        profile_same_task_same_thread_channel!(ChannelUniZeroCopyAtomic::<u32, BUFFER_SIZE, 1>::new(""), "Zero-Copy Atomic   ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_same_thread_channel!(ChannelUniZeroCopyAtomic::<u32, BUFFER_SIZE, 1>::new(""), "Zero-Copy Atomic   ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_different_thread_channel!(ChannelUniZeroCopyAtomic::<u32, BUFFER_SIZE, 1>::new(""), "Zero-Copy Atomic   ", FACTOR*BUFFER_SIZE as u32);

        profile_same_task_same_thread_channel!(ChannelUniZeroCopyFullSync::<u32, BUFFER_SIZE, 1>::new(""), "Zero-Copy FullSync ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_same_thread_channel!(ChannelUniZeroCopyFullSync::<u32, BUFFER_SIZE, 1>::new(""), "Zero-Copy FullSync ", FACTOR*BUFFER_SIZE as u32);
        profile_different_task_different_thread_channel!(ChannelUniZeroCopyFullSync::<u32, BUFFER_SIZE, 1>::new(""), "Zero-Copy FullSync ", FACTOR*BUFFER_SIZE as u32);

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

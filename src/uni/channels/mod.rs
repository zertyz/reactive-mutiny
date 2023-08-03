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
                let (mut stream, _stream_id) = channel.create_stream();
                let send_result = channel.send_with(|slot| *slot = "a").is_ok();
                assert!(send_result, "Even couldn't be sent!");
                exec_future(stream.next(), "receiving", 1.0, true).await;
            }
        }
    }
    doc_test!(movable_atomic_queue_doc_test,      movable::atomic::Atomic<&str, 1024, 1>);
    doc_test!(movable_crossbeam_channel_doc_test, movable::crossbeam::Crossbeam<&str, 1024, 1>);
    doc_test!(movable_full_sync_queue_doc_test,   movable::full_sync::FullSync<&str, 1024, 1>);
    doc_test!(zero_copy_atomic_queue_doc_test,    ChannelUniZeroCopyAtomic<&str, 1024, 2>);
    doc_test!(zero_copy_full_sync_queue_doc_test, ChannelUniZeroCopyFullSync<&str, 1024, 2>);


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

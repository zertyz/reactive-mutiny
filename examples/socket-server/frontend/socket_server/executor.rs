//! Those are experimental executors while we wait for Mutiny library to land opensource.\
//! here, `std::sync::mpsc::sync_channel` cannot be used because the receiver cannot be shared between threads,
//! therefore we have `futures` and `tokio` implementations available, even if we have to go great lengths and
//! run the producer into an async context -- as of, for now, our networking library `message-io` is sync.


use super::{
    types::*,
    connection::SocketEvent,
    protocol_model::{ClientMessages, ServerMessages},
};
use std::{
    sync::Arc,
    time::Duration,
};
use std::future::Future;
use futures::{stream, Stream, StreamExt, SinkExt};
use log::{debug, warn};
use tokio::sync::mpsc::error::TrySendError;
use crate::frontend::socket_server::connection::Peer;
use crate::logic::ogre_robot::types::DisconnectionReason;


// internal configuration
/////////////////////////

/// for the producer Channel
pub const SENDER_BUFFER: usize  = 8192;

/// for the concurrent executor
pub const CONCURRENCY: usize = 16;

// /// for the parallel executor
// pub const PAR_PARAMS: ParParamsConfig =
//     //ParParamsConfig::Default;
//     //ParParamsConfig::ScaleOfCpus { scale: 1.0 }
//     ParParamsConfig::Manual { num_workers: NumWorkers::Default, buf_size: BufSize::Fixed(8192) }
// ;


/// creates a tuple of (stream, producer, closer) tied together using `futures::channel::mpsc::channel`\
/// not as fast as `tokio`'s, waits if channel is full, but we have a nice close function
pub fn sync_futures_stream(_tokio_runtime: Arc<tokio::runtime::Runtime>)
                          -> (impl Stream<Item = SocketEvent<ClientMessages, ServerMessages, DisconnectionReason>>,  // stream of client requests
                              impl FnMut(SocketEvent<ClientMessages, ServerMessages, DisconnectionReason>) -> bool,  // producer of client requests (adds to the stream)
                              impl FnMut()) {                                                                        // closer (closes the stream)

    let (mut tx, rx) = futures::channel::mpsc::channel::<SocketEvent<ClientMessages, ServerMessages, DisconnectionReason>>(SENDER_BUFFER);
    let stream = rx;
    let mut tx_for_close = tx.clone();

    (
        stream,
        // sync to async producer (here, `futures`' `block_on()` is faster than `tokio`'s)
        move |incoming| {
            let future = tx.feed(incoming);
            // block_on futures here is faster than tokio's
            futures::executor::block_on(future).expect("Could not send Socket Server network event. Did the `Stream` upgraded by `processor::processor` end, for some reason?");
            true
        },
        // nice close function, asserting all elements are flushed and no other elements may be sent through the channel
        move || { tx_for_close.close_channel(); },
    )
}

/// creates creates a tuple of  (stream, producer, closer) tied together using `tokio::sync::mpsc::channel`\
/// tokio channel -- through `.try_send()` is ~ 15% faster than using `futures`'s\
/// producer function is able to tell if the channel is full (so the server answers TooBusy),
/// but the close function is horrible
pub fn sync_tokio_stream(_tokio_runtime: Arc<tokio::runtime::Runtime>)
                        -> (impl Stream<Item = SocketEvent<ClientMessages, ServerMessages, DisconnectionReason>>,  // stream of client requests
                            impl Fn(SocketEvent<ClientMessages, ServerMessages, DisconnectionReason>) -> bool,     // producer of client requests (adds to the stream)
                            impl Fn()) {                                                                           // closer (closes the stream)

    let (tx, mut rx) = tokio::sync::mpsc::channel::<SocketEvent<ClientMessages, ServerMessages, DisconnectionReason>>(SENDER_BUFFER);
    let stream = stream::poll_fn(move |cx| rx.poll_recv(cx));

    (
        stream,
        // blocking producer
        move |incoming| match tx.try_send(incoming) {
            Ok(_) => true,
            Err(err) => match err {
                TrySendError::Full(_) => false,
                TrySendError::Closed(err) => panic!("Could not send Socket Server network event. The `Stream` upgraded by `processor::processor` closed: {:?}", err),
            }
        },
        // stupid "close" function, as tokio channels don't provide a way of syncing or even closing a channel before they are dropped
        move || std::thread::sleep(Duration::from_secs(5)),
    )
}

/// dummy stream executor -- In use while Mutiny library is not released
pub async fn spawn_stream_executor(stream: impl Stream<Item = (Arc<Peer<ServerMessages, DisconnectionReason>>, bool)> + Send + Sync + 'static) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        debug!("Experimental Stream Executor started!");
        stream.for_each(|(peer, send_status)| async move {
            if !send_status {
                warn!("Experimental Stream Executor faced a bad time sending a response back to {:?} (peer id {}): result: {:?}", peer.peer_address, peer.peer_id, send_status);
            }
        }).await;
        warn!("Experimental Executor ended!");
    })
}

// /// dummy stream executor allowing parallel execution -- In use while Mutiny library is not released
// pub async fn spawn_parallel_stream_executor(stream: impl Stream<Item = (Arc<Peer<ServerMessages, DisconnectionReason>>, bool)> + Send + Sync + 'static) -> tokio::task::JoinHandle<()> {
//     tokio::spawn(async move {
//         debug!("Experimental Parallel Stream Executor started!");
//         stream.par_for_each(PAR_PARAMS, |(peer, send_status)| async move {
//             if !send_status {
//                 warn!("Experimental Parallel Stream Executor faced a bad time sending a response back to {:?} (peer id {}): result: {:?}", peer.peer_address, peer.peer_id, send_status);
//             }
//         }).await;
//         warn!("Experimental Stream Executor ended!");
//     })
// }
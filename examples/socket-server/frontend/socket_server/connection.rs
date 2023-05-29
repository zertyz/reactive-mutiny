//! Tokio version of the inspiring `message-io` crate, but improving it on the following (as of 2022-08):
//! 1) Tokio is used for async IO -- instead of something else `message-io` uses, which is out of Rust's async execution context;
//! 2) `message-io` has poor/no support for streams/textual protocols: it, eventually, breaks messages in overload scenarios,
//!    transforming 1 good message into 2 invalid ones -- its `FramedTCP` comm model is not subjected to that, for the length is prepended to the message;
//! 3) `message-io` is prone to DoS attacks: during flood tests, be it in Release or Debug mode, `message-io` answers to a single peer only -- the flooder.
//! 4) `message-io` uses ~3x more CPU and is single threaded -- throughput couldn't be measured for the network speed was the bottleneck; latency was not measured at all


use crate::{
    frontend::socket_server::{
        protocol_model::{ClientMessages, ServerMessages},
        serde::{SocketServerDeserializer, SocketServerSerializer},
    },
    logic::ogre_robot::types::DisconnectionReason,
};
use std::{
    cell::Cell,
    collections::VecDeque,
    fmt::Debug,
    future::Future,
    marker::PhantomData,
    net::SocketAddr,
    ops::Deref,
    sync::{Arc, atomic::{AtomicBool, AtomicU32, AtomicU8}, atomic::Ordering::Relaxed},
    task::{Context, Poll, Waker},
    time::Duration,
};
use futures::{StreamExt, Stream, stream, SinkExt};
use tokio::{
    io::{self,AsyncWriteExt, AsyncBufReadExt, BufReader, BufWriter, Interest},
    net::{TcpListener, TcpStream, tcp::WriteHalf},
    sync::Mutex,
};
use log::{trace, debug, warn, error};


/// The internal events this peer shares with the protocol processors.\
/// If this peer is a server, then:
///   * `RemotePeerMessages` are the client messages
///   * `LocalPeerMessages` are the server messages
///   * `LocalPeerDisconnectionReason` is the reason offered to the remote peer for disconnecting
#[derive(Debug)]
pub enum SocketEvent<RemotePeerMessages, LocalPeerMessages, LocalPeerDisconnectionReason>
                    where LocalPeerMessages:            Send + Debug + SocketServerSerializer<LocalPeerMessages, LocalPeerDisconnectionReason>,
                          LocalPeerDisconnectionReason: Send {
    Incoming     {peer: Arc<Peer<LocalPeerMessages, LocalPeerDisconnectionReason>>, message: RemotePeerMessages },
    Connected    {peer: Arc<Peer<LocalPeerMessages, LocalPeerDisconnectionReason>>},
    Disconnected {peer: Arc<Peer<LocalPeerMessages, LocalPeerDisconnectionReason>>},
    Shutdown     {timeout_ms: u32},
}

const CHAT_MSG_SIZE_HINT: usize = 2048;

/// Our `message-io` special version for '\n' separated textual protocols,
/// substituting the original crate with functional and performance improvements (see [self]).\
/// `shutdown_signaler` comes from a `tokio::sync::oneshot::channel`, which receives the maximum time, in
/// milliseconds, to wait for messages to flush before closing & to send the shutdown message to connected clients
pub async fn server_network_loop_for_text_protocol<DisconnectionReason: Send + Sync + 'static,
                                                   ClientMessages:      SocketServerDeserializer<ClientMessages> + Send + 'static,
                                                   ServerMessages:      Debug + SocketServerSerializer<ServerMessages, DisconnectionReason> + Send + Sync + 'static,
                                                   ProcessorFuture:     Future<Output=()> + Send + 'static>
                                                  (listening_interface:   &str,
                                                   listening_port:        u16,
                                                   mut shutdown_signaler: tokio::sync::oneshot::Receiver<u32>,
                                                   processor:             impl Fn(SocketEvent<ClientMessages, ServerMessages, DisconnectionReason>) -> ProcessorFuture + Send + Sync + 'static) {

    let processor = Arc::new(processor);
    let listener = TcpListener::bind(&format!("{}:{}", listening_interface, listening_port)).await.unwrap();

    loop {

        // wait for a connection -- or for a shutdown signal
        let (socket, _addr) = if let Some(accepted_connection) = tokio::select! {
            // incoming connection
            acceptance_result = listener.accept() => {
                if let Err(err) = acceptance_result {
                    error!("PROBLEM while accepting a connection: {:?}", err);
                    None
                } else {
                    Some(acceptance_result.unwrap())
                }
            }
            // shutdown signal
            result = &mut shutdown_signaler => {
                let timeout_ms = match result {
                    Ok(timeout_millis) => {
                        trace!("SocketServer: SHUTDOWN requested -- with timeout {}ms -- bailing out from the network loop", timeout_millis);
                        timeout_millis
                    },
                    Err(err) => {
                        error!("SocketServer: PROBLEM waiting for shutdown signal: {:?}", err);
                        error!("SocketServer: Shutting down anyway...");
                        5000    // see this as the "problematic shutdown timeout (millis) constant"
                    },
                };
                // issue the shutdown event
                processor(SocketEvent::Shutdown { timeout_ms }).await;
                break
            }
        } {
            accepted_connection
        } else {
            // error accepting -- not fatal: try again
            continue
        };

        // spawn a task to handle communications with that client
        let processor = Arc::clone(&processor);
        tokio::spawn(connection_loop_for_textual_protocol(socket, move |event| processor(event)));

    }

    debug!("SocketServer: bailing out of network loop -- we should be undergoing a shutdown...")

}

pub async fn client_for_text_protocol<DisconnectionReason: Send + Sync + 'static,
                                      ClientMessages:      SocketServerDeserializer<ClientMessages> + Send + 'static,
                                      ServerMessages:      Debug + SocketServerSerializer<ServerMessages, DisconnectionReason> + Send + Sync + 'static,
                                      ProcessorFuture:     Future<Output=()> + Send + 'static>
                                     (server_address:      &str,
                                      server_port:         u16,
                                      mut shutdown_signaler: tokio::sync::oneshot::Receiver<u32>,
                                      processor:             impl FnMut(SocketEvent<ClientMessages, ServerMessages, DisconnectionReason>) -> ProcessorFuture + Send + Sync + 'static)
                                     -> Result<(), Box<dyn std::error::Error + Send + Sync>> {

    let socket = TcpStream::connect(&format!("{}:{}", server_address, server_port)).await?;
    tokio::spawn(connection_loop_for_textual_protocol(socket, processor));
    Ok(())
}

/// connection (chat) loop (after the connection is set) usable either by clients & servers of the same protocol
async fn connection_loop_for_textual_protocol<DisconnectionReason: Send,
                                              ClientMessages:      SocketServerDeserializer<ClientMessages> + Send,
                                              ServerMessages:      Debug + SocketServerSerializer<ServerMessages, DisconnectionReason> + Send + Sync,
                                              ProcessorFuture:     Future<Output=()> + Send>
                                             (mut socket:    TcpStream,
                                              mut processor: impl FnMut(SocketEvent<ClientMessages, ServerMessages, DisconnectionReason>) -> ProcessorFuture + Send + Sync) {

    let mut read_buffer = Vec::with_capacity(CHAT_MSG_SIZE_HINT);
    socket.set_nodelay(true).expect("setting nodelay() for the socket");
    socket.set_ttl(30).expect("setting ttl(30) for the socket");

    let peer_address = socket.peer_addr().expect("could not get the peer address");

    let sender = FastSender::<ServerMessages, DisconnectionReason>::new();
    let mut sender_stream = Arc::clone(&sender).stream();
    let peer = Arc::new(Peer::new(sender, peer_address));

    // issue the connection event
    processor(SocketEvent::Connected {peer: peer.clone()}).await;

    'connection: loop {
        // wait for the socket to be readable or until we have something to write
        tokio::select!(

            biased;     // sending has priority over receiving

            // send?
            result = sender_stream.next() => {
                match result {
                    Some(to_send_message) => {
                        let to_send_text = ServerMessages::ss_serialize(&to_send_message);
                        if let Err(err) = socket.write_all(to_send_text.as_bytes()).await {
                            warn!("SocketServer::TokioMessageIO: PROBLEM in the connection with {:?} (peer id {}) while WRITING: '{:?}' -- dropping it", peer.peer_address, peer.peer_id, err);
                            peer.sender.close();
                            break 'connection
                        }
                        // sending was complete. Was it a "disconnect" message?
                        if ServerMessages::is_disconnect_message(&to_send_message).is_some() {
                            peer.sender.close();
                            break 'connection
                        }
                    },
                    None => {
                        warn!("SocketServer::TokioMessageIO: Sender for {:?} (peer id {}) ended (most likely, .close() was called on the `peer` by the processor. Closing the connection...", peer.peer_address, peer.peer_id);
                        peer.sender.close();
                        break 'connection
                    }
                }
            },
            // read?
            _ = socket.readable() => {
                match socket.try_read_buf(&mut read_buffer) {
                    Ok(0) => {
                        warn!("SocketServer::TokioMessageIO: PROBLEM with reading from {:?} (peer id {}) -- it is out of bytes! Dropping the connection", peer.peer_address, peer.peer_id);
                        peer.sender.close();
                        break 'connection
                    },
                    Ok(n) => {
                        let mut next_line_index = 0;
                        let mut search_start = read_buffer.len() - n;
                        loop {
                            if let Some(mut eol_pos) = read_buffer[next_line_index+search_start..].iter().position(|&b| b == '\n' as u8) {
                                eol_pos += next_line_index+search_start;
                                let line_bytes = &read_buffer[next_line_index..eol_pos];
                                match ClientMessages::ss_deserialize(&line_bytes) {
                                    Ok(client_message) => processor(SocketEvent::Incoming {peer: peer.clone(), message: client_message}).await,
                                    Err(err) => {
                                        let stripped_line = String::from_utf8_lossy(line_bytes);
                                        debug!("SocketServer: Unknown command received from {:?} (peer id {}): '{}'",
                                        peer.peer_address, peer.peer_id, stripped_line);
                                        ServerMessages::send_unknown_input_error(&peer, stripped_line.to_string(), err).await;
                                    }
                                }
                                next_line_index = eol_pos + 1;
                                if next_line_index >= read_buffer.len() {
                                    next_line_index = read_buffer.len();
                                    break
                                }
                                search_start = 0;
                            } else {
                                break
                            }
                        }
                        if next_line_index > 0 {
                            read_buffer.drain(0..next_line_index);
                        }
                    },
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {},
                    Err(err) => {
                        error!("SocketServer::TokioMessageIO: ERROR in the connection with {:?} (peer id {}) while READING: '{:?}' -- dropping it", peer.peer_address, peer.peer_id, err);
                        break 'connection
                    },
                }
            },
        );

    }

    // issue the disconnection event
    processor(SocketEvent::Disconnected {peer: peer.clone()});

}

static PEER_COUNTER: AtomicU32 = AtomicU32::new(0);
pub type PeerId = u32;
#[derive(Debug)]
pub struct Peer<MessagesType:            Send + Debug + SocketServerSerializer<MessagesType, DisconnectionReasonType>,
                DisconnectionReasonType: Send> {
    pub peer_id:      PeerId,
    pub sender:       Arc<FastSender<MessagesType, DisconnectionReasonType>>,
    pub peer_address: SocketAddr,
}

impl<MessagesType:            Send + Debug + SocketServerSerializer<MessagesType, DisconnectionReasonType>,
     DisconnectionReasonType: Send>
Peer<MessagesType,
     DisconnectionReasonType> {

    pub fn new(sender: Arc<FastSender<MessagesType, DisconnectionReasonType>>, peer_address: SocketAddr) -> Self {
        Self {
            peer_id: PEER_COUNTER.fetch_add(1, Relaxed),
            sender,
            peer_address,
        }
    }

}

/// as-fast-as-possible same-thread channel for sending messages to peers, relying on the fact that
/// our network chat for receiving and sending messages uses a single thread.\
/// It is 10x faster than tokio's MPSC, which is not needed here.
#[derive(Debug)]
pub struct FastSender<MessagesType:            Send + Debug + SocketServerSerializer<MessagesType, DisconnectionReasonType>,
                      DisconnectionReasonType: Send> {
    to_send_messages:           Mutex<VecDeque<MessagesType>>,
    waker:                      Option<Waker>,
    closed:                     bool,
    _disconnection_reason_type: PhantomData<DisconnectionReasonType>,
}

impl<MessagesType:            Send + Debug + SocketServerSerializer<MessagesType, DisconnectionReasonType>,
     DisconnectionReasonType: Send>
FastSender<MessagesType, DisconnectionReasonType> {

    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            to_send_messages:           Mutex::new(VecDeque::with_capacity(1)),
            waker:                      None,
            closed:                     false,
            _disconnection_reason_type: PhantomData::default(),
        })
    }

    #[inline(always)]
    pub async fn send(&self, message: MessagesType) {
        self.to_send_messages.lock().await
                             .push_back(message);
        // wake the stream
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }

    #[inline(always)]
    pub async fn flush(&self) {
        loop {
            let len = self.to_send_messages.lock().await.len();
            if len == 0 {
                break;
            } else {
                // wake the stream
                if let Some(waker) = &self.waker {
                    waker.wake_by_ref();
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    }

    /// should be called just once
    pub fn stream(self: Arc<Self>) -> impl Stream<Item=MessagesType> {
        stream::poll_fn(move |cx| {
            let mutable_self = unsafe {&mut *((Arc::as_ptr(&self)) as *mut Self)};
            mutable_self.waker.get_or_insert_with(|| cx.waker().clone());
            if let Ok(mut mutable_to_send_messages) = self.to_send_messages.try_lock() {
                match mutable_to_send_messages.pop_front() {
                    Some(message) => Poll::Ready(Some(message)),
                    None => {
                        if !self.closed {
                            Poll::Pending
                        } else {
                            Poll::Ready(None)
                        }
                    }
                }
            } else {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        })
    }



    /// Marks this sender as closed, which should cause the connection to be dropped
    pub fn close(&self) {
        let mutable_self = unsafe {&mut *((self as *const Self) as *mut Self)};
        mutable_self.closed = true;
        // wake the stream
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

}


/// Unit tests the [tokio_message_io](self) module
#[cfg(any(test,doc))]
mod tests {
    use std::time::SystemTime;
    use futures::future::BoxFuture;
    use super::*;

    #[cfg(debug_assertions)]
    const DEBUG: bool = true;
    #[cfg(not(debug_assertions))]
    const DEBUG: bool = false;


    #[ctor::ctor]
    fn suite_setup() {
        simple_logger::SimpleLogger::new().with_utc_timestamps().init().unwrap_or_else(|_| eprintln!("--> LOGGER WAS ALREADY STARTED"));
    }


    /// assures connection & dialogs work
    #[cfg_attr(not(doc),tokio::test)]
    async fn connect_and_disconnect() {
        const CLIENT_SECRET: &str = "open, sesame";
        const SERVER_SECRET: &str = "now the 40 of you may enter";
        let observed_secret = Arc::new(Mutex::new(None));

        let (server_shutdown_sender, server_shutdown_receiver) = tokio::sync::oneshot::channel::<u32>();
        let (_client_shutdown_sender, client_shutdown_receiver) = tokio::sync::oneshot::channel::<u32>();

        // server
        tokio::spawn(server_network_loop_for_text_protocol("127.0.0.1", 8570, server_shutdown_receiver, |event: SocketEvent<String, String, ()>| async {
            match event {
                SocketEvent::Incoming { peer, message: client_message } => {
                    peer.sender.send(format!("Client just sent '{}'\n", client_message)).await;
                    if client_message == CLIENT_SECRET {
                        peer.sender.send(format!("{}\n", SERVER_SECRET)).await;
                    } else {
                        panic!("Client sent the wrong secret: '{}' -- I was expecting '{}'", client_message, CLIENT_SECRET);
                    }
                },
                SocketEvent::Connected { peer } => {
                    peer.sender.send(format!("Welcome! State your business!\n")).await;
                },
                SocketEvent::Disconnected { peer } => {},
                SocketEvent::Shutdown { timeout_ms } => {
                    println!("Server: shutdown was requested... No connection will receive the drop message (nor even will be dropped) because I didn't keep track of the connected peers!");
                },
            }
        }));

        println!("### Waiting a little for the server to start...");
        tokio::time::sleep(Duration::from_millis(10)).await;

        // client
        let observed_secret_ref = Arc::clone(&observed_secret);
        client_for_text_protocol("127.0.0.1", 8570, client_shutdown_receiver, move |event: SocketEvent<String, String, ()>| {
            let observed_secret_ref = Arc::clone(&observed_secret_ref);
            async move {
                match event {
                    SocketEvent::Incoming { peer, message: server_message } => {
                        println!("Server said: '{}'", server_message);
                        let _ = observed_secret_ref.lock().await.insert(server_message);
                    },
                    SocketEvent::Connected { peer } => {
                        peer.sender.send(format!("{}\n", CLIENT_SECRET)).await;
                    },
                    SocketEvent::Disconnected { peer } => {
                        println!("Client: connection with {} (peer_id #{}) was dropped -- should not happen in this test", peer.peer_address, peer.peer_id);
                    },
                    SocketEvent::Shutdown { timeout_ms } => {},
                }
            }
        }).await.expect("Starting the client");
        println!("### Client is running concurrently, in the background...");

        tokio::time::sleep(Duration::from_millis(500)).await;
        server_shutdown_sender.send(500).expect("sending shutdown signal");

        println!("### Waiting a little for the shutdown signal to reach the server...");
        tokio::time::sleep(Duration::from_millis(10)).await;

        assert_eq!(*observed_secret.lock().await, Some(SERVER_SECRET.to_string()), "Communications didn't go according the plan");
    }

    /// Assures the minimum acceptable latency values -- either for Debug & Release modes.\
    /// One sends Ping(n); the other receives it and send Pong(n); the first receives it and sends Ping(n+1) and so on...\
    /// Latency is computed dividing the number of seconds per n*2 (we care about the server leg of the latency, while here we measure the round trip client<-->server)
    #[cfg_attr(not(doc),tokio::test)]
    async fn latency_measurements() {
        const TEST_DURATION_MS: u64 = 2000;
        const TEST_DURATION_NS: u64 = TEST_DURATION_MS * 1e6 as u64;
        let (server_shutdown_sender, server_shutdown_receiver) = tokio::sync::oneshot::channel::<u32>();
        let (_client_shutdown_sender, client_shutdown_receiver) = tokio::sync::oneshot::channel::<u32>();

        // server
        tokio::spawn(server_network_loop_for_text_protocol("127.0.0.1", 8571, server_shutdown_receiver, |event: SocketEvent<String, String, ()>| async {
            match event {
                SocketEvent::Incoming { peer, message: client_message } => {
                    // Message received: Ping(n)
                    // Answer: Pong(n)
                    let n_str = &client_message[5..(client_message.len()-1)];
                    let n = str::parse::<u32>(n_str).expect(&format!("could not convert '{}' to number. Original message: '{}'", n_str, client_message));
                    peer.sender.send(format!("Pong({})\n", n)).await;
                },
                SocketEvent::Connected { .. } => {},
                SocketEvent::Disconnected { .. } => {},
                SocketEvent::Shutdown { .. } => {},
            }
        }));

        println!("### Waiting a little for the server to start...");
        tokio::time::sleep(Duration::from_millis(10)).await;

        let counter = Arc::new(AtomicU32::new(0));
        let counter_ref = Arc::clone(&counter);
        // client
        client_for_text_protocol("127.0.0.1", 8571, client_shutdown_receiver, move |event: SocketEvent<String, String, ()>| {
            let counter_ref = Arc::clone(&counter_ref);
            async move {
                match event {
                    SocketEvent::Incoming { peer, message: server_message } => {
                        // Message received: Pong(n)
                        // Answer: Ping(n+1)
                        let n_str = &server_message[5..(server_message.len()-1)];
                        let n = str::parse::<u32>(n_str).expect(&format!("could not convert '{}' to number. Original message: '{}'", n_str, server_message));
                        let current_count = counter_ref.fetch_add(1, Relaxed);
                        if n != current_count {
                            panic!("Received '{}', where Client was expecting 'Pong({})'", server_message, current_count);
                        }
                        peer.sender.send(format!("Ping({})\n", current_count+1)).await;
                    },
                    SocketEvent::Connected { peer } => {
                        peer.sender.send(format!("Ping(0)\n")).await;
                    },
                    SocketEvent::Disconnected { .. } => {},
                    SocketEvent::Shutdown { .. } => {},
                }
            }
        }).await.expect("Starting the client");
        println!("### Measuring latency for 2 seconds...");

        tokio::time::sleep(Duration::from_millis(TEST_DURATION_MS)).await;
        server_shutdown_sender.send(500).expect("sending shutdown signal");

        println!("### Waiting a little for the shutdown signal to reach the server...");
        tokio::time::sleep(Duration::from_millis(10)).await;

        let counter = counter.load(Relaxed);
        let latency = Duration::from_nanos((TEST_DURATION_NS as f64 / (2.0 * counter as f64)) as u64);
        println!("Round trips counter: {}", counter);
        println!("Measured latency: {:?} ({})", latency, if DEBUG {"Debug mode"} else {"Release mode"});
        if DEBUG {
            assert!(counter > 13000, "Latency regression detected: we used to make 23273 round trips in 2 seconds (Debug mode) -- now only {} were made", counter);
        } else {
            assert!(counter > 290000, "Latency regression detected: we used to make 308955 round trips in 2 seconds (Release mode) -- now only {} were made", counter);

        }
    }

    /// When a client floods the server with messages, it should, at most, screw just that client up... or, maybe, not even that!\
    /// This test works like the latency test, but we don't wait for the answer to come to send another one -- we just do it like crazy\
    /// (currently, as of 2022-10-29, a flooding client won't have all its messages processed, for some reason that should be still investigated and improved upon)
    #[cfg_attr(not(doc),tokio::test(flavor = "multi_thread"))]
    async fn message_flooding_throughput_test() {
        const TEST_DURATION_MS: u64 = 2000;
        const TEST_DURATION_NS: u64 = TEST_DURATION_MS * 1e6 as u64;
        let (server_shutdown_sender, server_shutdown_receiver) = tokio::sync::oneshot::channel::<u32>();
        let (_client_shutdown_sender, client_shutdown_receiver) = tokio::sync::oneshot::channel::<u32>();

        // server -- do not answer to (flood) messages (just parses & counts them, making sure they are received in the right order)
        // message format is "DoNotAnswer(n)", where n should be sent by the client in natural order, starting from 0
        let received_messages_count = Arc::new(AtomicU32::new(0));
        let unordered = Arc::new(AtomicU32::new(0));    // if non-zero, will contain the last message received before the ordering went kaputt
        let received_messages_count_ref = Arc::clone(&received_messages_count);
        let unordered_ref = Arc::clone(&unordered);
        tokio::spawn(server_network_loop_for_text_protocol("127.0.0.1", 8572, server_shutdown_receiver, move |event: SocketEvent<String, String, ()>| {
            let received_messages_count = Arc::clone(&received_messages_count_ref);
            let unordered = Arc::clone(&unordered_ref);
            async move {
                match event {
                    SocketEvent::Incoming { peer, message: client_message } => {
                        // Message format: DoNotAnswer(n)
                        let n_str = &client_message[12..(client_message.len()-1)];
                        let n = str::parse::<u32>(n_str).expect(&format!("could not convert '{}' to number. Original message: '{}'", n_str, client_message));
                        let count = received_messages_count.fetch_add(1, Relaxed);
                        if count != n {
                            if unordered.compare_exchange(0, count, Relaxed, Relaxed).is_ok() {
                                println!("Server: ERROR: received order of messages broke at message #{}", count);
                            };
                        }
                    },
                    SocketEvent::Connected { .. } => {},
                    SocketEvent::Disconnected { .. } => {},
                    SocketEvent::Shutdown { .. } => {},
                }
            }
        }));

        println!("### Waiting a little for the server to start...");
        tokio::time::sleep(Duration::from_millis(10)).await;

        // client
        let sent_messages_count = Arc::new(AtomicU32::new(0));
        let sent_messages_count_ref = Arc::clone(&sent_messages_count);
        client_for_text_protocol("127.0.0.1", 8572, client_shutdown_receiver, move |event: SocketEvent<String, String, ()>| {
            let sent_messages_count = Arc::clone(&sent_messages_count_ref);
            async move {
                let sent_messages_count = Arc::clone(&sent_messages_count);
                match event {
                    SocketEvent::Incoming { peer, message: server_message } => {},
                    SocketEvent::Connected { peer } => {
                        tokio::spawn(async move {
                            let start = SystemTime::now();
                            let mut n = 0;
                            loop {
                                peer.sender.send(format!("DoNotAnswer({})\n", n)).await;
                                n += 1;
                                // flush & bailout check for timeout every 1024 messages
                                if n % (1<<10) == 0 {
                                    peer.sender.flush().await;
                                    if start.elapsed().unwrap().as_millis() as u64 >= TEST_DURATION_MS  {
                                        println!("Client sent {} messages before bailing out", n);
                                        sent_messages_count.store(n, Relaxed);
                                        break;
                                    }
                                }
                            }
                        });
                    },
                    SocketEvent::Disconnected { .. } => {},
                    SocketEvent::Shutdown { .. } => {},
                }
            }
        }).await.expect("Starting the client");
        println!("### Measuring latency for 2 seconds...");

        tokio::time::sleep(Duration::from_millis(TEST_DURATION_MS)).await;
        server_shutdown_sender.send(500).expect("sending shutdown signal");

        println!("### Waiting a little for the shutdown signal to reach the server...");
        tokio::time::sleep(Duration::from_millis(1000)).await;

        println!("### Server saw:");
        let received_messages_count = received_messages_count.load(Relaxed);
        let unordered = unordered.load(Relaxed);
        let sent_messages_count = sent_messages_count.load(Relaxed);
        let received_messages_percent = 100.0 * (received_messages_count as f64 / sent_messages_count as f64);
        println!("    {} received messages {}", received_messages_count, if unordered == 0 {format!("in order")} else {format!("unordered -- ordering broke at message #{}", unordered)});
        println!("    {:.2}% of sent ones", received_messages_percent);

        assert_eq!(unordered, 0, "Server should have received messages in order, but it was broken at message #{} -- total received was {}", unordered, received_messages_count);
        assert!(received_messages_percent >= 99.99, "Client flooding regression detected: the server used to receive 100% of the sent messages -- now only {:.2}% made it through", received_messages_percent);
        if DEBUG {
            assert!(received_messages_count > 100000, "Client flooding throughput regression detected: we used to send/receive 217088 flood messages in this test (Debug mode) -- now only {} were made", received_messages_count);
        } else {
            assert!(received_messages_count > 300000, "Client flooding throughput regression detected: we used to send/receive 477184 flood messages in this test (Release mode) -- now only {} were made", received_messages_count);

        }

    }


    /// Test implementation for our text-only protocol
    impl SocketServerSerializer<String, ()> for String {
        fn ss_serialize(message: &String) -> String {
            message.clone()
        }
        fn send_unknown_input_error(peer: &Arc<Peer<String, ()>>, raw_input: String, err: Box<dyn std::error::Error + Send + Sync>) -> BoxFuture<'_, ()> {
            let r = peer.sender.send(format!("ServerBug! Please, fix! Message '{}' is said not to be able to be converted into a String, which is a non-sense!: {}", raw_input, err));
            panic!("SocketServerSerializer<String>::send_unknown_input_error(): BUG!! How come String cannot be serialized into a String? isn't '{}' already a String?", raw_input);
            Box::pin(r)
        }
        fn is_disconnect_message(processor_answer: &String) -> Option<&()> {
            // for String communications, an empty line sent by the messages processor signals that the connection should be closed
            if processor_answer.is_empty() {
                Some(&())
            } else {
                None
            }
        }
    }

    /// Testable implementation for our text-only protocol
    impl SocketServerDeserializer<String> for String {
        fn ss_deserialize(message: &[u8]) -> Result<String, Box<dyn std::error::Error + Sync + Send>> {
            // TODO consider using the generic Cow<'a, str> instead of String, for increased performance allowing conversions not to take place...
            Ok(String::from_utf8_lossy(message).to_string())
        }
    }

}
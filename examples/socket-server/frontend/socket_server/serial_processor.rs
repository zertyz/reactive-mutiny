//! Here you'll see a demonstration of how to create an async processor that needs a single thread to perform simple operations,
//! and, for this reason, it is way faster than [parallel_processor].\
//! On the example implemented here, it is able to perform:
//!   - 868k/s input messages speed -- with 180% CPU -- for the following input:
//!     (I used a variation of the following command, writing the input to a file and then passing the output through an 8k buffer dd writing to /dev/null)
//!     clear; (for i in {1..5654356}; do for m in "Ping" "Speechless" "Pang" "Help" "Ping" "Speechless" "Pang" "Help"; do echo "$m";done; done) | nc -vvvv localhost 9758 | dd status=progress | wc -l
//!   - 4M/s was attained (similar CPU usage) with an input file from this command:
//!     (for i in {1..5654356}; do echo -en "Speechless\nSpeechless\nSpeechless\nSpeechless\nSpeechless\nSpeechless\nSpeechless\nSpeechless\n"; done) >/tmp/kickass.input2
//!   - IMPORTANT: set `sync_processors()` to use a waiting producer, like [super::executor::sync_futures_processors()], or else you'll simply get `TooBusy` answers
//!
//! Analysis:
//!   - One thread is executing `message-io` and another, this processor
//!   - The last test don't use allocations and do not send back any messages -- and it was almost 6 times faster
//!   - In the future, those figures are to be improved when `message-io` is replaced with a Tokio implementation
//!     (so there is no async/sync overhead)
//!
//! `message-io`: it was a negative surprise that `message-io` wasn't able to process any other connections when these flood tests were being executed
//!
//! ==================================================================
//! Easy: clear; (for msg in 'ClientIdentification(MarketDataBridge(version: "123", symbol: "PETR3", account_token: "mYtOkEn"))' 'MarketData(SymbolState(symbol: "PETR3", in_auction: false))'; do sleep 1; echo "> $msg" >&2; echo "$msg"; done; sleep 1; echo -en "\n### Now write what you want or hit CTRL-C\n\n" >&2; cat) | nc -vvvv 192.168.1.37 9758
//! Flood: clear; m1='ClientIdentification(MarketDataBridge(version: "123", symbol: "PETR3", account_token: "mYtOkEn"))'; m2='MarketData(SymbolState(symbol:"P3",in_auction:true))'; (sleep 1; echo "> $m1" >&2; echo "$m1"; sleep 1; echo "> $m2 (as much as possible)" >&2; while true; do echo "$m2"; done) | nc -vvvv 192.168.1.37 9758
//! Minimum flood, without message corruption: clear; m1='ClientIdentification(MarketDataBridge(version: "123", symbol: "PETR3", account_token: "mYtOkEn"))'; m2='MarketData(SymbolState(symbol:"P3",in_auction:true))'; (sleep 1; echo "> $m1" >&2; echo "$m1"; sleep 1; echo "> $m2 (as much as possible)" >&2; while true; do echo "$m2"; sleep 0.009; done) | nc -vvvv 192.168.1.37 9758
//!
//! Note: the flood test above, due to the use of message-io, makes the server see wrong messages, occasionally -- because a packet split a message in two, the two parts will be treated as two different messages (both invalid).
//!       This should be solved when we move out of message-io into our own tokio-only solution.
//!
//! References:
//!   - https://github.com/sloganking/Rust-and-Tokio-Chat-Server/blob/51d6bad64be0996202ba610d80296a829a0c950d/src/main.rs
//!   - https://github.com/tokio-rs/tokio/blob/master/examples/chat.rs

use super::{
    types::*,
    protocol::*,
    socket_server::{SocketServer},
    tokio_message_io::{SocketEvent, Peer, PeerId},
};
use crate::{
    Runtime,
    logic::ogre_robot::{
        types::*,
        events::dispatcher::Dispatcher,
    },
};
use std::{
    sync::Arc,
    collections::HashMap,
    fmt::Debug,
    ops::Deref,
};
use std::time::Duration;
use futures::{Stream, stream, StreamExt};
use minstant::Instant;
use tokio::sync::{RwLock, RwLockWriteGuard};
use log::{trace, warn};


/// customize this to hold the states you want for each client
#[derive(Debug)]
struct ClientStates {
    identification: TimeTrackedInfo<ClientIdentification>,
    /// used to send async messages to the client
    peer: Arc<Peer<ServerMessages, DisconnectionReason>>,
    //round_trips: // Option<RoundTripsData> -- count, average, last
    estimated_clock_skew_nanos: Option<i32>,
    // metrics session:
    //messages_count: HashMap<String -- ClientMessages names as keys, AtomicU32>  // move it to the Socket Server -- optionally enabled in generic const

}

/// Here is where the main "protocol" processor logic sits: returns a Stream pipeline able to
/// transform client inputs ([ClientMessages] requests) into server outputs ([ServerMessages] answers)
fn processor(dispatcher:      Arc<Dispatcher>,
             stream:          impl Stream<Item = SocketEvent<ClientMessages, ServerMessages, DisconnectionReason>>)
             -> impl Stream<Item = Result<(Arc<Peer<ServerMessages, DisconnectionReason>>, ServerMessages),
                                          (Arc<Peer<ServerMessages, DisconnectionReason>>, Box<dyn std::error::Error + Sync + Send>)>> {

    let client_states: Arc<RwLock<HashMap<PeerId, ClientStates>>> = Arc::new(RwLock::new(HashMap::new()));

    stream
        .map(move |socket_event: SocketEvent<ClientMessages, ServerMessages, DisconnectionReason>| {
            let dispatcher = Arc::clone(&dispatcher);
            let client_states = Arc::clone(&client_states);
            async move {
                match socket_event {

                    SocketEvent::Incoming { peer, message: client_message } => {

                        // standard read-only lock on `state`
                        let lock = client_states.read().await;
                        let state = lock.get(&peer.peer_id)
                            .map_or_else(|| Err( ( Arc::clone(&peer),
                                                   Box::from(format!("SocketServer.Processor expects incoming messages only from known clients. Peer id {}, who is not in our `client_states` hashmap, popped up from address {:?}", peer.peer_id, peer.peer_address)) ) ),
                                         |state| Ok(state))?;

                        let server_message = match client_message {

                            ClientMessages::ClientIdentification(client_identification) => {
                                // upgrade our lock into a writeable `state`
                                drop(lock);
                                let mut state = RwLockWriteGuard::map(client_states.write().await, |states| states.get_mut(&peer.peer_id).unwrap() );

                                match state.identification.set(client_identification) {
                                    Ok(client_identification) => {
                                        if dispatcher.register_from_client_identification(client_identification).await {
                                            ServerMessages::None
                                        } else {
                                            // TODO this failure is currently happening for the new connection -- it must be the other way round, as stated on the Reconnected docs
                                            ServerMessages::Disconnected(DisconnectionReason::Reconnected { ip: "yours".to_string() })
                                        }
                                    },
                                    Err(client_identification) => {
                                        let message = format!("Attempt to authenticate twice! Protocolar authentication was {:?};  Out of protocol attempted one is {:?}", state.identification, client_identification);
                                        let disconnection_reason = DisconnectionReason::ProtocolOffense { message };
                                        dispatcher.unregister_from_client_identification(&*state.identification, disconnection_reason.clone()).await;
                                        drop(state);
                                        client_states.write().await
                                            .remove(&peer.peer_id);
                                        ServerMessages::Disconnected(disconnection_reason)
                                    }
                                }
                            },

                            ClientMessages::UserAuthorization(_) => ServerMessages::None,

                            ClientMessages::KeepAliveRequest(n) => ServerMessages::KeepAliveAnswer(n+1),

                            ClientMessages::KeepAliveAnswer(_) => ServerMessages::None,

                            ClientMessages::MarketData(market_data) => {
                                dispatcher.market_data(&*state.identification, market_data.into()).await;
                                ServerMessages::None
                            },

                            ClientMessages::ExecutedOrder { .. } => ServerMessages::None,

                            ClientMessages::CancelledOrder { .. } => ServerMessages::None,

                            ClientMessages::PendingOrder { .. } => ServerMessages::None,

                            ClientMessages::ChartPoints { .. } => ServerMessages::None,

                            ClientMessages::GoodBye(_) => ServerMessages::Disconnected(DisconnectionReason::ClientInitiated),
                            ClientMessages::UnknownMessage(txt) => ServerMessages::Disconnected(DisconnectionReason::ProtocolOffense {message: format!("Unknown message received: '{}'. Bailing out...", txt)}),
                        };
                        Ok(Some((peer, server_message)))
                    },

                    SocketEvent::Connected { peer } => {
                        client_states.write().await
                            .insert(peer.peer_id,
                                    ClientStates { identification:             TimeTrackedInfo::Unset,
                                        peer:                       Arc::clone(&peer),
                                        estimated_clock_skew_nanos: None });
                        Ok(Some((peer, ServerMessages::Welcome/*(runtime.)*/)))
                    },

                    SocketEvent::Disconnected { peer } => {
                        let state = client_states.write().await
                            .remove(&peer.peer_id).expect("disconnected one was not present");
                        dispatcher.unregister_from_client_identification(&*state.identification, DisconnectionReason::ClientInitiated).await;
                        Ok(Some((peer, ServerMessages::None)))
                    },

                    SocketEvent::Shutdown { timeout_ms } => {
                        warn!("SocketServer processor: Sending goodbye message and disconnecting from {} clients", client_states.read().await.len());
                        for (_peer_id, client_info) in client_states.read().await.iter() {
                            client_info.peer.sender.send(ServerMessages::ShuttingDown).await;
                            client_info.peer.sender.close();
                        }
                        Ok(None)
                    },
                }
            }
        })

        // transforms `Result<Option<>>` into `Option<Result<>>`
        // (the former is used in the above map so we may use there the `?` operator without trouble)
        .filter_map(|fallible_future| async {
            match fallible_future.await {
                Ok(optional_msg) => match optional_msg {
                                        Some(msg) => Some(Ok(msg)),
                                        None => None,
                                    },
                Err(err)         => Some(Err(err)),
            }
        })
}


/// Returns a tied-together `(stream, producer, closer)` tuple which [socket_server] uses to transform [ClientMessages] into [ServerMessages].\
/// The tuple consists of:
///   - The `Stream` of (`Endpoint`, [ServerMessages]) -- [socket_server] will, then, apply operations at the end of it to deliver the messages
///   - The producer to send `SocketEvent<ClientMessages>` to that stream
///   - The closer of the stream\
/// Usage example:
/// ```no_compile
///     futures::executor::block_on(async {
///         state.detached_sender.produce((endpoint, ServerMessages::UnknownMessage("Just wanted to tell you to go sleep".to_string()))).await;
///    });
pub async fn sync_processors(runtime: &RwLock<Runtime>,
                             socket_server: &SocketServer<'static>) -> (impl Stream<Item = Result < (Arc<Peer<ServerMessages, DisconnectionReason>>, ServerMessages),
                                                                                                    (Arc<Peer<ServerMessages, DisconnectionReason>>, Box<dyn std::error::Error + Sync + Send>) > >,
                                                                        impl Fn(SocketEvent<ClientMessages, ServerMessages, DisconnectionReason>) -> bool,
                                                                        impl Fn()) {
    let tokio_runtime = Arc::clone(runtime.read().await.tokio_runtime.as_ref().unwrap());
    let (stream, producer, closer) = super::executor::sync_tokio_stream(tokio_runtime);
    let dispatcher = Runtime::do_for_ogre_robot(runtime, |ogre_robot| Box::pin(async {Arc::clone(&ogre_robot.dispatcher)})).await;
    (processor(dispatcher, stream), producer, closer)
}

/// see [super::executor::spawn_concurrent_stream_executor()]
pub async fn spawn_stream_executor(stream: impl Stream<Item = (Arc<Peer<ServerMessages, DisconnectionReason>>, bool)> + Send + Sync + 'static) -> tokio::task::JoinHandle<()> {
    super::executor::spawn_stream_executor(stream).await
}

#[derive(Debug)]
enum TimeTrackedInfo<InfoType: Debug> {
    Unset,
    Set { time: Instant,  info: InfoType },
}

impl<InfoType: Debug> TimeTrackedInfo<InfoType> {

    pub fn new() -> Self {
        Self::Unset
    }

    /// Allows setting a value once, keeping track of the moment it was set:
    /// consumes `info` if the previous value was `Unset` -- in which case it will now be `Set` and the function will return `Ok(&info)`;\
    /// otherwise, returns `info` back to the caller as `Err(info)`.\
    /// See [reset()] if your logic is intended to set the value multiple times
    pub fn set(&mut self, info: InfoType) -> Result<&InfoType, InfoType> {
        match self {
            TimeTrackedInfo::Unset => Ok(self.reset(info)),
            TimeTrackedInfo::Set { .. } => Err(info)
        }
    }

    /// Allows setting a value multiple times, keeping track of the moment it was set:\
    /// Returns a reference to `info`.\
    /// See [set()] if your logic is supposed to set the information only once
    pub fn reset(&mut self, info: InfoType) -> &InfoType {
        *self = Self::Set {
            time: Instant::now(),
            info
        };
        match *self {
            TimeTrackedInfo::Unset => panic!("BUG! Attempt to Deref a `TimeTrackedInfo` that is still `Unset`. Please, fix your code."),
            TimeTrackedInfo::Set { time: _time, ref info } => info,
        }
    }


}
impl<InfoType: Debug> Deref for TimeTrackedInfo<InfoType> {
    type Target = InfoType;
    fn deref(&self) -> &Self::Target {
        match self {
            TimeTrackedInfo::Unset => panic!("BUG! Attempt to Deref a `TimeTrackedInfo` that is still `Unset`. Please, fix your code."),
            TimeTrackedInfo::Set { time: _time, ref info } => info,
        }
    }
}
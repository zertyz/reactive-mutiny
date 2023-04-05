//! Defines the messages clients and server may exchange through a socket (either textual or binary),
//! as well as serializers & deserializers

pub use crate::logic::ogre_robot::types::*;
use std::error::Error;
use std::fmt::Write;
use std::future::Future;
use std::sync::Arc;
use futures::future::BoxFuture;
use serde::{Serialize, Deserialize};
use crate::frontend::socket_server::serde::{ron_deserializer, ron_serializer, SocketServerDeserializer, SocketServerSerializer};
use crate::frontend::socket_server::SocketServer;
use crate::frontend::socket_server::tokio_message_io::Peer;


/// Messages coming from the clients, suitable to be deserialized by this server
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum ClientMessages {

    /// Upon connection, this is the first message clients must send -- in order for the server to classify their hole in the system
    /// and determine if logging in is necessary
    ClientIdentification(ClientIdentification),

    /// If the server sent [ServerMessage::ProvideAuthorizationToContinue], client must send an authorization token to continue
    /// -- possibly a double factor challenge sent by Telegram
    UserAuthorization(String),

    /// Asks the server to return, as soon as possible, the given number + 1 with a [ServerMessages::KeepAliveAnswer]
    /// -- may be used to measure round trip times, as well as to check the peer's liveliness
    KeepAliveRequest(u32),

    /// Similar to [ClientMessages::KeepAliveRequest], but sent in response to [ServerMessages::KeepAliveRequest]
    KeepAliveAnswer(u32),

    /// Client updates info about negotiatable symbols
    MarketData(ClientMarketData),

    /// A [ClientIdentification::FullAdvisor] client informs the server that one of his orders was executed
    /// -- an independent [ClientMessages::Trade] event is likely to be also reported
    ExecutedOrder {
        /// in the form YYYYMMDD
        date: u32,
        /// in the form HHMMSSMMM
        time: u32,
        /// the symbol, for double checking -- must match the one stated when identifying the client
        symbol: String,
        /// the unitary paper currency value multiplied by 1000 -- or cent value multiplied by 10
        unitary_mill_value: u32,
        /// how many papers of that symbol were traded
        quantity: u32,
        /// if true, means the order is not totally filled -- it remains active within the Exchange
        /// until it is fully executed
        partial: bool,
        /// the order ID, as issued by the Exchange
        order_id: u32,
        /// the order ID, if added by the server
        ogre_id: u32,
    },

    /// A [ClientIdentification::FullAdvisor] client informs the server that his previous scheduled order
    /// has been cancelled
    CancelledOrder {
        ogre_id: u32,
        reason: ClientOrderCancellationReasons,
    },

    /// Client informs of an order scheduled for execution (but still not executed)
    /// -- either because the user just manually added it
    /// or because the server requested a list of orders awaiting execution
    /// TODO this should be an array -- so the server may know of orders it lost track cancelling
    PendingOrder {
        ogre_id:     u32,
        exchange_id: u32,
        // order_data...
    },

    /// A [ClientIdentification::FullAdvisor] client asks for any new drawable events (after `sequential`) to be sent back
    ChartPoints { sequential: u32 },

    /// Tells the server we are disconnecting on purpose -- and that it should not expect to see this client trying to reconnect.\
    /// If the disconnection has been done in the middle of a transaction, warnings & alerms should be raised.\
    /// The `String` param contains a textual explanation for the disconnection.
    GoodBye(String),

    /// Tells the server that it has sent a message the client does not understand
    UnknownMessage(String),
}

/// Messages generated by this server, suitable to be serialized here
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerMessages {

    /// Server greeting, sent as soon as the connection with the client is accepted.\
    /// From here, client must send [ClientMessages::ClientAnnounce]
    Welcome,

    /// Depending on the [ClientIdentification], the server may require [UserAuthorization] to continue.\
    /// Upon receiving this, client must answer with [ClientMessages::UserAuthorization]
    ProvideAuthorizationToContinue,

    /// If something goes wrong, server may decide to drop client -- wrong/missing login, wrong protocol,
    /// server being shutdown, ...
    Disconnected(DisconnectionReason),

    /// Asks the client to return, as soon as possible, the given number + 1 with a [ClientMessages::KeepAliveAnswer]
    /// -- may be used to measure round trip times, as well as to check the peer's liveliness.\
    /// If a [ClientIdentification::MarketDataBridge] times out responding, its asset's symbol is
    /// marked as `stale` and precautions are taken to minimize risks -- no new buying orders (cancelling un-executed ones),
    /// sound an alarm if there are open positions, sell all open positions at market price, etc.
    KeepAliveRequest(u32),

    /// Similar to [ServerMessages::KeepAliveRequest], but sent in response to [ClientMessages::KeepAliveRequest]
    KeepAliveAnswer(u32),

    /// Asks the client to execute an order within the Exchange.\
    /// The client is expected not to answer to this message if all is fine,
    /// however, [ClientMessages::CancelledOrder] may happen if scheduling
    /// was not possible.
    ScheduleOrder(OrderCommand),

    /// Asks the client to cancel an order within the Exchange
    CancelOrder {
        ogre_id: u32,
        reason: OrderCancellationReasons,
    },

    /// Asks the client to report all its scheduled orders -- either added by the server or not
    StateOpenPositions,

    /// Server informs drawable events, as asked by [ClientMessages::ChartPoints]
    ChartPoints {

    },

    /// Common messages to all protocols
    /// ////////////////////////////////

    /// If the processor answers with this message, nothing will be sent back to the client
    None,

    /// Whenever the server don't understand a message, this will be answered, along with the
    /// received message
    UnknownMessage(String),

    /// If the server cannot immediately process the message, or if its queue is full, this will be
    /// answered and the message from the client will be dropped -- clients are advised to try
    /// again, if the deadline didn't come yet
    TooBusy,

    /// If the processor results in `Err`, this will be sent along with the error description
    ProcessorError(String),

    /// Server sends this to connected clients once it has decided it is time to quit
    ShuttingDown,
}

/// Market data, as informed by the client -- with easy to generate info (when compared to the
/// heavily optimized internal versions of the same data that we use).\
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum ClientMarketData {

    /// Sent by a [ClientIdentification::MarketDataBridge] client to update the server with the symbol information
    /// -- updated after connection, sporadically or when an information change happens.\
    /// See [SymbolState] for the internal version of the same info, after being treated
    SymbolState {
        symbol: String,
        in_auction: bool,
    },

    /// Client informs of a book event -- either a new entry or an update
    /// -- see [SingleBook] for the internal version of the same info, after being treated
    Book {
        /// in the form YYYYMMDD
        date: u32,
        /// in the form HHMMSSMMM
        time: u32,
        /// the symbol, for double checking -- must match the one stated when identifying the client
        symbol: String,
        /// the price
        price_level_mills: u32,
        /// the number of orders waiting
        n_orders: u32,
        /// the total quantity of booked orders
        available_quantity: u32,
        /// the operation those orders want to make
        side: Parties,
    },

    /// a [ClientIdentification::MarketDataBridge] or even [ClientIdentification::FullAdvisor] informs the server that an external trade happened
    /// -- see [SingleTrade] for the internal version of the same info, after being treated
    Trade {
        /// in the form YYYYMMDD
        date: u32,
        /// in the form HHMMSSMMM
        time: u32,
        /// the symbol, for double checking -- must match the one stated when identifying the client
        symbol: String,
        /// the unitary paper currency value multiplied by 1000 -- or cent value multiplied by 10
        unitary_mill_value: u32,
        /// how many papers of that symbol were traded
        quantity: u32,
        /// who emitted the Market Order?
        aggressor: Parties,
    },
}
impl Into<MarketData> for ClientMarketData {
    fn into(self) -> MarketData {
        match self {
            ClientMarketData::SymbolState { symbol, in_auction } => MarketData::SymbolState(SymbolState { symbol, in_auction }),
            ClientMarketData::Book { .. } => todo!("Please, go ahead and develop the conversion from Client Book Into Book"),
            ClientMarketData::Trade { .. } => todo!("Please, go ahead and develop the conversion from Client Trade Into Trade"),
        }
    }
}

/// Reasons for the client to have aborted executing one of orders the server had scheduled for execution
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum ClientOrderCancellationReasons {
    /// The user -- a human operator -- had, for some reason, manually cancelled the order
    UserInitiated,
    /// The broker didn't respond in time if the order was accepted or not.\
    /// Real status is unknown and the server should inquire about the pending orders and either
    /// cancel it or schedule it again.
    TimeoutWhileScheduling { elapsed_nanos: u32 },
    /// The broker didn't accept the order, after all
    BrokerInitiated { message: String },
}

/// Allows [super::tokio_message_io::text_protocol_network_loop()] to send [ServerMessages] (to a client) using `RON` textual messages
impl SocketServerSerializer<ServerMessages, DisconnectionReason> for ServerMessages {
    fn ss_serialize(message: &ServerMessages) -> String {
        ron_serializer(message)
    }
    fn send_unknown_input_error(peer: &Arc<Peer<ServerMessages, DisconnectionReason>>, raw_input: String, err: Box<dyn std::error::Error + Send + Sync>) -> BoxFuture<'_, ()> {
        Box::pin(peer.sender.send(ServerMessages::UnknownMessage(format!("SocketServer::protocol: client input '{}' wasn't recognized as one of the possible `ServerMessages`: {}", raw_input, err))))
    }
    fn is_disconnect_message(processor_answer: &ServerMessages) -> Option<&DisconnectionReason> {
        if let ServerMessages::Disconnected(reason) = processor_answer {
            Some(reason)
        } else {
            None
        }
    }
}

/// Allows [super::tokio_message_io::text_protocol_network_loop()] to receive [ClientMessage] (from a client) in the `RON` textual format
impl SocketServerDeserializer<ClientMessages> for ClientMessages {
    fn ss_deserialize(message: &[u8]) -> Result<ClientMessages, Box<dyn std::error::Error + Sync + Send>> {
        ron_deserializer(message)
    }
}


/// Allows [super::tokio_message_io::text_protocol_network_loop()] to send [ClientMessages] (to a server) using `RON` textual messages
/// (used to allow testing our Socket Server, but implemented here for symmetry)
impl SocketServerSerializer<ClientMessages, String> for ClientMessages {
    fn ss_serialize(message: &ClientMessages) -> String {
        ron_serializer(message)
    }
    fn send_unknown_input_error(peer: &Arc<Peer<ClientMessages, String>>, raw_input: String, err: Box<dyn std::error::Error + Send + Sync>) -> BoxFuture<'_, ()> {
        Box::pin(peer.sender.send(ClientMessages::UnknownMessage(format!("SocketServer::protocol: server input '{}' wasn't recognized as one of the possible `ClientMessages`: {}", raw_input, err))))
    }
    fn is_disconnect_message(processor_answer: &ClientMessages) -> Option<&String> {
        if let ClientMessages::GoodBye(reason) = processor_answer {
            Some(reason)
        } else {
            None
        }
    }
}

/// Allows [super::tokio_message_io::text_protocol_network_loop()] to receive [ClientMessage] (from a client) in the `RON` textual format
/// (used to allow testing our Socket Server, but implemented here for symmetry)
impl SocketServerDeserializer<ServerMessages> for ServerMessages {
    fn ss_deserialize(message: &[u8]) -> Result<ServerMessages, Box<dyn std::error::Error + Sync + Send>> {
        ron_deserializer(message)
    }
}


/// Unit tests the [protocol](self) module
#[cfg(any(test, feature = "dox"))]
mod tests {
    use super::*;


    /// assures serialization / deserialization works for all client messages
    #[test]
    fn serde_for_client_messages() {
        // keep this in sync with all available ClientMessages variants, in the order they are declared there
        let client_messages = vec![
            ClientMessages::ClientIdentification(ClientIdentification::FullAdvisor      { version: format!("v.1.2.3"), symbol: format!("PETR3"), account_token: format!("AkD9jH7BcgH68Js7") }),
            ClientMessages::ClientIdentification(ClientIdentification::MarketDataBridge { version: format!("v.1.2.3"), symbol: format!("PETR3"), account_token: format!("AkD9jH7BcgH68Js7") }),
            ClientMessages::ClientIdentification(ClientIdentification::WatcherAdvisor   { version: format!("v.1.2.3"), symbol: format!("PETR3"), account_token: format!("AkD9jH7BcgH68Js7") }),
            ClientMessages::UserAuthorization(format!("PaSsD321")),
            ClientMessages::KeepAliveRequest(1),
            ClientMessages::KeepAliveAnswer(2),
            ClientMessages::MarketData(ClientMarketData::SymbolState { symbol: format!("PETR3"), in_auction: false }),
            ClientMessages::MarketData(ClientMarketData::Book        { date: 22011979, time: 213214001, symbol: format!("PETR3"), price_level_mills: 32120, n_orders: 100, available_quantity: 1000, side: Parties::Buyer }),
            ClientMessages::MarketData(ClientMarketData::Trade       { date: 22011979, time: 213214001, symbol: format!("PETR3"), unitary_mill_value: 32120, quantity: 100, aggressor: Parties::Buyer }),
            ClientMessages::ExecutedOrder { date: 22011979, time: 213214001, symbol: format!("PETR3"), unitary_mill_value: 32120, quantity: 100, partial: false, order_id: 1, ogre_id: 1 },
            ClientMessages::CancelledOrder { ogre_id: 1, reason: ClientOrderCancellationReasons::UserInitiated},
            ClientMessages::CancelledOrder { ogre_id: 1, reason: ClientOrderCancellationReasons::TimeoutWhileScheduling { elapsed_nanos: 1234567890 } },
            ClientMessages::CancelledOrder { ogre_id: 1, reason: ClientOrderCancellationReasons::BrokerInitiated        { message: format!("You didn't provide enough warranties for that operation") } },
            ClientMessages::PendingOrder { ogre_id: 1, exchange_id: 1 },
            ClientMessages::ChartPoints { sequential: 1 },
            ClientMessages::GoodBye(format!("done for today! the sea has awesome waves! time for body surfing!")),
            ClientMessages::UnknownMessage(format!("Not sure where this is used...")),
        ];
        for message in client_messages {
            let serialized = ClientMessages::ss_serialize(&message);
            let reconstructed = ClientMessages::ss_deserialize(serialized.as_bytes())
                .expect(&format!("deserialization failed for input '{}'", serialized));
            assert_eq!(reconstructed, message, "a client message couldn't resist serde. It was serialized to '{}'", serialized);
            println!("✓ {}", serialized.trim_end());
        }
    }

    /// assures serialization / deserialization works for all server messages
    #[test]
    fn serde_for_server_messages() {
        // keep this in sync with all available ServerMessages variants, in the order they are declared there
        let server_messages = vec![
            ServerMessages::Welcome,
            ServerMessages::ProvideAuthorizationToContinue,
            ServerMessages::Disconnected(DisconnectionReason::UnknownClientType),
            ServerMessages::Disconnected(DisconnectionReason::DeprecatedClientVersion {minimum_accepted_version: format!("v.1.2.3")}),
            ServerMessages::Disconnected(DisconnectionReason::UnknownAccount),
            ServerMessages::Disconnected(DisconnectionReason::AccountFrozen           {message: format!("too many wrong authorization attempts"), remaining_duration_nanos: 1234567890}),
            ServerMessages::Disconnected(DisconnectionReason::AccountDisabled         {message: format!("to enable it back, ask Luiz for a new password")}),
            ServerMessages::Disconnected(DisconnectionReason::AuthenticationFailure),
            ServerMessages::Disconnected(DisconnectionReason::ProtocolOffense         {message: format!("for instance... trying to provide two client identifications...")}),
            ServerMessages::Disconnected(DisconnectionReason::Reconnected             {ip:      format!("127.0.0.1")}),
            ServerMessages::Disconnected(DisconnectionReason::ClientInitiated),
            ServerMessages::Disconnected(DisconnectionReason::RiskManager(RiskManagementConnectionDroppingConditions::RoundTripTimeTooHigh { nanos: 1234 })),
            ServerMessages::Disconnected(DisconnectionReason::RiskManager(RiskManagementConnectionDroppingConditions::PingTimeout)),
            ServerMessages::Disconnected(DisconnectionReason::RiskManager(RiskManagementConnectionDroppingConditions::ClockSkewTooHigh { estimated_delta_nanos: 1234 })),
            ServerMessages::KeepAliveRequest(1),
            ServerMessages::KeepAliveAnswer(2),
            ServerMessages::ScheduleOrder(OrderCommand::Buy  (Order { ogre_id: 1, aggressor: Parties::Buyer,  order_type: OrderTypes::MarketOrder, date: 22011979, time: 213214001, symbol: format!("PETR3"), unitary_mill_value: 32120, quantity: 100 })),
            ServerMessages::ScheduleOrder(OrderCommand::Sell (Order { ogre_id: 1, aggressor: Parties::Seller, order_type: OrderTypes::MarketOrder, date: 22011979, time: 213214001, symbol: format!("PETR3"), unitary_mill_value: 32120, quantity: 100 })),
            ServerMessages::CancelOrder { ogre_id: 1, reason: OrderCancellationReasons::ClientInitiated        { message: format!("Oops") } },
            ServerMessages::CancelOrder { ogre_id: 1, reason: OrderCancellationReasons::TimeoutWhileScheduling { elapsed_nanos: 1234567890 } },
            ServerMessages::StateOpenPositions,
            ServerMessages::ChartPoints {},
            ServerMessages::None,
            ServerMessages::UnknownMessage(format!("Client, you've sent something I don't understand")),
            ServerMessages::TooBusy,
            ServerMessages::ProcessorError(format!("Client, something went wrong when I was calculating your answer... efforts dropped")),
            ServerMessages::ShuttingDown,
        ];
        for message in server_messages {
            let serialized = ServerMessages::ss_serialize(&message);
            let reconstructed = ServerMessages::ss_deserialize(serialized.as_bytes())
                .expect(&format!("deserialization failed for input '{}'", serialized));
            assert_eq!(reconstructed, message, "a server message couldn't resist serde. It was serialized to '{}'", serialized);
            println!("✓ {}", serialized.trim_end());
        }
    }

}
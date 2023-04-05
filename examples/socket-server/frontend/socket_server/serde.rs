//! SERializers & DEserializers (traits & implementations) for our [SocketServer]


use crate::frontend::socket_server::{
    protocol::*,
    tokio_message_io::Peer,
};
use std::{
    sync::Arc,
    fmt::Write,
};
use std::fmt::Debug;
use futures::future::BoxFuture;
use ron::{
    Options,
    ser::PrettyConfig,
};
use serde::{Serialize, Deserialize};
use lazy_static::lazy_static;


/// Trait that should be implemented by enums that model the "local messages" to be handled by the [SocketServer] --
/// "local messages" are, typically, server messages (see [ServerMessages]) -- but may also be client messages if we're building a client (for tests?)\
/// This trait, therefore, specifies how to:
///   * `serialize()` enum variants into a String (like RON, for textual protocols) to be sent to the remote peer
///   * inform the peer if any wrong input was sent
///   * identify local messages that should cause a disconnection
pub trait SocketServerSerializer<LocalPeerMessages, LocalPeerDisconnectionReason>
    where LocalPeerMessages:            Send + Debug + SocketServerSerializer<LocalPeerMessages, LocalPeerDisconnectionReason>,
          LocalPeerDisconnectionReason: Send {
    /// `SocketServer`s serializer: transforms a strong typed `message` into a `String`
    fn ss_serialize(message: &LocalPeerMessages) -> String;
    /// Called whenever the socket server can't serialize the input it receives from the socket -- sends back an answer stating that their given input was wrong
    fn send_unknown_input_error(peer: &Arc<Peer<LocalPeerMessages, LocalPeerDisconnectionReason>>, raw_input: String, err: Box<dyn std::error::Error + Send + Sync>) -> BoxFuture<'_, ()>;
    /// Informs if the given internal `processor_answer` is a "disconnect" message (usually issued by the messages processor)
    /// -- which the socket server must enforce by sending and, immediately, closing the connection
    fn is_disconnect_message(processor_answer: &LocalPeerMessages) -> Option<&LocalPeerDisconnectionReason>;
}

/// Trait that should be implemented by enums that model the "remote messages" to be handled by the [SocketServer] --
/// "remote messages" are, typically, client messages (see [ClientMessages]) -- but may also be server messages if we're building a client (for tests?)\
/// This trait, therefore, specifies how to:
///   * `deserialize()` enum variants received by the remote peer (like RON, for textual protocols)
pub trait SocketServerDeserializer<T> {
    /// `SocketServer`s deserializer: transform a textual `message` into a string typed value
    fn ss_deserialize(message: &[u8]) -> Result<T, Box<dyn std::error::Error + Sync + Send>>;
}


// RON SERDE
////////////

lazy_static! {

    static ref RON_EXTENSIONS: ron::extensions::Extensions = {
        let mut extensions = ron::extensions::Extensions::empty();
        extensions.insert(ron::extensions::Extensions::IMPLICIT_SOME);
        extensions.insert(ron::extensions::Extensions::UNWRAP_NEWTYPES);
        extensions.insert(ron::extensions::Extensions::UNWRAP_VARIANT_NEWTYPES);
        extensions
    };

    static ref RON_SERIALIZER_CONFIG: PrettyConfig = ron::ser::PrettyConfig::new()
        .depth_limit(10)
        .new_line(String::from(""))
        .indentor(String::from(""))
        .separate_tuple_members(true)
        .enumerate_arrays(false)
        .extensions(*RON_EXTENSIONS);

    static ref RON_DESERIALIZER_CONFIG: Options = ron::Options::default()
        ;//.with_default_extension(*RON_EXTENSIONS);

}

/// RON serializer
#[inline(always)]
pub fn ron_serializer<T: Serialize>(message: &T) -> String {
    let mut output_data = ron::ser::to_string(message).unwrap();
    write!(output_data, "\n").unwrap();
    output_data
}

/// RON deserializer
#[inline(always)]
pub fn ron_deserializer<T: for<'a> Deserialize<'a>>(message: &[u8]) -> Result<T, Box<dyn std::error::Error + Sync + Send>> {
    RON_DESERIALIZER_CONFIG.from_bytes(message)
        .map_err(|err| Box::from(format!("RON deserialization error for message '{:?}': {}", std::str::from_utf8(message), err)))
}


/// Unit tests for our socket server [serde](self) module
#[cfg(any(test, feature = "dox"))]
mod tests {
    use super::*;


    /// assures RON serialization / deserialization works for all client / server messages
    #[test]
    fn ron_serde_for_server_only() {
        let message = ServerMessages::UnknownMessage(String::from("This is an error message"));
        let expected = "UnknownMessage(\"This is an error message\")\n";
        let observed = ron_serializer(&message);
        assert_eq!(observed, expected, "RON serialization is not good");

        let message = "ClientIdentification(FullAdvisor(version:\"1a\",symbol:\"PETR3\",account_token:\"abc\"))".as_bytes();
        let expected = ClientMessages::ClientIdentification(ClientIdentification::FullAdvisor { version: "1a".to_string(), symbol: "PETR3".to_string(), account_token: "abc".to_string() });
        let observed = ron_deserializer::<ClientMessages>(message)
            .expect("RON deserialization failed");
        assert_eq!(observed, expected, "RON deserialization is not good");
    }
}
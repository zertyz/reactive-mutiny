//! Demonstrates an hypothetical microservice queue processor using a [Uni]

use reactive_mutiny::{
    uni::{Uni, UniBuilder},
    MutinyStream,
};
use std::{
    sync::Arc,
    time::Duration,
};
use std::sync::atomic::AtomicU32;
use futures::{SinkExt, Stream, StreamExt};


/// Queue consumer function, set to receive elements from "Queue A" and send responses to "Queue B" -- both in JSON format.
fn consume_and_answer(incoming_json_events_stream: MutinyStream<String>) -> impl Stream<Item=String> {
    process( incoming_json_events_stream
                  .map(|json_event| deserialize(json_event)) )
        .map(|output_event| serialize(output_event))

    // or, in the expanded form:
    // let deserialized_events_stream = incoming_json_events_stream
    //     .map(|json_event| deserialize(json_event));
    // let processed_events_stream = process(deserialized_events_stream);
    // let outgoing_json_events_stream = processed_events_stream
    //     .map(|output_event| serialize(output_event));
    // outgoing_json_events_stream

}

fn process(incoming_events_stream: impl Stream<Item=IncomingEventType>) -> impl Stream<Item=OutgoingEventType> {
    let mut state = 0;
    incoming_events_stream.map(move |incoming_event| {
        state += 1;     // simple demonstration that we're able to hold a state
        // logic goes here to transform the `incoming_event` into an `outgoing_event`.
        // notice that additional Mutiny events might be created here, if the logic is too complex
        OutgoingEventType {}
    })
}

#[tokio::main]
async fn main() {
    // somewhere else in the application (when it starts), the Uni is created like this
    // -- notice that it is here that that the answers are tied to "Queue B", but it could be anywhere else:
    let queue_a_events_processor: Arc<Uni<String, 1024, 1>> = UniBuilder::new()
        .on_stream_close(|_| async {})
        .spawn_non_futures_non_fallible_executor("Consumer for Queue A / producer for Queue B",
                                                 |stream| {
                                                     consume_and_answer(stream)
                                                         .inspect(|outgoing_json_event| queue_b_send(outgoing_json_event))
                                                 });

    // internally, when messages arrive in "Queue A", events are produced to the `Stream` like this
    let msg = format!("Incoming Json Message");
    println!("Queue A: sending '{}'", msg);
    queue_a_events_processor.try_send(msg);

    // when the app is to shutdown:
    queue_a_events_processor.close(Duration::from_secs(10)).await;
}

struct IncomingEventType {}
struct OutgoingEventType {}
fn queue_b_send(msg: &str) {
    println!("Queue B: sending '{}'", msg);
}
fn deserialize(json: String) -> IncomingEventType {
    IncomingEventType {}
}
fn serialize(_value: OutgoingEventType) -> String {
    format!("Serialized Json Message")
}
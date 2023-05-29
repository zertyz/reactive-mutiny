//! Demonstrates an hypothetical microservice queue processor using a [Uni] to enable "reactive programming".
//!
//! Here, an "event processing pipeline" is setup to "propagate the change" of an incoming message to an outgoing one,
//! using `Stream`s (instead of callbacks) -- AKA, reactive:
//!   1) Easy decoupling: Producers are decouple from Consumers and the "event processing pipeline" may be incremented by other modules, easing decoupled architectures, like microservices.
//!   2) Answers are optional, so the event may be modeled to cover both synchronous and asynchronous messaging patterns
//!   3) No matter how the pipeline is composed -- in a single function or spread through several modules: `reactive-mutiny`
//!      allows the compiler to see it in its entirety, so Release builds may do full optimizations.
//!
//! In this example, the input is `ExchangeEvent`, which is transformed into `AnalysisEvent` in the `process()` logic.

#[path = "../common/mod.rs"] mod common;

use common::*;
use reactive_mutiny::prelude::*;
use std::{
    time::Duration,
    future,
};
use futures::{Stream, StreamExt};


/// Simple reactive logic, holding a state.\
/// Here the input events are transformed into the output events -- single-threaded.\
/// See [AnalysisEvent] for more info.
fn process(exchange_events_stream: impl Stream<Item=ExchangeEvent>) -> impl Stream<Item=AnalysisEvent> {

    #[derive(Debug)]
    struct State {
        base_value:     f64,
        last_value:     f64,
        last_direction: Direction,
    }
    #[derive(Debug,PartialEq)]
    enum Direction {
        Up,
        Down,
        Stale,
    }
    let mut state_store = None;

    // The following expression generates the output stream.
    // We only care for the `TradeEvent` variant -- all other input events won't generate an "answer"
    exchange_events_stream.filter_map(move |incoming_event| future::ready({
        match incoming_event {
            ExchangeEvent::TradeEvent { unitary_value, .. } => {
                let state = state_store.get_or_insert_with(|| State {
                    base_value:     unitary_value,
                    last_value:     unitary_value,
                    last_direction: Direction::Stale,
                });
                let current_direction = if unitary_value > state.last_value {
                    Direction::Up
                } else if unitary_value < state.last_value {
                    Direction::Down
                } else {
                    Direction::Stale
                };
                let result = if current_direction == Direction::Stale {
                    None
                } else if current_direction == state.last_direction {
                    Some(AnalysisEvent { price_delta: unitary_value - state.base_value })
                } else if state.last_direction == Direction::Stale {
                    state.last_value = unitary_value;
                    state.last_direction = current_direction;
                    Some(AnalysisEvent { price_delta: unitary_value - state.base_value })
                } else {
                    state.base_value = unitary_value;
                    state.last_direction = current_direction;
                    None
                };
                state.last_value = unitary_value;
                result
            },
            _ => None,
        }
    }))

}

#[tokio::main]
async fn main() {

    // Somewhere, when the application starts, the Uni's event processing pipeline should be created
    // -- notice that it is here that that the answers are tied to "Queue B", but it could be anywhere else:
    let queue_a_events_handle = UniMove::<ExchangeEvent, 1024, 1>::new()
        .spawn_non_futures_non_fallible_executor("Consumer of binary `ExchangeEvent`s @ Queue A / producer of binary `AnalysisEvent`s @ Queue B",
                                                 |exchange_events| {
                                                     process(exchange_events)
                                                         .inspect(|outgoing_event| queue_b_send(outgoing_event))
                                                 },
                                                 |_| async {});

    // demonstration
    ////////////////

    // The application's queue client will propagate the events like this
    let queue_a_send = |incoming: ExchangeEvent| {
        println!("Queue A: received '{:?}'", incoming);
        queue_a_events_handle.try_send(|slot| *slot = incoming);
    };

    // simulates some events were received
    queue_a_send(ExchangeEvent::BookTopEvent { best_bid: 9.0, best_ask: 11.0 });
    queue_a_send(ExchangeEvent::TradeEvent { unitary_value: 9.12, quantity: 100, time: 10 });
    queue_a_send(ExchangeEvent::TradeEvent { unitary_value: 9.13, quantity: 100, time: 20 });     // delta: +0.01
    queue_a_send(ExchangeEvent::TradeEvent { unitary_value: 9.14, quantity: 100, time: 30 });     // delta: +0.02
    queue_a_send(ExchangeEvent::TradeEvent { unitary_value: 9.13, quantity: 100, time: 40 });
    queue_a_send(ExchangeEvent::TradeEvent { unitary_value: 9.13, quantity: 100, time: 50 });
    queue_a_send(ExchangeEvent::TradeEvent { unitary_value: 9.14, quantity: 100, time: 60 });
    queue_a_send(ExchangeEvent::TradeEvent { unitary_value: 9.13, quantity: 100, time: 70 });
    queue_a_send(ExchangeEvent::TradeEvent { unitary_value: 9.12, quantity: 100, time: 80 });     // delta: -0.01

    // when the app is to shutdown, kills the executor & closes the channel:
    let _ = queue_a_events_handle.close(Duration::from_secs(10)).await;
}

/// If the queue is to receive JSON textual data (and send JSONs as well), this method may be used:
/// ```nocompile
///     let queue_a_events_processor: Arc<Uni<String, 1024, 1>> = UniBuilder::new()
///         .on_stream_close(|_| async {})
///         .spawn_non_futures_non_fallible_executor("Consumer of json `ExchangeEvent`s @ Queue A / producer of json `AnalysisEvent`s @ Queue B",
///                                                  |exchange_events| json_process(exchange_events)
///                                                      .inspect(|outgoing_json_event| queue_b_send_json(outgoing_json_event)) );
fn _json_process(json_exchange_events_stream: impl Stream<Item=String>) -> impl Stream<Item=String> {
    process(json_exchange_events_stream
                .map(|json_event| _deserialize(json_event)) )
        .map(|output_binary_event| _serialize(output_binary_event))
}

// mocks
////////

fn queue_b_send(analysis_event: &AnalysisEvent) {
    println!("Queue B: sending {:?}", analysis_event);
}
fn _deserialize(_json: String) -> ExchangeEvent {
    ExchangeEvent::Ignored
}
fn _serialize(_value: AnalysisEvent) -> String {
    format!("Serialized Json Message")
}

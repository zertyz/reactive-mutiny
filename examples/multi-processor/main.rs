//! Demonstrates how to work with a [Multi] when "reactive programming" has been enabled by `reactive-mutiny`.
//!
//! This example is an extension of `uni-microservice`:
//! The main events are of types [ExchangeEvent] & [AnalysisEvents] -- which could be handled by a `Uni`.
//! For the sake of simplicity, the `Uni` part has been omitted here, but is present at the `uni-microservice` example.
//!
//! The focused aspects here are the "secondary events", which are [Multi]s: they are generated as part of processing the incoming events,
//! simulating an application with a more complex event processing logic -- in our case, the secondary events are "trading orders" sent to the Exchange.
//!
//! Note that those [Multi] events may have as many listeners as the application wants. All of them will receive all generated events
//! and listeners may subscribe / unsubscribe at any time -- these are the main aspect distinguishing a `Multi` from a `Uni`.
//! In this example, we have:
//!   - a listener to process the orders -- "sending them to the Exchange"
//!   - a logger
//!
//! Additionally, more listeners might be added -- programmatically:
//!   - an accountant, keeping track of the profits & losses
//!   - a monitor, keeping track of any bugs (emitting two consecutive Buy or Sell orders, for instance)

#[path = "../common/mod.rs"] mod common;

use common::*;
use reactive_mutiny::{
    multi::{MultiStreamType,Multi},
    stream_executor::StreamExecutor,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU32},
        mpsc::RecvTimeoutError::Timeout,
    },
    time::Duration,
    fmt::Debug,
    future,
};
use futures::{SinkExt, Stream, stream, StreamExt, TryStreamExt};

/// Represents a Market Order to be sent to the Exchange
#[derive(Debug)]
enum OrderEvent {
    Buy(Order),
    Sell(Order),
}

/// Core data for Market Orders
#[derive(Debug)]
struct Order {
    quantity: u32,
}

const BUFFER_SIZE: usize = 1024;
const MAX_STREAMS: usize = 16;

/// The processor of [AnalysisEvent]s, generating [Order] events for our [Multi]
struct DecisionMaker {
    /// the handler for our [Multi] events
    orders_event_handler: Multi<'static, OrderEvent, BUFFER_SIZE, MAX_STREAMS>,
}

impl DecisionMaker {

    pub fn new() -> Self {
        Self {
            orders_event_handler: Multi::new("Order events Handler"),
        }
    }

    pub async fn add_listener<IntoString:             Into<String>,
                              OutItemType:            Send + Debug,
                              OutStreamType:          Stream<Item=OutItemType> + Send + 'static>
                             (&self,
                              listener_name:    IntoString,
                              pipeline_builder: impl FnOnce(MultiStreamType<'static, OrderEvent, BUFFER_SIZE, MAX_STREAMS>) -> OutStreamType)
                             -> Result<(), Box<dyn std::error::Error>> {
        self.orders_event_handler.spawn_non_futures_non_fallible_executor_ref(1, format!("`OrderEvent`s listener '{}'", listener_name.into()), pipeline_builder, |_| async {}).await
            .map_err(|err| Box::from(format!("Error adding an `OrderEvent`s listener to the `DecisionMaker`: {:?}", err)))
            .map(|_| ())
    }

    /// The main logic -- a continuation to what we have in `uni-microservice`:\
    /// processes [AnalysisEvent]s (without an answer), generating [OrderEvent] events in the process
    fn decider<'a>(&'a mut self, analysis_events_stream: impl Stream<Item=AnalysisEvent> + 'a) -> impl Stream<Item=()> + 'a {
        let mut positions = 0;
        analysis_events_stream.map(move |analysis| {
            let order = if positions == 0 && analysis.price_delta > 0.00 {
                let quantity = 100;
                positions += quantity;
                Some(OrderEvent::Buy(Order{quantity: quantity}))
            } else if positions > 0 && analysis.price_delta < 0.00 {
                let quantity = positions;
                positions = 0;
                Some(OrderEvent::Sell(Order{quantity: quantity}))
            } else {
                None
            };
            if let Some(order) = order {
                self.orders_event_handler.send(order);
            }
            ()
        })
    }

    pub async fn close(&self) {
        self.orders_event_handler.close(Duration::ZERO).await;
    }

}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    // Configures the decision maker that will generate our `Multi` events with Buying & Selling decisions
    // -- through it, we may add as many reactive Multi event listeners as we want (each receiving all events).
    let mut decision_maker = DecisionMaker::new();

    // Here we use unnamed closures to build the event pipelines, for the sake of simplicity

    decision_maker.add_listener("Sender",
                                |order_stream| order_stream
                                   .inspect(|order| println!(">>> sending to the Exchange: {:?}", order)
                           )).await?;

    decision_maker.add_listener("Logger",
                                |state_msgs_stream| state_msgs_stream
                                    .inspect(|order| println!("### saving to the storage: {:?}", order)
                           )).await?;

    // additional `Multi`s might be added:
    //   - Accountant: limits the amount of traded money per day, by signaling that order emissions should cease
    //   - Monitor: checks that Sells and Buys are profitable
    //   - and so on...


    // demonstration
    ////////////////

    // simulates the reception of `AnalysisEvent`s -- `Uni`s are not used here for the sake of simplicity;
    // for details on how to use `Uni` on this part, take a look at the `uni-microservice` example.
    let stream_of_incoming_events = stream::iter([
        AnalysisEvent { price_delta:  0.01 },     // buy
        AnalysisEvent { price_delta:  0.02 },
        AnalysisEvent { price_delta:  0.03 },
        AnalysisEvent { price_delta: -0.01 },     // sell
        AnalysisEvent { price_delta: -0.02 },
        AnalysisEvent { price_delta:  0.01 },     // buy
        AnalysisEvent { price_delta: -0.01 },     // sell
    ]);
    decision_maker.decider(stream_of_incoming_events)
        .for_each(|_| async {
            println!("Processed another incoming `AnalysisEvent`...");
        }).await;

    // when it is time to exit the app:
    decision_maker.close().await;
    Ok(())
}
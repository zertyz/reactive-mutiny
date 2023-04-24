//! Demonstrates how to work with a [Multi].
//!
//! This example is an extension of `uni-microservice`.
//!
//! Here, the main events are of types [IncomingEventType] & [OutgoingEventType] -- that could be handled by a `Uni`.
//! For the sake of simplicity, the `Uni` part has been omitted here, but is present at the `uni-microservice` example.
//!
//! The focused part here are the "secondary events", which are [Multi]s: they are generated as part of processing the incoming events,
//! simulating an application with a more complex event processing logic -- in our case, the secondary events are "state messages".
//!
//! Note that those [Multi] events may have as many listeners as the application wants and all of them will receive all generated events
//! -- this is the main aspect distinguishing a `Multi` from a `Uni`.

use reactive_mutiny::{
    multi::{MultiStreamType,Multi,MultiBuilder},
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


/// The processor of [IncomingEventType]s, generating our [Multi] events in the process
struct MyProcessor {
    /// holds the listeners of the generated [Multi] events
    state_event_listeners: MultiBuilder<String, 1024, 16>,
}

impl MyProcessor {

    pub fn new() -> Self {
        Self {
            state_event_listeners: MultiBuilder::new("Listeners for the secondary `Multi` events containing 'state messages' -- generated when processing from the main `IncomingEventType`s"),
        }
    }

    pub async fn add_listener<IntoString:             Into<String>,
                              OutItemType:            Send + Debug,
                              OutStreamType:          Stream<Item=OutItemType> + Send + 'static>
                             (&self,
                              listener_name:    IntoString,
                              pipeline_builder: impl FnOnce(MultiStreamType<'static, String, 1024, 16>) -> OutStreamType)
                             -> Result<(), Box<dyn std::error::Error>> {
        self.state_event_listeners.spawn_non_futures_non_fallible_executor_ref(1, format!("`MyProcessor` state messages listener '{}'", listener_name.into()), pipeline_builder, |_| async {}).await
            .map_err(|err| Box::from(format!("Error adding a 'state messages' listener to `MyProcessor`: {:?}", err)))
            .map(|_| ())
    }

    /// the main logic -- to be considered as an extension of what we have in `uni-microservice`:\
    /// processes [IncomingEventType]s, generating [OutgoingEventType] as responses and generating a "secondary" [Multi] event in the process
    fn process<'a>(&'a mut self, incoming_events_stream: impl Stream<Item=IncomingEventType> + 'a) -> impl Stream<Item=OutgoingEventType> + 'a {
        let mut state = 0;
        incoming_events_stream.map(move |incoming_event| {
            state += 1;     // simple demonstration that we're able to hold a state
            // small logic demonstration to propagate a Multi event
            self.state_event_listeners.handle_ref().send(format!("A new event -- state is {}", state));
            OutgoingEventType {}
        })
    }

    pub async fn close(&self) {
        self.state_event_listeners.handle.close(Duration::ZERO).await;
    }

}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    // configures the main event processor that will generate the secondary Multi events
    // -- through the processor, we may add as many reactive Multi event listeners as we want (each receiving all Multi events).
    // Here we use unnamed closures to build the event pipelines, allowing us to omit most of the type declarations.
    let mut processor = MyProcessor::new();
    processor.add_listener("odd states",
                           |state_msgs_stream| state_msgs_stream
                               .filter(|state_msg| future::ready(state_msg.as_bytes()[state_msg.len()-1] % 2 == 1))
                               .inspect(|state_msg: &Arc<String>| println!("found an ODD state message: '{state_msg}'")
                           )).await?;
    processor.add_listener("even states",
                           |state_msgs_stream| state_msgs_stream
                               .filter(|state_msg| future::ready(state_msg.as_bytes()[state_msg.len()-1] % 2 == 0))
                               .inspect(|state_msg: &Arc<String>| println!("found an EVEN state message: '{state_msg}'")
                           )).await?;

    // simulates the reception of the main events -- `Uni`s are not used here for the sake of simplicity;
    // for details on how to use `Uni` on this part, take a look at the `uni-microservice` example.
    let stream_of_incoming_events = stream::iter([IncomingEventType {}, IncomingEventType {}, IncomingEventType {}, IncomingEventType {}]);
    processor.process(stream_of_incoming_events)
        .for_each(|_| async {
            println!("Processing another `IncomingEventType`...");
        }).await;

    // when it is time to exit the app:
    processor.close().await;
    Ok(())
}

struct IncomingEventType {}
struct OutgoingEventType {}

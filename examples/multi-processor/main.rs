//! Demonstrates how to work with a [Multi].
//!
//! This example is an extension of `uni-microservice`

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

};
use futures::{SinkExt, Stream, stream, StreamExt, TryStreamExt};


struct MyProcessor {
    listeners: MultiBuilder<String, 1024, 16>,
}

impl MyProcessor {

    pub fn new() -> Self {
        Self {
            listeners: MultiBuilder::new("Listeners for Queue A"),
        }
    }

    pub async fn add_listener<IntoString:             Into<String>,
                              OutItemType:            Send + Debug,
                              OutStreamType:          Stream<Item=OutItemType> + Send + 'static>
                             (&self,
                              listener_name:    IntoString,
                              pipeline_builder: impl FnOnce(MultiStreamType<'static, String, 1024, 16>) -> OutStreamType)
                             -> Result<(), Box<dyn std::error::Error>> {
        self.listeners.spawn_non_futures_non_fallible_executor_ref(1, format!("`MyProcessor` listener '{}'", listener_name.into()), pipeline_builder, |_| async {}).await
            .map_err(|err| Box::from(format!("Error adding a listener to `MyProcessor`: {:?}", err)))
            .map(|_| ())
    }

    /// the main logic -- to be considered as an extension of what we have in `uni-microservice`
    fn process<'a>(&'a mut self, incoming_events_stream: impl Stream<Item=IncomingEventType> + 'a) -> impl Stream<Item=OutgoingEventType> + 'a {
        let mut state = 0;
        incoming_events_stream.map(move |incoming_event| {
            state += 1;     // simple demonstration that we're able to hold a state
            // small logic demonstration to propagate a Multi event
            self.listeners.handle_ref().send(format!("Received a new event -- state is {}", state));
            OutgoingEventType {}
        })
    }

    fn close(&self) {
        self.listeners.handle.close(Duration::ZERO);
    }

}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    let mut processor = MyProcessor::new();
    processor.add_listener("odd states",
                           |state_msgs_stream| state_msgs_stream.inspect(|state_msg: &Arc<String>| {
                               if state_msg.as_bytes()[state_msg.len()-1] % 2 == 1 {
                                   println!("found an ODD state message: '{state_msg}'");
                               }
                           })).await?;
    processor.add_listener("even states",
                           |state_msgs_stream| state_msgs_stream.inspect(|state_msg: &Arc<String>| {
                               if state_msg.as_bytes()[state_msg.len()-1] % 2 == 0 {
                                   println!("found an EVEN state message: '{state_msg}'");
                               }
                           })).await?;

    let stream_of_incoming_events = stream::iter([IncomingEventType {}, IncomingEventType {}, IncomingEventType {}, IncomingEventType {}]);
    processor.process(stream_of_incoming_events)
        // simple way to process the whole stream, simulating what happens at runtime
        .for_each(|_| async {
            println!("Processing another incoming event...");
        }).await;

    processor.close();
    Ok(())
}

struct IncomingEventType {}
struct OutgoingEventType {}

//! see [super]

use crate::logic::ogre_robot::{
    events::Events,
    types::{AccountToken, MarketData, Symbol},
};
use std::{
    collections::HashMap,
    future::Future,
    sync::Arc,
    time::Duration,
};
use reactive_mutiny::prelude::advanced as reactive_mutiny;
use self::reactive_mutiny::ChannelProducer;
use futures::{Stream, StreamExt, TryStreamExt};
use tokio::sync::RwLock;


/// maximum acceptable subscribers for each symbol:
///   * if set too high, a little bit of space will be wasted
///   * if set too low, a runtime error will occur when the limit is exceeded
const MAX_SUBSCRIBERS_PER_SYMBOL: usize = 16;

/// how many events we may dispatch, concurrently, when routing [Events.market_data] events to the subscribers:
///   * if > 1, a slight performance penalty is paid, but latency may improve in case there is a subscriber with a full queue
///   * anyway, a subscriber with a full queue is expected to, eventually, hang the whole event processor for up to [TIMEOUT]
const CONCURRENCY: u32 = 1;

/// timeout when dispatching events:
///   * if other than ZERO, a small performance penalty is paid, but any hanging will have a limit
///   * ZERO means hangs might be eternal or of unknown maximum duration
const TIMEOUT: Duration = Duration::ZERO;

/// what is the size of the queue of every subscriber:
///   * if set too high, memory will be wasted
///   * if set too low, hanging might occur
const BUFFER: usize = 32;

/// what Ogre Robot's event dispatcher gives us
type DispatcherPayloadType = (AccountToken, MarketData);
type DispatcherStreamType  = reactive_mutiny::MutinyStream<'static, DispatcherPayloadType, reactive_mutiny::ChannelMultiArcAtomic<DispatcherPayloadType, BUFFER, MAX_SUBSCRIBERS_PER_SYMBOL>, Arc<DispatcherPayloadType>>;

/// what we give to subscribers
type SubscriberPayloadType = (AccountToken, MarketData);


/// Default Mutiny type for "per client" events
type SubscribersMulti = reactive_mutiny::MultiAtomicArc<SubscriberPayloadType, BUFFER, MAX_SUBSCRIBERS_PER_SYMBOL, {reactive_mutiny::Instruments::LogsWithExpensiveMetrics.into()}>;


pub struct MarketWatcher {
    subscribers: RwLock<HashMap<Symbol, SubscribersMulti>>,
}

impl MarketWatcher {

    pub async fn new(events: &Events) -> Arc<Self> {
        let instance = Arc::new(Self {
            subscribers: RwLock::new(HashMap::new()),
        });
        let returned_instance = Arc::clone(&instance);
        events.market_data.spawn_executor(CONCURRENCY, TIMEOUT, "OgreRobot's MarketWatcher",
                                          |stream| instance.event_processor(stream),
                                          |err| async {},
                                          |_| async {

                                              }).await
            .expect("MarketWatcher: new(): unexpected `spawn_executor()` error");
        returned_instance
    }

    pub async fn shutdown(&self) {
        let mut subscribers = self.subscribers.write().await;
        for (symbol, multi) in subscribers.drain() {
            multi.close(TIMEOUT).await;
        }
    }

    pub async fn subscribe<IntoSymbol:             Into<Symbol> + Copy,
                           IntoString:             Into<String>,
                           OutStreamType:          Stream<Item=()> + Send + 'static>
                          (&self,
                           symbol:           IntoSymbol,
                           subscriber_name:  IntoString,
                           pipeline_builder: impl FnOnce(DispatcherStreamType) -> OutStreamType) {
        let mut subscribers = self.subscribers.write().await;
        subscribers
            .entry(symbol.into())
            .or_insert_with(|| reactive_mutiny::Multi::new(format!("MarketData multi for symbol '{}'", symbol.into())))
            .spawn_non_futures_non_fallible_executor(1, subscriber_name,
                                                     pipeline_builder,
                                                     |_| async {}).await
            .expect(&format!("MarketWatcher: subscribe(symbol: '{}'): unexpected `spawn_non_futures_non_fallible_executor()` error", symbol.into()));
    }

    fn event_processor(self: Arc<Self>, stream: impl Stream<Item=Arc<DispatcherPayloadType>>)
                      -> impl Stream<Item=impl Future<Output=Result<(), Box<dyn std::error::Error + Send + Sync>>> + Send> {
        stream.map(move |dispatcher_payload| {
            let cloned_self = Arc::clone(&self);
            async move {
                let subscriber_payload = dispatcher_payload;
                let (symbol, _market_data) = (&subscriber_payload.0, &subscriber_payload.1);
                let subscribers = cloned_self.subscribers.read().await;
                match subscribers.get(symbol) {
                    Some(multi) => multi.channel.send_derived(&subscriber_payload),
                    None                        => {},
                }
                Ok(())
            }
        })
    }
}
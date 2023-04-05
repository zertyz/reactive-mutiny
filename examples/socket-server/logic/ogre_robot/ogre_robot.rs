use std::sync::Arc;
use crate::logic::ogre_robot::{
    events::{
        dispatcher::Dispatcher,
        Events,
    },
    market_watcher::MarketWatcher,
};


pub struct OgreRobot {
    pub dispatcher_events: Arc<Events>,
    pub dispatcher:        Arc<Dispatcher>,
    pub market_watcher:    Arc<MarketWatcher>,
}

impl OgreRobot {

    pub async fn new() -> Self {
        let dispatcher_events = Arc::new(Events::new());
        Self {
            dispatcher_events: Arc::clone(&dispatcher_events),
            dispatcher:        Arc::new(Dispatcher::new(Arc::clone(&dispatcher_events))),
            market_watcher:    MarketWatcher::new(&dispatcher_events).await,
        }
    }

    pub async fn shutdown(&self) {
        self.market_watcher.shutdown().await;
        self.dispatcher.shutdown();
        self.dispatcher_events.shutdown().await;
    }

}
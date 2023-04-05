use std::future::Future;
use crate::logic::ogre_robot::{
    events::{
        dispatcher::Dispatcher,
        Events,
    },
    market_watcher::MarketWatcher,
};
use std::sync::{
    Arc,
    atomic::{AtomicI32,Ordering::Relaxed},
};
use tokio::sync::Semaphore;


pub struct OgreRobot {
    pub dispatcher_events:  Arc<Events>,
    pub dispatcher:         Arc<Dispatcher>,
    pub market_watcher:     Arc<MarketWatcher>,
    /// stays locked until it is time to shutdown, so anybody can wait on it
    shutdown_signal:        Arc<Semaphore>,
    shutdown_waiters_count: AtomicI32,
}

impl OgreRobot {

    pub async fn new() -> Self {
        let dispatcher_events = Arc::new(Events::new());
        Self {
            dispatcher_events:      Arc::clone(&dispatcher_events),
            dispatcher:             Arc::new(Dispatcher::new(Arc::clone(&dispatcher_events))),
            market_watcher:         MarketWatcher::new(&dispatcher_events).await,
            shutdown_signal:        Arc::new(Semaphore::new(0)),
            shutdown_waiters_count: AtomicI32::new(0),
        }
    }

    pub async fn shutdown(&self) {
        self.market_watcher.shutdown().await;
        self.dispatcher.shutdown();
        self.dispatcher_events.shutdown().await;
        while self.shutdown_waiters_count.fetch_sub(1, Relaxed) > 0 {
            self.shutdown_signal.add_permits(1);
        }
    }
    
    pub fn shutdown_waiter(&self) -> impl Future<Output=()> {
        self.shutdown_waiters_count.fetch_add(1, Relaxed);
        let shutdown_signal = Arc::clone(&self.shutdown_signal);
        async move {
            let _ = shutdown_signal.acquire().await.unwrap();
        }
    }

}
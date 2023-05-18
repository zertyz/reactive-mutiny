//! Dispatches Ogre Robot's events to its internal components & subscribed friends


use crate::logic::ogre_robot::{
    types::*,
    events::Events,
};
use std::{
    collections::{HashMap,HashSet},
    sync::atomic::AtomicU32,
    sync::Arc,
};
use reactive_mutiny::multi::ChannelProducer;
use chrono::Timelike;
use neat_date_time::neat_time;
use tokio::sync::RwLock;


type OgreRobotResult<T> = Result<T, Box<dyn std::error::Error>>;
type NeatTime           = AtomicU32;
type AdvisorRegistry    = HashMap<Symbol, HashSet<AccountToken>>;


/// see [self]
pub struct Dispatcher {

   /// events container -- where the evets to be triggered are
   events: Arc<Events>,

    /// := {[symbol] = last_message_time, ...}
    /// holds symbols for which we currently have market data for them, along with the last message received
    market_data_providers: RwLock<HashMap<Symbol, NeatTime>>,

    /// := {[symbol] = [account_tokens1, account_token2, ...], ...}
    /// holds symbols for which we currently have active full advisors interested in, along with their account tokens
    full_advisors: RwLock<AdvisorRegistry>,

    /// := {[symbol] = [account_tokens1, account_token2, ...], ...}
    /// holds symbols for which we currently have active full advisors interested in, along with their account tokens
    watcher_advisors: RwLock<AdvisorRegistry>,

}

impl Dispatcher {

    pub fn new(events: Arc<Events>) -> Self {
        Self {
            events,
            market_data_providers: RwLock::new(HashMap::new()),
            full_advisors:         RwLock::new(HashMap::new()),
            watcher_advisors:      RwLock::new(HashMap::new()),
        }
    }

    pub fn shutdown(&self) {}

    pub async fn register_from_client_identification(&self, client_identification: &ClientIdentification) -> bool {
        if let Some(account_token) = match client_identification {
            ClientIdentification::MarketDataBridge { version, symbol, account_token } => self.register_market_data_provider(symbol, account_token).await,
            ClientIdentification::FullAdvisor      { version, symbol, account_token } => register_advisor(&self.full_advisors,    symbol, account_token).await,
            ClientIdentification::WatcherAdvisor   { version, symbol, account_token } => register_advisor(&self.watcher_advisors, symbol, account_token).await,
        } {
            self.events.identified_client_connected.channel.send(|slot| *slot = account_token);
            true
        } else {
            false
        }
    }

    pub async fn unregister_from_client_identification(&self, client_identification: &ClientIdentification, disconnection_reason: DisconnectionReason) -> bool {
        if let Some(account_token) = match client_identification {
            ClientIdentification::MarketDataBridge { version, symbol, account_token } => self.unregister_market_data_provider(symbol, account_token).await,
            ClientIdentification::FullAdvisor      { version, symbol, account_token } => unregister_advisor(&self.full_advisors, symbol,    account_token).await,
            ClientIdentification::WatcherAdvisor   { version, symbol, account_token } => unregister_advisor(&self.watcher_advisors, symbol, account_token).await,
        } {
            self.events.client_disconnected.channel.send(|slot| *slot = (account_token, disconnection_reason));
            true
        } else {
            false
        }
    }

    pub async fn market_data(&self, client_identification: &ClientIdentification, market_data: MarketData) {
        let account_token = match client_identification {
            ClientIdentification::MarketDataBridge { version, symbol, account_token } => account_token,
            ClientIdentification::FullAdvisor      { version, symbol, account_token } => account_token,
            ClientIdentification::WatcherAdvisor   { version, symbol, account_token } => account_token,
        };
        self.events.market_data.channel.send(|slot| *slot = (account_token.clone(), market_data));
    }

    async fn register_market_data_provider<IntoSymbol:       Into<Symbol>,
                                           IntoAccountToken: Into<AccountToken>>
                                          (&self,
                                           symbol:        IntoSymbol,
                                           account_token: IntoAccountToken) -> Option<AccountToken> {
        let mut result = None;
        self.market_data_providers.write().await
            .entry(symbol.into())
            .or_insert_with(|| {
                result = Some(account_token.into());
                AtomicU32::new(now())
            });
        result
    }

    async fn unregister_market_data_provider<IntoSymbol:       Into<Symbol>,
                                             IntoAccountToken: Into<AccountToken>>
                                            (&self,
                                             symbol:  IntoSymbol,
                                             account: IntoAccountToken) -> Option<AccountToken> {
        self.market_data_providers.write().await
            .remove(&symbol.into())
            .map(|_| account.into())
    }

}

fn now() -> u32 {
    let time_of_day = chrono::offset::Local::now().time();
    neat_time::u32_from_24h_hmsm(time_of_day.hour() as u8, time_of_day.minute() as u8, time_of_day.second() as u8, time_of_day.nanosecond() as u16 / 1000)
}

async fn register_advisor<IntoSymbol:       Into<Symbol>,
                          IntoAccountToken: Into<AccountToken>>
                         (advisor_registry: &RwLock<AdvisorRegistry>,
                          symbol:           IntoSymbol,
                          account:          IntoAccountToken) -> Option<AccountToken> {
    let mut advisor_registry = advisor_registry.write().await;
    let accounts = advisor_registry
        .entry(symbol.into())
        .or_insert(HashSet::new());
    let account = account.into();
    if accounts.contains(&account) {
        None
    } else {
        accounts.insert(account.clone());
        Some(account)
    }
}

async fn unregister_advisor<IntoSymbol:       Into<Symbol>,
                            IntoAccountToken: Into<AccountToken>>
                           (advisor_registry: &RwLock<AdvisorRegistry>,
                            symbol:           IntoSymbol,
                            account:          IntoAccountToken) -> Option<AccountToken> {
    let mut advisor_registry = advisor_registry.write().await;
    let accounts = advisor_registry
        .entry(symbol.into())
        .or_insert(HashSet::new());
    let account = account.into();
    if accounts.remove(&account) {
        Some(account)
    } else {
        None
    }
}

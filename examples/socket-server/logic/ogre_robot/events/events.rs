//! Contains the events this application's `ogre-robot` accepts

use crate::{
    logic::ogre_robot::{
        types::*,
        risk_manager,
        decision_maker,
        market_watcher,
    },
    frontend::socket_server::protocol::{ClientMessages, ServerMessages},
};
use std::{
    fmt::Debug,
    sync::Arc,
};
use reactive_mutiny::{
    multi::Multi,
    multis_close_async
};
use log::warn;


/// Default Mutiny type for "per client" events
type PerClientMulti<ItemType, const MAX_STREAMS: usize = 16> = Multi<'static, ItemType, 4096, MAX_STREAMS, {reactive_mutiny::Instruments::LogsWithExpensiveMetrics.into()}>;


/// Can I refer to internal fields?
pub struct Events {

    /// triggered for [ClientMessages::ClientIdentification]
    pub identified_client_connected: PerClientMulti<AccountToken>,

    /// triggered for [ServerMessages::Disconnected] or for when the server detect the connction has been dropped
    pub client_disconnected: PerClientMulti<(AccountToken, DisconnectionReason)>,

    pub market_data: PerClientMulti<(AccountToken, MarketData)>,

    /// triggered by [decision_maker] when it has determined that someone has to buy something
    /// (this one is not ready to be executed yet -- see [risk_managed_order])
    pub decision_maker_order: PerClientMulti<OrderCommand>,

    /// kicks in when the robot's Risk Manager detects a condition
    pub risk_manager_intervention: PerClientMulti<(AccountToken, Arc<RiskManagementConditions>)>,

    /// triggered by [risk_manager] when an [decision_maker_order] passes the audit
    pub risk_managed_order: PerClientMulti<OrderCommand>,

}


impl Events {

    pub fn new() -> Self {
        Self {
            identified_client_connected: Multi::new("Identified client just connected"),
            client_disconnected:         Multi::new("Client (was) disconnected"),

            market_data:                 Multi::new("Client shared some Market Data"),

            decision_maker_order:        Multi::new("Scheduled Order (by the DecisionMaker, to be approved by the RiskManager)"),

            risk_manager_intervention:   Multi::new("Risk Manager intervention"),
            risk_managed_order:          Multi::new("Risk Managed Order (audited and ready to be executed)"),
        }
    }

    pub async fn shutdown(&self) {
        // make sure all events are here
        multis_close_async!(std::time::Duration::from_secs(5),
            self.identified_client_connected,
            self.identified_client_connected,
            self.client_disconnected,
            self.market_data,
            self.decision_maker_order,
            self.risk_manager_intervention,
            self.risk_managed_order
        );
    }

}

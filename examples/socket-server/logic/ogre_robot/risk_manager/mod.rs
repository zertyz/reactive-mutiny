//! Proxies / monitors the following events, applying bad scenario detection logic:
//!   - `identified_client_connected`
//!   - `dm_order`
//!
//! In case the "bad scenario detection logic" is triggered, the event [super::events::Events::risk_manager_intervention] will be generated.
//! Contains all Ogre Robot shared / external types.
//!
//! Some types defined here will derive both `Serialize` & `Deserialize` -- those are the ones shared with network protocols

use serde::{Serialize, Deserialize};


/// Info that will uniquely identify a client's software
pub type Version = String;
/// Info that will uniquely identify an asset within an exchange
pub type Symbol = String;
/// Info that will uniquely identify a client
pub type AccountToken = String;
/// Date, through the `neat-date-time` crate, based on the epoch 1979-01-22
pub type NeatDate = u16;
/// Time, through the `neat-date-time` crate, based on the 24h representation -- ~21µs precision
pub type NeatTime = u32;
/// In currency unit, multiplied by 1000 -- or in cent, multiplied by 10
pub type MonetaryMillValue = u32;


/// Auto-declarations from the client, affecting how the server takes his messages into account
/// and what types of commands it supports
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum ClientIdentification {

    /// Client is a Market Data Bridge: it will only report trades & other market data
    /// -- the server will take these messages with a higher weight
    MarketDataBridge { version: Version, symbol: Symbol, account_token: AccountToken },
    /// Client that it is an Advisor that is able to execute orders -- it will also
    /// provide market data, but the server will take that only to verify the `MarketDataBridge`s trustability
    FullAdvisor      { version: Version, symbol: Symbol, account_token: AccountToken },
    /// Auto-declaration of the connected client that it is just a Watcher Advisor: no orders should be sent to it
    /// -- this one will not provided market data as it is likely to have a very high ping
    WatcherAdvisor   { version: Version, symbol: Symbol, account_token: AccountToken },
}

/// Details for the client's disconnection
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DisconnectionReason {
    UnknownClientType,
    DeprecatedClientVersion { minimum_accepted_version: Version },
    UnknownAccount,
    /// happens if the server determined it has been abused
    AccountFrozen           { message: String, remaining_duration_nanos: u32 },
    AccountDisabled         { message: String },
    AuthenticationFailure,
    ProtocolOffense         { message: String },
    /// happens, for the old connection, if the same [ClientIdentification] connected again
    Reconnected             { ip: String }, // this might be moved into a protocol offense
    /// happens when the connection is simply dropped by the client -- for no apparent reason
    ClientInitiated,
    RiskManager             (RiskManagementConnectionDroppingConditions),
}

/// Reasons why orders might be cancelled / rejected by the Exchange or Broker
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OrderCancellationReasons {
    ClientInitiated         { message: String },
    TimeoutWhileScheduling  { elapsed_nanos: u32 },
}

/// All possible detectable conditions that may lead to orders (scheduled by the `DecisionMaker`) to be denied by the `RiskManager`.\
/// Nonetheless, even if the order is not scheduled, these events may also go into the [ClientIdentification::WatcherAdvisor] & [ClientIdentification::FullAdvisor] graphs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RiskManagementOrderRefusalConditions {
    /// happens when the server detects [ClientIdentification::MarketDataBridge] faced problems and data is untrusteable
    MarketDataGap               { symbol: Symbol, start_time: u32, duration_nanos: u32 },
    /// an operation was blocked due to its negotiated financial amount being above thresholds
    AmountTooHigh               { amount_millis_limit: u32 },
    /// an operation was blocked due to its negotiated quantity being above thresholds
    QuantityTooHigh             { quantity_limit: u32 },
    /// an operation was blocked (and all subsequent operations are likely to be blocked) due to the daily limit being reached
    DailyCountTooHigh           { daily_count_limit: u32 },
    /// a buying operation was blocked due to the minute limit of buy/sell pair of operations being reached
    ThroughputTooHigh           { round_transactions_per_minute_limit: u32 },
}

/// All possible connectivity problems the Risk Manager may detect, potentially allowing for a client connection to be dropped,
/// in the hope it will reconnect and issues get better.\
/// The conditions present here may also go into the [ClientIdentification::WatcherAdvisor] & [ClientIdentification::FullAdvisor] graphs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RiskManagementConnectionDroppingConditions {
    RoundTripTimeTooHigh    { nanos: u32 },
    /// happens when the client appears unresponsive
    PingTimeout,
    ClockSkewTooHigh        { estimated_delta_nanos: u32 },
}

/// All possible `RiskManager` detectable conditions
#[derive(Debug)]
pub enum RiskManagementConditions {
    /// informs, whoever it may concern, that an intention to place an order, from `DecisionMaker`, won't be made into an Order Event endorsed by the `RiskManager`
    OrderRefused(RiskManagementOrderRefusalConditions),
    /// informs the network handler that the connection with the client should be dropped
    ConnectionShouldBeDropped(RiskManagementConnectionDroppingConditions),
}

/// Represents an order execution command --\
/// TODO: MAYBE THIS IS BETTER IF MOVED TO THE PROTOCOL MODULE, AS IT IS IN THE "EASY" FORMAT
///       (it seems having an internal / optimized internal representation of this is not needed... ?
///        unless if we're registering it on RKYV ).\
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OrderCommand {
    Buy(Order),
    Sell(Order),
}

/// Represents an order to be executed OR an already executed order
/// TODO: make 2 versions of this (like we do for trades): one for the client and the other one for the internal representation (and rip off Serialize/Deserialize from here)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Order {
    /// the order ID, as registered in our Robot
    pub ogre_id: u32,
    /// the party who took initiative to make the trade
    pub aggressor: Parties,
    /// the type of order -- supported by the target exchange
    pub order_type: OrderTypes,
    /// in the form YYYYMMDD
    pub date: u32,
    /// in the form HHMMSSMMM
    pub time: u32,
    /// the symbol -- as recognized by the target exchange
    pub symbol: Symbol,
    /// the unitary paper currency value multiplied by 1000 -- or cent value multiplied by 10
    pub unitary_mill_value: u32,
    /// the number of papers from `symbol`
    pub quantity: u32,
}

/// The parties in trades or to-be-trades
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Parties {
    /// ... or Buy, or the one that bids
    Buyer,
    /// ... or Sell, or the one that asks
    Seller,
}

/// The supported order types & their data
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OrderTypes {
    MarketOrder,
    LimitedOrder { price_limit_mill: u32 },
}

/// Payload for [Events.market_data] event
#[derive(Clone, Debug, PartialEq)]
pub enum MarketData {
    SymbolState(SymbolState),
    BookState(SingleBook),
    Trade(SingleTrade),
}

/// Sent by a [ClientIdentification::MarketDataBridge] client to update the server with the symbol information
/// -- updated after connection, sporadically or when a state change happens
#[derive(Clone, Debug, PartialEq)]
pub struct SymbolState {
    pub symbol: String,
    pub in_auction: bool,
}

/// Represents a book entry to be kept in containers that groups them by `symbol` and `side`,
/// which are omitted to preserve space (together with `date` & `time`)
#[derive(Clone, Debug, PartialEq)]
pub struct GroupedBook {
    /// the unitary paper currency value -- see [MonetaryMillValue]
    price_level_mills: MonetaryMillValue,
    /// the number of orders waiting
    n_orders: u32,
    /// the total quantity of booked orders
    available_quantity: u32,
}

/// Represents the full info of a book event -- to be yet grouped and stored (see [GroupedBook])
#[derive(Clone, Debug, PartialEq)]
pub struct SingleBook {
    date: NeatDate,
    time: NeatTime,
    symbol: Symbol,
    /// the unitary paper currency value -- see [MonetaryMillValue]
    price_level_mills: MonetaryMillValue,
    /// the number of orders waiting
    n_orders: u32,
    /// the total quantity of booked orders
    available_quantity: u32,
    /// the operation those orders want to make
    side: Parties,
}

/// Represents a trade made to be kept in containers that groups them by `symbol` and `date`,
/// which are omitted to preserve space.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupedTrade {
    /// time at which this trade occurred
    time: NeatTime,
    /// the unitary paper currency value -- see [MonetaryMillValue]
    unitary_mill_value: MonetaryMillValue,
    /// how many papers of that symbol were traded
    quantity: u32,
    /// who emitted the Market Order?
    aggressor: Parties,
}

/// Represents the full info of a trade -- to be yet grouped and stored (see [GroupedTrade])
#[derive(Clone, Debug, PartialEq)]
pub struct SingleTrade {
    date: NeatDate,
    time: NeatTime,
    symbol: Symbol,
    /// the unitary paper currency value -- see [MonetaryMillValue]
    unitary_mill_value: MonetaryMillValue,
    /// how many papers of that symbol were traded
    quantity: u32,
    /// who emitted the Market Order?
    aggressor: Parties,
}
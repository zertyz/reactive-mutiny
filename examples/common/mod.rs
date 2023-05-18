//! Common code for `uni-microservice` & `multi-processor` examples


use std::ops::Deref;

/// The input event. For the examples, an hypothetical trading exchange shares with us a stream of events
/// for which we only care for book tops & trades -- all for the same asset
#[derive(Debug)]
pub enum ExchangeEvent {

    /// Issued when the book of orders have a change in the selling or buying prices available for immediate transactions
    BookTopEvent {
        best_bid: f64,
        best_ask: f64,
    },

    /// Issued when two parties agreed on a transaction which caused transfer of money & property
    TradeEvent {
        unitary_value: f64,
        quantity:      u128,
        time:          u64,
    },

    /// Any other events issued by the Exchange are ignored
    Ignored,
}
impl Deref for ExchangeEvent {
    type Target = ExchangeEvent;

    fn deref(&self) -> &Self::Target {
        self
    }
}

/// The result of analysing a sequence of [ExchangeEvent]s.\
/// When issued, simply tells if the prices are going UP or DOWN and by HOW MUCH.
#[derive(Debug)]
pub struct AnalysisEvent {

    /// This delta's base value is zeroed out whenever the price trend changes direction: if prices were going up (and are now going down) and vice-versa.\
    /// When the next trade follows the previous trend, the base will be kept and the delta will be computed with `unitary_value - base_value`
    pub price_delta: f64,
}


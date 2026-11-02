//! Trading Center domain model.
//!
//! Rules enforced by these types:
//! * Money and quantities are `Decimal`. Indicators may use `f64` because they are
//!   statistics, not balances.
//! * Every order starts life as a persisted [`OrderIntent`] with a UUID that doubles as the
//!   broker client order id where the broker supports one (Bitvavo).
//! * Forecasts ([`Decision::forecast`]) and realized results are distinct types so the UI
//!   can never mix them.

pub mod clock;
pub mod instrument;
pub mod money;
pub mod order;
pub mod signal;

pub use clock::{Clock, FixedClock, SystemClock};
pub use instrument::{AssetClass, Broker, Candle, Instrument, InstrumentId, Interval};
pub use money::{format_huf, parse_decimal, Currency, Money};
pub use order::{
    Fill, IntentReason, OrderIntent, OrderKind, OrderState, Position, PositionBook, Side, TimeInForce,
};
pub use signal::{AgentKind, Decision, DecisionAction, Direction, Forecast, Signal};

/// Errors raised by domain-level validation.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CoreError {
    #[error("invalid currency code {0:?}")]
    InvalidCurrency(String),
    #[error("invalid decimal {0:?}")]
    InvalidDecimal(String),
    #[error("currency mismatch: {0} vs {1}")]
    CurrencyMismatch(Currency, Currency),
    #[error("arithmetic overflow")]
    Overflow,
    #[error("invalid instrument id {0:?}")]
    InvalidInstrumentId(String),
    #[error("invalid state transition {from:?} -> {to:?}")]
    InvalidTransition { from: OrderState, to: OrderState },
    #[error("invalid value: {0}")]
    Invalid(String),
}

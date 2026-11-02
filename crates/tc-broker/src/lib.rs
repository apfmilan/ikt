//! Broker connector contract shared by `tc-broker-bitvavo` and `tc-broker-t212`.
//!
//! Every order-placing call goes through [`live::check_live_allowed`]: unless the engine
//! process explicitly armed live trading at start-up (after verified credentials and the
//! user's passkey-signed authorization), requests to real broker hosts are refused.
//! Tests never arm the guard, so they cannot reach live order endpoints.

pub mod live;
pub mod ratelimit;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tc_core::{Broker, Candle, Currency, Instrument, InstrumentId, Interval, OrderIntent, OrderKind, Side};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    #[error("rate limited until {retry_at:?}")]
    RateLimited { retry_at: Option<DateTime<Utc>> },
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("broker rejected request ({status}): {code} {message}")]
    Rejected { status: u16, code: String, message: String },
    #[error("network error: {0}")]
    Network(String),
    #[error("timeout")]
    Timeout,
    #[error("unexpected response: {0}")]
    Decode(String),
    #[error("not supported by this broker: {0}")]
    Unsupported(&'static str),
    #[error("live trading is not armed for this process")]
    LiveDisabled,
    #[error("no credentials configured")]
    NoCredentials,
}

impl BrokerError {
    /// After this error, did the request possibly reach the broker and execute?
    /// Timeouts, network errors after send and 5xx are uncertain → intent becomes UNKNOWN.
    pub fn outcome_uncertain(&self) -> bool {
        match self {
            BrokerError::Timeout | BrokerError::Network(_) | BrokerError::Decode(_) => true,
            BrokerError::Rejected { status, .. } => *status >= 500 || *status == 408,
            _ => false,
        }
    }
}

pub type BrokerResult<T> = Result<T, BrokerError>;

/// Cash balance per currency.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CashBalance {
    pub currency: Currency,
    pub available: Decimal,
    pub reserved: Decimal,
}

/// Account snapshot as reported by the broker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountSnapshot {
    pub broker: Broker,
    pub account_currency: Currency,
    pub cash: Vec<CashBalance>,
    /// Total account value in account currency when the broker reports it (T212 summary).
    pub total_value: Option<Decimal>,
    pub as_of: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrokerPosition {
    pub instrument: InstrumentId,
    pub quantity: Decimal,
    /// Quantity not locked by resting sell orders.
    pub free_quantity: Decimal,
    pub avg_price: Option<Decimal>,
    pub current_price: Option<Decimal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerOrderStatus {
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
    Expired,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrokerFill {
    pub fill_id: String,
    pub quantity: Decimal,
    pub price: Decimal,
    pub fee: Decimal,
    pub fee_currency: Option<Currency>,
    pub ts: DateTime<Utc>,
}

/// Normalised view of an order at the broker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrokerOrder {
    pub broker_order_id: String,
    pub client_order_id: Option<Uuid>,
    pub instrument: InstrumentId,
    pub side: Side,
    pub kind: OrderKind,
    pub quantity: Decimal,
    pub filled_quantity: Decimal,
    pub avg_fill_price: Option<Decimal>,
    pub fee_paid: Decimal,
    pub fee_currency: Option<Currency>,
    pub status: BrokerOrderStatus,
    /// Order was created through the API (T212 `initiatedFrom == API`).
    pub via_api: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub fills: Vec<BrokerFill>,
}

/// Result of a place-order call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PlaceOutcome {
    /// Broker acknowledged the order.
    Accepted { order: BrokerOrder },
    /// Broker definitively refused it; nothing was placed.
    Rejected { code: String, message: String },
    /// Outcome unknown (timeout, connection reset after send, 5xx). Must be reconciled.
    Unknown { reason: String },
}

/// Lookup key for an existing order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderRef {
    pub instrument: InstrumentId,
    pub broker_order_id: Option<String>,
    pub client_order_id: Option<Uuid>,
}

/// External cash movement (deposit/withdrawal) used to separate deposits from trading P&L.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CashMovement {
    pub broker_ref: String,
    pub kind: CashMovementKind,
    pub amount: Decimal,
    pub currency: Currency,
    pub ts: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CashMovementKind {
    Deposit,
    Withdrawal,
    Fee,
    Interest,
    Dividend,
}

/// Result of the onboarding / periodic credential verification.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CredentialReport {
    pub authenticated: bool,
    pub can_read_account: bool,
    pub can_trade: bool,
    /// `Some(false)` = verified no withdrawal permission; `Some(true)` = withdrawal possible
    /// (key must be refused); `None` = could not be determined.
    pub can_withdraw: Option<bool>,
    pub account_currency: Option<Currency>,
    pub notes: Vec<String>,
}

/// Best bid/ask and 24 h stats.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Quote {
    pub instrument: InstrumentId,
    pub bid: Option<Decimal>,
    pub ask: Option<Decimal>,
    pub last: Option<Decimal>,
    /// 24 h quote-currency volume (crypto) when available.
    pub volume_quote_24h: Option<Decimal>,
    pub ts: DateTime<Utc>,
}

/// Rolling connector health counters, exported to the risk snapshot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HealthCounters {
    pub last_ok: Option<DateTime<Utc>>,
    pub last_error: Option<DateTime<Utc>>,
    pub last_rate_limited: Option<DateTime<Utc>>,
    pub rate_budget_remaining: Option<Decimal>,
    pub requests_5m: u32,
    pub errors_5m: u32,
    pub clock_skew_ms: i64,
}

/// Trading connector. Implementations must be cancel-safe and never retry order placement.
#[async_trait]
pub trait BrokerClient: Send + Sync {
    fn broker(&self) -> Broker;
    async fn verify_credentials(&self) -> BrokerResult<CredentialReport>;
    async fn instruments(&self) -> BrokerResult<Vec<Instrument>>;
    async fn account(&self) -> BrokerResult<AccountSnapshot>;
    async fn positions(&self) -> BrokerResult<Vec<BrokerPosition>>;
    async fn open_orders(&self) -> BrokerResult<Vec<BrokerOrder>>;
    /// Orders created at or after `since` (filled, cancelled, ...), newest first.
    async fn order_history(&self, since: DateTime<Utc>) -> BrokerResult<Vec<BrokerOrder>>;
    async fn get_order(&self, r: &OrderRef) -> BrokerResult<Option<BrokerOrder>>;
    /// Place exactly once. Never retried internally. Uncertain results return `Unknown`.
    async fn place_order(&self, intent: &OrderIntent, instrument: &Instrument) -> BrokerResult<PlaceOutcome>;
    async fn cancel_order(&self, r: &OrderRef) -> BrokerResult<()>;
    async fn cash_movements(&self, since: DateTime<Utc>) -> BrokerResult<Vec<CashMovement>>;
    fn health(&self) -> HealthCounters;
}

/// Market data source (Bitvavo for crypto; a configured provider for stocks/ETFs).
#[async_trait]
pub trait MarketDataSource: Send + Sync {
    fn name(&self) -> &'static str;
    /// Closed candles, oldest first. `end` = exclusive upper bound (None = now).
    async fn candles(
        &self,
        instrument: &InstrumentId,
        interval: Interval,
        limit: usize,
        end: Option<DateTime<Utc>>,
    ) -> BrokerResult<Vec<Candle>>;
    async fn quote(&self, instrument: &InstrumentId) -> BrokerResult<Quote>;
}

/// Mask an identifier for logs: keeps the first 4 characters.
pub fn mask(s: &str) -> String {
    let keep: String = s.chars().take(4).collect();
    format!("{keep}…({} chars)", s.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncertain_outcomes() {
        assert!(BrokerError::Timeout.outcome_uncertain());
        assert!(BrokerError::Rejected { status: 503, code: "x".into(), message: String::new() }.outcome_uncertain());
        assert!(BrokerError::Rejected { status: 408, code: "x".into(), message: String::new() }.outcome_uncertain());
        assert!(!BrokerError::Rejected { status: 400, code: "x".into(), message: String::new() }.outcome_uncertain());
        assert!(!BrokerError::Auth("x".into()).outcome_uncertain());
    }

    #[test]
    fn masking() {
        assert_eq!(mask("abcdefgh"), "abcd…(8 chars)");
    }
}

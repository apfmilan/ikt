use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::{CoreError, InstrumentId, Money};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn as_str(&self) -> &'static str {
        match self {
            Side::Buy => "buy",
            Side::Sell => "sell",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeInForce {
    /// Good till cancelled.
    Gtc,
    /// Trading 212 `DAY`.
    Day,
    /// Bitvavo immediate-or-cancel.
    Ioc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OrderKind {
    Market,
    Limit {
        price: Decimal,
    },
    /// Market order once `stop` is touched (protective stop-loss at the broker).
    Stop {
        stop: Decimal,
    },
    StopLimit {
        stop: Decimal,
        limit: Decimal,
    },
}

/// Why an order exists. Exits are protective and must never be blocked by entry-only
/// risk checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentReason {
    Entry,
    LongTermInvest,
    StopLoss,
    TakeProfit,
    TrailingStop,
    PartialExit,
    StrategyExit,
    ManualSell,
    EmergencyFlatten,
}

impl IntentReason {
    pub fn is_entry(&self) -> bool {
        matches!(self, IntentReason::Entry | IntentReason::LongTermInvest)
    }

    pub fn is_protective_exit(&self) -> bool {
        !self.is_entry()
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            IntentReason::Entry => "entry",
            IntentReason::LongTermInvest => "long_term_invest",
            IntentReason::StopLoss => "stop_loss",
            IntentReason::TakeProfit => "take_profit",
            IntentReason::TrailingStop => "trailing_stop",
            IntentReason::PartialExit => "partial_exit",
            IntentReason::StrategyExit => "strategy_exit",
            IntentReason::ManualSell => "manual_sell",
            IntentReason::EmergencyFlatten => "emergency_flatten",
        }
    }
}

/// Lifecycle of an order intent. `Unknown` means the send outcome is not known (timeout,
/// crash after send); it is resolved only by reconciliation, never by blind retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderState {
    /// Persisted and risk-approved, nothing sent yet.
    Pending,
    /// Attempt row committed; HTTP request in flight.
    Submitting,
    /// Outcome unknown after send; reconciliation required.
    Unknown,
    /// Accepted by the broker, resting / awaiting trigger.
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
    Expired,
    /// Broker explicitly rejected the order.
    Rejected,
    /// Reconciliation proved the order never reached the broker.
    NotPlaced,
    /// Withdrawn locally before sending (kill switch, risk re-check failed).
    Aborted,
}

impl OrderState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            OrderState::Filled
                | OrderState::Cancelled
                | OrderState::Expired
                | OrderState::Rejected
                | OrderState::NotPlaced
                | OrderState::Aborted
        )
    }

    /// States that represent possible exposure at the broker (count towards in-flight limits).
    pub fn is_live_or_uncertain(&self) -> bool {
        matches!(
            self,
            OrderState::Submitting
                | OrderState::Unknown
                | OrderState::Open
                | OrderState::PartiallyFilled
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            OrderState::Pending => "pending",
            OrderState::Submitting => "submitting",
            OrderState::Unknown => "unknown",
            OrderState::Open => "open",
            OrderState::PartiallyFilled => "partially_filled",
            OrderState::Filled => "filled",
            OrderState::Cancelled => "cancelled",
            OrderState::Expired => "expired",
            OrderState::Rejected => "rejected",
            OrderState::NotPlaced => "not_placed",
            OrderState::Aborted => "aborted",
        }
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        Ok(match s {
            "pending" => OrderState::Pending,
            "submitting" => OrderState::Submitting,
            "unknown" => OrderState::Unknown,
            "open" => OrderState::Open,
            "partially_filled" => OrderState::PartiallyFilled,
            "filled" => OrderState::Filled,
            "cancelled" => OrderState::Cancelled,
            "expired" => OrderState::Expired,
            "rejected" => OrderState::Rejected,
            "not_placed" => OrderState::NotPlaced,
            "aborted" => OrderState::Aborted,
            o => return Err(CoreError::Invalid(format!("unknown order state {o}"))),
        })
    }

    /// Validate a state transition. The state machine is the single authority for
    /// order lifecycle changes; the store refuses writes that fail this check.
    pub fn transition(self, to: OrderState) -> Result<OrderState, CoreError> {
        use OrderState::*;
        let ok = match (self, to) {
            (Pending, Submitting | Aborted) => true,
            (
                Submitting,
                Open | PartiallyFilled | Filled | Rejected | Unknown | Cancelled | Expired,
            ) => true,
            (
                Unknown,
                Open | PartiallyFilled | Filled | Cancelled | Expired | Rejected | NotPlaced,
            ) => true,
            (Open, PartiallyFilled | Filled | Cancelled | Expired) => true,
            (PartiallyFilled, PartiallyFilled | Filled | Cancelled | Expired) => true,
            _ => false,
        };
        if ok {
            Ok(to)
        } else {
            Err(CoreError::InvalidTransition { from: self, to })
        }
    }
}

/// A risk-checked instruction to trade. `id` is generated once and reused as Bitvavo
/// `clientOrderId`; for Trading 212 (no client id) it is the local correlation key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderIntent {
    pub id: Uuid,
    pub instrument: InstrumentId,
    pub side: Side,
    pub kind: OrderKind,
    /// Base quantity, always positive. Connectors apply venue conventions (T212 sell = negative).
    pub quantity: Decimal,
    pub time_in_force: TimeInForce,
    pub reason: IntentReason,
    pub decision_id: Option<Uuid>,
    /// Reference price used for sizing/risk (last trade or mid).
    pub reference_price: Decimal,
    pub created_at: DateTime<Utc>,
}

impl OrderIntent {
    pub fn notional(&self) -> Decimal {
        self.quantity * self.reference_price
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fill {
    pub intent_id: Uuid,
    pub broker_fill_id: String,
    pub instrument: InstrumentId,
    pub side: Side,
    pub quantity: Decimal,
    pub price: Decimal,
    pub fee: Money,
    pub ts: DateTime<Utc>,
}

/// Open position with managed exits. Long-only: quantity is always > 0.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub instrument: InstrumentId,
    pub quantity: Decimal,
    pub avg_price: Decimal,
    pub opened_at: DateTime<Utc>,
    pub stop_price: Option<Decimal>,
    pub take_profit: Option<Decimal>,
    /// Highest close since entry; drives the trailing stop.
    pub high_watermark: Decimal,
    pub strategy: String,
    /// Long-term investment positions are never touched by trading exits.
    pub long_term: bool,
}

impl Position {
    pub fn market_value(&self, price: Decimal) -> Decimal {
        self.quantity * price
    }

    pub fn unrealized_pnl(&self, price: Decimal) -> Decimal {
        (price - self.avg_price) * self.quantity
    }
}

/// Positions keyed by instrument; applies fills with average-cost accounting.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PositionBook {
    pub positions: BTreeMap<InstrumentId, Position>,
}

impl PositionBook {
    /// Apply a fill. Returns realized P&L (quote currency, before fees) for sells.
    pub fn apply_fill(
        &mut self,
        fill: &Fill,
        strategy: &str,
        long_term: bool,
    ) -> Result<Decimal, CoreError> {
        if fill.quantity <= Decimal::ZERO || fill.price <= Decimal::ZERO {
            return Err(CoreError::Invalid(
                "fill quantity and price must be positive".into(),
            ));
        }
        match fill.side {
            Side::Buy => {
                let p = self
                    .positions
                    .entry(fill.instrument.clone())
                    .or_insert_with(|| Position {
                        instrument: fill.instrument.clone(),
                        quantity: Decimal::ZERO,
                        avg_price: Decimal::ZERO,
                        opened_at: fill.ts,
                        stop_price: None,
                        take_profit: None,
                        high_watermark: fill.price,
                        strategy: strategy.to_owned(),
                        long_term,
                    });
                let new_qty = p.quantity + fill.quantity;
                p.avg_price = (p.avg_price * p.quantity + fill.price * fill.quantity) / new_qty;
                p.quantity = new_qty;
                if fill.price > p.high_watermark {
                    p.high_watermark = fill.price;
                }
                Ok(Decimal::ZERO)
            }
            Side::Sell => {
                let p = self.positions.get_mut(&fill.instrument).ok_or_else(|| {
                    CoreError::Invalid(format!("sell without position in {}", fill.instrument))
                })?;
                if fill.quantity > p.quantity {
                    return Err(CoreError::Invalid(format!(
                        "sell {} exceeds position {} in {} (no shorting)",
                        fill.quantity, p.quantity, fill.instrument
                    )));
                }
                let realized = (fill.price - p.avg_price) * fill.quantity;
                p.quantity -= fill.quantity;
                if p.quantity.is_zero() {
                    self.positions.remove(&fill.instrument);
                }
                Ok(realized)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Broker, Currency};
    use rust_decimal_macros::dec;

    fn fill(side: Side, q: Decimal, p: Decimal) -> Fill {
        Fill {
            intent_id: Uuid::new_v4(),
            broker_fill_id: Uuid::new_v4().to_string(),
            instrument: InstrumentId::new(Broker::Bitvavo, "ETH-EUR"),
            side,
            quantity: q,
            price: p,
            fee: Money::zero(Currency::EUR),
            ts: Utc::now(),
        }
    }

    #[test]
    fn state_machine_forbids_blind_retry_paths() {
        use OrderState::*;
        assert!(Pending.transition(Submitting).is_ok());
        assert!(Submitting.transition(Unknown).is_ok());
        // Unknown can only be resolved by reconciliation results, never back to Submitting.
        assert!(Unknown.transition(Submitting).is_err());
        assert!(Unknown.transition(NotPlaced).is_ok());
        assert!(Filled.transition(Cancelled).is_err());
        assert!(Rejected.transition(Open).is_err());
        assert!(Pending.transition(Filled).is_err());
        for s in [Filled, Cancelled, Expired, Rejected, NotPlaced, Aborted] {
            assert!(s.is_terminal());
            assert_eq!(OrderState::parse(s.as_str()).unwrap(), s);
        }
    }

    #[test]
    fn position_book_average_cost_and_realized_pnl() {
        let mut b = PositionBook::default();
        b.apply_fill(&fill(Side::Buy, dec!(1), dec!(100)), "trend", false)
            .unwrap();
        b.apply_fill(&fill(Side::Buy, dec!(1), dec!(200)), "trend", false)
            .unwrap();
        let id = InstrumentId::new(Broker::Bitvavo, "ETH-EUR");
        assert_eq!(b.positions[&id].avg_price, dec!(150));
        let r = b
            .apply_fill(&fill(Side::Sell, dec!(1), dec!(180)), "trend", false)
            .unwrap();
        assert_eq!(r, dec!(30));
        let r = b
            .apply_fill(&fill(Side::Sell, dec!(1), dec!(120)), "trend", false)
            .unwrap();
        assert_eq!(r, dec!(-30));
        assert!(b.positions.is_empty());
    }

    #[test]
    fn no_shorting() {
        let mut b = PositionBook::default();
        assert!(b
            .apply_fill(&fill(Side::Sell, dec!(1), dec!(1)), "x", false)
            .is_err());
        b.apply_fill(&fill(Side::Buy, dec!(1), dec!(10)), "x", false)
            .unwrap();
        assert!(b
            .apply_fill(&fill(Side::Sell, dec!(2), dec!(10)), "x", false)
            .is_err());
    }

    #[test]
    fn exits_are_protective() {
        assert!(IntentReason::StopLoss.is_protective_exit());
        assert!(IntentReason::ManualSell.is_protective_exit());
        assert!(!IntentReason::Entry.is_protective_exit());
    }
}

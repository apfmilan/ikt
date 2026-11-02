//! Atomic fill booking and other multi-row invariants (review findings C1, R4, R12).
//!
//! A fill, the intent's cumulative progress/state, the position change, the closed trade and the
//! ledger rows are written in ONE transaction keyed by the broker fill id. If any statement fails,
//! nothing is written and the next reconciliation retries; if the fill id was already booked the
//! call is a no-op. This keeps positions, trades and the ledger exactly consistent with fills.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::Row;
use tc_core::{Broker, InstrumentId, OrderState, Side};
use uuid::Uuid;

use crate::trading::{EquitySnapshot, IntentUpdate, PositionRow, TradeRow};
use crate::{now_micros, Store, StoreError, StoreResult};

#[derive(Clone, Debug)]
pub enum PositionChange {
    Upsert(PositionRow),
    Delete(InstrumentId),
}

#[derive(Clone, Debug)]
pub struct LedgerEntry {
    pub ts: DateTime<Utc>,
    pub broker: String,
    pub kind: String,
    pub amount: Decimal,
    pub currency: String,
    pub amount_huf: Decimal,
    pub fx_rate: Decimal,
    pub r#ref: String,
}

#[derive(Clone, Debug)]
pub struct FillCommit {
    pub broker: Broker,
    pub fill_id: String,
    pub intent_id: Uuid,
    pub instrument: InstrumentId,
    pub side: Side,
    pub quantity: Decimal,
    pub price: Decimal,
    pub fee: Decimal,
    pub fee_currency: Option<String>,
    pub ts: DateTime<Utc>,
    /// New intent state, validated by the state machine (same-state allowed only for
    /// `partially_filled` → `partially_filled`, or `None` for no state change).
    pub intent_to: Option<OrderState>,
    pub intent_update: IntentUpdate,
    pub position: PositionChange,
    pub trade: Option<TradeRow>,
    pub ledger: Vec<LedgerEntry>,
}

impl Store {
    /// Book one fill atomically. Returns `false` if this fill id was already booked.
    pub async fn commit_fill(&self, c: &FillCommit) -> StoreResult<bool> {
        let mut tx = self.pool.begin().await?;
        let inserted = sqlx::query(
            "INSERT IGNORE INTO tc_fills (broker, broker_fill_id, intent_id, instrument_id, side, quantity, price, fee, fee_currency, ts) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(c.broker.as_str())
        .bind(&c.fill_id)
        .bind(c.intent_id.to_string())
        .bind(c.instrument.as_str())
        .bind(c.side.as_str())
        .bind(c.quantity)
        .bind(c.price)
        .bind(c.fee)
        .bind(&c.fee_currency)
        .bind(c.ts)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted == 0 {
            tx.rollback().await?;
            return Ok(false);
        }
        // Intent progress (+ optional validated transition) and its event row.
        let cur: String =
            sqlx::query_scalar("SELECT state FROM tc_order_intents WHERE id = ? FOR UPDATE")
                .bind(c.intent_id.to_string())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(StoreError::NotFound)?;
        let cur = OrderState::parse(&cur)?;
        let to = match c.intent_to {
            Some(t) if t != cur || t == OrderState::PartiallyFilled => cur.transition(t)?,
            _ => cur,
        };
        let u = &c.intent_update;
        let now = now_micros();
        sqlx::query(
            "UPDATE tc_order_intents SET state = ?, broker_order_id = COALESCE(?, broker_order_id), filled_quantity = COALESCE(?, filled_quantity),
               avg_fill_price = COALESCE(?, avg_fill_price), fee_paid = COALESCE(?, fee_paid), fee_currency = COALESCE(?, fee_currency),
               updated_at = ?, version = version + 1 WHERE id = ?",
        )
        .bind(to.as_str())
        .bind(&u.broker_order_id)
        .bind(u.filled_quantity)
        .bind(u.avg_fill_price)
        .bind(u.fee_paid)
        .bind(&u.fee_currency)
        .bind(now)
        .bind(c.intent_id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO tc_order_events (intent_id, from_state, to_state, detail, ts) VALUES (?, ?, ?, CAST(? AS JSON), ?)")
            .bind(c.intent_id.to_string())
            .bind(cur.as_str())
            .bind(to.as_str())
            .bind(serde_json::to_string(&serde_json::json!({ "fill_id": c.fill_id, "qty": c.quantity, "price": c.price }))?)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        match &c.position {
            PositionChange::Upsert(p) => {
                sqlx::query(
                    "INSERT INTO tc_positions (instrument_id, quantity, avg_price, opened_at, stop_price, take_profit, initial_risk, high_watermark, strategy, long_term, bars_held, partial_taken, stop_intent_id, decision_id, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                     ON DUPLICATE KEY UPDATE quantity = VALUES(quantity), avg_price = VALUES(avg_price), stop_price = VALUES(stop_price), take_profit = VALUES(take_profit),
                       initial_risk = VALUES(initial_risk), high_watermark = VALUES(high_watermark), bars_held = VALUES(bars_held), partial_taken = VALUES(partial_taken),
                       stop_intent_id = VALUES(stop_intent_id), updated_at = VALUES(updated_at)",
                )
                .bind(p.instrument.as_str())
                .bind(p.quantity)
                .bind(p.avg_price)
                .bind(p.opened_at)
                .bind(p.stop_price)
                .bind(p.take_profit)
                .bind(p.initial_risk)
                .bind(p.high_watermark)
                .bind(&p.strategy)
                .bind(p.long_term)
                .bind(p.bars_held)
                .bind(p.partial_taken)
                .bind(&p.stop_intent_id)
                .bind(&p.decision_id)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            }
            PositionChange::Delete(id) => {
                sqlx::query("DELETE FROM tc_positions WHERE instrument_id = ?")
                    .bind(id.as_str())
                    .execute(&mut *tx)
                    .await?;
            }
        }
        if let Some(t) = &c.trade {
            sqlx::query(
                "INSERT INTO tc_trades (instrument_id, strategy, entry_at, exit_at, entry_price, exit_price, quantity, gross_quote, fees_quote, net_quote, net_huf, fees_huf, return_pct, exit_reason, stopped_out, decision_id, forecast_json)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CAST(? AS JSON))",
            )
            .bind(&t.instrument)
            .bind(&t.strategy)
            .bind(t.entry_at)
            .bind(t.exit_at)
            .bind(t.entry_price)
            .bind(t.exit_price)
            .bind(t.quantity)
            .bind(t.gross_quote)
            .bind(t.fees_quote)
            .bind(t.net_quote)
            .bind(t.net_huf)
            .bind(t.fees_huf)
            .bind(t.return_pct)
            .bind(&t.exit_reason)
            .bind(t.stopped_out)
            .bind(&t.decision_id)
            .bind(serde_json::to_string(&t.forecast)?)
            .execute(&mut *tx)
            .await?;
        }
        for l in &c.ledger {
            sqlx::query("INSERT IGNORE INTO tc_ledger (ts, broker, kind, amount, currency, amount_huf, fx_rate, ref, note) VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL)")
                .bind(l.ts)
                .bind(&l.broker)
                .bind(&l.kind)
                .bind(l.amount)
                .bind(&l.currency)
                .bind(l.amount_huf)
                .bind(l.fx_rate)
                .bind(&l.r#ref)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    /// Update non-state progress fields (e.g. broker order id learned) without a transition.
    pub async fn update_intent_fields(&self, id: Uuid, u: &IntentUpdate) -> StoreResult<()> {
        sqlx::query("UPDATE tc_order_intents SET broker_order_id = COALESCE(?, broker_order_id), updated_at = ? WHERE id = ?")
            .bind(&u.broker_order_id)
            .bind(now_micros())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Abort `pending` intents older than `older_than`. Attempt-before-send guarantees nothing was
    /// sent for a `pending` intent, so aborting is always safe; it unblocks the instrument.
    pub async fn abort_stale_pending(&self, older_than: DateTime<Utc>) -> StoreResult<Vec<Uuid>> {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM tc_order_intents WHERE state = 'pending' AND updated_at < ?",
        )
        .bind(older_than)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for id in ids {
            let Ok(u) = Uuid::parse_str(&id) else {
                continue;
            };
            let upd = IntentUpdate {
                last_error: Some("stale pending intent aborted (never sent)".into()),
                ..Default::default()
            };
            if self
                .transition_intent(u, OrderState::Aborted, &upd)
                .await
                .is_ok()
            {
                out.push(u);
            }
        }
        Ok(out)
    }

    /// Net external flows (deposits − withdrawals, HUF) with ledger id > `after_id`, and the max id.
    pub async fn flows_after_ledger_id(&self, after_id: i64) -> StoreResult<(Decimal, i64)> {
        let r = sqlx::query(
            "SELECT COALESCE(SUM(CASE WHEN kind = 'deposit' THEN amount_huf WHEN kind = 'withdrawal' THEN -ABS(amount_huf) ELSE 0 END), 0) AS s,
                    COALESCE(MAX(id), ?) AS m
             FROM tc_ledger WHERE id > ? AND kind IN ('deposit', 'withdrawal')",
        )
        .bind(after_id)
        .bind(after_id)
        .fetch_one(&self.pool)
        .await?;
        Ok((r.try_get("s")?, r.try_get("m")?))
    }

    /// Highest ledger id (cursor for the first snapshot).
    pub async fn max_ledger_id(&self) -> StoreResult<i64> {
        Ok(
            sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM tc_ledger")
                .fetch_one(&self.pool)
                .await?
                .unwrap_or(0),
        )
    }

    /// Ledger cursor stored with the latest snapshot.
    pub async fn snapshot_ledger_cursor(&self) -> StoreResult<i64> {
        Ok(sqlx::query_scalar::<_, Option<i64>>(
            "SELECT ledger_id_applied FROM tc_equity_snapshots ORDER BY ts DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?
        .flatten()
        .unwrap_or(0))
    }

    pub async fn insert_snapshot_with_cursor(
        &self,
        s: &EquitySnapshot,
        ledger_id_applied: i64,
    ) -> StoreResult<()> {
        sqlx::query(
            "INSERT IGNORE INTO tc_equity_snapshots (ts, equity_huf, cash_huf, positions_huf, net_deposits_huf, units, nav_unit, nav_peak, per_broker, ledger_id_applied)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, CAST(? AS JSON), ?)",
        )
        .bind(s.ts)
        .bind(s.equity_huf)
        .bind(s.cash_huf)
        .bind(s.positions_huf)
        .bind(s.net_deposits_huf)
        .bind(s.units)
        .bind(s.nav_unit)
        .bind(s.nav_peak)
        .bind(serde_json::to_string(&s.per_broker)?)
        .bind(ledger_id_applied)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// First snapshot at or after `t` (baseline when none exists before a day/week start).
    pub async fn first_snapshot_since(&self, t: DateTime<Utc>) -> StoreResult<Option<Decimal>> {
        Ok(sqlx::query_scalar(
            "SELECT nav_unit FROM tc_equity_snapshots WHERE ts >= ? ORDER BY ts LIMIT 1",
        )
        .bind(t)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Clear the user's no-withdrawal attestation (credentials changed: a new key must be re-checked).
    pub async fn reset_attestation(&self, broker: &str) -> StoreResult<()> {
        sqlx::query("UPDATE tc_credential_status SET attested_no_withdraw = FALSE, can_withdraw = NULL WHERE broker = ?")
            .bind(broker)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Whether this connection-held engine lock is still ours (named locks drop with connections).
    pub async fn lock_still_held(lock: &mut crate::EngineLock) -> bool {
        let r: Result<Option<i64>, _> =
            sqlx::query_scalar("SELECT IS_USED_LOCK('tc_engine_singleton') = CONNECTION_ID()")
                .fetch_one(&mut lock.conn)
                .await;
        matches!(r, Ok(Some(1)))
    }
}

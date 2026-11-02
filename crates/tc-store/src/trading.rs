//! Trading data: instruments, candles, order intents (state machine), fills, positions,
//! trades, ledger, equity snapshots, decisions, strategy registry, FX.

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tc_core::{
    AssetClass, Broker, Candle, Currency, Decision, Instrument, InstrumentId, IntentReason,
    Interval, OrderIntent, OrderKind, OrderState, Side, TimeInForce,
};
use uuid::Uuid;

use crate::{now_micros, Store, StoreError, StoreResult};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct IntentRow {
    pub intent: OrderIntent,
    pub strategy: String,
    pub state: OrderState,
    pub broker_order_id: Option<String>,
    pub filled_quantity: Decimal,
    pub avg_fill_price: Option<Decimal>,
    pub fee_paid: Decimal,
    pub fee_currency: Option<String>,
    pub last_error: Option<String>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub version: i32,
}

/// Fields updated together with a state transition.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IntentUpdate {
    pub broker_order_id: Option<String>,
    pub filled_quantity: Option<Decimal>,
    pub avg_fill_price: Option<Decimal>,
    pub fee_paid: Option<Decimal>,
    pub fee_currency: Option<String>,
    pub last_error: Option<String>,
    pub detail: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PositionRow {
    pub instrument: InstrumentId,
    pub quantity: Decimal,
    pub avg_price: Decimal,
    pub opened_at: DateTime<Utc>,
    pub stop_price: Option<Decimal>,
    pub take_profit: Option<Decimal>,
    pub initial_risk: Option<Decimal>,
    pub high_watermark: Decimal,
    pub strategy: String,
    pub long_term: bool,
    pub bars_held: i32,
    pub partial_taken: bool,
    pub stop_intent_id: Option<String>,
    pub decision_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TradeRow {
    pub id: i64,
    pub instrument: String,
    pub strategy: String,
    pub entry_at: DateTime<Utc>,
    pub exit_at: DateTime<Utc>,
    pub entry_price: Decimal,
    pub exit_price: Decimal,
    pub quantity: Decimal,
    pub gross_quote: Decimal,
    pub fees_quote: Decimal,
    pub net_quote: Decimal,
    pub net_huf: Decimal,
    pub fees_huf: Decimal,
    pub return_pct: Decimal,
    pub exit_reason: String,
    pub stopped_out: bool,
    pub decision_id: Option<String>,
    pub forecast: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LedgerRow {
    pub id: i64,
    pub ts: DateTime<Utc>,
    pub broker: String,
    pub kind: String,
    pub amount: Decimal,
    pub currency: String,
    pub amount_huf: Decimal,
    pub fx_rate: Decimal,
    pub r#ref: String,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EquitySnapshot {
    pub ts: DateTime<Utc>,
    pub equity_huf: Decimal,
    pub cash_huf: Decimal,
    pub positions_huf: Decimal,
    pub net_deposits_huf: Decimal,
    pub units: Decimal,
    pub nav_unit: Decimal,
    pub nav_peak: Decimal,
    pub per_broker: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StrategyRow {
    pub name: String,
    pub version: i32,
    pub status: String,
    pub asset_class: String,
    pub report: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EdgeRow {
    pub strategy: String,
    pub market_group: String,
    pub regime: String,
    pub trades: i32,
    pub mean_gross_bps: f64,
    pub se_bps: f64,
    pub win_rate: f64,
}

fn s_json(r: &sqlx::mysql::MySqlRow, col: &str) -> StoreResult<serde_json::Value> {
    let s: Option<String> = r.try_get(col)?;
    Ok(match s {
        Some(s) => serde_json::from_str(&s)?,
        None => serde_json::Value::Null,
    })
}

fn tif_str(t: TimeInForce) -> &'static str {
    match t {
        TimeInForce::Gtc => "gtc",
        TimeInForce::Day => "day",
        TimeInForce::Ioc => "ioc",
    }
}

fn tif_parse(s: &str) -> TimeInForce {
    match s {
        "day" => TimeInForce::Day,
        "ioc" => TimeInForce::Ioc,
        _ => TimeInForce::Gtc,
    }
}

fn reason_parse(s: &str) -> StoreResult<IntentReason> {
    serde_json::from_value(serde_json::Value::String(s.to_owned()))
        .map_err(|e| StoreError::Invalid(e.to_string()))
}

fn side_parse(s: &str) -> Side {
    if s == "sell" {
        Side::Sell
    } else {
        Side::Buy
    }
}

const INTENT_COLS: &str = "id, instrument_id, side, CAST(kind_json AS CHAR) AS kind_json, quantity, time_in_force, reason, strategy, decision_id, reference_price, state, broker_order_id, filled_quantity, avg_fill_price, fee_paid, fee_currency, last_error, created_at, submitted_at, updated_at, version";

fn intent_from_row(r: &sqlx::mysql::MySqlRow) -> StoreResult<IntentRow> {
    let id: String = r.try_get("id")?;
    let kind: String = r.try_get("kind_json")?;
    let decision: Option<String> = r.try_get("decision_id")?;
    Ok(IntentRow {
        intent: OrderIntent {
            id: Uuid::parse_str(&id).map_err(|e| StoreError::Invalid(e.to_string()))?,
            instrument: InstrumentId::parse(&r.try_get::<String, _>("instrument_id")?)?,
            side: side_parse(&r.try_get::<String, _>("side")?),
            kind: serde_json::from_str::<OrderKind>(&kind)?,
            quantity: r.try_get("quantity")?,
            time_in_force: tif_parse(&r.try_get::<String, _>("time_in_force")?),
            reason: reason_parse(&r.try_get::<String, _>("reason")?)?,
            decision_id: decision.and_then(|d| Uuid::parse_str(&d).ok()),
            reference_price: r.try_get("reference_price")?,
            created_at: r.try_get("created_at")?,
        },
        strategy: r.try_get("strategy")?,
        state: OrderState::parse(&r.try_get::<String, _>("state")?)?,
        broker_order_id: r.try_get("broker_order_id")?,
        filled_quantity: r.try_get("filled_quantity")?,
        avg_fill_price: r.try_get("avg_fill_price")?,
        fee_paid: r.try_get("fee_paid")?,
        fee_currency: r.try_get("fee_currency")?,
        last_error: r.try_get("last_error")?,
        submitted_at: r.try_get("submitted_at")?,
        updated_at: r.try_get("updated_at")?,
        version: r.try_get("version")?,
    })
}

const POS_COLS: &str = "instrument_id, quantity, avg_price, opened_at, stop_price, take_profit, initial_risk, high_watermark, strategy, long_term, bars_held, partial_taken, stop_intent_id, decision_id";

fn pos_from_row(r: &sqlx::mysql::MySqlRow) -> StoreResult<PositionRow> {
    Ok(PositionRow {
        instrument: InstrumentId::parse(&r.try_get::<String, _>("instrument_id")?)?,
        quantity: r.try_get("quantity")?,
        avg_price: r.try_get("avg_price")?,
        opened_at: r.try_get("opened_at")?,
        stop_price: r.try_get("stop_price")?,
        take_profit: r.try_get("take_profit")?,
        initial_risk: r.try_get("initial_risk")?,
        high_watermark: r.try_get("high_watermark")?,
        strategy: r.try_get("strategy")?,
        long_term: r.try_get("long_term")?,
        bars_held: r.try_get("bars_held")?,
        partial_taken: r.try_get("partial_taken")?,
        stop_intent_id: r.try_get("stop_intent_id")?,
        decision_id: r.try_get("decision_id")?,
    })
}

impl Store {
    // ---------- instruments ----------
    pub async fn upsert_instrument(
        &self,
        i: &Instrument,
        meta: &serde_json::Value,
    ) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO tc_instruments (id, broker, symbol, name, asset_class, quote_currency, isin, min_notional, min_quantity, quantity_decimals, tick_size, tradable, first_seen, updated_at, meta)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CAST(? AS JSON))
             ON DUPLICATE KEY UPDATE name = VALUES(name), quote_currency = VALUES(quote_currency), isin = VALUES(isin), min_notional = VALUES(min_notional),
               min_quantity = VALUES(min_quantity), quantity_decimals = VALUES(quantity_decimals), tick_size = VALUES(tick_size), tradable = VALUES(tradable),
               updated_at = VALUES(updated_at), meta = VALUES(meta)",
        )
        .bind(i.id.as_str())
        .bind(i.id.broker().as_str())
        .bind(i.id.symbol())
        .bind(&i.name)
        .bind(i.asset_class.as_str())
        .bind(i.quote_currency.as_str())
        .bind(&i.isin)
        .bind(i.min_notional)
        .bind(i.min_quantity)
        .bind(i.quantity_decimals as i32)
        .bind(i.tick_size)
        .bind(i.tradable)
        .bind(i.first_seen)
        .bind(now_micros())
        .bind(serde_json::to_string(meta)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn instruments(&self, broker: Option<Broker>) -> StoreResult<Vec<Instrument>> {
        let sql = "SELECT id, name, asset_class, quote_currency, isin, min_notional, min_quantity, quantity_decimals, tick_size, tradable, first_seen FROM tc_instruments";
        let rows = match broker {
            Some(b) => {
                sqlx::query(&format!("{sql} WHERE broker = ? ORDER BY id"))
                    .bind(b.as_str())
                    .fetch_all(&self.pool)
                    .await?
            }
            None => {
                sqlx::query(&format!("{sql} ORDER BY id"))
                    .fetch_all(&self.pool)
                    .await?
            }
        };
        rows.iter()
            .map(|r| {
                let ac: String = r.try_get("asset_class")?;
                Ok(Instrument {
                    id: InstrumentId::parse(&r.try_get::<String, _>("id")?)?,
                    name: r.try_get("name")?,
                    asset_class: match ac.as_str() {
                        "crypto" => AssetClass::Crypto,
                        "etf" => AssetClass::Etf,
                        _ => AssetClass::Stock,
                    },
                    quote_currency: Currency::new(&r.try_get::<String, _>("quote_currency")?)?,
                    isin: r.try_get("isin")?,
                    min_notional: r.try_get("min_notional")?,
                    min_quantity: r.try_get("min_quantity")?,
                    quantity_decimals: r.try_get::<i32, _>("quantity_decimals")?.max(0) as u32,
                    tick_size: r.try_get("tick_size")?,
                    tradable: r.try_get("tradable")?,
                    first_seen: r.try_get("first_seen")?,
                })
            })
            .collect()
    }

    pub async fn instrument(&self, id: &InstrumentId) -> StoreResult<Option<Instrument>> {
        Ok(self
            .instruments(Some(id.broker()))
            .await?
            .into_iter()
            .find(|i| &i.id == id))
    }

    // ---------- candles ----------
    pub async fn upsert_candles(
        &self,
        id: &InstrumentId,
        interval: Interval,
        candles: &[Candle],
    ) -> StoreResult<()> {
        let mut tx = self.pool.begin().await?;
        for c in candles {
            c.validate()?;
            sqlx::query(
                "INSERT INTO tc_candles (instrument_id, bar_interval, ts, o, h, l, c, v) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                 ON DUPLICATE KEY UPDATE o = VALUES(o), h = VALUES(h), l = VALUES(l), c = VALUES(c), v = VALUES(v)",
            )
            .bind(id.as_str())
            .bind(interval.as_str())
            .bind(c.ts)
            .bind(c.open)
            .bind(c.high)
            .bind(c.low)
            .bind(c.close)
            .bind(c.volume)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Most recent `limit` candles, oldest first.
    pub async fn candles(
        &self,
        id: &InstrumentId,
        interval: Interval,
        limit: i64,
    ) -> StoreResult<Vec<Candle>> {
        let rows = sqlx::query("SELECT ts, o, h, l, c, v FROM tc_candles WHERE instrument_id = ? AND bar_interval = ? ORDER BY ts DESC LIMIT ?")
            .bind(id.as_str())
            .bind(interval.as_str())
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        let mut out: Vec<Candle> = rows
            .iter()
            .map(|r| {
                Ok(Candle {
                    ts: r.try_get("ts")?,
                    open: r.try_get("o")?,
                    high: r.try_get("h")?,
                    low: r.try_get("l")?,
                    close: r.try_get("c")?,
                    volume: r.try_get("v")?,
                })
            })
            .collect::<StoreResult<_>>()?;
        out.reverse();
        Ok(out)
    }

    // ---------- intents ----------
    /// Persist a new, risk-approved intent in state `pending` (before anything is sent).
    pub async fn insert_intent(
        &self,
        i: &OrderIntent,
        strategy: &str,
        risk_verdict: &serde_json::Value,
    ) -> StoreResult<()> {
        let now = now_micros();
        sqlx::query(
            "INSERT INTO tc_order_intents (id, instrument_id, side, kind_json, quantity, time_in_force, reason, strategy, decision_id, reference_price, state, risk_verdict, created_at, updated_at)
             VALUES (?, ?, ?, CAST(? AS JSON), ?, ?, ?, ?, ?, ?, 'pending', CAST(? AS JSON), ?, ?)",
        )
        .bind(i.id.to_string())
        .bind(i.instrument.as_str())
        .bind(i.side.as_str())
        .bind(serde_json::to_string(&i.kind)?)
        .bind(i.quantity)
        .bind(tif_str(i.time_in_force))
        .bind(i.reason.as_str())
        .bind(strategy)
        .bind(i.decision_id.map(|d| d.to_string()))
        .bind(i.reference_price)
        .bind(serde_json::to_string(risk_verdict)?)
        .bind(i.created_at)
        .bind(now)
        .execute(&self.pool)
        .await?;
        sqlx::query("INSERT INTO tc_order_events (intent_id, from_state, to_state, detail, ts) VALUES (?, 'none', 'pending', CAST(? AS JSON), ?)")
            .bind(i.id.to_string())
            .bind(serde_json::to_string(risk_verdict)?)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Validated, atomic state transition + event row. Returns the updated row.
    pub async fn transition_intent(
        &self,
        id: Uuid,
        to: OrderState,
        upd: &IntentUpdate,
    ) -> StoreResult<IntentRow> {
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query(&format!(
            "SELECT {INTENT_COLS} FROM tc_order_intents WHERE id = ? FOR UPDATE"
        ))
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?;
        let cur = intent_from_row(&r)?;
        cur.state.transition(to)?;
        let now = now_micros();
        sqlx::query(
            "UPDATE tc_order_intents SET state = ?, broker_order_id = COALESCE(?, broker_order_id), filled_quantity = COALESCE(?, filled_quantity),
               avg_fill_price = COALESCE(?, avg_fill_price), fee_paid = COALESCE(?, fee_paid), fee_currency = COALESCE(?, fee_currency),
               last_error = COALESCE(?, last_error), submitted_at = IF(? = 'submitting', ?, submitted_at), updated_at = ?, version = version + 1
             WHERE id = ? AND version = ?",
        )
        .bind(to.as_str())
        .bind(&upd.broker_order_id)
        .bind(upd.filled_quantity)
        .bind(upd.avg_fill_price)
        .bind(upd.fee_paid)
        .bind(&upd.fee_currency)
        .bind(upd.last_error.as_ref().map(|e| e.chars().take(512).collect::<String>()))
        .bind(to.as_str())
        .bind(now)
        .bind(now)
        .bind(id.to_string())
        .bind(cur.version)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO tc_order_events (intent_id, from_state, to_state, detail, ts) VALUES (?, ?, ?, CAST(? AS JSON), ?)")
            .bind(id.to_string())
            .bind(cur.state.as_str())
            .bind(to.as_str())
            .bind(serde_json::to_string(&upd.detail)?)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        let r = sqlx::query(&format!(
            "SELECT {INTENT_COLS} FROM tc_order_intents WHERE id = ?"
        ))
        .bind(id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        let out = intent_from_row(&r)?;
        tx.commit().await?;
        Ok(out)
    }

    pub async fn intent(&self, id: Uuid) -> StoreResult<Option<IntentRow>> {
        let r = sqlx::query(&format!(
            "SELECT {INTENT_COLS} FROM tc_order_intents WHERE id = ?"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        r.as_ref().map(intent_from_row).transpose()
    }

    /// Intents in non-terminal states, plus recent terminal ones (for duplicate checks).
    pub async fn active_and_recent_intents(
        &self,
        since: DateTime<Utc>,
    ) -> StoreResult<Vec<IntentRow>> {
        let rows = sqlx::query(&format!(
            "SELECT {INTENT_COLS} FROM tc_order_intents WHERE state IN ('pending','submitting','unknown','open','partially_filled') OR created_at >= ? ORDER BY created_at"
        ))
        .bind(since)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(intent_from_row).collect()
    }

    pub async fn intents_page(&self, only_open: bool, limit: i64) -> StoreResult<Vec<IntentRow>> {
        let filter = if only_open {
            "WHERE state IN ('pending','submitting','unknown','open','partially_filled')"
        } else {
            ""
        };
        let rows = sqlx::query(&format!(
            "SELECT {INTENT_COLS} FROM tc_order_intents {filter} ORDER BY created_at DESC LIMIT ?"
        ))
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(intent_from_row).collect()
    }

    /// Count entry intents created since `t` (order-rate limits).
    pub async fn entries_since(&self, t: DateTime<Utc>) -> StoreResult<i64> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM tc_order_intents WHERE reason = 'entry' AND created_at >= ? AND state NOT IN ('aborted','not_placed','rejected')")
            .bind(t)
            .fetch_one(&self.pool)
            .await?)
    }

    // ---------- fills ----------
    /// Insert a fill once (idempotent on broker fill id). Returns true if new.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_fill(
        &self,
        broker: Broker,
        broker_fill_id: &str,
        intent_id: Option<Uuid>,
        instrument: &InstrumentId,
        side: Side,
        qty: Decimal,
        price: Decimal,
        fee: Decimal,
        fee_ccy: Option<&str>,
        ts: DateTime<Utc>,
    ) -> StoreResult<bool> {
        let r = sqlx::query(
            "INSERT IGNORE INTO tc_fills (broker, broker_fill_id, intent_id, instrument_id, side, quantity, price, fee, fee_currency, ts) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(broker.as_str())
        .bind(broker_fill_id)
        .bind(intent_id.map(|u| u.to_string()))
        .bind(instrument.as_str())
        .bind(side.as_str())
        .bind(qty)
        .bind(price)
        .bind(fee)
        .bind(fee_ccy)
        .bind(ts)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() == 1)
    }

    // ---------- positions ----------
    pub async fn positions(&self) -> StoreResult<Vec<PositionRow>> {
        let rows = sqlx::query(&format!(
            "SELECT {POS_COLS} FROM tc_positions ORDER BY opened_at"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(pos_from_row).collect()
    }

    pub async fn upsert_position(&self, p: &PositionRow) -> StoreResult<()> {
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
        .bind(now_micros())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_position(&self, id: &InstrumentId) -> StoreResult<()> {
        sqlx::query("DELETE FROM tc_positions WHERE instrument_id = ?")
            .bind(id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ---------- trades ----------
    pub async fn insert_trade(&self, t: &TradeRow) -> StoreResult<i64> {
        let r = sqlx::query(
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
        .execute(&self.pool)
        .await?;
        Ok(r.last_insert_id() as i64)
    }

    pub async fn trades(
        &self,
        limit: i64,
        since: Option<DateTime<Utc>>,
    ) -> StoreResult<Vec<TradeRow>> {
        let rows = sqlx::query(
            "SELECT id, instrument_id, strategy, entry_at, exit_at, entry_price, exit_price, quantity, gross_quote, fees_quote, net_quote, net_huf, fees_huf, return_pct, exit_reason, stopped_out, decision_id, CAST(forecast_json AS CHAR) AS forecast
             FROM tc_trades WHERE exit_at >= ? ORDER BY exit_at DESC LIMIT ?",
        )
        .bind(since.unwrap_or(DateTime::<Utc>::UNIX_EPOCH))
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(TradeRow {
                    id: r.try_get("id")?,
                    instrument: r.try_get("instrument_id")?,
                    strategy: r.try_get("strategy")?,
                    entry_at: r.try_get("entry_at")?,
                    exit_at: r.try_get("exit_at")?,
                    entry_price: r.try_get("entry_price")?,
                    exit_price: r.try_get("exit_price")?,
                    quantity: r.try_get("quantity")?,
                    gross_quote: r.try_get("gross_quote")?,
                    fees_quote: r.try_get("fees_quote")?,
                    net_quote: r.try_get("net_quote")?,
                    net_huf: r.try_get("net_huf")?,
                    fees_huf: r.try_get("fees_huf")?,
                    return_pct: r.try_get("return_pct")?,
                    exit_reason: r.try_get("exit_reason")?,
                    stopped_out: r.try_get("stopped_out")?,
                    decision_id: r.try_get("decision_id")?,
                    forecast: s_json(r, "forecast")?,
                })
            })
            .collect()
    }

    // ---------- ledger ----------
    /// Idempotent ledger append. Returns true when the row is new.
    #[allow(clippy::too_many_arguments)]
    pub async fn ledger_append(
        &self,
        ts: DateTime<Utc>,
        broker: &str,
        kind: &str,
        amount: Decimal,
        currency: &str,
        amount_huf: Decimal,
        fx_rate: Decimal,
        r#ref: &str,
        note: Option<&str>,
    ) -> StoreResult<bool> {
        let r = sqlx::query(
            "INSERT IGNORE INTO tc_ledger (ts, broker, kind, amount, currency, amount_huf, fx_rate, ref, note) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(ts)
        .bind(broker)
        .bind(kind)
        .bind(amount)
        .bind(currency)
        .bind(amount_huf)
        .bind(fx_rate)
        .bind(r#ref)
        .bind(note)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn ledger(&self, limit: i64) -> StoreResult<Vec<LedgerRow>> {
        let rows = sqlx::query("SELECT id, ts, broker, kind, amount, currency, amount_huf, fx_rate, ref, note FROM tc_ledger ORDER BY ts DESC, id DESC LIMIT ?")
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|r| {
                Ok(LedgerRow {
                    id: r.try_get("id")?,
                    ts: r.try_get("ts")?,
                    broker: r.try_get("broker")?,
                    kind: r.try_get("kind")?,
                    amount: r.try_get("amount")?,
                    currency: r.try_get("currency")?,
                    amount_huf: r.try_get("amount_huf")?,
                    fx_rate: r.try_get("fx_rate")?,
                    r#ref: r.try_get("ref")?,
                    note: r.try_get("note")?,
                })
            })
            .collect()
    }

    /// Net external flows (deposits − withdrawals) in HUF within [from, to).
    pub async fn net_flows_huf(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> StoreResult<Decimal> {
        let v: Option<Decimal> = sqlx::query_scalar(
            "SELECT SUM(CASE WHEN kind = 'deposit' THEN amount_huf WHEN kind = 'withdrawal' THEN -ABS(amount_huf) ELSE 0 END) FROM tc_ledger WHERE ts >= ? AND ts < ?",
        )
        .bind(from)
        .bind(to)
        .fetch_one(&self.pool)
        .await?;
        Ok(v.unwrap_or_default())
    }

    // ---------- equity snapshots ----------
    pub async fn insert_snapshot(&self, s: &EquitySnapshot) -> StoreResult<()> {
        sqlx::query(
            "INSERT IGNORE INTO tc_equity_snapshots (ts, equity_huf, cash_huf, positions_huf, net_deposits_huf, units, nav_unit, nav_peak, per_broker) VALUES (?, ?, ?, ?, ?, ?, ?, ?, CAST(? AS JSON))",
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
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    fn snap_from(r: &sqlx::mysql::MySqlRow) -> StoreResult<EquitySnapshot> {
        Ok(EquitySnapshot {
            ts: r.try_get("ts")?,
            equity_huf: r.try_get("equity_huf")?,
            cash_huf: r.try_get("cash_huf")?,
            positions_huf: r.try_get("positions_huf")?,
            net_deposits_huf: r.try_get("net_deposits_huf")?,
            units: r.try_get("units")?,
            nav_unit: r.try_get("nav_unit")?,
            nav_peak: r.try_get("nav_peak")?,
            per_broker: s_json(r, "per_broker")?,
        })
    }

    const SNAP_COLS: &'static str = "ts, equity_huf, cash_huf, positions_huf, net_deposits_huf, units, nav_unit, nav_peak, CAST(per_broker AS CHAR) AS per_broker";

    pub async fn latest_snapshot(&self) -> StoreResult<Option<EquitySnapshot>> {
        let r = sqlx::query(&format!(
            "SELECT {} FROM tc_equity_snapshots ORDER BY ts DESC LIMIT 1",
            Self::SNAP_COLS
        ))
        .fetch_optional(&self.pool)
        .await?;
        r.as_ref().map(Self::snap_from).transpose()
    }

    /// Last snapshot at or before `t`.
    pub async fn snapshot_at(&self, t: DateTime<Utc>) -> StoreResult<Option<EquitySnapshot>> {
        let r = sqlx::query(&format!(
            "SELECT {} FROM tc_equity_snapshots WHERE ts <= ? ORDER BY ts DESC LIMIT 1",
            Self::SNAP_COLS
        ))
        .bind(t)
        .fetch_optional(&self.pool)
        .await?;
        r.as_ref().map(Self::snap_from).transpose()
    }

    /// Down-sampled equity curve (one point per `bucket_secs`).
    pub async fn equity_curve(
        &self,
        since: DateTime<Utc>,
        bucket_secs: i64,
    ) -> StoreResult<Vec<EquitySnapshot>> {
        let rows = sqlx::query(&format!(
            "SELECT {} FROM tc_equity_snapshots WHERE ts IN (SELECT MAX(ts) FROM tc_equity_snapshots WHERE ts >= ? GROUP BY FLOOR(UNIX_TIMESTAMP(ts) / ?)) ORDER BY ts",
            Self::SNAP_COLS
        ))
        .bind(since)
        .bind(bucket_secs.max(60))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::snap_from).collect()
    }

    // ---------- decisions ----------
    pub async fn insert_decision(
        &self,
        d: &Decision,
        risk: Option<&serde_json::Value>,
    ) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO tc_decisions (id, created_at, instrument_id, action, strategy, confidence, forecast, signals, rationale, risk)
             VALUES (?, ?, ?, ?, ?, ?, CAST(? AS JSON), CAST(? AS JSON), CAST(? AS JSON), CAST(? AS JSON))",
        )
        .bind(d.id.to_string())
        .bind(d.created_at)
        .bind(d.instrument.as_str())
        .bind(d.action.as_str())
        .bind(&d.strategy)
        .bind(if d.confidence.is_finite() { d.confidence } else { 0.0 })
        .bind(serde_json::to_string(&d.forecast)?)
        .bind(serde_json::to_string(&d.signals)?)
        .bind(serde_json::to_string(&d.rationale)?)
        .bind(risk.map(serde_json::to_string).transpose()?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Decisions as JSON objects shaped per docs/api.md.
    pub async fn decisions(
        &self,
        limit: i64,
        instrument: Option<&str>,
        id: Option<&str>,
    ) -> StoreResult<Vec<serde_json::Value>> {
        let base = "SELECT id, created_at, instrument_id, action, strategy, confidence, CAST(forecast AS CHAR) AS forecast, CAST(signals AS CHAR) AS signals, CAST(rationale AS CHAR) AS rationale, CAST(risk AS CHAR) AS risk FROM tc_decisions";
        let rows = if let Some(id) = id {
            sqlx::query(&format!("{base} WHERE id = ?"))
                .bind(id)
                .fetch_all(&self.pool)
                .await?
        } else if let Some(i) = instrument {
            sqlx::query(&format!(
                "{base} WHERE instrument_id = ? ORDER BY created_at DESC LIMIT ?"
            ))
            .bind(i)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(&format!("{base} ORDER BY created_at DESC LIMIT ?"))
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        };
        rows.iter()
            .map(|r| {
                Ok(serde_json::json!({
                    "id": r.try_get::<String, _>("id")?,
                    "created_at": r.try_get::<DateTime<Utc>, _>("created_at")?,
                    "instrument": r.try_get::<String, _>("instrument_id")?,
                    "action": r.try_get::<String, _>("action")?,
                    "strategy": r.try_get::<String, _>("strategy")?,
                    "confidence": r.try_get::<f64, _>("confidence")?,
                    "forecast": s_json(r, "forecast")?,
                    "signals": s_json(r, "signals")?,
                    "rationale": s_json(r, "rationale")?,
                    "risk": s_json(r, "risk")?,
                }))
            })
            .collect()
    }

    // ---------- strategies ----------
    pub async fn upsert_strategy(&self, s: &StrategyRow) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO tc_strategies (name, version, status, asset_class, report, updated_at) VALUES (?, ?, ?, ?, CAST(? AS JSON), ?)
             ON DUPLICATE KEY UPDATE version = VALUES(version), status = VALUES(status), asset_class = VALUES(asset_class), report = VALUES(report), updated_at = VALUES(updated_at)",
        )
        .bind(&s.name)
        .bind(s.version)
        .bind(&s.status)
        .bind(&s.asset_class)
        .bind(serde_json::to_string(&s.report)?)
        .bind(s.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn strategies(&self) -> StoreResult<Vec<StrategyRow>> {
        let rows = sqlx::query("SELECT name, version, status, asset_class, CAST(report AS CHAR) AS report, updated_at FROM tc_strategies ORDER BY name")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|r| {
                Ok(StrategyRow {
                    name: r.try_get("name")?,
                    version: r.try_get("version")?,
                    status: r.try_get("status")?,
                    asset_class: r.try_get("asset_class")?,
                    report: s_json(r, "report")?,
                    updated_at: r.try_get("updated_at")?,
                })
            })
            .collect()
    }

    pub async fn put_edges(&self, rows: &[EdgeRow]) -> StoreResult<()> {
        let mut tx = self.pool.begin().await?;
        for e in rows {
            sqlx::query(
                "INSERT INTO tc_strategy_edges (strategy, market_group, regime, trades, mean_gross_bps, se_bps, win_rate, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                 ON DUPLICATE KEY UPDATE trades = VALUES(trades), mean_gross_bps = VALUES(mean_gross_bps), se_bps = VALUES(se_bps), win_rate = VALUES(win_rate), updated_at = VALUES(updated_at)",
            )
            .bind(&e.strategy)
            .bind(&e.market_group)
            .bind(&e.regime)
            .bind(e.trades)
            .bind(e.mean_gross_bps)
            .bind(if e.se_bps.is_finite() { e.se_bps } else { 1e9 })
            .bind(e.win_rate)
            .bind(now_micros())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn edges(&self, strategy: &str, market_group: &str) -> StoreResult<Vec<EdgeRow>> {
        let rows = sqlx::query("SELECT strategy, market_group, regime, trades, mean_gross_bps, se_bps, win_rate FROM tc_strategy_edges WHERE strategy = ? AND market_group = ?")
            .bind(strategy)
            .bind(market_group)
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|r| {
                Ok(EdgeRow {
                    strategy: r.try_get("strategy")?,
                    market_group: r.try_get("market_group")?,
                    regime: r.try_get("regime")?,
                    trades: r.try_get("trades")?,
                    mean_gross_bps: r.try_get("mean_gross_bps")?,
                    se_bps: r.try_get("se_bps")?,
                    win_rate: r.try_get("win_rate")?,
                })
            })
            .collect()
    }

    // ---------- FX ----------
    pub async fn put_fx(
        &self,
        date: NaiveDate,
        rates: &[(String, Decimal)],
        source: &str,
    ) -> StoreResult<()> {
        let mut tx = self.pool.begin().await?;
        for (ccy, per_eur) in rates {
            sqlx::query(
                "INSERT INTO tc_fx_rates (rate_date, currency, per_eur, source, fetched_at) VALUES (?, ?, ?, ?, ?)
                 ON DUPLICATE KEY UPDATE per_eur = VALUES(per_eur), source = VALUES(source), fetched_at = VALUES(fetched_at)",
            )
            .bind(date)
            .bind(ccy)
            .bind(per_eur)
            .bind(source)
            .bind(now_micros())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Latest FX table: (date, currency → units per 1 EUR).
    pub async fn latest_fx(&self) -> StoreResult<Option<(NaiveDate, Vec<(String, Decimal)>)>> {
        let d: Option<NaiveDate> = sqlx::query_scalar("SELECT MAX(rate_date) FROM tc_fx_rates")
            .fetch_one(&self.pool)
            .await?;
        let Some(d) = d else {
            return Ok(None);
        };
        let rows = sqlx::query("SELECT currency, per_eur FROM tc_fx_rates WHERE rate_date = ?")
            .bind(d)
            .fetch_all(&self.pool)
            .await?;
        let v = rows
            .iter()
            .map(|r| Ok((r.try_get("currency")?, r.try_get("per_eur")?)))
            .collect::<StoreResult<Vec<_>>>()?;
        Ok(Some((d, v)))
    }
}

impl Store {
    /// Execution plan stored with the intent (stop, take-profit, strategy) inside `risk_verdict`.
    pub async fn intent_meta(&self, id: Uuid) -> StoreResult<serde_json::Value> {
        let s: Option<String> = sqlx::query_scalar("SELECT CAST(risk_verdict AS CHAR) FROM tc_order_intents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .flatten();
        Ok(match s {
            Some(s) => serde_json::from_str(&s)?,
            None => serde_json::Value::Null,
        })
    }
}

impl Store {
    /// Total bought quantity and buy fees for an instrument since `since` (entry-fee attribution).
    pub async fn buy_fills_since(&self, instrument: &InstrumentId, since: DateTime<Utc>) -> StoreResult<(Decimal, Decimal)> {
        let r = sqlx::query("SELECT COALESCE(SUM(quantity), 0) AS q, COALESCE(SUM(fee), 0) AS f FROM tc_fills WHERE instrument_id = ? AND side = 'buy' AND ts >= ?")
            .bind(instrument.as_str())
            .bind(since)
            .fetch_one(&self.pool)
            .await?;
        Ok((r.try_get("q")?, r.try_get("f")?))
    }

    /// Broker order ids already linked to an intent (T212 reconciliation must not reuse them).
    pub async fn linked_broker_order_ids(&self) -> StoreResult<Vec<String>> {
        Ok(sqlx::query_scalar("SELECT broker_order_id FROM tc_order_intents WHERE broker_order_id IS NOT NULL").fetch_all(&self.pool).await?)
    }
}

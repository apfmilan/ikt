//! Execution, reconciliation and bookkeeping (ADR-0002, reviewed 2026-11-02).
//!
//! * Attempt-before-send: the intent moves to `submitting` in a committed transaction before
//!   the HTTP request. If the DB is unavailable, nothing is sent. The kill switch is re-read from
//!   MySQL immediately before each send.
//! * Uncertain outcomes become `unknown` and are resolved only by reconciliation (no retry).
//!   Bitvavo: lookup by clientOrderId; after the 5 s signing window + 30 s the order cannot exist.
//!   Trading 212: match pending orders + history; unresolved after 10 min → REDUCE_ONLY + alert.
//! * Fills are booked atomically (fill + intent + position + trade + ledger in one transaction,
//!   keyed by the fill id) and only when the broker reports a fill price and FX is available;
//!   otherwise booking is deferred to the next reconciliation.
//! * Protective stops: every tracked position keeps a broker-resting stop sized to the full
//!   position. Under FREEZE nothing is cancelled or sent. Before an exit or a stop raise cancels
//!   the resting stop, the replacement order must already pass the risk engine; cancellations are
//!   confirmed with the broker (fills during the cancel are booked) before anything else is sent.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;
use tc_broker::{
    BrokerClient, BrokerError, BrokerOrder, BrokerOrderStatus, OrderRef, PlaceOutcome,
};
use tc_core::{
    Broker, Currency, Instrument, InstrumentId, IntentReason, OrderIntent, OrderKind, OrderState,
    Side, TimeInForce,
};
use tc_plan::UnitLedger;
use tc_risk::{KillMode, OrderClass, RiskRequest};
use tc_store::booking::{FillCommit, LedgerEntry, PositionChange};
use tc_store::trading::{EquitySnapshot, IntentRow, IntentUpdate, PositionRow, TradeRow};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{decide, market, notify, Engine};

async fn client(eng: &Engine, b: Broker) -> Option<Arc<dyn BrokerClient>> {
    let br = eng.brokers.read().await;
    match b {
        Broker::Bitvavo => br.bitvavo.clone().map(|c| c as Arc<dyn BrokerClient>),
        Broker::Trading212 => br.t212.clone().map(|c| c as Arc<dyn BrokerClient>),
        Broker::Revolut => None,
    }
}

/// Most restrictive kill mode for broker + strategy, read from MySQL (DB error → FREEZE).
pub async fn kill_mode(eng: &Engine, broker: Broker, strategy: &str) -> KillMode {
    match eng.store.kill_state().await {
        Ok(k) => k
            .global
            .max(k.brokers.get(&broker).copied().unwrap_or_default())
            .max(k.strategies.get(strategy).copied().unwrap_or_default()),
        Err(_) => KillMode::Freeze,
    }
}

pub fn kill_blocks(mode: KillMode, class: OrderClass) -> bool {
    match mode {
        KillMode::Normal => false,
        KillMode::ReduceOnly => class.is_entry(),
        KillMode::Flatten => class.is_entry() || class == OrderClass::ExitSignal,
        KillMode::Freeze => true,
    }
}

async fn transition(
    eng: &Engine,
    id: Uuid,
    to: OrderState,
    upd: IntentUpdate,
) -> Option<IntentRow> {
    match eng.store.transition_intent(id, to, &upd).await {
        Ok(r) => {
            eng.event("order", json!({ "id": id, "state": to.as_str() }))
                .await;
            Some(r)
        }
        Err(e) => {
            warn!(%id, to = to.as_str(), error = %e, "intent transition refused");
            None
        }
    }
}

fn oref(r: &IntentRow) -> OrderRef {
    OrderRef {
        instrument: r.intent.instrument.clone(),
        broker_order_id: r.broker_order_id.clone(),
        client_order_id: (r.intent.instrument.broker() == Broker::Bitvavo).then_some(r.intent.id),
    }
}

// ---------------------------------------------------------------- send
/// Send one persisted `pending` intent to its broker exactly once.
pub async fn execute(eng: &Engine, intent: &OrderIntent, inst: &Instrument, strategy: &str) {
    let t = Instant::now();
    let broker = intent.instrument.broker();
    let class = OrderClass::from_reason(intent.reason);
    let abort = |reason: String| IntentUpdate {
        last_error: Some(reason),
        ..Default::default()
    };
    let Some(c) = client(eng, broker).await else {
        transition(
            eng,
            intent.id,
            OrderState::Aborted,
            abort(format!("no {broker} credentials")),
        )
        .await;
        return;
    };
    let mode = kill_mode(eng, broker, strategy).await;
    if kill_blocks(mode, class) {
        transition(
            eng,
            intent.id,
            OrderState::Aborted,
            abort(format!("kill switch {}", mode.as_str())),
        )
        .await;
        return;
    }
    if !eng.live_armed {
        transition(
            eng,
            intent.id,
            OrderState::Aborted,
            abort("live order sending is disarmed (TC_LIVE_TRADING not enabled)".into()),
        )
        .await;
        notify::alert(
            eng,
            "errors",
            "Megbízás nem lett elküldve",
            "Az élő küldés nincs engedélyezve a szerveren (TC_LIVE_TRADING).",
        )
        .await;
        return;
    }
    // Attempt-before-send: committed before any network I/O.
    let Some(row) = transition(
        eng,
        intent.id,
        OrderState::Submitting,
        IntentUpdate::default(),
    )
    .await
    else {
        return;
    };
    info!(id = %intent.id, instrument = %intent.instrument, side = intent.side.as_str(), qty = %intent.quantity, reason = intent.reason.as_str(), "sending order");
    match c.place_order(intent, inst).await {
        Ok(PlaceOutcome::Accepted { order }) => {
            if let Err(e) = apply_broker_order(eng, &row, &order).await {
                warn!(id = %intent.id, error = %e, "booking deferred to reconciliation");
            }
            let (title, body) = if intent.reason.is_entry() {
                (
                    "🟢 Vétel",
                    format!(
                        "{} {} @ ~{}",
                        intent.instrument.symbol(),
                        intent.quantity,
                        intent.reference_price.round_dp(4)
                    ),
                )
            } else {
                (
                    "🔴 Eladás",
                    format!(
                        "{} {} ({})",
                        intent.instrument.symbol(),
                        intent.quantity,
                        intent.reason.as_str()
                    ),
                )
            };
            notify::send(eng, "trades", title, &body).await;
        }
        Ok(PlaceOutcome::Rejected { code, message }) => {
            transition(
                eng,
                intent.id,
                OrderState::Rejected,
                abort(format!("{code}: {message}")),
            )
            .await;
            notify::alert(
                eng,
                "errors",
                "Megbízás elutasítva",
                &format!("{}: {code} {message}", intent.instrument.symbol()),
            )
            .await;
        }
        Ok(PlaceOutcome::Unknown { reason }) => mark_unknown(eng, intent, reason).await,
        Err(e) if e.outcome_uncertain() => mark_unknown(eng, intent, e.to_string()).await,
        Err(BrokerError::Forbidden(m)) => {
            transition(
                eng,
                intent.id,
                OrderState::Rejected,
                abort(format!("forbidden: {m}")),
            )
            .await;
            let _ = eng
                .store
                .set_kill(
                    &format!("broker:{}", broker.as_str()),
                    KillMode::ReduceOnly,
                    "403 on order: missing trade scope?",
                    "engine",
                )
                .await;
            notify::alert(
                eng,
                "critical_risk",
                "API jogosultság hiányzik",
                &format!("{broker}: a kulcs nem adhat le megbízást (403). Új belépések leállítva."),
            )
            .await;
        }
        Err(e) => {
            transition(eng, intent.id, OrderState::Rejected, abort(e.to_string())).await;
            notify::alert(
                eng,
                "errors",
                "Megbízás hiba",
                &format!("{}: {e}", intent.instrument.symbol()),
            )
            .await;
        }
    }
    eng.agent(
        "execution",
        "active",
        t,
        format!(
            "{} {} {}",
            intent.reason.as_str(),
            intent.instrument,
            intent.quantity
        ),
        json!({}),
    )
    .await;
}

async fn mark_unknown(eng: &Engine, intent: &OrderIntent, reason: String) {
    warn!(id = %intent.id, %reason, "order outcome UNKNOWN; reconciliation will resolve (no retry)");
    transition(
        eng,
        intent.id,
        OrderState::Unknown,
        IntentUpdate {
            last_error: Some(reason),
            ..Default::default()
        },
    )
    .await;
}

fn map_status(s: BrokerOrderStatus) -> OrderState {
    match s {
        BrokerOrderStatus::Open => OrderState::Open,
        BrokerOrderStatus::PartiallyFilled => OrderState::PartiallyFilled,
        BrokerOrderStatus::Filled => OrderState::Filled,
        BrokerOrderStatus::Cancelled => OrderState::Cancelled,
        BrokerOrderStatus::Expired => OrderState::Expired,
        BrokerOrderStatus::Rejected => OrderState::Rejected,
    }
}

// ---------------------------------------------------------------- booking
/// Apply the broker's view of an order: state + atomic incremental fill booking.
pub async fn apply_broker_order(eng: &Engine, row: &IntentRow, order: &BrokerOrder) -> Result<()> {
    let mut to = map_status(order.status);
    if to == OrderState::Open && order.filled_quantity > Decimal::ZERO {
        to = OrderState::PartiallyFilled;
    }
    let delta = order.filled_quantity - row.filled_quantity;
    if delta <= Decimal::ZERO {
        if to != row.state {
            let upd = IntentUpdate {
                broker_order_id: Some(order.broker_order_id.clone()),
                ..Default::default()
            };
            transition(eng, row.intent.id, to, upd).await;
        } else if row.broker_order_id.is_none() {
            eng.store
                .update_intent_fields(
                    row.intent.id,
                    &IntentUpdate {
                        broker_order_id: Some(order.broker_order_id.clone()),
                        ..Default::default()
                    },
                )
                .await?;
        }
        return Ok(());
    }
    let Some(avg) = order.avg_fill_price.filter(|p| *p > Decimal::ZERO) else {
        // No fill price yet (e.g. Trading 212 resting order): book when a priced report arrives.
        if row.broker_order_id.is_none() {
            eng.store
                .update_intent_fields(
                    row.intent.id,
                    &IntentUpdate {
                        broker_order_id: Some(order.broker_order_id.clone()),
                        ..Default::default()
                    },
                )
                .await?;
        }
        return Err(anyhow!(
            "fill without price for {}; deferred",
            row.intent.id
        ));
    };
    let prev_cost = row.avg_fill_price.unwrap_or_default() * row.filled_quantity;
    let mut price = (avg * order.filled_quantity - prev_cost) / delta;
    if price <= Decimal::ZERO {
        price = avg;
    }
    let fee_delta = (order.fee_paid - row.fee_paid).max(Decimal::ZERO);
    let ts = order.updated_at.min(Utc::now());
    let fill_id = format!(
        "{}@{}",
        order.broker_order_id,
        order.filled_quantity.normalize()
    );
    let upd = IntentUpdate {
        broker_order_id: Some(order.broker_order_id.clone()),
        filled_quantity: Some(order.filled_quantity),
        avg_fill_price: Some(avg),
        fee_paid: Some(order.fee_paid),
        fee_currency: order.fee_currency.map(|c| c.to_string()),
        last_error: None,
        detail: json!({}),
    };
    let commit = build_commit(
        eng,
        row,
        to,
        upd,
        &fill_id,
        delta,
        price,
        fee_delta,
        order.fee_currency,
        ts,
    )
    .await?;
    let closes = matches!(commit.position, PositionChange::Delete(_));
    let long_term = matches!(&commit.position, PositionChange::Upsert(p) if p.long_term);
    if !eng.store.commit_fill(&commit).await? {
        return Ok(()); // already booked
    }
    eng.event(
        "order",
        json!({ "id": row.intent.id, "state": to.as_str(), "filled": order.filled_quantity }),
    )
    .await;
    eng.event(
        "position",
        json!({ "instrument": row.intent.instrument.as_str() }),
    )
    .await;
    let id = &row.intent.instrument;
    if closes {
        Box::pin(cancel_resting_sells(eng, id)).await;
    } else if !long_term {
        Box::pin(ensure_protective_stop(eng, id)).await;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn build_commit(
    eng: &Engine,
    row: &IntentRow,
    to: OrderState,
    upd: IntentUpdate,
    fill_id: &str,
    qty: Decimal,
    price: Decimal,
    fee: Decimal,
    fee_ccy: Option<Currency>,
    ts: DateTime<Utc>,
) -> Result<FillCommit> {
    let id = &row.intent.instrument;
    let inst = market::instrument(eng, id)
        .await
        .ok_or_else(|| anyhow!("no metadata for {id}"))?;
    let ccy = inst.quote_currency;
    let fee_ccy = fee_ccy.unwrap_or(ccy);
    let fx = eng
        .fx
        .read()
        .await
        .clone()
        .ok_or_else(|| anyhow!("no FX table; booking deferred"))?;
    let huf = |x: Decimal, c: Currency| {
        fx.to_huf(x, c)
            .ok_or_else(|| anyhow!("no FX rate for {c}; booking deferred"))
    };
    let rate = fx
        .huf_per(ccy)
        .ok_or_else(|| anyhow!("no FX rate for {ccy}"))?;
    let meta = eng.store.intent_meta(row.intent.id).await?;
    let plan = meta.get("plan").cloned().unwrap_or_default();
    let broker = id.broker();
    let mut ledger = Vec::new();
    if fee > Decimal::ZERO {
        ledger.push(LedgerEntry {
            ts,
            broker: broker.as_str().into(),
            kind: "fee".into(),
            amount: -fee,
            currency: fee_ccy.to_string(),
            amount_huf: -huf(fee, fee_ccy)?,
            fx_rate: fx.huf_per(fee_ccy).unwrap_or(rate),
            r#ref: format!("{fill_id}:fee"),
        });
    }
    let existing = eng
        .store
        .positions()
        .await?
        .into_iter()
        .find(|p| &p.instrument == id);
    let (position, trade) = match row.intent.side {
        Side::Buy => {
            let p = match existing {
                Some(mut p) => {
                    let q = p.quantity + qty;
                    p.avg_price = (p.avg_price * p.quantity + price * qty) / q;
                    p.quantity = q;
                    p
                }
                None => PositionRow {
                    instrument: id.clone(),
                    quantity: qty,
                    avg_price: price,
                    opened_at: ts,
                    stop_price: plan
                        .get("stop")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok()),
                    take_profit: None,
                    initial_risk: plan
                        .get("initial_risk")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok()),
                    high_watermark: price,
                    strategy: row.strategy.clone(),
                    long_term: plan
                        .get("long_term")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    bars_held: 0,
                    partial_taken: false,
                    stop_intent_id: None,
                    decision_id: row.intent.decision_id.map(|d| d.to_string()),
                },
            };
            (PositionChange::Upsert(p), None)
        }
        Side::Sell => {
            let mut p =
                existing.ok_or_else(|| anyhow!("sell fill without tracked position on {id}"))?;
            let (bought, buy_fees) = eng.store.buy_fills_since(id, p.opened_at).await?;
            let entry_fee = if bought > Decimal::ZERO {
                buy_fees * qty.min(bought) / bought
            } else {
                Decimal::ZERO
            };
            let fee_quote = if fee_ccy == ccy {
                fee
            } else {
                fx.convert(fee, fee_ccy, ccy).unwrap_or(fee)
            };
            let gross = (price - p.avg_price) * qty;
            let fees = fee_quote + entry_fee;
            let net = gross - fees;
            ledger.push(LedgerEntry {
                ts,
                broker: broker.as_str().into(),
                kind: "realized_pnl".into(),
                // Gross: fees are separate ledger rows, so the ledger sums to net exactly once.
                amount: gross,
                currency: ccy.to_string(),
                amount_huf: huf(gross, ccy)?,
                fx_rate: rate,
                r#ref: format!("{fill_id}:pnl"),
            });
            let trade = TradeRow {
                id: 0,
                instrument: id.as_str().to_owned(),
                strategy: p.strategy.clone(),
                entry_at: p.opened_at,
                exit_at: ts,
                entry_price: p.avg_price,
                exit_price: price,
                quantity: qty,
                gross_quote: gross,
                fees_quote: fees,
                net_quote: net,
                net_huf: huf(net, ccy)?,
                fees_huf: huf(fees, ccy)?,
                return_pct: if p.avg_price > Decimal::ZERO {
                    (price / p.avg_price - Decimal::ONE) * Decimal::ONE_HUNDRED
                } else {
                    Decimal::ZERO
                },
                exit_reason: row.intent.reason.as_str().to_owned(),
                stopped_out: matches!(
                    row.intent.reason,
                    IntentReason::StopLoss | IntentReason::TrailingStop
                ),
                decision_id: p.decision_id.clone(),
                forecast: json!(null),
            };
            p.quantity -= qty;
            let change = if p.quantity <= Decimal::ZERO || p.quantity < inst.min_quantity {
                PositionChange::Delete(id.clone())
            } else {
                if row.intent.reason == IntentReason::PartialExit {
                    p.partial_taken = true;
                }
                PositionChange::Upsert(p)
            };
            (change, Some(trade))
        }
    };
    Ok(FillCommit {
        broker,
        fill_id: fill_id.to_owned(),
        intent_id: row.intent.id,
        instrument: id.clone(),
        side: row.intent.side,
        quantity: qty,
        price,
        fee,
        fee_currency: Some(fee_ccy.to_string()),
        ts,
        intent_to: Some(to),
        intent_update: upd,
        position,
        trade,
        ledger,
    })
}

// ---------------------------------------------------------------- protective orders
/// Resting sell orders (protective stops) for an instrument.
async fn resting_sells(eng: &Engine, id: &InstrumentId) -> Vec<IntentRow> {
    eng.store
        .active_and_recent_intents(Utc::now())
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| {
            &r.intent.instrument == id
                && r.intent.side == Side::Sell
                && matches!(r.state, OrderState::Open | OrderState::PartiallyFilled)
        })
        .collect()
}

/// Cancel an order and confirm its final state with the broker (fills during the cancel are
/// booked). Returns true only when the broker reports a terminal state. Never runs under FREEZE.
async fn cancel_and_confirm(eng: &Engine, r: &IntentRow) -> bool {
    let broker = r.intent.instrument.broker();
    if kill_mode(eng, broker, &r.strategy).await == KillMode::Freeze {
        return false;
    }
    let Some(c) = client(eng, broker).await else {
        return false;
    };
    if let Err(e) = c.cancel_order(&oref(r)).await {
        warn!(instrument = %r.intent.instrument, error = %e, "cancel failed");
        return false;
    }
    match c.get_order(&oref(r)).await {
        Ok(Some(order)) => {
            if let Err(e) = apply_broker_order(eng, r, &order).await {
                warn!(error = %e, "booking after cancel deferred");
            }
            matches!(
                order.status,
                BrokerOrderStatus::Cancelled
                    | BrokerOrderStatus::Filled
                    | BrokerOrderStatus::Expired
                    | BrokerOrderStatus::Rejected
            )
        }
        Ok(None) => transition(
            eng,
            r.intent.id,
            OrderState::Cancelled,
            IntentUpdate::default(),
        )
        .await
        .is_some(),
        Err(e) => {
            warn!(error = %e, "cancel confirmation failed; will reconcile");
            false
        }
    }
}

/// Cancel all resting sells on an instrument; true if none remain.
async fn cancel_resting_sells(eng: &Engine, id: &InstrumentId) -> bool {
    let mut ok = true;
    for r in resting_sells(eng, id).await {
        ok &= cancel_and_confirm(eng, &r).await;
    }
    ok
}

/// Keep a broker-resting stop sized to the full tracked position.
pub async fn ensure_protective_stop(eng: &Engine, id: &InstrumentId) {
    let Some(p) = eng
        .store
        .positions()
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|p| &p.instrument == id)
    else {
        return;
    };
    if p.long_term {
        return;
    }
    let Some(stop) = p.stop_price else { return };
    if kill_mode(eng, id.broker(), &p.strategy).await == KillMode::Freeze {
        return;
    }
    let active = eng
        .store
        .active_and_recent_intents(Utc::now())
        .await
        .unwrap_or_default();
    if active.iter().any(|r| {
        &r.intent.instrument == id
            && matches!(
                r.state,
                OrderState::Pending | OrderState::Submitting | OrderState::Unknown
            )
    }) {
        return; // something uncertain in flight; reconcile first
    }
    let Some(inst) = market::instrument(eng, id).await else {
        return;
    };
    let needed = inst.floor_quantity(p.quantity);
    let resting = resting_sells(eng, id).await;
    let covered: Decimal = resting
        .iter()
        .map(|r| r.intent.quantity - r.filled_quantity)
        .sum();
    if covered >= needed {
        return;
    }
    let stop_px = inst.round_price(stop, Side::Sell);
    let intent = OrderIntent {
        id: Uuid::new_v4(),
        instrument: id.clone(),
        side: Side::Sell,
        kind: OrderKind::Stop { stop: stop_px },
        quantity: needed,
        time_in_force: TimeInForce::Gtc,
        reason: IntentReason::StopLoss,
        decision_id: None,
        reference_price: stop_px,
        created_at: tc_store::now_micros(),
    };
    // Partial-fill growth: replace an undersized stop, but only once the new one is approved.
    if risk_ok(eng, &intent, &inst, &p.strategy, true)
        .await
        .is_none()
    {
        notify::alert(
            eng,
            "critical_risk",
            "Védő stop nem helyezhető el",
            &format!("{id}: a kockázati motor elutasította"),
        )
        .await;
        return;
    }
    if !resting.is_empty() && !cancel_resting_sells(eng, id).await {
        return;
    }
    if eng
        .store
        .insert_intent(
            &intent,
            &p.strategy,
            &json!({ "plan": { "protective": true } }),
        )
        .await
        .is_ok()
    {
        execute(eng, &intent, &inst, &p.strategy).await;
    }
}

/// Risk-check a protective/exit order. With `ignore_resting_sells`, resting stops on the same
/// instrument are excluded (the caller cancels them only after this approval).
async fn risk_ok(
    eng: &Engine,
    intent: &OrderIntent,
    inst: &Instrument,
    strategy: &str,
    ignore_resting_sells: bool,
) -> Option<Decimal> {
    let quote = decide::quote_for(eng, &intent.instrument).await;
    let mut snap = decide::build_snapshot(
        eng,
        &intent.instrument,
        strategy,
        "live",
        quote.as_ref(),
        Utc::now(),
    )
    .await;
    if ignore_resting_sells {
        snap.intents.retain(|i| {
            !(i.instrument == intent.instrument
                && i.side == Side::Sell
                && matches!(i.state, OrderState::Open | OrderState::PartiallyFilled))
        });
    }
    let req = RiskRequest {
        instrument: intent.instrument.clone(),
        asset_class: inst.asset_class,
        side: intent.side,
        class: OrderClass::from_reason(intent.reason),
        strategy: strategy.to_owned(),
        quantity: intent.quantity,
        price: intent.reference_price,
        stop_price: None,
        gap_p99: Decimal::ZERO,
        edge_bps: Decimal::ZERO,
        edge_se_bps: Decimal::ZERO,
        size_multiplier: Decimal::ONE,
        cluster: String::new(),
        created_at: Utc::now(),
    };
    let (engine, _, _) = eng.risk_engine().await;
    let v = engine.evaluate(&req, &snap, Utc::now());
    if v.is_veto() {
        warn!(instrument = %intent.instrument, codes = ?v.codes(), reason = intent.reason.as_str(), "exit vetoed by risk engine");
        eng.event("alert", json!({ "kind": "exit_vetoed", "instrument": intent.instrument.as_str(), "codes": v.codes() })).await;
    }
    v.approved_quantity()
}

/// Exit (part of) a position. The exit is risk-approved BEFORE the resting stop is cancelled,
/// so a veto never leaves the position without its broker stop.
pub async fn exit_position(
    eng: &Engine,
    id: &InstrumentId,
    qty: Decimal,
    reason: IntentReason,
    text: &str,
) -> serde_json::Value {
    let Some(p) = eng
        .store
        .positions()
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|p| &p.instrument == id)
    else {
        return json!({ "ok": false, "error": "no position" });
    };
    let class = OrderClass::from_reason(reason);
    let mode = kill_mode(eng, id.broker(), &p.strategy).await;
    if kill_blocks(mode, class) {
        return json!({ "ok": false, "error": format!("kill switch {}", mode.as_str()) });
    }
    let Some(inst) = market::instrument(eng, id).await else {
        return json!({ "ok": false, "error": "no instrument metadata" });
    };
    let mut q = inst.floor_quantity(qty.min(p.quantity));
    if p.quantity - q < inst.min_quantity {
        q = inst.floor_quantity(p.quantity); // remainder below venue minimum: sell all
    }
    let price = market::last_price(eng, id).await.unwrap_or(p.avg_price);
    let intent = OrderIntent {
        id: Uuid::new_v4(),
        instrument: id.clone(),
        side: Side::Sell,
        kind: OrderKind::Market,
        quantity: q,
        time_in_force: TimeInForce::Gtc,
        reason,
        decision_id: None,
        reference_price: price,
        created_at: tc_store::now_micros(),
    };
    let Some(approved) = risk_ok(eng, &intent, &inst, &p.strategy, true).await else {
        if matches!(class, OrderClass::ManualSell | OrderClass::Flatten) {
            notify::alert(
                eng,
                "critical_risk",
                "Eladás elutasítva",
                &format!("{id}: a kockázati motor elutasította; a védő stop aktív marad."),
            )
            .await;
        }
        return json!({ "ok": false, "error": "risk engine veto (stop left in place)" });
    };
    if !cancel_resting_sells(eng, id).await {
        return json!({ "ok": false, "error": "resting stop cancel not confirmed yet; retry" });
    }
    // Fills during the cancel may have reduced or closed the position.
    let Some(p) = eng
        .store
        .positions()
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|p| &p.instrument == id)
    else {
        return json!({ "ok": true, "info": "position closed while cancelling the stop" });
    };
    let q = inst.floor_quantity(approved.min(p.quantity));
    let intent = OrderIntent {
        quantity: q,
        ..intent
    };
    if eng
        .store
        .insert_intent(&intent, &p.strategy, &json!({ "plan": { "exit": text } }))
        .await
        .is_err()
    {
        Box::pin(ensure_protective_stop(eng, id)).await;
        return json!({ "ok": false, "error": "persist failed" });
    }
    execute(eng, &intent, &inst, &p.strategy).await;
    json!({ "ok": true, "intent_id": intent.id, "quantity": q })
}

/// Trailing-stop update: approve the new stop first, then replace the resting one.
pub async fn raise_stop(eng: &Engine, id: &InstrumentId, new_stop: Decimal) {
    let Some(mut p) = eng
        .store
        .positions()
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|p| &p.instrument == id)
    else {
        return;
    };
    if p.stop_price.is_some_and(|s| new_stop <= s)
        || kill_blocks(
            kill_mode(eng, id.broker(), &p.strategy).await,
            OrderClass::ExitProtective,
        )
    {
        return;
    }
    let Some(inst) = market::instrument(eng, id).await else {
        return;
    };
    let probe = OrderIntent {
        id: Uuid::new_v4(),
        instrument: id.clone(),
        side: Side::Sell,
        kind: OrderKind::Stop { stop: new_stop },
        quantity: inst.floor_quantity(p.quantity),
        time_in_force: TimeInForce::Gtc,
        reason: IntentReason::TrailingStop,
        decision_id: None,
        reference_price: new_stop,
        created_at: tc_store::now_micros(),
    };
    if risk_ok(eng, &probe, &inst, &p.strategy, true)
        .await
        .is_none()
    {
        return; // keep the existing stop
    }
    if !cancel_resting_sells(eng, id).await {
        return;
    }
    p.stop_price = Some(new_stop);
    if eng.store.upsert_position(&p).await.is_ok() {
        ensure_protective_stop(eng, id).await;
    }
}

// ---------------------------------------------------------------- commands
pub async fn process_commands(eng: &Engine) {
    for _ in 0..10 {
        let cmd = match eng.store.claim_command().await {
            Ok(Some(c)) => c,
            Ok(None) => return,
            Err(e) => {
                warn!(error = %e, "command claim failed");
                return;
            }
        };
        info!(id = cmd.id, kind = %cmd.kind, "command");
        let s = |k: &str| {
            cmd.payload
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned()
        };
        let (ok, result) = match cmd.kind.as_str() {
            "verify_credentials" => (true, crate::creds::verify(eng, &s("broker")).await),
            "manual_sell" => match InstrumentId::parse(&s("instrument")) {
                Ok(id) => {
                    let pos_q = eng
                        .store
                        .positions()
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .find(|p| p.instrument == id)
                        .map(|p| p.quantity);
                    let q = s("quantity");
                    let qty = if q == "all" {
                        pos_q
                    } else {
                        q.parse::<Decimal>().ok()
                    };
                    match qty {
                        Some(qty) => {
                            let r = exit_position(
                                eng,
                                &id,
                                qty,
                                IntentReason::ManualSell,
                                "manual sell",
                            )
                            .await;
                            (r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false), r)
                        }
                        None => (false, json!({ "error": "no position" })),
                    }
                }
                Err(_) => (false, json!({ "error": "bad instrument" })),
            },
            "flatten" => (true, flatten(eng, &s("scope")).await),
            "notify" => {
                notify::alert(eng, &s("category"), &s("title"), &s("body")).await;
                (true, json!({}))
            }
            other => (
                false,
                json!({ "error": format!("unknown command {other}") }),
            ),
        };
        if let Err(e) = eng.store.finish_command(cmd.id, ok, &result).await {
            error!(id = cmd.id, error = %e, "finish_command failed (command already executed)");
        }
        eng.event(
            "command",
            json!({ "id": cmd.id, "kind": cmd.kind, "ok": ok, "result": result }),
        )
        .await;
    }
}

/// Emergency flatten of trading positions within `scope` (`global` or `broker:<b>`).
/// The long-term ETF core is kept.
async fn flatten(eng: &Engine, scope: &str) -> serde_json::Value {
    let only: Option<Broker> = scope
        .strip_prefix("broker:")
        .and_then(|b| Broker::parse(b).ok());
    let in_scope = |id: &InstrumentId| only.is_none_or(|b| id.broker() == b);
    for r in eng
        .store
        .active_and_recent_intents(Utc::now())
        .await
        .unwrap_or_default()
    {
        if in_scope(&r.intent.instrument)
            && r.intent.side == Side::Buy
            && matches!(r.state, OrderState::Open | OrderState::PartiallyFilled)
        {
            cancel_and_confirm(eng, &r).await;
        }
    }
    reconcile(eng).await;
    let mut out = Vec::new();
    for p in eng
        .store
        .positions()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|p| !p.long_term && in_scope(&p.instrument))
    {
        let r = exit_position(
            eng,
            &p.instrument,
            p.quantity,
            IntentReason::EmergencyFlatten,
            "emergency flatten",
        )
        .await;
        out.push(json!({ "instrument": p.instrument.as_str(), "result": r }));
    }
    notify::alert(
        eng,
        "critical_risk",
        "Vészleállítás",
        &format!("{} pozíció zárása elindítva ({scope})", out.len()),
    )
    .await;
    json!({ "scope": scope, "positions": out })
}

// ---------------------------------------------------------------- exits
pub async fn manage_exits(eng: &Engine) {
    let t = Instant::now();
    let positions = eng.store.positions().await.unwrap_or_default();
    let mut checked = 0;
    for p in positions.iter().filter(|p| !p.long_term) {
        checked += 1;
        if kill_mode(eng, p.instrument.broker(), &p.strategy).await == KillMode::Freeze {
            continue;
        }
        ensure_protective_stop(eng, &p.instrument).await;
        // Safety net when no broker stop is resting (e.g. placement vetoed or failed).
        if resting_sells(eng, &p.instrument).await.is_empty() {
            if let (Some(stop), Some(q)) =
                (p.stop_price, decide::quote_for(eng, &p.instrument).await)
            {
                if q.bid.is_some_and(|b| b <= stop) {
                    exit_position(
                        eng,
                        &p.instrument,
                        p.quantity,
                        IntentReason::StopLoss,
                        "engine-side stop (no resting broker stop)",
                    )
                    .await;
                }
            }
        }
    }
    // Maker entry limits expire after one daily bar (S3: abstain if unfilled).
    for r in eng
        .store
        .active_and_recent_intents(Utc::now())
        .await
        .unwrap_or_default()
    {
        if r.intent.reason == IntentReason::Entry
            && matches!(r.state, OrderState::Open | OrderState::PartiallyFilled)
            && Utc::now() - r.intent.created_at > Duration::hours(24)
        {
            cancel_and_confirm(eng, &r).await;
        }
    }
    eng.agent(
        "exit",
        "active",
        t,
        format!("{checked} position(s) monitored"),
        json!({ "positions": checked }),
    )
    .await;
}

// ---------------------------------------------------------------- reconciliation
/// Reconcile intents, positions and cash movements; write a unitized equity snapshot when every
/// broker and currency could be valued. Returns true when clean.
pub async fn reconcile(eng: &Engine) -> bool {
    let t = Instant::now();
    let mut mismatches: Vec<String> = Vec::new();
    let mut per_broker = serde_json::Map::new();
    let mut equity_huf = Decimal::ZERO;
    let mut cash_total = Decimal::ZERO;
    let fx = eng.fx.read().await.clone();
    let now = Utc::now();
    let mut stuck_unknown = false;
    let mut configured = 0usize;
    let mut valued = 0usize;

    // Pending intents were never sent (attempt-before-send); stale ones only block instruments.
    match eng
        .store
        .abort_stale_pending(now - Duration::seconds(60))
        .await
    {
        Ok(v) if !v.is_empty() => warn!(count = v.len(), "aborted stale pending intents"),
        _ => {}
    }

    for b in [Broker::Bitvavo, Broker::Trading212] {
        let Some(c) = client(eng, b).await else {
            continue;
        };
        configured += 1;
        let bt = Instant::now();
        // 1) Resolve non-terminal intents.
        let intents = eng
            .store
            .active_and_recent_intents(now)
            .await
            .unwrap_or_default();
        let linked = eng
            .store
            .linked_broker_order_ids()
            .await
            .unwrap_or_default();
        let needs_history = b == Broker::Trading212
            && intents.iter().any(|r| {
                r.intent.instrument.broker() == b
                    && r.broker_order_id.is_none()
                    && !r.state.is_terminal()
            });
        let history = if needs_history {
            let mut h = c.open_orders().await.unwrap_or_default();
            h.extend(
                c.order_history(now - Duration::hours(2))
                    .await
                    .unwrap_or_default(),
            );
            h
        } else {
            vec![]
        };
        for r in intents.iter().filter(|r| {
            r.intent.instrument.broker() == b
                && !r.state.is_terminal()
                && r.state != OrderState::Pending
        }) {
            let age = now - r.submitted_at.unwrap_or(r.intent.created_at);
            if r.state == OrderState::Submitting && age < Duration::seconds(30) {
                continue; // still in flight in this process
            }
            let found = if r.broker_order_id.is_some() || b == Broker::Bitvavo {
                c.get_order(&oref(r)).await
            } else {
                Ok(match_t212(r, &history, &linked))
            };
            match found {
                Ok(Some(order)) => {
                    let row = if r.state == OrderState::Submitting {
                        transition(
                            eng,
                            r.intent.id,
                            OrderState::Unknown,
                            IntentUpdate {
                                last_error: Some("resolved by reconciliation".into()),
                                ..Default::default()
                            },
                        )
                        .await
                    } else {
                        Some(r.clone())
                    };
                    if let Some(row) = row {
                        if let Err(e) = apply_broker_order(eng, &row, &order).await {
                            warn!(id = %row.intent.id, error = %e, "fill booking deferred");
                        }
                    }
                }
                Ok(None) => {
                    if matches!(r.state, OrderState::Unknown | OrderState::Submitting) {
                        if b == Broker::Bitvavo && age > Duration::seconds(35) {
                            if r.state == OrderState::Submitting {
                                transition(
                                    eng,
                                    r.intent.id,
                                    OrderState::Unknown,
                                    IntentUpdate::default(),
                                )
                                .await;
                            }
                            transition(
                                eng,
                                r.intent.id,
                                OrderState::NotPlaced,
                                IntentUpdate {
                                    last_error: Some("not found after signing window".into()),
                                    ..Default::default()
                                },
                            )
                            .await;
                        } else if age > Duration::minutes(10) {
                            stuck_unknown = true;
                            mismatches.push(format!(
                                "{} UNKNOWN for {} min",
                                r.intent.instrument,
                                age.num_minutes()
                            ));
                        }
                    }
                }
                Err(e) => {
                    warn!(instrument = %r.intent.instrument, error = %e, "order lookup failed")
                }
            }
        }
        // 2) Account and positions; value everything in HUF or skip the snapshot.
        let (account, positions) = (c.account().await, c.positions().await);
        let mut broker_ok = false;
        if let (Ok(acc), Ok(pos), Some(fx)) = (&account, &positions, fx.as_ref()) {
            eng.last_account_sync.write().await.insert(b, Utc::now());
            let mut ok = true;
            let mut cash_huf = Decimal::ZERO;
            let mut avail_huf = Decimal::ZERO;
            for cb in &acc.cash {
                match (
                    fx.to_huf(cb.available + cb.reserved, cb.currency),
                    fx.to_huf(cb.available, cb.currency),
                ) {
                    (Some(total), Some(avail)) => {
                        cash_huf += total;
                        avail_huf += avail;
                    }
                    _ if (cb.available + cb.reserved).is_zero() => {}
                    _ => {
                        ok = false;
                        mismatches.push(format!("{b}: no FX for {}", cb.currency));
                    }
                }
            }
            let mut pos_huf = Decimal::ZERO;
            for bp in pos {
                let ccy = market::instrument(eng, &bp.instrument)
                    .await
                    .map(|i| i.quote_currency)
                    .unwrap_or(acc.account_currency);
                let px = match bp.current_price {
                    Some(p) => Some(p),
                    None => market::last_price(eng, &bp.instrument).await,
                };
                match px.and_then(|px| fx.to_huf(px * bp.quantity, ccy)) {
                    Some(v) => pos_huf += v,
                    None => {
                        ok = false;
                        mismatches.push(format!("{}: cannot value holding", bp.instrument));
                    }
                }
            }
            let total = match acc
                .total_value
                .and_then(|tv| fx.to_huf(tv, acc.account_currency))
            {
                Some(tv) => tv,
                None => cash_huf + pos_huf,
            };
            if ok {
                valued += 1;
                broker_ok = true;
                equity_huf += total;
                cash_total += cash_huf;
                per_broker.insert(
                    b.as_str().into(),
                    json!({
                        "equity_huf": total.round_dp(0).to_string(), "cash_huf": cash_huf.round_dp(0).to_string(),
                        "cash_available_huf": avail_huf.round_dp(0).to_string(), "positions_huf": pos_huf.round_dp(0).to_string(),
                        "currency": acc.account_currency.as_str(), "as_of": acc.as_of,
                    }),
                );
            }
            // Tracked positions must not exceed broker holdings (extra manual holdings are tolerated).
            for tp in eng
                .store
                .positions()
                .await
                .unwrap_or_default()
                .iter()
                .filter(|p| p.instrument.broker() == b)
            {
                let held = pos
                    .iter()
                    .find(|x| x.instrument == tp.instrument)
                    .map(|x| x.quantity)
                    .unwrap_or_default();
                if held + Decimal::new(1, 8) < tp.quantity {
                    mismatches.push(format!(
                        "{}: tracked {} > broker {}",
                        tp.instrument, tp.quantity, held
                    ));
                }
            }
        } else {
            mismatches.push(format!("{b}: account, positions or FX unavailable"));
        }
        eng.agent(
            &format!("connector_{}", b.as_str()),
            if broker_ok { "active" } else { "degraded" },
            bt,
            if broker_ok {
                "kapcsolat rendben".to_owned()
            } else {
                "számla vagy árfolyam nem elérhető".to_owned()
            },
            json!({}),
        )
        .await;
        // 3) Cash movements → ledger (only with an FX rate, so HUF amounts are never zero-filled).
        match c.cash_movements(now - Duration::days(35)).await {
            Ok(moves) => {
                for m in moves {
                    let kind = match m.kind {
                        tc_broker::CashMovementKind::Deposit => "deposit",
                        tc_broker::CashMovementKind::Withdrawal => "withdrawal",
                        tc_broker::CashMovementKind::Fee => "fee",
                        tc_broker::CashMovementKind::Interest => "interest",
                        tc_broker::CashMovementKind::Dividend => "dividend",
                    };
                    let (Some(rate), Some(huf)) = (
                        fx.as_ref().and_then(|f| f.huf_per(m.currency)),
                        fx.as_ref().and_then(|f| f.to_huf(m.amount, m.currency)),
                    ) else {
                        mismatches.push(format!("{b}: cash movement without FX ({})", m.currency));
                        continue;
                    };
                    if let Ok(true) = eng
                        .store
                        .ledger_append(
                            m.ts,
                            b.as_str(),
                            kind,
                            m.amount,
                            m.currency.as_str(),
                            huf,
                            rate,
                            &m.broker_ref,
                            None,
                        )
                        .await
                    {
                        if kind == "deposit" {
                            notify::send(
                                eng,
                                "trades",
                                "💶 Befizetés érkezett",
                                &tc_core::format_huf(huf, false).replace('\u{a0}', " "),
                            )
                            .await;
                        }
                    }
                }
            }
            Err(e) => mismatches.push(format!("{b}: cash movements unavailable ({e})")),
        }
    }

    // 4) Unitized equity snapshot — only when every configured broker was fully valued.
    if configured > 0 && valued == configured {
        let prev = eng.store.latest_snapshot().await.ok().flatten();
        let mut ledger = match &prev {
            Some(p) => UnitLedger {
                units: p.units,
                net_deposits_huf: p.net_deposits_huf,
                nav_peak: p.nav_peak,
            },
            None => UnitLedger::default(),
        };
        let cursor = if prev.is_none() {
            // First snapshot: existing capital counts as deposited (never as profit).
            ledger.apply_flow(equity_huf, Decimal::ZERO);
            eng.store.max_ledger_id().await.unwrap_or(0)
        } else {
            let since = eng.store.snapshot_ledger_cursor().await.unwrap_or(0);
            match eng.store.flows_after_ledger_id(since).await {
                Ok((flows, max_id)) => {
                    if !flows.is_zero() {
                        ledger.apply_flow(flows, equity_huf - flows);
                    }
                    max_id
                }
                Err(e) => {
                    warn!(error = %e, "flow query failed; snapshot skipped");
                    -1
                }
            }
        };
        if cursor >= 0 {
            let dd = ledger.mark(equity_huf);
            let snap = EquitySnapshot {
                ts: tc_store::now_micros(),
                equity_huf,
                cash_huf: cash_total,
                positions_huf: equity_huf - cash_total,
                net_deposits_huf: ledger.net_deposits_huf,
                units: ledger.units,
                nav_unit: ledger.nav(equity_huf),
                nav_peak: ledger.nav_peak,
                per_broker: serde_json::Value::Object(per_broker),
            };
            if let Err(e) = eng.store.insert_snapshot_with_cursor(&snap, cursor).await {
                warn!(error = %e, "snapshot write failed");
            }
            latch_loss_limits(eng, dd, snap.nav_unit).await;
            eng.event("overview", json!({ "equity_huf": equity_huf.round_dp(0) }))
                .await;
        }
    }

    let clean = mismatches.is_empty();
    {
        let mut m = eng.recon_mismatch.write().await;
        let was = m.clone();
        *m = if clean {
            None
        } else {
            Some(mismatches.join("; "))
        };
        if !clean && was.is_none() {
            notify::alert(
                eng,
                "critical_risk",
                "Egyeztetési eltérés",
                &mismatches.join("; "),
            )
            .await;
        }
    }
    if stuck_unknown {
        // Latched until the user releases it with a passkey (spec §4).
        let k = eng
            .store
            .kill_state()
            .await
            .map(|k| k.global)
            .unwrap_or(KillMode::Freeze);
        if k == KillMode::Normal {
            let _ = eng
                .store
                .set_kill(
                    "global",
                    KillMode::ReduceOnly,
                    "UNKNOWN order unresolved > 10 min",
                    "engine",
                )
                .await;
            notify::alert(
                eng,
                "critical_risk",
                "Bizonytalan megbízás",
                "Egy megbízás állapota 10 perce ismeretlen. Új belépések leállítva.",
            )
            .await;
        }
    }
    eng.agent(
        "reconciler",
        if clean { "active" } else { "error" },
        t,
        if clean {
            "Egyeztetés rendben".to_owned()
        } else {
            mismatches.join("; ")
        },
        json!({ "equity_huf": equity_huf.round_dp(0), "mismatches": mismatches }),
    )
    .await;
    clean
}

/// Drawdown halt and weekly loss are latched via the kill switch (user releases with step-up).
async fn latch_loss_limits(eng: &Engine, dd: Decimal, nav: Decimal) {
    let (_, cfg, _) = eng.risk_engine().await;
    let week0 = decide::week_start(Utc::now());
    let base = match eng.store.snapshot_at(week0).await {
        Ok(Some(s)) => Some(s.nav_unit),
        _ => eng.store.first_snapshot_since(week0).await.ok().flatten(),
    };
    let week = base
        .filter(|b| *b > Decimal::ZERO)
        .map(|b| nav / b - Decimal::ONE)
        .unwrap_or_default();
    let reason = if dd >= cfg.drawdown_halt {
        Some(format!(
            "drawdown {}% ≥ halt",
            (dd * Decimal::ONE_HUNDRED).round_dp(2)
        ))
    } else if week <= -cfg.weekly_loss {
        Some(format!(
            "weekly loss {}%",
            (week * Decimal::ONE_HUNDRED).round_dp(2)
        ))
    } else {
        None
    };
    if let Some(r) = reason {
        let k = eng
            .store
            .kill_state()
            .await
            .map(|k| k.global)
            .unwrap_or(KillMode::Freeze);
        if k == KillMode::Normal {
            let _ = eng
                .store
                .set_kill("global", KillMode::ReduceOnly, &r, "engine")
                .await;
            notify::alert(
                eng,
                "critical_risk",
                "Veszteségkorlát elérve",
                &format!("{r}. Új belépések leállítva; feloldás Face ID-val."),
            )
            .await;
        }
    }
}

/// Trading 212 has no client order id: match by ticker, side, quantity, API origin and time.
fn match_t212(r: &IntentRow, candidates: &[BrokerOrder], linked: &[String]) -> Option<BrokerOrder> {
    let t0 = r.submitted_at.unwrap_or(r.intent.created_at);
    candidates
        .iter()
        .filter(|o| {
            o.instrument == r.intent.instrument
                && o.side == r.intent.side
                && o.quantity == r.intent.quantity
                && o.via_api
                && (o.created_at - t0).num_seconds().abs() <= 120
                && !linked.contains(&o.broker_order_id)
        })
        .min_by_key(|o| (o.created_at - t0).num_milliseconds().abs())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kill_modes_block_the_right_classes() {
        assert!(!kill_blocks(
            KillMode::ReduceOnly,
            OrderClass::ExitProtective
        ));
        assert!(kill_blocks(KillMode::ReduceOnly, OrderClass::Entry));
        assert!(kill_blocks(KillMode::Flatten, OrderClass::ExitSignal));
        assert!(!kill_blocks(KillMode::Flatten, OrderClass::Flatten));
        for c in [
            OrderClass::Entry,
            OrderClass::ExitProtective,
            OrderClass::ManualSell,
            OrderClass::Flatten,
        ] {
            assert!(kill_blocks(KillMode::Freeze, c), "FREEZE must block {c:?}");
        }
    }

    fn row(qty: Decimal, state: OrderState) -> IntentRow {
        IntentRow {
            intent: OrderIntent {
                id: Uuid::new_v4(),
                instrument: InstrumentId::new(Broker::Trading212, "AAPL_US_EQ"),
                side: Side::Buy,
                kind: OrderKind::Market,
                quantity: qty,
                time_in_force: TimeInForce::Day,
                reason: IntentReason::Entry,
                decision_id: None,
                reference_price: Decimal::ONE,
                created_at: Utc::now() - Duration::seconds(5),
            },
            strategy: "x".into(),
            state,
            broker_order_id: None,
            filled_quantity: Decimal::ZERO,
            avg_fill_price: None,
            fee_paid: Decimal::ZERO,
            fee_currency: None,
            last_error: None,
            submitted_at: Some(Utc::now() - Duration::seconds(5)),
            updated_at: Utc::now(),
            version: 0,
        }
    }

    fn order(id: &str, qty: Decimal, via_api: bool, created: DateTime<Utc>) -> BrokerOrder {
        BrokerOrder {
            broker_order_id: id.into(),
            client_order_id: None,
            instrument: InstrumentId::new(Broker::Trading212, "AAPL_US_EQ"),
            side: Side::Buy,
            kind: OrderKind::Market,
            quantity: qty,
            filled_quantity: qty,
            avg_fill_price: Some(Decimal::ONE),
            fee_paid: Decimal::ZERO,
            fee_currency: None,
            status: BrokerOrderStatus::Filled,
            via_api,
            created_at: created,
            updated_at: created,
            fills: vec![],
        }
    }

    #[test]
    fn t212_matching_requires_api_origin_quantity_time_and_unlinked_id() {
        let r = row(Decimal::TWO, OrderState::Unknown);
        let t = Utc::now();
        let cands = vec![
            order("1", Decimal::TWO, false, t), // placed in the app, not API
            order("2", Decimal::ONE, true, t),  // wrong quantity
            order("3", Decimal::TWO, true, t - Duration::hours(1)), // too old
            order("4", Decimal::TWO, true, t),  // already linked elsewhere
            order("5", Decimal::TWO, true, t),
        ];
        let m = match_t212(&r, &cands, &["4".to_owned()]).expect("match");
        assert_eq!(m.broker_order_id, "5");
        assert!(match_t212(&r, &cands[..4], &["4".to_owned()]).is_none());
    }
}

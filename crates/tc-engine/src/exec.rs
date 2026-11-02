//! Execution, reconciliation and bookkeeping.
//!
//! * Attempt-before-send: the intent moves to `submitting` in a committed transaction before
//!   the HTTP request. If the DB is unavailable, nothing is sent.
//! * The kill switch is re-read from MySQL immediately before each send.
//! * An uncertain outcome becomes `unknown`; only reconciliation resolves it (never a blind
//!   retry). Bitvavo: lookup by clientOrderId; after the 5 s signing window + 30 s the order
//!   cannot exist → `not_placed`. Trading 212 (no client id): match pending orders + history by
//!   ticker, side, quantity, `initiatedFrom=API` and time; unresolved after 10 min → alert +
//!   broker REDUCE_ONLY.
//! * Every entry fill gets a broker-resting protective stop; exits cancel it first (Bitvavo
//!   reserves the balance of resting orders).

use chrono::{Duration, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;
use tc_broker::{
    BrokerClient, BrokerError, BrokerOrder, BrokerOrderStatus, OrderRef, PlaceOutcome,
};
use tc_core::{
    Broker, Instrument, InstrumentId, IntentReason, OrderIntent, OrderKind, OrderState, Side,
    TimeInForce,
};
use tc_plan::UnitLedger;
use tc_risk::{KillMode, OrderClass, RiskRequest};
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

fn kill_blocks(mode: KillMode, class: OrderClass) -> bool {
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
    // Kill switch re-read immediately before sending (no cache).
    let mode = match eng.store.kill_state().await {
        Ok(k) => k
            .global
            .max(k.brokers.get(&broker).copied().unwrap_or_default())
            .max(k.strategies.get(strategy).copied().unwrap_or_default()),
        Err(_) => KillMode::Freeze,
    };
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
    let outcome = c.place_order(intent, inst).await;
    match outcome {
        Ok(PlaceOutcome::Accepted { order }) => {
            apply_broker_order(eng, &row, &order).await;
            if intent.reason.is_entry() {
                notify::send(
                    eng,
                    "trades",
                    "🟢 Vétel",
                    &format!(
                        "{} {} @ ~{}",
                        intent.instrument.symbol(),
                        intent.quantity,
                        intent.reference_price.round_dp(4)
                    ),
                )
                .await;
            } else {
                notify::send(
                    eng,
                    "trades",
                    "🔴 Eladás",
                    &format!(
                        "{} {} ({})",
                        intent.instrument.symbol(),
                        intent.quantity,
                        intent.reason.as_str()
                    ),
                )
                .await;
            }
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

/// Apply the broker's view of an order: state transition + incremental fill booking.
pub async fn apply_broker_order(eng: &Engine, row: &IntentRow, order: &BrokerOrder) {
    let to = map_status(order.status);
    let delta = order.filled_quantity - row.filled_quantity;
    if to == row.state && delta <= Decimal::ZERO && row.broker_order_id.is_some() {
        return;
    }
    // Incremental fill price from cumulative averages.
    let prev_cost = row.avg_fill_price.unwrap_or_default() * row.filled_quantity;
    let new_cost = order.avg_fill_price.unwrap_or_default() * order.filled_quantity;
    let fee_delta = (order.fee_paid - row.fee_paid).max(Decimal::ZERO);
    let upd = IntentUpdate {
        broker_order_id: Some(order.broker_order_id.clone()),
        filled_quantity: Some(order.filled_quantity),
        avg_fill_price: order.avg_fill_price,
        fee_paid: Some(order.fee_paid),
        fee_currency: order.fee_currency.map(|c| c.to_string()),
        last_error: None,
        detail: json!({ "broker_status": format!("{:?}", order.status), "filled": order.filled_quantity }),
    };
    let target = if to == row.state && to != OrderState::PartiallyFilled {
        // Same state but new data (e.g. broker id learned while Open): nothing to transition.
        None
    } else {
        Some(to)
    };
    let updated = match target {
        Some(s) if row.state != s || s == OrderState::PartiallyFilled => {
            transition(eng, row.intent.id, s, upd).await
        }
        _ => None,
    };
    if delta > Decimal::ZERO && (updated.is_some() || target.is_none()) {
        let price = (new_cost - prev_cost) / delta;
        let fill_id = format!(
            "{}@{}",
            order.broker_order_id,
            order.filled_quantity.normalize()
        );
        let new = eng
            .store
            .insert_fill(
                row.intent.instrument.broker(),
                &fill_id,
                Some(row.intent.id),
                &row.intent.instrument,
                row.intent.side,
                delta,
                price,
                fee_delta,
                order.fee_currency.map(|c| c.to_string()).as_deref(),
                Utc::now(),
            )
            .await
            .unwrap_or(false);
        if new {
            book_fill(eng, row, delta, price, fee_delta).await;
        }
    }
}

/// Book an incremental fill into positions, trades and the ledger.
async fn book_fill(eng: &Engine, row: &IntentRow, qty: Decimal, price: Decimal, fee: Decimal) {
    let id = &row.intent.instrument;
    let meta = eng
        .store
        .intent_meta(row.intent.id)
        .await
        .unwrap_or_default();
    let plan = meta.get("plan").cloned().unwrap_or_default();
    let long_term = plan
        .get("long_term")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let strategy = row.strategy.clone();
    let inst = market::instrument(eng, id).await;
    let ccy = inst
        .as_ref()
        .map(|i| i.quote_currency)
        .unwrap_or(tc_core::Currency::EUR);
    let fx = eng.fx.read().await.clone();
    let to_huf = |x: Decimal| {
        fx.as_ref()
            .and_then(|f| f.to_huf(x, ccy))
            .unwrap_or_default()
    };
    let fx_rate = fx.as_ref().and_then(|f| f.huf_per(ccy)).unwrap_or_default();
    let now = tc_store::now_micros();
    if fee > Decimal::ZERO {
        let _ = eng
            .store
            .ledger_append(
                now,
                id.broker().as_str(),
                "fee",
                -fee,
                ccy.as_str(),
                -to_huf(fee),
                fx_rate,
                &format!("{}:{}", row.intent.id, qty.normalize()),
                None,
            )
            .await;
    }
    let positions = eng.store.positions().await.unwrap_or_default();
    let existing = positions.into_iter().find(|p| &p.instrument == id);
    match row.intent.side {
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
                    opened_at: now,
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
                    strategy,
                    long_term,
                    bars_held: 0,
                    partial_taken: false,
                    stop_intent_id: None,
                    decision_id: row.intent.decision_id.map(|d| d.to_string()),
                },
            };
            let _ = eng.store.upsert_position(&p).await;
            if !p.long_term && p.stop_price.is_some() {
                Box::pin(ensure_protective_stop(eng, id)).await;
            }
        }
        Side::Sell => {
            let Some(mut p) = existing else {
                error!(instrument = %id, "sell fill without tracked position");
                *eng.recon_mismatch.write().await =
                    Some(format!("sell fill without position on {id}"));
                return;
            };
            let (bought, buy_fees) = eng
                .store
                .buy_fills_since(id, p.opened_at)
                .await
                .unwrap_or_default();
            let entry_fee_share = if bought > Decimal::ZERO {
                buy_fees * qty / bought
            } else {
                Decimal::ZERO
            };
            let gross = (price - p.avg_price) * qty;
            let fees = fee + entry_fee_share;
            let net = gross - fees;
            let stopped = matches!(
                row.intent.reason,
                IntentReason::StopLoss | IntentReason::TrailingStop
            );
            let trade = TradeRow {
                id: 0,
                instrument: id.as_str().to_owned(),
                strategy: p.strategy.clone(),
                entry_at: p.opened_at,
                exit_at: now,
                entry_price: p.avg_price,
                exit_price: price,
                quantity: qty,
                gross_quote: gross,
                fees_quote: fees,
                net_quote: net,
                net_huf: to_huf(net),
                fees_huf: to_huf(fees),
                return_pct: if p.avg_price > Decimal::ZERO {
                    (price / p.avg_price - Decimal::ONE) * Decimal::ONE_HUNDRED
                } else {
                    Decimal::ZERO
                },
                exit_reason: row.intent.reason.as_str().to_owned(),
                stopped_out: stopped,
                decision_id: p.decision_id.clone(),
                forecast: json!(null),
            };
            let _ = eng.store.insert_trade(&trade).await;
            let _ = eng
                .store
                .ledger_append(
                    now,
                    id.broker().as_str(),
                    "realized_pnl",
                    net,
                    ccy.as_str(),
                    to_huf(net),
                    fx_rate,
                    &format!("{}:{}", row.intent.id, qty.normalize()),
                    None,
                )
                .await;
            p.quantity -= qty;
            if p.quantity <= Decimal::ZERO
                || inst.as_ref().is_some_and(|i| p.quantity < i.min_quantity)
            {
                let _ = eng.store.delete_position(id).await;
                cancel_resting_sells(eng, id).await;
            } else {
                if row.intent.reason == IntentReason::PartialExit {
                    p.partial_taken = true;
                }
                let _ = eng.store.upsert_position(&p).await;
                Box::pin(ensure_protective_stop(eng, id)).await;
            }
            eng.event(
                "position",
                json!({ "instrument": id.as_str(), "realized_net_huf": to_huf(net).round_dp(0) }),
            )
            .await;
        }
    }
}

/// Resting protective sell orders for an instrument.
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

/// Cancel resting sell orders (stops) before any other exit. Returns false if a cancel failed.
async fn cancel_resting_sells(eng: &Engine, id: &InstrumentId) -> bool {
    let Some(c) = client(eng, id.broker()).await else {
        return false;
    };
    let mut ok = true;
    for r in resting_sells(eng, id).await {
        let oref = OrderRef {
            instrument: id.clone(),
            broker_order_id: r.broker_order_id.clone(),
            client_order_id: Some(r.intent.id),
        };
        match c.cancel_order(&oref).await {
            Ok(()) => {
                transition(
                    eng,
                    r.intent.id,
                    OrderState::Cancelled,
                    IntentUpdate::default(),
                )
                .await;
            }
            Err(e) => {
                ok = false;
                warn!(instrument = %id, error = %e, "cancel of resting order failed");
            }
        }
    }
    ok
}

/// Place a broker-resting stop for the tracked position if none is resting or in flight.
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
    let Some(stop) = p.stop_price else { return };
    let active = eng
        .store
        .active_and_recent_intents(Utc::now())
        .await
        .unwrap_or_default();
    let mine = active
        .iter()
        .filter(|r| &r.intent.instrument == id && !r.state.is_terminal());
    for r in mine {
        // A sell is already resting/in flight (an existing stop), or any order is uncertain.
        if r.intent.side == Side::Sell
            || matches!(
                r.state,
                OrderState::Pending | OrderState::Submitting | OrderState::Unknown
            )
        {
            return;
        }
    }
    let Some(inst) = market::instrument(eng, id).await else {
        return;
    };
    let qty = inst.floor_quantity(p.quantity);
    let stop_px = inst.round_price(stop, Side::Sell);
    let intent = OrderIntent {
        id: Uuid::new_v4(),
        instrument: id.clone(),
        side: Side::Sell,
        kind: OrderKind::Stop { stop: stop_px },
        quantity: qty,
        time_in_force: TimeInForce::Gtc,
        reason: IntentReason::StopLoss,
        decision_id: None,
        reference_price: stop_px,
        created_at: tc_store::now_micros(),
    };
    if risk_ok(eng, &intent, &inst, &p.strategy).await.is_some()
        && eng
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

/// Risk-check an exit (protective classes only face exit-side checks). Returns the approved qty.
async fn risk_ok(
    eng: &Engine,
    intent: &OrderIntent,
    inst: &Instrument,
    strategy: &str,
) -> Option<Decimal> {
    let quote = decide::quote_for(eng, &intent.instrument).await;
    let snap = decide::build_snapshot(
        eng,
        &intent.instrument,
        strategy,
        "live",
        quote.as_ref(),
        Utc::now(),
    )
    .await;
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

/// Exit (part of) a position: cancel resting stop first, then a market sell.
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
    let Some(inst) = market::instrument(eng, id).await else {
        return json!({ "ok": false, "error": "no instrument metadata" });
    };
    if !cancel_resting_sells(eng, id).await {
        notify::alert(
            eng,
            "critical_risk",
            "Kilépés sikertelen",
            &format!("{id}: a védő stop nem vonható vissza; a stop aktív marad."),
        )
        .await;
        return json!({ "ok": false, "error": "could not cancel resting stop" });
    }
    let mut q = inst.floor_quantity(qty.min(p.quantity));
    // A remainder below the venue minimum is sold in full.
    if p.quantity - q < inst.min_quantity {
        q = inst.floor_quantity(p.quantity);
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
    let Some(approved) = risk_ok(eng, &intent, &inst, &p.strategy).await else {
        Box::pin(ensure_protective_stop(eng, id)).await;
        return json!({ "ok": false, "error": "risk engine veto" });
    };
    let intent = OrderIntent {
        quantity: approved,
        ..intent
    };
    if eng
        .store
        .insert_intent(&intent, &p.strategy, &json!({ "plan": { "exit": text } }))
        .await
        .is_err()
    {
        return json!({ "ok": false, "error": "persist failed" });
    }
    execute(eng, &intent, &inst, &p.strategy).await;
    json!({ "ok": true, "intent_id": intent.id, "quantity": approved })
}

/// Trailing-stop update: replace the resting broker stop at a higher level.
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
    if p.stop_price.is_some_and(|s| new_stop <= s) {
        return;
    }
    p.stop_price = Some(new_stop);
    let _ = eng.store.upsert_position(&p).await;
    if cancel_resting_sells(eng, id).await {
        Box::pin(ensure_protective_stop(eng, id)).await;
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
        let (ok, result) = match cmd.kind.as_str() {
            "verify_credentials" => {
                let b = cmd
                    .payload
                    .get("broker")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned();
                (true, crate::creds::verify(eng, &b).await)
            }
            "manual_sell" => {
                let id = cmd
                    .payload
                    .get("instrument")
                    .and_then(|v| v.as_str())
                    .and_then(|s| InstrumentId::parse(s).ok());
                let q = cmd
                    .payload
                    .get("quantity")
                    .and_then(|v| v.as_str())
                    .unwrap_or("all")
                    .to_owned();
                match id {
                    Some(id) => {
                        let pos_q = eng
                            .store
                            .positions()
                            .await
                            .unwrap_or_default()
                            .into_iter()
                            .find(|p| p.instrument == id)
                            .map(|p| p.quantity);
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
                    None => (false, json!({ "error": "bad instrument" })),
                }
            }
            "flatten" => (true, flatten(eng).await),
            other => (
                false,
                json!({ "error": format!("unknown command {other}") }),
            ),
        };
        let _ = eng.store.finish_command(cmd.id, ok, &result).await;
        eng.event(
            "command",
            json!({ "id": cmd.id, "kind": cmd.kind, "ok": ok, "result": result }),
        )
        .await;
    }
}

/// Emergency flatten of trading positions (long-term ETF core is kept).
async fn flatten(eng: &Engine) -> serde_json::Value {
    let mut out = Vec::new();
    // Cancel open entry orders first.
    for r in eng
        .store
        .active_and_recent_intents(Utc::now())
        .await
        .unwrap_or_default()
    {
        if r.intent.side == Side::Buy
            && matches!(r.state, OrderState::Open | OrderState::PartiallyFilled)
        {
            if let Some(c) = client(eng, r.intent.instrument.broker()).await {
                let oref = OrderRef {
                    instrument: r.intent.instrument.clone(),
                    broker_order_id: r.broker_order_id.clone(),
                    client_order_id: Some(r.intent.id),
                };
                if c.cancel_order(&oref).await.is_ok() {
                    transition(
                        eng,
                        r.intent.id,
                        OrderState::Cancelled,
                        IntentUpdate::default(),
                    )
                    .await;
                }
            }
        }
    }
    reconcile(eng).await;
    for p in eng
        .store
        .positions()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|p| !p.long_term)
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
        &format!("{} pozíció zárása elindítva", out.len()),
    )
    .await;
    json!({ "positions": out })
}

// ---------------------------------------------------------------- exits
pub async fn manage_exits(eng: &Engine) {
    let t = Instant::now();
    let positions = eng.store.positions().await.unwrap_or_default();
    let mut checked = 0;
    for p in positions.iter().filter(|p| !p.long_term) {
        checked += 1;
        ensure_protective_stop(eng, &p.instrument).await;
        // Safety net when no broker stop is resting (e.g. placement failed).
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
    // Maker entry limits expire after one bar (S3: abstain if unfilled).
    for r in eng
        .store
        .active_and_recent_intents(Utc::now())
        .await
        .unwrap_or_default()
    {
        if r.intent.reason == IntentReason::Entry
            && r.state == OrderState::Open
            && Utc::now() - r.intent.created_at > Duration::hours(24)
        {
            if let Some(c) = client(eng, r.intent.instrument.broker()).await {
                let oref = OrderRef {
                    instrument: r.intent.instrument.clone(),
                    broker_order_id: r.broker_order_id.clone(),
                    client_order_id: Some(r.intent.id),
                };
                if c.cancel_order(&oref).await.is_ok() {
                    transition(
                        eng,
                        r.intent.id,
                        OrderState::Cancelled,
                        IntentUpdate::default(),
                    )
                    .await;
                }
            }
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
/// Reconcile intents, positions, cash movements; write an equity snapshot.
/// Returns true when clean (no mismatch, no UNKNOWN older than 10 min).
pub async fn reconcile(eng: &Engine) -> bool {
    let t = Instant::now();
    let mut mismatches: Vec<String> = Vec::new();
    let mut per_broker = serde_json::Map::new();
    let mut equity_huf = Decimal::ZERO;
    let mut cash_total = Decimal::ZERO;
    let fx = eng.fx.read().await.clone();
    let now = Utc::now();
    let mut stuck_unknown = false;
    let mut any_broker = false;
    for b in [Broker::Bitvavo, Broker::Trading212] {
        let Some(c) = client(eng, b).await else {
            continue;
        };
        any_broker = true;
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
        let history = if b == Broker::Trading212
            && intents.iter().any(|r| {
                r.intent.instrument.broker() == b
                    && r.broker_order_id.is_none()
                    && !r.state.is_terminal()
            }) {
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
                let oref = OrderRef {
                    instrument: r.intent.instrument.clone(),
                    broker_order_id: r.broker_order_id.clone(),
                    client_order_id: (b == Broker::Bitvavo).then_some(r.intent.id),
                };
                c.get_order(&oref).await
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
                        apply_broker_order(eng, &row, &order).await;
                    }
                }
                Ok(None) => {
                    let window = if b == Broker::Bitvavo {
                        Duration::seconds(35)
                    } else {
                        Duration::minutes(10)
                    };
                    if matches!(r.state, OrderState::Unknown | OrderState::Submitting) {
                        if age > window && b == Broker::Bitvavo {
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
                        } else if age > window {
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
        // 2) Account, positions.
        let account = c.account().await;
        let positions = c.positions().await;
        if let (Ok(acc), Ok(pos)) = (&account, &positions) {
            eng.last_account_sync.write().await.insert(b, Utc::now());
            let cash_huf: Decimal = acc
                .cash
                .iter()
                .filter_map(|cb| {
                    fx.as_ref()
                        .and_then(|f| f.to_huf(cb.available + cb.reserved, cb.currency))
                })
                .sum();
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
                if let Some(px) = px {
                    pos_huf += fx
                        .as_ref()
                        .and_then(|f| f.to_huf(px * bp.quantity, ccy))
                        .unwrap_or_default();
                }
            }
            let total = match acc.total_value {
                Some(tv) => fx
                    .as_ref()
                    .and_then(|f| f.to_huf(tv, acc.account_currency))
                    .unwrap_or(cash_huf + pos_huf),
                None => cash_huf + pos_huf,
            };
            equity_huf += total;
            cash_total += cash_huf;
            per_broker.insert(
                b.as_str().into(),
                json!({ "equity_huf": total.round_dp(0).to_string(), "cash_huf": cash_huf.round_dp(0).to_string(), "positions_huf": pos_huf.round_dp(0).to_string(), "currency": acc.account_currency.as_str(), "as_of": acc.as_of }),
            );
            // Tracked positions must not exceed what the broker holds (manual extra holdings are tolerated).
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
            mismatches.push(format!("{b}: account/positions unavailable"));
        }
        // 3) Cash movements → ledger (deposits tracked separately from profit).
        if let Ok(moves) = c.cash_movements(now - Duration::days(35)).await {
            for m in moves {
                let kind = match m.kind {
                    tc_broker::CashMovementKind::Deposit => "deposit",
                    tc_broker::CashMovementKind::Withdrawal => "withdrawal",
                    tc_broker::CashMovementKind::Fee => "fee",
                    tc_broker::CashMovementKind::Interest => "interest",
                    tc_broker::CashMovementKind::Dividend => "dividend",
                };
                let rate = fx
                    .as_ref()
                    .and_then(|f| f.huf_per(m.currency))
                    .unwrap_or_default();
                let huf = fx
                    .as_ref()
                    .and_then(|f| f.to_huf(m.amount, m.currency))
                    .unwrap_or_default();
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
    }

    // 4) Unitized equity snapshot.
    if any_broker && !per_broker.is_empty() {
        let prev = eng.store.latest_snapshot().await.ok().flatten();
        let mut ledger = match &prev {
            Some(p) => UnitLedger {
                units: p.units,
                net_deposits_huf: p.net_deposits_huf,
                nav_peak: p.nav_peak,
            },
            None => UnitLedger::default(),
        };
        let since = prev
            .as_ref()
            .map(|p| p.ts)
            .unwrap_or(now - Duration::days(3650));
        let flows = eng
            .store
            .net_flows_huf(since, now + Duration::seconds(1))
            .await
            .unwrap_or_default();
        if prev.is_none() {
            // First snapshot: existing capital counts as deposited (never as profit).
            ledger.apply_flow(equity_huf, Decimal::ZERO);
        } else if !flows.is_zero() {
            ledger.apply_flow(flows, equity_huf - flows);
        }
        ledger.mark(equity_huf);
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
        let _ = eng.store.insert_snapshot(&snap).await;
        eng.event("overview", json!({ "equity_huf": equity_huf.round_dp(0) }))
            .await;
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
            if stuck_unknown {
                let _ = eng
                    .store
                    .set_kill(
                        "global",
                        KillMode::ReduceOnly,
                        "UNKNOWN order unresolved > 10 min",
                        "engine",
                    )
                    .await;
            }
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

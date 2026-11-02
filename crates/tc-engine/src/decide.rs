//! Decision cycle: strategy agents → opinions → fusion (abstain unless edge > costs +
//! uncertainty) → sizing → deterministic risk engine (absolute veto) → persisted intent →
//! execution. Every candidate (taken or abstained) is recorded as an explainable decision.

use chrono::{Datelike, Duration, Utc};
use chrono_tz::Europe::Budapest;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Instant;
use tc_core::{
    AgentKind, AssetClass, Broker, Decision, DecisionAction, Direction, Forecast, InstrumentId,
    IntentReason, Interval, OrderIntent, OrderKind, Side, Signal, TimeInForce,
};
use tc_plan::PlanConfig;
use tc_risk::{
    BrokerHealth, ClosedTrade, CredentialStatus, InstrumentStatus, IntentSummary, KillMode,
    MarketData, OrderClass, PositionExposure, RiskRequest, RiskSnapshot,
};
use tc_strategy::sizing::{self, Sizing, SizingInput};
use tc_strategy::{fuse, Action, FusionParams, OpenTrade, Opinion, Series};
use tracing::warn;
use uuid::Uuid;

use crate::market::{self, to_f64};
use crate::{exec, Engine, Mode};

fn dec(x: f64) -> Decimal {
    Decimal::from_f64(x).unwrap_or_default()
}

/// Market-group key for strategy edge statistics.
pub fn market_group(id: &InstrumentId) -> &'static str {
    match id.broker() {
        Broker::Bitvavo => "bitvavo_eur",
        _ => "t212_us_equity",
    }
}

pub async fn cycle(eng: &Engine) {
    let t = Instant::now();
    let mut ids = eng.universe.read().await.clone();
    for p in eng.store.positions().await.unwrap_or_default() {
        if !p.long_term && !ids.contains(&p.instrument) {
            ids.push(p.instrument.clone());
        }
    }
    for sym in market::STOCK_UNIVERSE {
        let id = InstrumentId::new(Broker::Trading212, &format!("{sym}_US_EQ"));
        if eng
            .store
            .candles(&id, Interval::D1, 1)
            .await
            .map(|c| !c.is_empty())
            .unwrap_or(false)
        {
            ids.push(id);
        }
    }
    let strategies = eng.store.strategies().await.unwrap_or_default();
    let mut considered = 0;
    let mut entries = 0;
    let mut abstained = 0;
    for id in ids {
        match decide_instrument(eng, &id, &strategies).await {
            Ok(Some(true)) => {
                considered += 1;
                entries += 1
            }
            Ok(Some(false)) => {
                considered += 1;
                abstained += 1
            }
            Ok(None) => {}
            Err(e) => {
                warn!(instrument = %id, error = %e, "decision failed");
                // Retry this bar on the next cycle.
                eng.last_decision_bar.lock().await.remove(&id);
            }
        }
    }
    let live: Vec<&str> = strategies
        .iter()
        .filter(|s| s.status == "live" || s.status == "live_half")
        .map(|s| s.name.as_str())
        .collect();
    eng.agent(
        "ranker",
        "active",
        t,
        format!("{considered} candidate(s): {entries} sent to execution, {abstained} abstained"),
        json!({ "live_strategies": live }),
    )
    .await;
    bot_status(eng).await;
}

/// Returns Some(true) if an entry intent was created, Some(false) if a candidate abstained,
/// None if nothing to decide.
async fn decide_instrument(
    eng: &Engine,
    id: &InstrumentId,
    strategies: &[tc_store::trading::StrategyRow],
) -> anyhow::Result<Option<bool>> {
    let candles = eng.store.candles(id, Interval::D1, 700).await?;
    let Some(last) = candles.last() else {
        return Ok(None);
    };
    {
        let mut seen = eng.last_decision_bar.lock().await;
        if seen.get(id) == Some(&last.ts) {
            return Ok(None);
        }
        seen.insert(id.clone(), last.ts);
    }
    let s = Series::from_candles(&candles);
    let regime = eng
        .regimes
        .lock()
        .await
        .get(id)
        .map(|r| r.current)
        .unwrap_or(tc_strategy::Regime::Transition);
    let position = eng
        .store
        .positions()
        .await?
        .into_iter()
        .find(|p| &p.instrument == id && !p.long_term);
    let now = Utc::now();
    let horizon_end =
        last.ts + Duration::seconds(2 * Interval::D1.seconds()) + Duration::minutes(10);

    // 1) Manage an open position with its owning strategy.
    if let Some(p) = &position {
        if let Some(st) = tc_strategy::registry()
            .into_iter()
            .find(|x| x.name() == p.strategy)
        {
            let ot = OpenTrade {
                entry_price: to_f64(p.avg_price),
                stop: p.stop_price.map(to_f64).unwrap_or(0.0),
                initial_risk: p.initial_risk.map(to_f64).unwrap_or(0.0),
                high_since_entry: to_f64(p.high_watermark)
                    .max(s.high.last().copied().unwrap_or(0.0)),
                bars_held: p.bars_held.max(0) as u32 + 1,
                partial_taken: p.partial_taken,
            };
            let mut np = p.clone();
            np.bars_held += 1;
            np.high_watermark = dec(ot.high_since_entry);
            eng.store.upsert_position(&np).await?;
            match st.evaluate(&s, regime, Some(&ot)) {
                Action::Exit {
                    reason,
                    fraction,
                    rationale,
                } => {
                    let qty = if fraction >= 1.0 {
                        p.quantity
                    } else {
                        p.quantity * dec(fraction)
                    };
                    let ir = if fraction < 1.0 {
                        IntentReason::PartialExit
                    } else {
                        IntentReason::StrategyExit
                    };
                    exec::exit_position(eng, id, qty, ir, &format!("{reason:?}: {rationale}"))
                        .await;
                }
                Action::UpdateStop { stop } => exec::raise_stop(eng, id, dec(stop)).await,
                _ => {}
            }
        }
        return Ok(None);
    }

    // 2) Entry candidates from every strategy (approved or not, for transparency).
    let mut signals = Vec::new();
    let mut opinions = Vec::new();
    let mut best: Option<(String, f64, Option<f64>, String)> = None; // (strategy, stop, limit, rationale)
    for st in tc_strategy::registry() {
        let Action::Enter {
            stop,
            limit,
            strength,
            rationale,
        } = st.evaluate(&s, regime, None)
        else {
            continue;
        };
        let row = strategies.iter().find(|r| r.name == st.name());
        let edges = eng
            .store
            .edges(st.name(), market_group(id))
            .await
            .unwrap_or_default();
        let edge = edges.iter().find(|e| e.regime == regime.as_str());
        let (mu, se, p_win) = match edge {
            Some(e) if e.trades >= 5 => (e.mean_gross_bps, e.se_bps, Some(e.win_rate)),
            _ => (0.0, f64::INFINITY, None),
        };
        let reliability = if row.is_some_and(|r| r.status == "live") {
            1.0
        } else {
            0.5
        };
        signals.push(Signal {
            agent: AgentKind::Technical,
            strategy: st.name().to_owned(),
            instrument: id.clone(),
            direction: Direction::Long,
            strength,
            confidence: p_win.unwrap_or(0.0),
            expected_edge_bps: mu,
            edge_stderr_bps: if se.is_finite() { se } else { -1.0 },
            horizon_bars: 1,
            generated_at: now,
            data_as_of: last.ts,
            valid_until: horizon_end,
            rationale: rationale.clone(),
        });
        opinions.push(Opinion {
            agent: st.name().to_owned(),
            direction: 1,
            mu_bps: mu,
            se_bps: se,
            reliability,
            p_win,
            as_of: last.ts,
            valid_until: horizon_end,
            veto_only: false,
        });
        if best.is_none() {
            best = Some((st.name().to_owned(), stop, limit, rationale));
        }
    }
    let Some((strategy, stop, limit, why)) = best else {
        return Ok(None);
    };

    // Volatility / shock agent: veto-only.
    if let Some((vz, zr)) = tc_strategy::shock(&s) {
        if vz >= 3.0 || zr.abs() >= 4.0 {
            opinions.push(Opinion {
                agent: "volatility".into(),
                direction: -1,
                mu_bps: 0.0,
                se_bps: 1.0,
                reliability: 1.0,
                p_win: None,
                as_of: last.ts,
                valid_until: horizon_end,
                veto_only: true,
            });
            signals.push(Signal {
                agent: AgentKind::Volatility,
                strategy: "shock".into(),
                instrument: id.clone(),
                direction: Direction::Flat,
                strength: -1.0,
                confidence: 1.0,
                expected_edge_bps: 0.0,
                edge_stderr_bps: 0.0,
                horizon_bars: 2,
                generated_at: now,
                data_as_of: last.ts,
                valid_until: horizon_end,
                rationale: format!("shock: volume z {vz:.1}, return z {zr:.1}"),
            });
        }
    }

    // Costs: both legs taker + spread + slippage (+ FX for non-EUR equities).
    let quote = quote_for(eng, id).await;
    let spread_bps = quote.as_ref().and_then(|q| match (q.bid, q.ask) {
        (Some(b), Some(a)) if b > Decimal::ZERO && a >= b => Some(to_f64(
            (a - b) / ((a + b) / Decimal::TWO) * Decimal::from(10_000),
        )),
        _ => None,
    });
    let taker = to_f64(
        *eng.fees
            .read()
            .await
            .get(&id.broker())
            .unwrap_or(&default_fee(id.broker())),
    );
    let (_, rc, _) = eng.risk_engine().await;
    let fx_bps = if id.broker() == Broker::Trading212 {
        2.0 * to_f64(rc.fx_fee_bps)
    } else {
        0.0
    };
    let cost_bps =
        2.0 * taker * 1e4 + spread_bps.unwrap_or(10.0) + 2.0 * to_f64(rc.slippage_bps) + fx_bps;
    let fused = fuse(&opinions, cost_bps, now, FusionParams::default());

    let mut rationale = vec![
        format!("{strategy}: {why}"),
        format!("regime {}", regime.as_str()),
    ];
    rationale.extend(fused.notes.iter().cloned());
    let forecast = Forecast {
        p_win: fused.p_win,
        expected_net_bps: fused.net_edge_bps,
        cost_bps,
        uncertainty_bps: fused.sigma_hat_bps,
        p05_bps: fused.net_edge_bps - 1.645 * fused.sigma_hat_bps,
        p95_bps: fused.net_edge_bps + 1.645 * fused.sigma_hat_bps,
    };
    let mut decision = Decision {
        id: Uuid::new_v4(),
        instrument: id.clone(),
        action: DecisionAction::Abstain,
        strategy: strategy.clone(),
        confidence: fused.confidence,
        forecast,
        signals,
        rationale,
        created_at: now,
    };

    if !fused.enter {
        decision
            .rationale
            .push(format!("abstain: {:?}", fused.abstain));
        eng.store.insert_decision(&decision, None).await?;
        eng.event(
            "decision",
            json!({ "id": decision.id, "instrument": id.as_str(), "action": "abstain" }),
        )
        .await;
        return Ok(Some(false));
    }
    if eng.mode().await == Mode::Recovery {
        decision
            .rationale
            .push("abstain: engine in RECOVERY (reconciliation pending)".into());
        eng.store.insert_decision(&decision, None).await?;
        return Ok(Some(false));
    }

    // Sizing in quote currency.
    let inst = market::instrument(eng, id)
        .await
        .ok_or_else(|| anyhow::anyhow!("instrument metadata missing"))?;
    let price = quote
        .as_ref()
        .and_then(|q| q.ask.or(q.last))
        .unwrap_or(last.close);
    let fx = eng.fx.read().await.clone();
    let huf_per = fx
        .as_ref()
        .and_then(|f| f.huf_per(inst.quote_currency))
        .unwrap_or_default();
    let snap = eng.store.latest_snapshot().await?;
    let trading_equity_huf = trading_equity(eng, snap.as_ref()).await;
    let status = strategies
        .iter()
        .find(|r| r.name == strategy)
        .map(|r| r.status.clone())
        .unwrap_or_default();
    let risk_frac = if status == "live" {
        rc.risk_per_trade
    } else {
        rc.risk_per_trade / Decimal::TWO
    };
    let cash_huf = broker_cash_huf(eng, id.broker()).await;
    let sz = if huf_per > Decimal::ZERO {
        sizing::size(&SizingInput {
            equity: trading_equity_huf / huf_per,
            price,
            stop: dec(stop),
            risk_frac,
            risk_frac_max: risk_frac * Decimal::new(125, 2),
            max_position_frac: rc.position_cap,
            vol_cap_notional: None,
            cash: cash_huf / huf_per,
            min_notional: inst.min_notional,
            min_base: inst.min_quantity,
            quantity_decimals: inst.quantity_decimals,
        })
    } else {
        Sizing::Abstain {
            reason: "no FX rate".into(),
        }
    };
    let qty = match sz {
        Sizing::Trade { quantity, .. } => quantity,
        Sizing::Abstain { reason } => {
            decision.rationale.push(format!("abstain: {reason}"));
            eng.store.insert_decision(&decision, None).await?;
            return Ok(Some(false));
        }
    };

    let req = RiskRequest {
        instrument: id.clone(),
        asset_class: inst.asset_class,
        side: Side::Buy,
        class: OrderClass::Entry,
        strategy: strategy.clone(),
        quantity: qty,
        price,
        stop_price: Some(dec(stop)),
        gap_p99: gap_p99(&s),
        edge_bps: dec(fused.mu_hat_bps),
        edge_se_bps: dec(fused.sigma_hat_bps),
        size_multiplier: Decimal::ONE,
        cluster: cluster(id, inst.asset_class),
        created_at: now,
    };
    let snapshot = build_snapshot(eng, id, &strategy, &status, quote.as_ref(), last.ts).await;
    let (engine, _, _) = eng.risk_engine().await;
    let verdict = engine.evaluate(&req, &snapshot, Utc::now());
    let verdict_json = serde_json::to_value(&verdict).unwrap_or_default();
    let Some(approved) = verdict.approved_quantity() else {
        decision.rationale.push("risk engine veto".into());
        eng.store
            .insert_decision(&decision, Some(&verdict_json))
            .await?;
        eng.agent(
            "risk",
            "active",
            Instant::now(),
            format!("veto {}: {:?}", id, verdict.codes()),
            risk_metrics(&snapshot),
        )
        .await;
        return Ok(Some(false));
    };
    decision.action = DecisionAction::Enter;
    eng.store
        .insert_decision(&decision, Some(&verdict_json))
        .await?;
    eng.agent(
        "risk",
        "active",
        Instant::now(),
        format!("allow {id} qty {approved}"),
        risk_metrics(&snapshot),
    )
    .await;

    let intent = OrderIntent {
        id: Uuid::new_v4(),
        instrument: id.clone(),
        side: Side::Buy,
        kind: match limit {
            Some(l) => OrderKind::Limit {
                price: inst.round_price(dec(l), Side::Buy),
            },
            None => OrderKind::Market,
        },
        quantity: approved,
        time_in_force: TimeInForce::Gtc,
        reason: IntentReason::Entry,
        decision_id: Some(decision.id),
        reference_price: price,
        created_at: tc_store::now_micros(),
    };
    let meta = json!({ "verdict": verdict_json, "plan": { "stop": dec(stop), "strategy": strategy, "initial_risk": price - dec(stop), "decision_id": decision.id } });
    eng.store.insert_intent(&intent, &strategy, &meta).await?;
    exec::execute(eng, &intent, &inst, &strategy).await;
    Ok(Some(true))
}

fn default_fee(b: Broker) -> Decimal {
    match b {
        Broker::Bitvavo => Decimal::new(25, 4),
        _ => Decimal::ZERO,
    }
}

fn cluster(id: &InstrumentId, ac: AssetClass) -> String {
    match ac {
        AssetClass::Crypto => "crypto".into(),
        _ => format!("{}_equity", id.broker().as_str()),
    }
}

fn gap_p99(s: &Series) -> Decimal {
    // 99th percentile of adverse open-vs-prior-close gaps over the history.
    let mut gaps: Vec<f64> = (1..s.len())
        .map(|i| (1.0 - s.open[i] / s.close[i - 1]).max(0.0))
        .collect();
    if gaps.is_empty() {
        return Decimal::new(3, 2);
    }
    gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    dec(gaps[((gaps.len() as f64) * 0.99) as usize % gaps.len()])
}

pub async fn quote_for(eng: &Engine, id: &InstrumentId) -> Option<tc_broker::Quote> {
    use tc_broker::MarketDataSource;
    match id.broker() {
        Broker::Bitvavo => eng.public_bitvavo.quote(id).await.ok(),
        Broker::Trading212 => {
            let md = eng.brokers.read().await.eodhd.clone()?;
            md.quote(id).await.ok()
        }
        Broker::Revolut => None,
    }
}

/// Trading equity in HUF: total equity minus the long-term ETF core.
pub async fn trading_equity(
    eng: &Engine,
    snap: Option<&tc_store::trading::EquitySnapshot>,
) -> Decimal {
    let total = snap.map(|s| s.equity_huf).unwrap_or_default();
    let mut lt = Decimal::ZERO;
    let fx = eng.fx.read().await.clone();
    for p in eng
        .store
        .positions()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.long_term)
    {
        if let (Some(px), Some(inst)) = (
            market::last_price(eng, &p.instrument).await,
            market::instrument(eng, &p.instrument).await,
        ) {
            if let Some(h) = fx
                .as_ref()
                .and_then(|f| f.to_huf(px * p.quantity, inst.quote_currency))
            {
                lt += h;
            }
        }
    }
    (total - lt).max(Decimal::ZERO)
}

pub async fn broker_cash_huf(eng: &Engine, broker: Broker) -> Decimal {
    let snap = eng.store.latest_snapshot().await.ok().flatten();
    snap.and_then(|s| {
        s.per_broker
            .get(broker.as_str())
            .and_then(|b| b.get("cash_available_huf"))
            .and_then(|v| v.as_str())
            .and_then(|v| v.parse().ok())
    })
    .unwrap_or_default()
}

fn risk_metrics(s: &RiskSnapshot) -> serde_json::Value {
    let total: Decimal = s
        .positions
        .iter()
        .filter(|p| !p.long_term)
        .map(|p| p.value_huf)
        .sum();
    let crypto: Decimal = s
        .positions
        .iter()
        .filter(|p| p.asset_class == AssetClass::Crypto)
        .map(|p| p.value_huf)
        .sum();
    let pct = |x: Decimal| {
        if s.equity_huf > Decimal::ZERO {
            (x / s.equity_huf * Decimal::ONE_HUNDRED).round_dp(1)
        } else {
            Decimal::ZERO
        }
    };
    json!({
        "exposure_pct": pct(total),
        "crypto_pct": pct(crypto),
        "equity_pct": pct(total - crypto),
        "day_pnl_pct": if s.nav_unit_day_start > Decimal::ZERO { ((s.nav_unit / s.nav_unit_day_start - Decimal::ONE) * Decimal::ONE_HUNDRED).round_dp(2) } else { Decimal::ZERO },
        "drawdown_pct": if s.nav_unit_peak > Decimal::ZERO { ((Decimal::ONE - s.nav_unit / s.nav_unit_peak) * Decimal::ONE_HUNDRED).round_dp(2) } else { Decimal::ZERO },
        "entries_today": s.entries_today,
    })
}

/// Capture everything the risk engine needs at one instant.
pub async fn build_snapshot(
    eng: &Engine,
    id: &InstrumentId,
    strategy: &str,
    strategy_status: &str,
    quote: Option<&tc_broker::Quote>,
    candle_ts: chrono::DateTime<Utc>,
) -> RiskSnapshot {
    let now = Utc::now();
    let (_, rc, _) = eng.risk_engine().await;
    let hash = tc_store::sha256_hex(&serde_json::to_vec(&rc).unwrap_or_default());
    let live_authorized = eng
        .store
        .live_authorizations(&hash)
        .await
        .unwrap_or_default();
    let kill = eng
        .store
        .kill_state()
        .await
        .unwrap_or(tc_store::control::KillState {
            global: KillMode::Freeze,
            brokers: BTreeMap::new(),
            strategies: BTreeMap::new(),
        });
    let mut kill_global = kill.global;
    if eng.mode().await == Mode::Recovery {
        kill_global = kill_global.max(KillMode::ReduceOnly);
    }
    let creds = eng.store.credential_statuses().await.unwrap_or_default();
    let credentials = creds
        .iter()
        .filter_map(|c| {
            Broker::parse(&c.broker).ok().map(|b| {
                (
                    b,
                    CredentialStatus {
                        withdraw_disabled_verified: c.can_withdraw == Some(false),
                        withdraw_disabled_attested: c.attested_no_withdraw
                            && c.can_withdraw != Some(true),
                        scopes_ok: c.scopes_ok,
                        checked_at: c.checked_at,
                    },
                )
            })
        })
        .collect();
    let brokers = eng.brokers.read().await.clone();
    let mut health = BTreeMap::new();
    use tc_broker::BrokerClient;
    if let Some(c) = &brokers.bitvavo {
        health.insert(Broker::Bitvavo, to_health(c.health()));
    }
    if let Some(c) = &brokers.t212 {
        health.insert(Broker::Trading212, to_health(c.health()));
    }
    let inst = market::instrument(eng, id).await;
    let fx = eng.fx.read().await.clone();
    let fx_to_huf = match (&fx, &inst) {
        (Some(f), Some(i)) => f.huf_per(i.quote_currency).map(|r| {
            let ts = f
                .as_of
                .and_hms_opt(16, 0, 0)
                .map(|d| d.and_utc())
                .unwrap_or(now);
            (r, ts)
        }),
        _ => None,
    };
    let eur_huf = fx
        .as_ref()
        .and_then(|f| f.huf_per(tc_core::Currency::EUR))
        .unwrap_or_default();
    let liquidity = quote.and_then(|q| q.volume_quote_24h);
    let market = quote.map(|q| MarketData {
        quote_ts: q.ts.min(now),
        bid: q.bid,
        ask: q.ask,
        liquidity_notional: liquidity,
        candle_ts,
        candle_interval_secs: Interval::D1.seconds(),
    });
    let listed_at = eng
        .listed_at
        .lock()
        .await
        .get(id)
        .copied()
        .or(inst.as_ref().map(|i| i.first_seen))
        .unwrap_or(now);
    let instrument = inst.as_ref().map(|i| {
        let session = match id.broker() {
            Broker::Bitvavo => None,
            _ => Some(brokers.t212.as_ref().and_then(|c| {
                c.session_for(
                    id.symbol(),
                    now.with_timezone(&chrono_tz::America::New_York)
                        .date_naive(),
                )
            })),
        };
        InstrumentStatus {
            tradable: i.tradable,
            eligible: i.asset_class != AssetClass::Crypto
                || i.quote_currency == tc_core::Currency::EUR,
            ineligible_reason: None,
            listed_at,
            min_notional: i.min_notional,
            min_quantity: i.min_quantity,
            quantity_decimals: i.quantity_decimals,
            session,
            long_term_whitelisted: false,
        }
    });
    let snap = eng.store.latest_snapshot().await.ok().flatten();
    let day0 = budapest_midnight(now);
    let week0 = week_start(now);
    // Baselines: last snapshot before the boundary, else the first one after it (never "no loss").
    let nav_day = baseline(eng, day0).await;
    let nav_week = baseline(eng, week0).await;
    let nav_now = snap.as_ref().map(|s| s.nav_unit).unwrap_or_default();
    let mut ccys = std::collections::HashMap::new();
    for r in eng
        .store
        .active_and_recent_intents(now - Duration::minutes(10))
        .await
        .unwrap_or_default()
    {
        if !ccys.contains_key(&r.intent.instrument) {
            if let Some(i) = market::instrument(eng, &r.intent.instrument).await {
                ccys.insert(r.intent.instrument.clone(), i.quote_currency);
            }
        }
    }
    let ccy_of = |id: &InstrumentId| ccys.get(id).copied();
    let positions = exposures(eng, fx.as_ref()).await;
    let intents = eng
        .store
        .active_and_recent_intents(now - Duration::minutes(10))
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| IntentSummary {
            instrument: r.intent.instrument.clone(),
            side: r.intent.side,
            state: r.state,
            class: OrderClass::from_reason(r.intent.reason),
            created_at: r.intent.created_at,
            notional_huf: ccy_of(&r.intent.instrument)
                .and_then(|c| {
                    fx.as_ref().and_then(|f| {
                        f.to_huf(
                            (r.intent.quantity - r.filled_quantity) * r.intent.reference_price,
                            c,
                        )
                    })
                })
                .unwrap_or(
                    (r.intent.quantity - r.filled_quantity) * r.intent.reference_price * eur_huf,
                ),
        })
        .collect();
    let closed_trades = eng
        .store
        .trades(50, Some(now - Duration::days(7)))
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|t| {
            Some(ClosedTrade {
                instrument: InstrumentId::parse(&t.instrument).ok()?,
                closed_at: t.exit_at,
                net_pnl_huf: t.net_huf,
                stopped_out: t.stopped_out,
            })
        })
        .collect();
    let reserved: Decimal = eng
        .store
        .active_and_recent_intents(now)
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| {
            r.intent.side == Side::Buy
                && matches!(
                    r.state,
                    tc_core::OrderState::Pending
                        | tc_core::OrderState::Submitting
                        | tc_core::OrderState::Unknown
                        | tc_core::OrderState::Open
                        | tc_core::OrderState::PartiallyFilled
                )
        })
        .map(|r| {
            let n = (r.intent.quantity - r.filled_quantity) * r.intent.reference_price;
            ccy_of(&r.intent.instrument)
                .and_then(|c| fx.as_ref().and_then(|f| f.to_huf(n, c)))
                .unwrap_or(n * eur_huf)
        })
        .sum();
    let account_ts = eng
        .last_account_sync
        .read()
        .await
        .get(&id.broker())
        .copied()
        .unwrap_or(now - Duration::days(1));
    RiskSnapshot {
        live_authorized,
        strategy_approved: strategy_status == "live" || strategy_status == "live_half",
        kill_global,
        kill_broker: kill.brokers,
        kill_strategy: kill
            .strategies
            .into_iter()
            .filter(|(k, _)| k == strategy)
            .collect(),
        credentials,
        health,
        market,
        instrument,
        fx_to_huf,
        account_ts,
        recon_mismatch: eng.recon_mismatch.read().await.is_some(),
        nav_unit: nav_now,
        nav_unit_day_start: nav_day.unwrap_or(nav_now),
        nav_unit_week_start: nav_week.unwrap_or(nav_now),
        nav_unit_peak: snap.as_ref().map(|s| s.nav_peak).unwrap_or_default(),
        drawdown_halt_latched: false,
        equity_huf: trading_equity(eng, snap.as_ref()).await,
        cash_available_huf: broker_cash_huf(eng, id.broker()).await,
        cash_reserved_local_huf: reserved,
        taker_fee: *eng
            .fees
            .read()
            .await
            .get(&id.broker())
            .unwrap_or(&default_fee(id.broker())),
        positions,
        intents,
        closed_trades,
        entries_last_hour: eng
            .store
            .entries_since(now - Duration::hours(1))
            .await
            .unwrap_or(i64::MAX) as u32,
        entries_today: eng.store.entries_since(day0).await.unwrap_or(i64::MAX) as u32,
    }
}

fn to_health(h: tc_broker::HealthCounters) -> BrokerHealth {
    let err = if h.requests_5m > 0 {
        Decimal::from(h.errors_5m) / Decimal::from(h.requests_5m)
    } else {
        Decimal::ZERO
    };
    BrokerHealth {
        last_ok: h.last_ok,
        last_rate_limited: h.last_rate_limited,
        rate_budget_remaining: h.rate_budget_remaining.unwrap_or(Decimal::ONE),
        error_rate_5m: err,
        clock_skew_ms: h.clock_skew_ms,
    }
}

pub fn budapest_midnight(now: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
    use chrono::TimeZone;
    let local = now.with_timezone(&Budapest).date_naive();
    Budapest
        .from_local_datetime(&local.and_time(chrono::NaiveTime::MIN))
        .earliest()
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(now - Duration::hours(24))
}

async fn exposures(eng: &Engine, fx: Option<&tc_marketdata::FxTable>) -> Vec<PositionExposure> {
    let mut out = Vec::new();
    for p in eng.store.positions().await.unwrap_or_default() {
        let inst = market::instrument(eng, &p.instrument).await;
        let px = market::last_price(eng, &p.instrument)
            .await
            .unwrap_or(p.avg_price);
        let ccy = inst
            .as_ref()
            .map(|i| i.quote_currency)
            .unwrap_or(tc_core::Currency::EUR);
        let ac = inst
            .as_ref()
            .map(|i| i.asset_class)
            .unwrap_or(AssetClass::Crypto);
        let value = fx
            .and_then(|f| f.to_huf(px * p.quantity, ccy))
            .unwrap_or_default();
        out.push(PositionExposure {
            instrument: p.instrument.clone(),
            asset_class: ac,
            cluster: cluster(&p.instrument, ac),
            value_huf: value,
            free_quantity: p.quantity,
            long_term: p.long_term,
        });
    }
    out
}

/// Summaries for the three bot cards (crypto, stocks, long-term).
pub async fn bot_status(eng: &Engine) {
    let t = Instant::now();
    let positions = eng.store.positions().await.unwrap_or_default();
    let mode = eng.mode().await;
    let kill = eng.store.kill_state().await.ok();
    let blocked = |b: Broker| {
        kill.as_ref()
            .map(|k| {
                k.global.max(k.brokers.get(&b).copied().unwrap_or_default()) != KillMode::Normal
            })
            .unwrap_or(true)
    };
    let brokers = eng.brokers.read().await.clone();
    let n_crypto = positions
        .iter()
        .filter(|p| p.instrument.broker() == Broker::Bitvavo)
        .count();
    let n_stock = positions
        .iter()
        .filter(|p| p.instrument.broker() == Broker::Trading212 && !p.long_term)
        .count();
    let n_lt = positions.iter().filter(|p| p.long_term).count();
    let last = eng
        .store
        .decisions(1, None, None)
        .await
        .ok()
        .and_then(|d| d.first().and_then(|x| x.get("created_at").cloned()));
    let st = |configured: bool, b: Broker| {
        if !configured {
            "blocked"
        } else if mode == Mode::Recovery || blocked(b) {
            "paused"
        } else {
            "active"
        }
    };
    eng.agent(
        "bot_crypto",
        st(brokers.bitvavo.is_some(), Broker::Bitvavo),
        t,
        if brokers.bitvavo.is_some() {
            "Bitvavo EUR piacok figyelése"
        } else {
            "Bitvavo kulcs hiányzik"
        },
        json!({ "open_positions": n_crypto, "last_decision_at": last }),
    )
    .await;
    let plan = eng
        .store
        .effective_config::<PlanConfig>("plan", Utc::now())
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    eng.agent(
        "bot_invest",
        if plan.etf_targets.is_empty() {
            "idle"
        } else {
            st(brokers.t212.is_some(), Broker::Trading212)
        },
        t,
        if plan.etf_targets.is_empty() {
            "Hosszú távú befektetés: ETF célok nincsenek beállítva".to_owned()
        } else {
            format!("Indul: {}", plan.long_term_start.label())
        },
        json!({ "open_positions": n_lt }),
    )
    .await;
    if brokers.t212.is_some() && brokers.eodhd.is_some() {
        eng.agent(
            "bot_stocks",
            st(true, Broker::Trading212),
            t,
            "Trading 212 részvények (napi jelzések)",
            json!({ "open_positions": n_stock }),
        )
        .await;
    }
}

/// Monthly DCA into the configured ETF targets from the long-term start month.
pub async fn long_term_invest(eng: &Engine) {
    let t = Instant::now();
    let now = Utc::now();
    let plan = eng
        .store
        .effective_config::<PlanConfig>("plan", now)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let ym = tc_plan::YearMonth::of(now.with_timezone(&Budapest).date_naive());
    if ym < plan.long_term_start || plan.etf_targets.is_empty() || eng.mode().await != Mode::Running
    {
        return;
    }
    let b = eng.brokers.read().await.clone();
    let (Some(_t212), Some(eod)) = (b.t212, b.eodhd) else {
        eng.agent(
            "long_term_invest",
            "blocked",
            t,
            "Trading 212 vagy EODHD kulcs hiányzik",
            json!({}),
        )
        .await;
        return;
    };
    let month_start = month_start(now);
    let done = eng
        .store
        .active_and_recent_intents(month_start)
        .await
        .unwrap_or_default()
        .iter()
        .any(|r| {
            r.intent.reason == IntentReason::LongTermInvest
                && r.intent.created_at >= month_start
                && !matches!(
                    r.state,
                    tc_core::OrderState::Aborted
                        | tc_core::OrderState::Rejected
                        | tc_core::OrderState::NotPlaced
                )
        });
    if done {
        return;
    }
    let budget_huf = plan
        .month(ym)
        .long_term_huf
        .min(broker_cash_huf(eng, Broker::Trading212).await);
    let fx = eng.fx.read().await.clone();
    let mut placed = 0;
    for (tgt, amount_huf) in plan.etf_targets.iter().zip(
        tc_plan::allocate_long_term(&plan, budget_huf)
            .into_iter()
            .map(|x| x.1),
    ) {
        let Ok(id) = InstrumentId::parse(&tgt.instrument) else {
            continue;
        };
        let (Some(sym), Some(inst)) = (tgt.data_symbol.clone(), market::instrument(eng, &id).await)
        else {
            continue;
        };
        let _ = eod.set_symbol(id.clone(), &sym);
        if let Err(e) = market::refresh_candles(eng, &id, Interval::D1).await {
            warn!(instrument = %id, error = %e, "ETF price refresh failed");
        }
        let Some(price) = market::last_price(eng, &id).await else {
            continue;
        };
        let Some(huf_per) = fx.as_ref().and_then(|f| f.huf_per(inst.quote_currency)) else {
            continue;
        };
        let qty = inst.floor_quantity(amount_huf / huf_per / price);
        if qty <= Decimal::ZERO {
            continue;
        }
        let req = RiskRequest {
            instrument: id.clone(),
            asset_class: inst.asset_class,
            side: Side::Buy,
            class: OrderClass::LongTerm,
            strategy: "long_term_dca".into(),
            quantity: qty,
            price,
            stop_price: None,
            gap_p99: Decimal::ZERO,
            edge_bps: Decimal::ZERO,
            edge_se_bps: Decimal::ZERO,
            size_multiplier: Decimal::ONE,
            cluster: "long_term".into(),
            created_at: now,
        };
        let mut snapshot = build_snapshot(
            eng,
            &id,
            "long_term_dca",
            "live",
            quote_for(eng, &id).await.as_ref(),
            now,
        )
        .await;
        if let Some(i) = snapshot.instrument.as_mut() {
            i.long_term_whitelisted = true;
        }
        let (engine, _, _) = eng.risk_engine().await;
        let verdict = engine.evaluate(&req, &snapshot, Utc::now());
        let Some(q) = verdict.approved_quantity() else {
            warn!(instrument = %id, codes = ?verdict.codes(), "long-term buy vetoed");
            continue;
        };
        let intent = OrderIntent {
            id: Uuid::new_v4(),
            instrument: id.clone(),
            side: Side::Buy,
            kind: OrderKind::Market,
            quantity: q,
            time_in_force: TimeInForce::Day,
            reason: IntentReason::LongTermInvest,
            decision_id: None,
            reference_price: price,
            created_at: tc_store::now_micros(),
        };
        let meta = json!({ "verdict": verdict, "plan": { "strategy": "long_term_dca", "long_term": true } });
        if eng
            .store
            .insert_intent(&intent, "long_term_dca", &meta)
            .await
            .is_ok()
        {
            exec::execute(eng, &intent, &inst, "long_term_dca").await;
            placed += 1;
        }
    }
    eng.agent(
        "long_term_invest",
        "active",
        t,
        format!("{} ETF order(s) for {}", placed, ym.label()),
        json!({ "budget_huf": budget_huf }),
    )
    .await;
}

/// Monday 00:00 Europe/Budapest of the current week, as UTC (DST-correct).
pub fn week_start(now: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
    use chrono::TimeZone;
    let local = now.with_timezone(&Budapest).date_naive();
    let monday = local - Duration::days(local.weekday().num_days_from_monday() as i64);
    Budapest
        .from_local_datetime(&monday.and_time(chrono::NaiveTime::MIN))
        .earliest()
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(now - Duration::days(7))
}

/// First day of the current month 00:00 Europe/Budapest, as UTC (DST-correct).
pub fn month_start(now: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
    use chrono::TimeZone;
    let local = now.with_timezone(&Budapest).date_naive();
    let first = local.with_day(1).unwrap_or(local);
    Budapest
        .from_local_datetime(&first.and_time(chrono::NaiveTime::MIN))
        .earliest()
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(now - Duration::days(31))
}

/// NAV per unit at a period boundary: last snapshot before it, else the first one after it.
async fn baseline(eng: &Engine, t: chrono::DateTime<Utc>) -> Option<Decimal> {
    match eng.store.snapshot_at(t).await {
        Ok(Some(s)) => Some(s.nav_unit),
        _ => eng.store.first_snapshot_since(t).await.ok().flatten(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn period_starts_are_dst_correct() {
        // 2026-11-02 is in CET (UTC+1): local midnight = 23:00 UTC the previous day.
        let now = Utc
            .with_ymd_and_hms(2026, 11, 2, 10, 0, 0)
            .single()
            .expect("t");
        assert_eq!(
            month_start(now),
            Utc.with_ymd_and_hms(2026, 10, 31, 23, 0, 0)
                .single()
                .expect("t")
        );
        assert_eq!(
            week_start(now),
            Utc.with_ymd_and_hms(2026, 11, 1, 23, 0, 0)
                .single()
                .expect("t")
        );
        // Summer time (UTC+2): 1 July local midnight = 30 June 22:00 UTC.
        let summer = Utc
            .with_ymd_and_hms(2026, 7, 15, 12, 0, 0)
            .single()
            .expect("t");
        assert_eq!(
            month_start(summer),
            Utc.with_ymd_and_hms(2026, 6, 30, 22, 0, 0)
                .single()
                .expect("t")
        );
    }

    #[test]
    fn market_groups() {
        assert_eq!(
            market_group(&InstrumentId::new(Broker::Bitvavo, "BTC-EUR")),
            "bitvavo_eur"
        );
        assert_eq!(
            market_group(&InstrumentId::new(Broker::Trading212, "AAPL_US_EQ")),
            "t212_us_equity"
        );
    }
}

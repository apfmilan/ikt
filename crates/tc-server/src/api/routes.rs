//! REST handlers (docs/api.md). Realized figures come from broker-reconciled data
//! (snapshots, fills, ledger); forecasts only from decision records — never mixed.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};
use chrono_tz::Europe::Budapest;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use tc_core::{Broker, InstrumentId, Interval};
use tc_plan::PlanConfig;
use tc_risk::{KillMode, RiskConfig};
use tc_store::sha256_hex;

use super::{require_stepup, ApiError, ApiResult, AppState, Session};

// ---------- helpers ----------
pub fn risk_config_hash(c: &RiskConfig) -> String {
    sha256_hex(&serde_json::to_vec(c).unwrap_or_default())
}

async fn risk_config(st: &AppState) -> ApiResult<(RiskConfig, bool)> {
    match st
        .store
        .effective_config::<RiskConfig>("risk", Utc::now())
        .await?
    {
        Some(c) => Ok((c, true)),
        None => Ok((RiskConfig::default(), false)),
    }
}

async fn plan_config(st: &AppState) -> ApiResult<(PlanConfig, bool)> {
    match st
        .store
        .effective_config::<PlanConfig>("plan", Utc::now())
        .await?
    {
        Some(c) => Ok((c, true)),
        None => Ok((PlanConfig::default(), false)),
    }
}

/// HUF conversion from the latest stored ECB table (units per EUR).
struct Fx(BTreeMap<String, Decimal>);

impl Fx {
    async fn load(st: &AppState) -> ApiResult<Fx> {
        let mut m = BTreeMap::new();
        m.insert("EUR".to_owned(), Decimal::ONE);
        if let Some((_, rates)) = st.store.latest_fx().await? {
            m.extend(rates);
        }
        Ok(Fx(m))
    }
    fn to_huf(&self, amount: Decimal, ccy: &str) -> Option<Decimal> {
        let huf = self.0.get("HUF")?;
        let r = self.0.get(ccy)?;
        if r.is_zero() {
            return None;
        }
        Some(amount / *r * *huf)
    }
}

/// Start of today in Europe/Budapest, as UTC.
pub fn budapest_day_start(now: DateTime<Utc>) -> DateTime<Utc> {
    let local = now.with_timezone(&Budapest).date_naive();
    Budapest
        .from_local_datetime(&local.and_time(NaiveTime::MIN))
        .earliest()
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(now - Duration::hours(24))
}

fn kill_str(m: KillMode) -> &'static str {
    m.as_str()
}

// ---------- health ----------
pub async fn health(State(st): State<Arc<AppState>>) -> (StatusCode, Json<Value>) {
    let db = st.store.ping().await;
    let hb = st.store.heartbeat_of("engine").await.ok().flatten();
    let age = hb.as_ref().map(|(t, _)| (Utc::now() - *t).num_seconds());
    let ok = db && age.is_some_and(|a| a < 120);
    let code = if db {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        code,
        Json(json!({
            "status": if ok { "ok" } else { "degraded" },
            "db": db,
            "engine_heartbeat_age_s": age,
            "version": env!("CARGO_PKG_VERSION"),
        })),
    )
}

// ---------- overview ----------
pub async fn overview(State(st): State<Arc<AppState>>, _s: Session) -> ApiResult<Json<Value>> {
    let now = Utc::now();
    let snap = st.store.latest_snapshot().await?;
    let day0 = budapest_day_start(now);
    let start = st.store.snapshot_at(day0).await?;
    let flows_today = st.store.net_flows_huf(day0, now).await?;
    let kill = st.store.kill_state().await?;
    let hb = st.store.heartbeat_of("engine").await?;
    let (rc, _) = risk_config(&st).await?;
    let live = st.store.live_authorizations(&risk_config_hash(&rc)).await?;
    let creds = st.store.credential_statuses().await?;
    let agents = st.store.agent_statuses().await?;
    let curve = st
        .store
        .equity_curve(now - Duration::days(90), 6 * 3600)
        .await?;
    let fees_total: Decimal = st
        .store
        .trades(10_000, None)
        .await?
        .iter()
        .map(|t| t.fees_huf)
        .sum();

    let mode = match hb.as_ref() {
        Some((_, m)) if m == "recovery" => "recovery".to_owned(),
        _ => kill_str(kill.global).to_owned(),
    };
    let realized = snap.as_ref().map(|s| {
        let pnl_today = start
            .as_ref()
            .map(|a| tc_plan::net_pnl(a.equity_huf, s.equity_huf, flows_today));
        let dd = if s.nav_peak > Decimal::ZERO {
            (Decimal::ONE - s.nav_unit / s.nav_peak) * Decimal::ONE_HUNDRED
        } else {
            Decimal::ZERO
        };
        json!({
            "equity_huf": s.equity_huf.round_dp(0),
            "net_deposits_huf": s.net_deposits_huf.round_dp(0),
            "net_pnl_total_huf": (s.equity_huf - s.net_deposits_huf).round_dp(0),
            "net_pnl_today_huf": pnl_today.map(|p| p.round_dp(0)),
            "fees_total_huf": fees_total.round_dp(0),
            "nav_unit": s.nav_unit.round_dp(6),
            "drawdown_pct": dd.round_dp(2),
        })
    });
    let per_broker = snap
        .as_ref()
        .map(|s| s.per_broker.clone())
        .unwrap_or(Value::Null);
    let brokers: Vec<Value> = [Broker::Trading212, Broker::Bitvavo]
        .iter()
        .map(|b| {
            let c = creds.iter().find(|c| c.broker == b.as_str());
            let pb = per_broker.get(b.as_str()).cloned().unwrap_or(Value::Null);
            let health = st_health(&agents, *b);
            json!({
                "broker": b.as_str(),
                "connected": c.is_some_and(|c| c.authenticated),
                "equity_huf": pb.get("equity_huf"),
                "cash_huf": pb.get("cash_huf"),
                "currency": c.and_then(|c| c.account_currency.clone()),
                "last_sync": pb.get("as_of"),
                "health": health,
            })
        })
        .collect();
    let bots = bots_from_agents(&agents, &per_broker);
    Ok(Json(json!({
        "as_of": snap.as_ref().map(|s| s.ts),
        "mode": mode,
        "live": { "trading212": live.get(&Broker::Trading212).copied().unwrap_or(false), "bitvavo": live.get(&Broker::Bitvavo).copied().unwrap_or(false) },
        "realized": realized.unwrap_or_else(|| json!({ "equity_huf": null, "net_deposits_huf": null, "net_pnl_total_huf": null, "net_pnl_today_huf": null, "fees_total_huf": null, "nav_unit": null, "drawdown_pct": null })),
        "brokers": brokers,
        "bots": bots,
        "equity_curve": curve.iter().map(|s| json!({ "t": s.ts, "equity_huf": s.equity_huf.round_dp(0), "net_deposits_huf": s.net_deposits_huf.round_dp(0) })).collect::<Vec<_>>(),
    })))
}

fn st_health(agents: &[tc_store::control::AgentStatusRow], b: Broker) -> &'static str {
    let name = format!("connector_{}", b.as_str());
    match agents.iter().find(|a| a.agent == name) {
        Some(a) if a.status == "active" || a.status == "idle" => "ok",
        Some(a) if a.status == "degraded" => "degraded",
        Some(_) => "down",
        None => "down",
    }
}

fn bots_from_agents(
    agents: &[tc_store::control::AgentStatusRow],
    per_broker: &Value,
) -> Vec<Value> {
    let bot = |id: &str, name: &str, agent: &str| {
        let a = agents.iter().find(|a| a.agent == agent);
        json!({
            "id": id,
            "name": name,
            "status": a.map(|a| a.status.clone()).unwrap_or_else(|| "idle".into()),
            "today_pnl_huf": a.and_then(|a| a.metrics.get("today_pnl_huf").cloned()),
            "open_positions": a.and_then(|a| a.metrics.get("open_positions").cloned()).unwrap_or(json!(0)),
            "last_decision_at": a.and_then(|a| a.metrics.get("last_decision_at").cloned()),
            "message": a.map(|a| a.summary.clone()).unwrap_or_else(|| "Nincs adat".into()),
        })
    };
    let _ = per_broker;
    vec![
        bot("crypto", "Kripto bot", "bot_crypto"),
        bot("stocks", "Részvény bot", "bot_stocks"),
        bot("invest", "Befektetés", "bot_invest"),
    ]
}

pub async fn agents(State(st): State<Arc<AppState>>, _s: Session) -> ApiResult<Json<Value>> {
    let rows = st.store.agent_statuses().await?;
    Ok(Json(json!(rows
        .iter()
        .map(|a| json!({
            "agent": a.agent, "status": a.status, "last_run_at": a.last_run_at,
            "last_duration_ms": a.last_duration_ms, "summary": a.summary, "metrics": a.metrics,
        }))
        .collect::<Vec<_>>())))
}

// ---------- positions & orders ----------
pub async fn positions(State(st): State<Arc<AppState>>, _s: Session) -> ApiResult<Json<Value>> {
    let fx = Fx::load(&st).await?;
    let instruments = st.store.instruments(None).await?;
    let mut out = Vec::new();
    for p in st.store.positions().await? {
        let inst = instruments.iter().find(|i| i.id == p.instrument);
        let ccy = inst
            .map(|i| i.quote_currency.as_str().to_owned())
            .unwrap_or_else(|| "EUR".into());
        let last = st
            .store
            .candles(&p.instrument, Interval::H1, 1)
            .await?
            .last()
            .map(|c| c.close);
        let last = match last {
            Some(l) => Some(l),
            None => st
                .store
                .candles(&p.instrument, Interval::D1, 1)
                .await?
                .last()
                .map(|c| c.close),
        };
        let value = last.map(|l| l * p.quantity);
        let upnl = last.map(|l| (l - p.avg_price) * p.quantity);
        out.push(json!({
            "instrument": p.instrument.as_str(),
            "symbol": p.instrument.symbol(),
            "name": inst.map(|i| i.name.clone()),
            "broker": p.instrument.broker().as_str(),
            "asset_class": inst.map(|i| i.asset_class.as_str()),
            "quantity": p.quantity,
            "avg_price": p.avg_price,
            "last_price": last,
            "currency": ccy,
            "value_huf": value.and_then(|v| fx.to_huf(v, &ccy)).map(|v| v.round_dp(0)),
            "unrealized_huf": upnl.and_then(|v| fx.to_huf(v, &ccy)).map(|v| v.round_dp(0)),
            "unrealized_pct": last.filter(|_| p.avg_price > Decimal::ZERO).map(|l| ((l / p.avg_price - Decimal::ONE) * Decimal::ONE_HUNDRED).round_dp(2)),
            "stop_price": p.stop_price,
            "take_profit": p.take_profit,
            "strategy": p.strategy,
            "opened_at": p.opened_at,
            "long_term": p.long_term,
            "decision_id": p.decision_id,
        }));
    }
    Ok(Json(json!(out)))
}

#[derive(Deserialize)]
pub struct SellBody {
    instrument: String,
    quantity: String,
}

pub async fn manual_sell(
    State(st): State<Arc<AppState>>,
    s: Session,
    headers: HeaderMap,
    Json(b): Json<SellBody>,
) -> ApiResult<Json<Value>> {
    let pk = require_stepup(&st, &headers, "manual_sell").await?;
    let id =
        InstrumentId::parse(&b.instrument).map_err(|e| ApiError::bad_request(e.to_string()))?;
    if b.quantity != "all" {
        let q: Decimal = b
            .quantity
            .parse()
            .map_err(|_| ApiError::bad_request("quantity must be 'all' or a decimal"))?;
        if q <= Decimal::ZERO {
            return Err(ApiError::bad_request("quantity must be positive"));
        }
    }
    if !st
        .store
        .positions()
        .await?
        .iter()
        .any(|p| p.instrument == id)
    {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "no_position",
            "no open position",
        ));
    }
    let cmd = st
        .store
        .enqueue_command(
            "manual_sell",
            &json!({ "instrument": id.as_str(), "quantity": b.quantity }),
            &format!("passkey:{pk}"),
            Some("manual_sell"),
        )
        .await?;
    let _ = s;
    Ok(Json(json!({ "command_id": cmd })))
}

#[derive(Deserialize)]
pub struct ListQ {
    state: Option<String>,
    limit: Option<i64>,
    instrument: Option<String>,
}

pub async fn orders(
    State(st): State<Arc<AppState>>,
    _s: Session,
    Query(q): Query<ListQ>,
) -> ApiResult<Json<Value>> {
    let rows = st
        .store
        .intents_page(
            q.state.as_deref() == Some("open"),
            q.limit.unwrap_or(100).clamp(1, 500),
        )
        .await?;
    Ok(Json(json!(rows
        .iter()
        .map(|r| json!({
            "id": r.intent.id, "instrument": r.intent.instrument.as_str(), "side": r.intent.side.as_str(),
            "kind": r.intent.kind, "quantity": r.intent.quantity, "filled_quantity": r.filled_quantity,
            "state": r.state.as_str(), "reason": r.intent.reason.as_str(), "broker_order_id": r.broker_order_id,
            "created_at": r.intent.created_at, "updated_at": r.updated_at, "avg_fill_price": r.avg_fill_price,
            "fee": r.fee_paid, "fee_currency": r.fee_currency, "strategy": r.strategy, "last_error": r.last_error,
        }))
        .collect::<Vec<_>>())))
}

pub async fn trades(
    State(st): State<Arc<AppState>>,
    _s: Session,
    Query(q): Query<ListQ>,
) -> ApiResult<Json<Value>> {
    let rows = st
        .store
        .trades(q.limit.unwrap_or(100).clamp(1, 1000), None)
        .await?;
    Ok(Json(json!(rows
        .iter()
        .map(|t| json!({
            "id": t.id, "instrument": t.instrument, "strategy": t.strategy, "entry_at": t.entry_at, "exit_at": t.exit_at,
            "entry_price": t.entry_price, "exit_price": t.exit_price, "quantity": t.quantity,
            "realized": { "gross_huf": (t.net_huf + t.fees_huf).round_dp(0), "fees_huf": t.fees_huf.round_dp(0), "net_huf": t.net_huf.round_dp(0), "return_pct": t.return_pct.round_dp(2) },
            "forecast": t.forecast, "exit_reason": t.exit_reason, "decision_id": t.decision_id,
        }))
        .collect::<Vec<_>>())))
}

pub async fn transactions(
    State(st): State<Arc<AppState>>,
    _s: Session,
    Query(q): Query<ListQ>,
) -> ApiResult<Json<Value>> {
    let rows = st
        .store
        .ledger(q.limit.unwrap_or(200).clamp(1, 2000))
        .await?;
    Ok(Json(json!(rows
        .iter()
        .map(|l| json!({ "id": l.id, "ts": l.ts, "broker": l.broker, "kind": l.kind, "amount": l.amount, "currency": l.currency, "amount_huf": l.amount_huf.round_dp(0), "ref": l.r#ref }))
        .collect::<Vec<_>>())))
}

pub async fn decisions(
    State(st): State<Arc<AppState>>,
    _s: Session,
    Query(q): Query<ListQ>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!(
        st.store
            .decisions(
                q.limit.unwrap_or(100).clamp(1, 500),
                q.instrument.as_deref(),
                None
            )
            .await?
    )))
}

pub async fn decision(
    State(st): State<Arc<AppState>>,
    _s: Session,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let mut v = st.store.decisions(1, None, Some(&id)).await?;
    v.pop()
        .map(Json)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "decision not found"))
}

#[derive(Deserialize)]
pub struct ChartQ {
    interval: Option<String>,
    limit: Option<i64>,
}

pub async fn chart(
    State(st): State<Arc<AppState>>,
    _s: Session,
    Path(id): Path<String>,
    Query(q): Query<ChartQ>,
) -> ApiResult<Json<Value>> {
    let id = InstrumentId::parse(&id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let interval = Interval::parse(q.interval.as_deref().unwrap_or("1h"))
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let candles = st
        .store
        .candles(&id, interval, q.limit.unwrap_or(300).clamp(10, 1500))
        .await?;
    let mut markers = Vec::new();
    for t in st
        .store
        .trades(200, None)
        .await?
        .iter()
        .filter(|t| t.instrument == id.as_str())
    {
        markers.push(
            json!({ "t": t.entry_at, "kind": "entry", "price": t.entry_price, "text": t.strategy }),
        );
        markers.push(
            json!({ "t": t.exit_at, "kind": "exit", "price": t.exit_price, "text": t.exit_reason }),
        );
    }
    let mut lines = Vec::new();
    if let Some(p) = st
        .store
        .positions()
        .await?
        .into_iter()
        .find(|p| p.instrument == id)
    {
        markers.push(
            json!({ "t": p.opened_at, "kind": "entry", "price": p.avg_price, "text": p.strategy }),
        );
        lines.push(json!({ "kind": "avg_price", "price": p.avg_price }));
        if let Some(s) = p.stop_price {
            lines.push(json!({ "kind": "stop", "price": s }));
        }
        if let Some(tp) = p.take_profit {
            lines.push(json!({ "kind": "take_profit", "price": tp }));
        }
    }
    Ok(Json(json!({
        "candles": candles.iter().map(|c| json!({ "t": c.ts, "o": c.open, "h": c.high, "l": c.low, "c": c.close, "v": c.volume })).collect::<Vec<_>>(),
        "markers": markers,
        "lines": lines,
    })))
}

pub async fn performance(State(st): State<Arc<AppState>>, _s: Session) -> ApiResult<Json<Value>> {
    let trades = st.store.trades(10_000, None).await?;
    let n = trades.len();
    let wins = trades.iter().filter(|t| t.net_huf > Decimal::ZERO).count();
    let gains: Decimal = trades
        .iter()
        .filter(|t| t.net_huf > Decimal::ZERO)
        .map(|t| t.net_huf)
        .sum();
    let losses: Decimal = trades
        .iter()
        .filter(|t| t.net_huf < Decimal::ZERO)
        .map(|t| -t.net_huf)
        .sum();
    let net: Decimal = trades.iter().map(|t| t.net_huf).sum();
    let fees: Decimal = trades.iter().map(|t| t.fees_huf).sum();
    let curve = st
        .store
        .equity_curve(DateTime::<Utc>::UNIX_EPOCH, 86_400)
        .await?;
    let max_dd = curve
        .iter()
        .map(|s| {
            if s.nav_peak > Decimal::ZERO {
                Decimal::ONE - s.nav_unit / s.nav_peak
            } else {
                Decimal::ZERO
            }
        })
        .max()
        .unwrap_or_default();
    let first_units = curve.first().map(|s| s.nav_unit).unwrap_or(Decimal::ONE);
    let total_ret = curve
        .last()
        .map(|s| {
            if first_units > Decimal::ZERO {
                s.nav_unit / first_units - Decimal::ONE
            } else {
                Decimal::ZERO
            }
        })
        .unwrap_or_default();
    let strategies = st.store.strategies().await?;
    let mut by: BTreeMap<String, (usize, usize, Decimal, Decimal)> = BTreeMap::new();
    for t in &trades {
        let e = by.entry(t.strategy.clone()).or_default();
        e.0 += 1;
        if t.net_huf > Decimal::ZERO {
            e.1 += 1;
        }
        e.2 += t.net_huf;
        e.3 += t.fees_huf;
    }
    for s in &strategies {
        by.entry(s.name.clone()).or_default();
    }
    let mut daily: BTreeMap<String, Decimal> = BTreeMap::new();
    for t in &trades {
        *daily
            .entry(t.exit_at.with_timezone(&Budapest).date_naive().to_string())
            .or_default() += t.net_huf;
    }
    Ok(Json(json!({
        "realized": {
            "total_return_pct": (total_ret * Decimal::ONE_HUNDRED).round_dp(2),
            "win_rate": if n > 0 { json!(wins as f64 / n as f64) } else { Value::Null },
            "profit_factor": if losses > Decimal::ZERO { json!((gains / losses).round_dp(2)) } else { Value::Null },
            "max_drawdown_pct": (max_dd * Decimal::ONE_HUNDRED).round_dp(2),
            "trades": n,
            "fees_huf": fees.round_dp(0),
            "expectancy_huf": if n > 0 { json!((net / Decimal::from(n as i64)).round_dp(0)) } else { Value::Null },
        },
        "by_strategy": by.iter().map(|(name, (cnt, w, netv, f))| {
            let s = strategies.iter().find(|s| &s.name == name);
            json!({
                "strategy": name,
                "status": s.map(|s| s.status.clone()).unwrap_or_else(|| "disabled".into()),
                "trades": cnt,
                "win_rate": if *cnt > 0 { json!(*w as f64 / *cnt as f64) } else { Value::Null },
                "net_huf": netv.round_dp(0),
                "fees_huf": f.round_dp(0),
                "backtest_oos": s.map(|s| s.report.get("oos").cloned().unwrap_or(Value::Null)),
            })
        }).collect::<Vec<_>>(),
        "daily": daily.iter().map(|(d, v)| json!({ "date": d, "net_pnl_huf": v.round_dp(0) })).collect::<Vec<_>>(),
    })))
}

pub async fn strategies(State(st): State<Arc<AppState>>, _s: Session) -> ApiResult<Json<Value>> {
    let rows = st.store.strategies().await?;
    Ok(Json(json!(rows
        .iter()
        .map(|s| json!({ "name": s.name, "version": s.version, "status": s.status, "asset_class": s.asset_class, "backtest": s.report, "updated_at": s.updated_at }))
        .collect::<Vec<_>>())))
}

// ---------- plan & risk ----------
pub async fn plan_get(State(st): State<Arc<AppState>>, _s: Session) -> ApiResult<Json<Value>> {
    let (cfg, _) = plan_config(&st).await?;
    let ledger = st.store.ledger(100_000).await?;
    let deposited: Decimal = ledger
        .iter()
        .filter(|l| l.kind == "deposit")
        .map(|l| l.amount_huf)
        .sum();
    let withdrawn: Decimal = ledger
        .iter()
        .filter(|l| l.kind == "withdrawal")
        .map(|l| l.amount_huf.abs())
        .sum();
    let mut deposits_by_month: BTreeMap<String, Decimal> = BTreeMap::new();
    for l in ledger.iter().filter(|l| l.kind == "deposit") {
        let d = l.ts.with_timezone(&Budapest).date_naive();
        *deposits_by_month
            .entry(tc_plan::YearMonth::of(d).label())
            .or_default() += l.amount_huf;
    }
    let snap = st.store.latest_snapshot().await?;
    let profit = snap
        .as_ref()
        .map(|s| s.equity_huf - s.net_deposits_huf)
        .unwrap_or_default();
    let now_ym = tc_plan::YearMonth::of(Utc::now().with_timezone(&Budapest).date_naive());
    let withdrawable = tc_plan::split_profit(&cfg, profit).withdrawable_huf;
    Ok(Json(json!({
        "config": cfg,
        "schedule": cfg.schedule(24).iter().map(|m| json!({
            "month": m.month.label(), "planned_trading_huf": m.trading_huf, "planned_long_term_huf": m.long_term_huf,
            "actual_deposits_huf": deposits_by_month.get(&m.month.label()).map(|d| d.round_dp(0)),
        })).collect::<Vec<_>>(),
        "progress": {
            "planned_total_huf": cfg.planned_total(now_ym),
            "deposited_total_huf": (deposited - withdrawn).round_dp(0),
            "trading_profit_total_huf": profit.round_dp(0),
            "withdrawable_huf": withdrawable.round_dp(0),
        },
    })))
}

pub async fn plan_put(
    State(st): State<Arc<AppState>>,
    _s: Session,
    headers: HeaderMap,
    Json(cfg): Json<PlanConfig>,
) -> ApiResult<Json<Value>> {
    let pk = require_stepup(&st, &headers, "plan_change").await?;
    cfg.validate()
        .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid_plan", e.0))?;
    st.store
        .put_config("plan", &cfg, Utc::now(), &format!("passkey:{pk}"))
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn risk_get(State(st): State<Arc<AppState>>, _s: Session) -> ApiResult<Json<Value>> {
    let (cfg, _) = risk_config(&st).await?;
    let pending = st
        .store
        .pending_config::<RiskConfig>("risk", Utc::now())
        .await?;
    let util = st
        .store
        .agent_statuses()
        .await?
        .into_iter()
        .find(|a| a.agent == "risk")
        .map(|a| a.metrics)
        .unwrap_or(Value::Null);
    Ok(Json(json!({
        "config": cfg,
        "hard_max": {
            "risk_per_trade": tc_risk::HardMax::RISK_PER_TRADE, "position_cap": tc_risk::HardMax::POSITION_CAP,
            "crypto_cap": tc_risk::HardMax::CRYPTO_CAP, "equity_cap": tc_risk::HardMax::EQUITY_CAP, "total_cap": tc_risk::HardMax::TOTAL_CAP,
            "daily_loss": tc_risk::HardMax::DAILY_LOSS, "weekly_loss": tc_risk::HardMax::WEEKLY_LOSS, "drawdown_halt": tc_risk::HardMax::DRAWDOWN_HALT,
            "max_positions": tc_risk::HardMax::MAX_POSITIONS, "max_entries_per_hour": tc_risk::HardMax::MAX_ENTRIES_PER_HOUR,
            "max_entries_per_day": tc_risk::HardMax::MAX_ENTRIES_PER_DAY,
        },
        "utilization": if util.is_object() { util } else { json!({ "exposure_pct": null, "crypto_pct": null, "equity_pct": null, "day_pnl_pct": null, "drawdown_pct": null, "entries_today": null }) },
        "pending_change": pending.map(|(c, at)| json!({ "config": c, "effective_at": at })),
    })))
}

async fn apply_risk(st: &AppState, new: RiskConfig, by: &str) -> ApiResult<Value> {
    new.validate()
        .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "hard_limit", e.0))?;
    let (cur, exists) = risk_config(st).await?;
    let now = Utc::now();
    if exists && cur.is_loosening(&new) {
        let at = now + Duration::hours(24);
        st.store.put_config("risk", &new, at, by).await?;
        st.store
            .revoke_live(None, "risk limits loosened", by)
            .await?;
        Ok(json!({ "ok": true, "effective_at": at, "live_revoked": true }))
    } else {
        st.store.put_config("risk", &new, now, by).await?;
        Ok(json!({ "ok": true, "effective_at": now }))
    }
}

pub async fn risk_put(
    State(st): State<Arc<AppState>>,
    _s: Session,
    headers: HeaderMap,
    Json(cfg): Json<RiskConfig>,
) -> ApiResult<Json<Value>> {
    let pk = require_stepup(&st, &headers, "risk_change").await?;
    Ok(Json(apply_risk(&st, cfg, &format!("passkey:{pk}")).await?))
}

// ---------- controls ----------
#[derive(Deserialize)]
pub struct KillBody {
    mode: String,
    #[serde(default)]
    broker: Option<String>,
}

pub async fn kill(
    State(st): State<Arc<AppState>>,
    s: Session,
    headers: HeaderMap,
    Json(b): Json<KillBody>,
) -> ApiResult<Json<Value>> {
    let mode = match b.mode.as_str() {
        "reduce_only" => KillMode::ReduceOnly,
        "flatten" => KillMode::Flatten,
        "freeze" => KillMode::Freeze,
        "normal" => KillMode::Normal,
        _ => {
            return Err(ApiError::bad_request(
                "mode must be reduce_only|flatten|freeze|normal",
            ))
        }
    };
    let scope = match &b.broker {
        Some(x) => format!(
            "broker:{}",
            Broker::parse(x)
                .map_err(|e| ApiError::bad_request(e.to_string()))?
                .as_str()
        ),
        None => "global".to_owned(),
    };
    let mut actor = s.actor();
    if mode == KillMode::Normal {
        let pk = require_stepup(&st, &headers, "kill_release").await?;
        actor = format!("passkey:{pk}");
    }
    st.store.set_kill(&scope, mode, "user", &actor).await?;
    if mode == KillMode::Flatten {
        st.store
            .enqueue_command("flatten", &json!({ "scope": scope }), &actor, None)
            .await?;
    }
    st.store
        .push_event("mode", &json!({ "scope": scope, "mode": mode.as_str() }))
        .await?;
    Ok(Json(
        json!({ "ok": true, "scope": scope, "mode": mode.as_str() }),
    ))
}

#[derive(Deserialize)]
pub struct LiveBody {
    broker: String,
    enabled: bool,
}

pub async fn live(
    State(st): State<Arc<AppState>>,
    s: Session,
    headers: HeaderMap,
    Json(b): Json<LiveBody>,
) -> ApiResult<Json<Value>> {
    let broker = Broker::parse(&b.broker).map_err(|e| ApiError::bad_request(e.to_string()))?;
    if !broker.supports_automated_trading() {
        return Err(ApiError::bad_request("signal-only venue"));
    }
    if b.enabled {
        let pk = require_stepup(&st, &headers, "live_enable").await?;
        authorize_one(&st, broker, pk).await?;
    } else {
        st.store
            .revoke_live(Some(broker), "user disabled", &s.actor())
            .await?;
    }
    Ok(Json(json!({ "ok": true })))
}

async fn authorize_one(st: &AppState, broker: Broker, pk: i64) -> ApiResult<()> {
    let (cfg, exists) = risk_config(st).await?;
    if !exists {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "risk_not_configured",
            "confirm risk limits first",
        ));
    }
    let creds = st.store.credential_statuses().await?;
    let c = creds
        .iter()
        .find(|c| c.broker == broker.as_str())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "not_verified",
                "credentials not verified",
            )
        })?;
    if !c.authenticated {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "not_verified",
            "credentials not verified",
        ));
    }
    if c.can_withdraw == Some(true) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "withdraw_capable",
            "API key has withdrawal permission; create a new key without it",
        ));
    }
    if c.can_withdraw.is_none() && !c.attested_no_withdraw {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "withdraw_unverified",
            "confirm that withdrawal permission is disabled",
        ));
    }
    st.store
        .authorize_live(broker, &risk_config_hash(&cfg), pk, 30)
        .await?;
    Ok(())
}

// ---------- onboarding ----------
pub async fn onboarding(State(st): State<Arc<AppState>>, _s: Session) -> ApiResult<Json<Value>> {
    let creds = st.store.credential_statuses().await?;
    let have = st.store.credential_brokers().await?;
    let (rc, risk_set) = risk_config(&st).await?;
    let (_, plan_set) = plan_config(&st).await?;
    let live = st.store.live_authorizations(&risk_config_hash(&rc)).await?;
    let broker_step = |b: &str| {
        let c = creds.iter().find(|c| c.broker == b);
        let status = match (have.iter().any(|x| x == b), c) {
            (false, _) => "todo",
            (true, None) => "todo",
            (true, Some(c)) if !c.authenticated => "error",
            (true, Some(c)) if c.can_withdraw == Some(true) => "error",
            (true, Some(_)) => "done",
        };
        json!({ "id": b, "status": status, "detail": c.map(|c| step_text(c)), "data": c.map(|c| json!({
            "authenticated": c.authenticated, "can_trade": c.can_trade, "can_withdraw": c.can_withdraw,
            "attested_no_withdraw": c.attested_no_withdraw, "account_currency": c.account_currency, "notes": c.notes, "checked_at": c.checked_at,
        })) })
    };
    let t212 = broker_step("trading212");
    let bv = broker_step("bitvavo");
    let md = json!({ "id": "market_data", "status": if have.iter().any(|x| x == "eodhd") { "done" } else { "todo" },
        "detail": "EODHD API key: required for stock/ETF prices (Trading 212 provides none)" });
    let any_live = live.values().any(|v| *v);
    let steps = vec![
        json!({ "id": "passkey", "status": "done", "detail": Value::Null }),
        t212,
        bv,
        md,
        json!({ "id": "risk", "status": if risk_set { "done" } else { "todo" }, "detail": Value::Null }),
        json!({ "id": "plan", "status": if plan_set { "done" } else { "todo" }, "detail": Value::Null }),
        json!({ "id": "authorize", "status": if any_live { "done" } else { "todo" }, "detail": live.iter().filter(|(_, v)| **v).map(|(b, _)| b.as_str()).collect::<Vec<_>>().join(", "), "data": live }),
    ];
    let ready = risk_set && any_live;
    Ok(Json(json!({ "steps": steps, "ready": ready })))
}

#[derive(Deserialize)]
pub struct CredBody {
    broker: String,
    api_key: String,
    #[serde(default)]
    api_secret: Option<String>,
    #[serde(default)]
    operator_id: Option<i64>,
}

pub async fn credentials(
    State(st): State<Arc<AppState>>,
    s: Session,
    headers: HeaderMap,
    Json(b): Json<CredBody>,
) -> ApiResult<Json<Value>> {
    let broker = b.broker.as_str();
    if !["trading212", "bitvavo", "eodhd"].contains(&broker) {
        return Err(ApiError::bad_request(
            "broker must be trading212, bitvavo or eodhd",
        ));
    }
    let key = b.api_key.trim();
    if key.len() < 8 || key.len() > 512 {
        return Err(ApiError::bad_request("api_key looks invalid"));
    }
    let needs_secret = broker != "eodhd";
    let secret = b.api_secret.as_deref().map(str::trim).unwrap_or_default();
    if needs_secret && (secret.len() < 8 || secret.len() > 512) {
        return Err(ApiError::bad_request("api_secret is required"));
    }
    if broker == "bitvavo" && b.operator_id.is_none_or(|o| o <= 0) {
        return Err(ApiError::bad_request(
            "operator_id (positive integer) is required for Bitvavo",
        ));
    }
    let mut actor = s.actor();
    if st
        .store
        .credential_brokers()
        .await?
        .iter()
        .any(|x| x == broker)
    {
        let pk = require_stepup(&st, &headers, "credentials_change").await?;
        actor = format!("passkey:{pk}");
    }
    let sk = st.sealing_key.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "no_engine_key",
            "engine public key not configured",
        )
    })?;
    const KV: u32 = 1;
    let seal = |field: &str, v: &str| {
        sk.seal(v.as_bytes(), &tc_vault::context(broker, field, KV))
            .map_err(|e| ApiError::bad_request(e.to_string()))
    };
    st.store
        .put_credential(broker, "api_key", KV as i32, &seal("api_key", key)?)
        .await?;
    if needs_secret {
        st.store
            .put_credential(
                broker,
                "api_secret",
                KV as i32,
                &seal("api_secret", secret)?,
            )
            .await?;
    }
    if let Some(op) = b.operator_id {
        st.store
            .put_credential(
                broker,
                "operator_id",
                KV as i32,
                &seal("operator_id", &op.to_string())?,
            )
            .await?;
    }
    if let Ok(br) = Broker::parse(broker) {
        st.store
            .revoke_live(Some(br), "credentials changed", &actor)
            .await?;
    }
    st.store
        .audit(&actor, "credentials_stored", &json!({ "broker": broker }))
        .await?;
    let cmd = st
        .store
        .enqueue_command(
            "verify_credentials",
            &json!({ "broker": broker }),
            &actor,
            None,
        )
        .await?;
    Ok(Json(json!({ "stored": true, "verify_command_id": cmd })))
}

#[derive(Deserialize)]
pub struct BrokerBody {
    broker: String,
    #[serde(default)]
    withdraw_disabled: Option<bool>,
}

pub async fn verify(
    State(st): State<Arc<AppState>>,
    s: Session,
    Json(b): Json<BrokerBody>,
) -> ApiResult<Json<Value>> {
    if !["trading212", "bitvavo", "eodhd"].contains(&b.broker.as_str()) {
        return Err(ApiError::bad_request("unknown broker"));
    }
    let id = st
        .store
        .enqueue_command(
            "verify_credentials",
            &json!({ "broker": b.broker }),
            &s.actor(),
            None,
        )
        .await?;
    Ok(Json(json!({ "command_id": id })))
}

pub async fn attest(
    State(st): State<Arc<AppState>>,
    s: Session,
    Json(b): Json<BrokerBody>,
) -> ApiResult<Json<Value>> {
    if b.withdraw_disabled != Some(true) {
        return Err(ApiError::bad_request("withdraw_disabled must be true"));
    }
    st.store.attest_no_withdraw(&b.broker, &s.actor()).await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn onboarding_risk(
    State(st): State<Arc<AppState>>,
    s: Session,
    headers: HeaderMap,
    Json(cfg): Json<RiskConfig>,
) -> ApiResult<Json<Value>> {
    let (_, exists) = risk_config(&st).await?;
    let by = if exists {
        format!(
            "passkey:{}",
            require_stepup(&st, &headers, "risk_change").await?
        )
    } else {
        s.actor()
    };
    Ok(Json(apply_risk(&st, cfg, &by).await?))
}

#[derive(Deserialize)]
pub struct AuthorizeBody {
    brokers: Vec<String>,
}

pub async fn authorize(
    State(st): State<Arc<AppState>>,
    _s: Session,
    headers: HeaderMap,
    Json(b): Json<AuthorizeBody>,
) -> ApiResult<Json<Value>> {
    let pk = require_stepup(&st, &headers, "live_enable").await?;
    let mut done = Vec::new();
    for x in &b.brokers {
        let br = Broker::parse(x).map_err(|e| ApiError::bad_request(e.to_string()))?;
        if !br.supports_automated_trading() {
            return Err(ApiError::bad_request("signal-only venue"));
        }
        authorize_one(&st, br, pk).await?;
        done.push(br.as_str());
    }
    Ok(Json(
        json!({ "ok": true, "authorized": done, "expires_in_days": 30 }),
    ))
}

// ---------- push & notifications ----------
pub async fn vapid(State(st): State<Arc<AppState>>) -> ApiResult<Json<Value>> {
    let k = st.vapid_public_key.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "no_vapid",
            "push not configured",
        )
    })?;
    Ok(Json(json!({ "public_key": k })))
}

#[derive(Deserialize)]
pub struct SubKeys {
    p256dh: String,
    auth: String,
}

#[derive(Deserialize)]
pub struct SubBody {
    endpoint: String,
    #[serde(default)]
    keys: Option<SubKeys>,
    #[serde(default)]
    device_name: Option<String>,
}

pub async fn push_subscribe(
    State(st): State<Arc<AppState>>,
    s: Session,
    Json(b): Json<SubBody>,
) -> ApiResult<Json<Value>> {
    let url =
        url::Url::parse(&b.endpoint).map_err(|_| ApiError::bad_request("invalid endpoint"))?;
    if url.scheme() != "https" || b.endpoint.len() > 1024 {
        return Err(ApiError::bad_request("endpoint must be https"));
    }
    let k = b
        .keys
        .ok_or_else(|| ApiError::bad_request("keys required"))?;
    st.store
        .add_push_sub(
            &b.endpoint,
            &k.p256dh,
            &k.auth,
            b.device_name.as_deref().unwrap_or("Eszköz"),
        )
        .await?;
    st.store
        .audit(
            &s.actor(),
            "push_subscribed",
            &json!({ "host": url.host_str() }),
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn push_unsubscribe(
    State(st): State<Arc<AppState>>,
    _s: Session,
    Json(b): Json<SubBody>,
) -> ApiResult<Json<Value>> {
    st.store.remove_push_sub(&b.endpoint).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(serde::Serialize, Deserialize, Clone)]
pub struct NotifSettings {
    pub trades: bool,
    pub market_open: bool,
    pub errors: bool,
    pub critical_risk: bool,
    pub daily_summary: bool,
    pub daily_summary_time: String,
}

impl Default for NotifSettings {
    fn default() -> Self {
        NotifSettings {
            trades: true,
            market_open: false,
            errors: true,
            critical_risk: true,
            daily_summary: true,
            daily_summary_time: "21:00".into(),
        }
    }
}

pub async fn notif_get(
    State(st): State<Arc<AppState>>,
    _s: Session,
) -> ApiResult<Json<NotifSettings>> {
    Ok(Json(
        st.store
            .effective_config::<NotifSettings>("notifications", Utc::now())
            .await?
            .unwrap_or_default(),
    ))
}

pub async fn notif_put(
    State(st): State<Arc<AppState>>,
    s: Session,
    Json(b): Json<NotifSettings>,
) -> ApiResult<Json<Value>> {
    if NaiveTime::parse_from_str(&b.daily_summary_time, "%H:%M").is_err() {
        return Err(ApiError::bad_request("daily_summary_time must be HH:MM"));
    }
    st.store
        .put_config("notifications", &b, Utc::now(), &s.actor())
        .await?;
    Ok(Json(json!({ "ok": true })))
}

/// One-line human-readable onboarding status (UI renders `detail` as text).
fn step_text(c: &tc_store::control::CredentialStatusRow) -> String {
    let mut parts = vec![if c.authenticated { "kapcsolódva".to_owned() } else { "nem sikerült kapcsolódni".to_owned() }];
    if let Some(ccy) = &c.account_currency {
        parts.push(ccy.clone());
    }
    match (c.can_withdraw, c.attested_no_withdraw) {
        (Some(true), _) => parts.push("UTALÁSI JOG ENGEDÉLYEZVE – a kulcs elutasítva".into()),
        (Some(false), _) => parts.push("utalás tiltva (ellenőrizve)".into()),
        (None, true) => parts.push("utalás tiltva (megerősítve)".into()),
        (None, false) => parts.push("utalási jog nem ellenőrizhető – erősítsd meg".into()),
    }
    if let Some(last) = c.notes.as_array().and_then(|n| n.iter().rev().find_map(|x| x.as_str().filter(|s| s.starts_with("error")))) {
        parts.push(last.to_owned());
    }
    parts.join(" · ")
}

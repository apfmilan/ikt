//! Deterministic pre-trade risk engine. It has absolute veto over every order: entries,
//! exits, manual sells and emergency flattening. Spec: docs/decisions/0003-risk-engine.md.
//!
//! `evaluate` is a pure function of (request, snapshot, config, now). Missing or stale
//! inputs veto (fail closed). The engine may reduce a quantity, never increase it.

pub mod limits;

use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tc_core::{AssetClass, Broker, InstrumentId, IntentReason, OrderState, Side};

pub use limits::{dec_from_f64, HardMax, LimitViolation, RiskConfig};

/// Risk order classes derived from [`IntentReason`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderClass {
    Entry,
    LongTerm,
    ExitProtective,
    ExitSignal,
    ManualSell,
    Flatten,
}

impl OrderClass {
    pub fn from_reason(r: IntentReason) -> Self {
        match r {
            IntentReason::Entry => OrderClass::Entry,
            IntentReason::LongTermInvest => OrderClass::LongTerm,
            IntentReason::StopLoss
            | IntentReason::TakeProfit
            | IntentReason::TrailingStop
            | IntentReason::PartialExit => OrderClass::ExitProtective,
            IntentReason::StrategyExit => OrderClass::ExitSignal,
            IntentReason::ManualSell => OrderClass::ManualSell,
            IntentReason::EmergencyFlatten => OrderClass::Flatten,
        }
    }

    pub fn is_entry(&self) -> bool {
        matches!(self, OrderClass::Entry | OrderClass::LongTerm)
    }
}

/// Kill-switch modes, applied at global, broker and strategy level (most restrictive wins).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum KillMode {
    #[default]
    Normal,
    /// No new entries; exits allowed.
    ReduceOnly,
    /// Flattening in progress; only flatten/protective exits.
    Flatten,
    /// Nothing is sent by the engine. Broker-resting stops remain.
    Freeze,
}

impl KillMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            KillMode::Normal => "normal",
            KillMode::ReduceOnly => "reduce_only",
            KillMode::Flatten => "flatten",
            KillMode::Freeze => "freeze",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VetoCode {
    LiveNotAuthorized,
    KillSwitch,
    CredWithdrawCapable,
    CredScope,
    BrokerUnhealthy,
    DataStale,
    DataMissing,
    ReconHalt,
    InstrumentIneligible,
    MarketClosed,
    NewListing,
    DailyLoss,
    WeeklyLoss,
    DrawdownHalt,
    LossStreak,
    SizeUpAfterLoss,
    RevengeReentry,
    OrderRate,
    Duplicate,
    InFlight,
    MaxPositions,
    StrategyNotApproved,
    EdgeBelowCost,
    SpreadWide,
    Illiquid,
    RiskPerTrade,
    BelowMinOrder,
    PositionCap,
    ClassCap,
    TotalCap,
    Concentration,
    InsufficientCash,
    Oversell,
    Leverage,
    VenueNotTradable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Veto {
    pub code: VetoCode,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    Allow { quantity: Decimal },
    AllowResized { quantity: Decimal, requested: Decimal, binding: VetoCode },
    Veto { vetoes: Vec<Veto> },
}

impl Verdict {
    pub fn approved_quantity(&self) -> Option<Decimal> {
        match self {
            Verdict::Allow { quantity } | Verdict::AllowResized { quantity, .. } => Some(*quantity),
            Verdict::Veto { .. } => None,
        }
    }

    pub fn is_veto(&self) -> bool {
        matches!(self, Verdict::Veto { .. })
    }

    pub fn codes(&self) -> Vec<VetoCode> {
        match self {
            Verdict::Veto { vetoes } => vetoes.iter().map(|v| v.code).collect(),
            _ => vec![],
        }
    }
}

/// The order being evaluated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RiskRequest {
    pub instrument: InstrumentId,
    pub asset_class: AssetClass,
    pub side: Side,
    pub class: OrderClass,
    pub strategy: String,
    pub quantity: Decimal,
    /// Reference price in the instrument quote currency.
    pub price: Decimal,
    /// Protective stop for entries (required for `Entry`).
    pub stop_price: Option<Decimal>,
    /// 99th percentile adverse gap as a fraction of price (e.g. 0.03); widens the effective stop.
    pub gap_p99: Decimal,
    /// Expected gross edge and its standard error, bps.
    pub edge_bps: Decimal,
    pub edge_se_bps: Decimal,
    /// Size multiplier requested relative to base sizing (must be <= 1 after losses).
    pub size_multiplier: Decimal,
    /// Correlation cluster key (e.g. "crypto", "us_tech").
    pub cluster: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct BrokerHealth {
    pub last_ok: Option<DateTime<Utc>>,
    pub last_rate_limited: Option<DateTime<Utc>>,
    /// Remaining fraction of the rate-limit budget [0, 1].
    pub rate_budget_remaining: Decimal,
    /// Error rate over the last 5 minutes [0, 1].
    pub error_rate_5m: Decimal,
    pub clock_skew_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CredentialStatus {
    /// Withdrawal permission positively verified absent (Bitvavo probe) or N/A (T212).
    pub withdraw_disabled_verified: bool,
    /// User attested in onboarding that withdrawal is disabled (used only when the probe is
    /// inconclusive; recorded in the audit log).
    pub withdraw_disabled_attested: bool,
    pub scopes_ok: bool,
    pub checked_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketData {
    pub quote_ts: DateTime<Utc>,
    pub bid: Option<Decimal>,
    pub ask: Option<Decimal>,
    /// Quote-currency volume over 24 h (crypto) or 20-day ADV (equities), converted to EUR/USD by caller.
    pub liquidity_notional: Option<Decimal>,
    pub candle_ts: DateTime<Utc>,
    pub candle_interval_secs: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstrumentStatus {
    pub tradable: bool,
    pub eligible: bool,
    pub ineligible_reason: Option<String>,
    pub listed_at: DateTime<Utc>,
    pub min_notional: Decimal,
    pub min_quantity: Decimal,
    pub quantity_decimals: u32,
    /// Session for equities in UTC; `None` for 24/7 markets. `Some(None)` = unknown.
    pub session: Option<Option<(DateTime<Utc>, DateTime<Utc>)>>,
    /// Long-term investment whitelist membership (ETF plan).
    pub long_term_whitelisted: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IntentSummary {
    pub instrument: InstrumentId,
    pub side: Side,
    pub state: OrderState,
    pub class: OrderClass,
    pub created_at: DateTime<Utc>,
    /// Remaining notional in HUF (for open buy orders / in-flight).
    pub notional_huf: Decimal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PositionExposure {
    pub instrument: InstrumentId,
    pub asset_class: AssetClass,
    pub cluster: String,
    pub value_huf: Decimal,
    pub free_quantity: Decimal,
    pub long_term: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClosedTrade {
    pub instrument: InstrumentId,
    pub closed_at: DateTime<Utc>,
    pub net_pnl_huf: Decimal,
    pub stopped_out: bool,
}

/// Everything the engine needs, captured at one instant by the caller.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RiskSnapshot {
    pub live_authorized: BTreeMap<Broker, bool>,
    pub strategy_approved: bool,
    pub kill_global: KillMode,
    pub kill_broker: BTreeMap<Broker, KillMode>,
    pub kill_strategy: BTreeMap<String, KillMode>,
    pub credentials: BTreeMap<Broker, CredentialStatus>,
    pub health: BTreeMap<Broker, BrokerHealth>,
    pub market: Option<MarketData>,
    pub instrument: Option<InstrumentStatus>,
    /// Quote currency -> HUF rate for the instrument, and when the FX rate was published.
    pub fx_to_huf: Option<(Decimal, DateTime<Utc>)>,
    pub account_ts: DateTime<Utc>,
    pub recon_mismatch: bool,
    /// Unitized NAV (per unit) now, at day start (Europe/Budapest), week start, and peak.
    pub nav_unit: Decimal,
    pub nav_unit_day_start: Decimal,
    pub nav_unit_week_start: Decimal,
    pub nav_unit_peak: Decimal,
    /// Drawdown halt latched until the user re-authorizes.
    pub drawdown_halt_latched: bool,
    pub equity_huf: Decimal,
    /// Available cash at the instrument's broker, HUF.
    pub cash_available_huf: Decimal,
    pub cash_reserved_local_huf: Decimal,
    pub taker_fee: Decimal,
    pub positions: Vec<PositionExposure>,
    pub intents: Vec<IntentSummary>,
    pub closed_trades: Vec<ClosedTrade>,
    pub entries_last_hour: u32,
    pub entries_today: u32,
}

const BPS: Decimal = Decimal::from_parts(10_000, 0, 0, false, 0);

pub struct RiskEngine {
    cfg: RiskConfig,
}

impl RiskEngine {
    /// Refuses configurations that violate hard limits.
    pub fn new(cfg: RiskConfig) -> Result<Self, LimitViolation> {
        cfg.validate()?;
        Ok(RiskEngine { cfg })
    }

    pub fn config(&self) -> &RiskConfig {
        &self.cfg
    }

    pub fn evaluate(&self, req: &RiskRequest, s: &RiskSnapshot, now: DateTime<Utc>) -> Verdict {
        let mut v = Vec::<Veto>::new();
        let c = &self.cfg;
        let broker = req.instrument.broker();
        let entry = req.class.is_entry();
        macro_rules! veto {
            ($code:expr, $detail:expr $(,)?) => {
                v.push(Veto { code: $code, detail: $detail })
            };
        }

        if req.quantity <= Decimal::ZERO || req.price <= Decimal::ZERO {
            veto!(VetoCode::DataMissing, "quantity and price must be positive".into());
            return Verdict::Veto { vetoes: v };
        }
        // 0. Venue: Revolut and unknown venues are signal-only.
        if !broker.supports_automated_trading() {
            veto!(VetoCode::VenueNotTradable, format!("{broker} is signal-only"));
            return Verdict::Veto { vetoes: v };
        }
        // 26b. Long-only, no leverage: entries must be buys, exits must be sells.
        if entry && req.side != Side::Buy {
            veto!(VetoCode::Leverage, "entries must be spot buys (no shorting)".into());
        }
        if !entry && req.side != Side::Sell {
            veto!(VetoCode::Leverage, "exits must be sells".into());
        }

        // 1. Live authorization (entries; exits of existing positions remain possible).
        if entry && !s.live_authorized.get(&broker).copied().unwrap_or(false) {
            veto!(VetoCode::LiveNotAuthorized, format!("live trading not authorized for {broker}"));
        }

        // 2. Kill switch: most restrictive of global / broker / strategy.
        let mode = s
            .kill_global
            .max(s.kill_broker.get(&broker).copied().unwrap_or_default())
            .max(s.kill_strategy.get(&req.strategy).copied().unwrap_or_default());
        let kill_blocks = match mode {
            KillMode::Normal => false,
            KillMode::ReduceOnly => entry,
            KillMode::Flatten => entry || req.class == OrderClass::ExitSignal,
            KillMode::Freeze => true,
        };
        if kill_blocks {
            veto!(VetoCode::KillSwitch, format!("kill switch {}", mode.as_str()));
        }

        // 3. Credentials: never trade with a key that may be able to withdraw.
        match s.credentials.get(&broker) {
            None => veto!(VetoCode::CredScope, "no verified credentials".into()),
            Some(cs) => {
                if !(cs.withdraw_disabled_verified || cs.withdraw_disabled_attested) {
                    veto!(VetoCode::CredWithdrawCapable, "withdrawal permission not ruled out".into());
                }
                if !cs.scopes_ok {
                    veto!(VetoCode::CredScope, "missing required key scopes".into());
                }
            }
        }

        // 4. Connector health (applies to exits too: an unhealthy connector cannot send safely).
        match s.health.get(&broker) {
            None => veto!(VetoCode::BrokerUnhealthy, "no health data".into()),
            Some(h) => {
                let ok_recent = h.last_ok.is_some_and(|t| now - t <= Duration::seconds(60));
                let limited = h.last_rate_limited.is_some_and(|t| now - t <= Duration::seconds(120));
                if !ok_recent || limited || h.error_rate_5m >= d("0.20") || h.rate_budget_remaining < d("0.20") {
                    veto!(
                        VetoCode::BrokerUnhealthy,
                        format!(
                            "ok_recent={ok_recent} rate_limited={limited} err={} budget={}",
                            h.error_rate_5m, h.rate_budget_remaining
                        ),
                    );
                }
                if entry && h.clock_skew_ms.abs() > 1000 {
                    veto!(VetoCode::DataStale, format!("clock skew {} ms", h.clock_skew_ms));
                }
            }
        }

        // 5. Data freshness (entries only: stale data never blocks a protective exit).
        let max_quote_age = match req.asset_class {
            AssetClass::Crypto => c.quote_max_age_secs_crypto,
            _ => c.quote_max_age_secs_equity,
        };
        let mut spread_bps: Option<Decimal> = None;
        match &s.market {
            None => {
                if entry || req.class == OrderClass::ExitSignal {
                    veto!(VetoCode::DataMissing, "no market data".into());
                }
            }
            Some(m) => {
                if let (Some(b), Some(a)) = (m.bid, m.ask) {
                    if b > Decimal::ZERO && a >= b {
                        spread_bps = Some((a - b) / ((a + b) / d("2")) * BPS);
                    }
                }
                if entry {
                    if (now - m.quote_ts).num_seconds() > max_quote_age {
                        veto!(VetoCode::DataStale, format!("quote {}s old", (now - m.quote_ts).num_seconds()));
                    }
                    if (now - m.candle_ts).num_seconds() > 2 * m.candle_interval_secs.max(1) + m.candle_interval_secs {
                        veto!(VetoCode::DataStale, "candles older than 2 intervals".into());
                    }
                    if spread_bps.is_none() {
                        veto!(VetoCode::DataMissing, "no valid bid/ask".into());
                    }
                }
            }
        }
        let fx = match s.fx_to_huf {
            Some((rate, ts)) if rate > Decimal::ZERO => {
                if entry && now - ts > Duration::days(c.fx_max_age_days) {
                    veto!(VetoCode::DataStale, "FX rate too old".into());
                }
                Some(rate)
            }
            _ => {
                veto!(VetoCode::DataMissing, "no FX rate to HUF".into());
                None
            }
        };
        if entry && (now - s.account_ts).num_seconds() > c.account_max_age_secs {
            veto!(VetoCode::DataStale, "account snapshot stale".into());
        }

        // 6. Reconciliation.
        if entry {
            if s.recon_mismatch {
                veto!(VetoCode::ReconHalt, "reconciliation mismatch open".into());
            }
            if s
                .intents
                .iter()
                .any(|i| i.state == OrderState::Unknown && (now - i.created_at).num_seconds() > c.max_unknown_age_secs)
            {
                veto!(VetoCode::ReconHalt, "UNKNOWN intent older than limit".into());
            }
        }

        // 7-9. Instrument eligibility, market hours, new listing.
        let inst = s.instrument.as_ref();
        match inst {
            None => veto!(VetoCode::DataMissing, "no instrument metadata".into()),
            Some(i) => {
                if entry {
                    if !i.tradable || !i.eligible {
                        veto!(
                            VetoCode::InstrumentIneligible,
                            i.ineligible_reason.clone().unwrap_or_else(|| "not tradable".into()),
                        );
                    }
                    if req.class == OrderClass::LongTerm && !i.long_term_whitelisted {
                        veto!(VetoCode::InstrumentIneligible, "not in long-term investment whitelist".into());
                    }
                    match i.session {
                        None => {}
                        Some(None) => veto!(VetoCode::DataMissing, "market session unknown".into()),
                        Some(Some((open, close))) => {
                            let buf = Duration::minutes(c.market_open_buffer_mins);
                            if now < open + buf || now > close - buf {
                                veto!(VetoCode::MarketClosed, "outside session (with 15 min buffers)".into());
                            }
                        }
                    }
                    if req.class == OrderClass::Entry && now - i.listed_at < Duration::days(c.new_listing_days) {
                        veto!(VetoCode::NewListing, format!("listed {} days ago", (now - i.listed_at).num_days()));
                    }
                } else if let Some(Some((open, close))) = i.session {
                    // Exits on equities outside the session would queue at the broker; allowed only
                    // as broker-resting stops, which are placed separately.
                    if (now < open || now > close) && req.class != OrderClass::ExitProtective {
                        veto!(VetoCode::MarketClosed, "market closed; protective stop remains at broker".into());
                    }
                }
            }
        }

        // 10-11. Loss limits and drawdown on unitized NAV (deposits cannot hide losses).
        let mut risk_mult = Decimal::ONE;
        if req.class == OrderClass::Entry {
            if s.nav_unit <= Decimal::ZERO || s.nav_unit_day_start <= Decimal::ZERO || s.nav_unit_peak <= Decimal::ZERO {
                veto!(VetoCode::DataMissing, "NAV unavailable".into());
            } else {
                let day = s.nav_unit / s.nav_unit_day_start - Decimal::ONE;
                let week = if s.nav_unit_week_start > Decimal::ZERO {
                    s.nav_unit / s.nav_unit_week_start - Decimal::ONE
                } else {
                    Decimal::ZERO
                };
                let dd = Decimal::ONE - s.nav_unit / s.nav_unit_peak;
                if day <= -c.daily_loss {
                    veto!(VetoCode::DailyLoss, format!("day return {}", pct(day)));
                }
                if week <= -c.weekly_loss {
                    veto!(VetoCode::WeeklyLoss, format!("week return {}", pct(week)));
                }
                if s.drawdown_halt_latched || dd >= c.drawdown_halt {
                    veto!(VetoCode::DrawdownHalt, format!("drawdown {}", pct(dd)));
                } else if dd >= c.drawdown_reduce {
                    risk_mult = d("0.5");
                }
            }

            // 12. Loss chasing.
            let mut trades: Vec<&ClosedTrade> = s.closed_trades.iter().collect();
            trades.sort_by(|a, b| b.closed_at.cmp(&a.closed_at));
            let streak = trades.iter().take_while(|t| t.net_pnl_huf < Decimal::ZERO).count() as u32;
            if streak >= c.loss_streak {
                if let Some(last) = trades.first() {
                    if now - last.closed_at < Duration::hours(c.loss_streak_cooldown_hours) {
                        veto!(VetoCode::LossStreak, format!("{streak} consecutive losses"));
                    }
                }
            }
            let loss_24h = trades
                .iter()
                .any(|t| t.net_pnl_huf < Decimal::ZERO && now - t.closed_at < Duration::hours(24));
            if loss_24h && req.size_multiplier > Decimal::ONE {
                veto!(VetoCode::SizeUpAfterLoss, "size increase after a loss".into());
            }
            if trades.iter().any(|t| {
                t.stopped_out && t.instrument == req.instrument && now - t.closed_at < Duration::hours(c.revenge_reentry_hours)
            }) {
                veto!(VetoCode::RevengeReentry, "re-entry after stop-out too soon".into());
            }

            // 13. Order rate.
            if s.entries_last_hour >= c.max_entries_per_hour || s.entries_today >= c.max_entries_per_day {
                veto!(VetoCode::OrderRate, format!("{} entries/h, {} today", s.entries_last_hour, s.entries_today));
            }
            // 15. Open positions (trading positions only).
            let open = s.positions.iter().filter(|p| !p.long_term).count() as u32;
            let has_pos = s.positions.iter().any(|p| p.instrument == req.instrument);
            if !has_pos && open >= c.max_positions {
                veto!(VetoCode::MaxPositions, format!("{open} open positions"));
            }
            // 16. Strategy approval.
            if !s.strategy_approved {
                veto!(VetoCode::StrategyNotApproved, format!("strategy {} not live-approved", req.strategy));
            }
        }

        // 14. Duplicates / in-flight (all classes).
        for i in s.intents.iter().filter(|i| i.instrument == req.instrument) {
            if matches!(i.state, OrderState::Submitting | OrderState::Unknown | OrderState::Pending) {
                veto!(VetoCode::InFlight, format!("intent in state {}", i.state.as_str()));
            } else if i.side == req.side && i.state == OrderState::Open && !entry {
                veto!(VetoCode::InFlight, "resting sell order must be cancelled first".into());
            } else if i.side == req.side
                && entry
                && (now - i.created_at).num_seconds().abs() < c.duplicate_window_secs
            {
                veto!(VetoCode::Duplicate, "same instrument and side within duplicate window".into());
            }
        }

        // 18. Spread (entries and signal exits).
        if let Some(sp) = spread_bps {
            let max = match req.asset_class {
                AssetClass::Crypto => c.max_spread_bps_crypto,
                _ => c.max_spread_bps_equity,
            };
            if (entry || req.class == OrderClass::ExitSignal) && sp > max {
                veto!(VetoCode::SpreadWide, format!("spread {} bps > {max}", sp.round_dp(1)));
            }
        }

        // 19. Liquidity (entries).
        if entry {
            let min_liq = match req.asset_class {
                AssetClass::Crypto => c.min_volume_24h_eur_crypto,
                _ => c.min_adv_usd_equity,
            };
            match s.market.as_ref().and_then(|m| m.liquidity_notional) {
                None => veto!(VetoCode::DataMissing, "no liquidity data".into()),
                Some(liq) if liq < min_liq => veto!(VetoCode::Illiquid, format!("liquidity {liq} < {min_liq}")),
                Some(_) => {}
            }
        }

        // 26a. Oversell: sells cannot exceed free quantity.
        if req.side == Side::Sell {
            let free = s
                .positions
                .iter()
                .find(|p| p.instrument == req.instrument)
                .map(|p| p.free_quantity)
                .unwrap_or(Decimal::ZERO);
            if req.quantity > free {
                veto!(VetoCode::Oversell, format!("sell {} > free {free}", req.quantity));
            }
        }

        // Sizing checks need FX and instrument metadata; bail if missing.
        let (Some(fx), Some(inst)) = (fx, inst) else {
            return Verdict::Veto { vetoes: v };
        };
        let price_huf = req.price * fx;
        let mut qty = req.quantity;
        let mut binding: Option<VetoCode> = None;
        let clamp = |limit_qty: Decimal, code: VetoCode, qty: &mut Decimal, binding: &mut Option<VetoCode>| {
            let l = limit_qty.max(Decimal::ZERO);
            if l < *qty {
                *qty = l;
                *binding = Some(code);
            }
        };

        if req.class == OrderClass::Entry {
            // 20. Per-trade risk with effective stop (farther of stop and p99 gap).
            match req.stop_price {
                None => veto!(VetoCode::RiskPerTrade, "entry without protective stop".into()),
                Some(stop) if stop >= req.price || stop <= Decimal::ZERO => {
                    veto!(VetoCode::RiskPerTrade, "stop must be below entry".into())
                }
                Some(stop) => {
                    let gap_stop = req.price * (Decimal::ONE - req.gap_p99.max(Decimal::ZERO));
                    let stop_eff = stop.min(gap_stop);
                    let per_unit_loss_huf = (req.price - stop_eff) * fx + price_huf * s.taker_fee * d("2");
                    let budget = s.equity_huf * c.risk_per_trade * risk_mult * req.size_multiplier.min(Decimal::ONE);
                    if per_unit_loss_huf > Decimal::ZERO {
                        clamp(budget / per_unit_loss_huf, VetoCode::RiskPerTrade, &mut qty, &mut binding);
                    }
                }
            }
        }

        if entry {
            // 22. Position cap: existing position + open/in-flight buys + this order.
            let existing: Decimal = s
                .positions
                .iter()
                .filter(|p| p.instrument == req.instrument)
                .map(|p| p.value_huf)
                .sum::<Decimal>()
                + s.intents
                    .iter()
                    .filter(|i| i.instrument == req.instrument && i.side == Side::Buy && i.state.is_live_or_uncertain())
                    .map(|i| i.notional_huf)
                    .sum::<Decimal>();
            if req.class == OrderClass::Entry {
                clamp(
                    (s.equity_huf * c.position_cap - existing) / price_huf,
                    VetoCode::PositionCap,
                    &mut qty,
                    &mut binding,
                );
            }

            // 23. Asset-class and total exposure (trading positions; long-term has its own budget).
            let pending_buys: Decimal = s
                .intents
                .iter()
                .filter(|i| i.side == Side::Buy && i.state.is_live_or_uncertain())
                .map(|i| i.notional_huf)
                .sum();
            if req.class == OrderClass::Entry {
                let class_cap = match req.asset_class {
                    AssetClass::Crypto => c.crypto_cap,
                    _ => c.equity_cap,
                };
                let class_exposure: Decimal = s
                    .positions
                    .iter()
                    .filter(|p| !p.long_term && same_class(p.asset_class, req.asset_class))
                    .map(|p| p.value_huf)
                    .sum();
                clamp(
                    (s.equity_huf * class_cap - class_exposure - pending_buys) / price_huf,
                    VetoCode::ClassCap,
                    &mut qty,
                    &mut binding,
                );
                let total: Decimal = s.positions.iter().filter(|p| !p.long_term).map(|p| p.value_huf).sum();
                clamp(
                    (s.equity_huf * c.total_cap - total - pending_buys) / price_huf,
                    VetoCode::TotalCap,
                    &mut qty,
                    &mut binding,
                );

                // 24. Concentration per correlation cluster.
                let in_cluster: Vec<&PositionExposure> =
                    s.positions.iter().filter(|p| !p.long_term && p.cluster == req.cluster).collect();
                let cluster_new = !in_cluster.iter().any(|p| p.instrument == req.instrument);
                if cluster_new && in_cluster.len() as u32 >= c.max_positions_per_cluster {
                    veto!(VetoCode::Concentration, format!("{} positions in cluster {}", in_cluster.len(), req.cluster));
                }
                let cluster_value: Decimal = in_cluster.iter().map(|p| p.value_huf).sum();
                clamp(
                    (s.equity_huf * c.cluster_cap - cluster_value) / price_huf,
                    VetoCode::Concentration,
                    &mut qty,
                    &mut binding,
                );
            }

            // 25. Cash after local reservations and a floor.
            let floor = (s.equity_huf * c.cash_floor_frac).max(c.cash_floor_min_huf);
            let usable = s.cash_available_huf - s.cash_reserved_local_huf - floor;
            let per_unit_cash = price_huf * (Decimal::ONE + s.taker_fee + c.fx_fee_bps / BPS + c.cash_buffer_frac);
            clamp(usable / per_unit_cash, VetoCode::InsufficientCash, &mut qty, &mut binding);
        }

        // 21. Minimum order size after flooring to venue precision (all classes).
        qty = qty.round_dp_with_strategy(inst.quantity_decimals, rust_decimal::RoundingStrategy::ToZero);
        let notional_quote = qty * req.price;
        let min_notional = inst.min_notional * d("1.02");
        if qty <= Decimal::ZERO || qty < inst.min_quantity || notional_quote < min_notional {
            let code = binding.unwrap_or(VetoCode::BelowMinOrder);
            veto!(
                code,
                format!("final quantity {qty} (notional {}) below venue minimum {}", notional_quote.round_dp(2), min_notional),
            );
            if code != VetoCode::BelowMinOrder {
                veto!(VetoCode::BelowMinOrder, "resized order below venue minimum".into());
            }
        }

        // 17. Fee-aware expected edge on the final size (entries).
        if req.class == OrderClass::Entry {
            let spread = spread_bps.unwrap_or(Decimal::ZERO);
            let fx_leg = if req.asset_class == AssetClass::Crypto { Decimal::ZERO } else { c.fx_fee_bps * d("2") };
            let cost_rt = s.taker_fee * BPS * d("2") + spread + c.slippage_bps * d("2") + fx_leg;
            let conservative = req.edge_bps - req.edge_se_bps;
            if conservative < cost_rt + c.edge_buffer_bps {
                veto!(
                    VetoCode::EdgeBelowCost,
                    format!(
                        "edge-se {} bps < cost {} + buffer {} bps",
                        conservative.round_dp(1),
                        cost_rt.round_dp(1),
                        c.edge_buffer_bps
                    ),
                );
            } else {
                let expected_net_huf = (req.edge_bps - cost_rt) / BPS * qty * price_huf;
                if expected_net_huf < c.min_expected_net_huf {
                    veto!(VetoCode::EdgeBelowCost, format!("expected net {} HUF too small", expected_net_huf.round_dp(0)));
                }
            }
        }

        if !v.is_empty() {
            v.sort_by_key(|x| x.code);
            v.dedup_by(|a, b| a.code == b.code && a.detail == b.detail);
            return Verdict::Veto { vetoes: v };
        }
        if qty < req.quantity {
            Verdict::AllowResized { quantity: qty, requested: req.quantity, binding: binding.unwrap_or(VetoCode::BelowMinOrder) }
        } else {
            Verdict::Allow { quantity: qty }
        }
    }
}

fn same_class(a: AssetClass, b: AssetClass) -> bool {
    matches!(
        (a, b),
        (AssetClass::Crypto, AssetClass::Crypto) | (AssetClass::Stock | AssetClass::Etf, AssetClass::Stock | AssetClass::Etf)
    )
}

fn d(s: &str) -> Decimal {
    s.parse().unwrap_or(Decimal::ZERO)
}

fn pct(x: Decimal) -> String {
    format!("{}%", (x * d("100")).round_dp(2))
}

#[cfg(test)]
mod tests;

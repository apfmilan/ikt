//! Walk-forward backtester (docs/decisions/0005-strategies-validation.md §4).
//!
//! * Signals at bar close `t`, fills at bar `t+1` open (no look-ahead: strategies see only
//!   the prefix `[0, t]`).
//! * Costs: taker/maker fee per side, half spread per side, slippage per side; gapped stops
//!   fill at `min(stop, open) - 0.25*ATR14`.
//! * Fixed-fractional sizing on normalised equity (risk 1 %, notional cap 50 %). Venue minimum
//!   order sizes are not modelled here; live sizing (`tc-strategy::sizing`) enforces them.
//! * Only concatenated out-of-sample results count for promotion.

pub mod metrics;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tc_strategy::indicators::atr;
use tc_strategy::regime::{self, Regime, RegimeTracker};
use tc_strategy::strategies::parameter_grid;
use tc_strategy::{Action, OpenTrade, Series, Strategy};

pub use metrics::{deflated_sharpe, Metrics, Promotion, PromotionRules};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Costs {
    pub taker_fee: f64,
    pub maker_fee: f64,
    pub half_spread_bps: f64,
    pub slippage_bps: f64,
}

impl Costs {
    /// Bitvavo default tier (verified 2026-11-02 docs example: taker 0.25 %, maker 0.15 %).
    pub fn bitvavo_default() -> Self {
        Costs { taker_fee: 0.0025, maker_fee: 0.0015, half_spread_bps: 5.0, slippage_bps: 5.0 }
    }

    pub fn doubled(self) -> Self {
        Costs {
            taker_fee: self.taker_fee * 2.0,
            maker_fee: self.maker_fee * 2.0,
            half_spread_bps: self.half_spread_bps * 2.0,
            slippage_bps: self.slippage_bps * 2.0,
        }
    }

    fn buy_price(&self, px: f64) -> f64 {
        px * (1.0 + (self.half_spread_bps + self.slippage_bps) / 1e4)
    }

    fn sell_price(&self, px: f64) -> f64 {
        px * (1.0 - (self.half_spread_bps + self.slippage_bps) / 1e4)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trade {
    pub entry_ts: DateTime<Utc>,
    pub exit_ts: DateTime<Utc>,
    pub entry_price: f64,
    pub exit_price: f64,
    /// Fraction of equity allocated at entry.
    pub weight: f64,
    /// Gross return (price only) and net return (after all costs), as fractions.
    pub gross_ret: f64,
    pub net_ret: f64,
    /// Net P&L in units of initial risk.
    pub r_multiple: f64,
    pub regime: Regime,
    pub bars_held: u32,
    pub exit_reason: String,
    pub cost_ret: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SimResult {
    pub trades: Vec<Trade>,
    /// Per-bar equity returns within the evaluated window.
    pub bar_returns: Vec<f64>,
    pub exposure_bars: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SimParams {
    pub risk_frac: f64,
    pub max_weight: f64,
    pub bars_per_year: f64,
}

impl Default for SimParams {
    fn default() -> Self {
        SimParams { risk_frac: 0.01, max_weight: 0.5, bars_per_year: 365.0 }
    }
}

struct Open {
    t: OpenTrade,
    entry_ts: DateTime<Utc>,
    entry_cost: f64,
    weight: f64,
    regime: Regime,
    remaining: f64,
    realized_frac_ret: f64,
}

/// Simulate `strategy` on bars `[start, end)` of `s`, using earlier bars as warm-up.
pub fn simulate(s: &Series, strategy: &dyn Strategy, costs: Costs, p: SimParams, start: usize, end: usize) -> SimResult {
    let n = s.len().min(end);
    let mut out = SimResult::default();
    let mut tracker = RegimeTracker::default();
    // Warm the regime tracker on history before `start` (bounded window for speed).
    let warm_from = start.saturating_sub(10);
    for t in warm_from..start.min(n) {
        update_regime(&mut tracker, s, t, p.bars_per_year);
    }
    let mut pos: Option<Open> = None;
    let mut equity = 1.0_f64;
    let mut pending: Option<Action> = None;
    for t in start..n {
        // 1) Execute pending action at bar t open.
        let (o, h, l, c) = (s.open[t], s.high[t], s.low[t], s.close[t]);
        let prev_close = if t > 0 { s.close[t - 1] } else { o };
        let mut bar_ret = 0.0;
        if let Some(act) = pending.take() {
            match act {
                Action::Enter { stop, limit, .. } if pos.is_none() => {
                    let (fill, fee) = match limit {
                        Some(lim) => {
                            if l < lim {
                                (lim, costs.maker_fee)
                            } else {
                                (f64::NAN, 0.0)
                            }
                        }
                        None => (costs.buy_price(o), costs.taker_fee),
                    };
                    if fill.is_finite() && stop < fill {
                        let risk = fill - stop;
                        let weight = (p.risk_frac * fill / risk).min(p.max_weight);
                        let entry_cost = fee + (fill / o - 1.0).max(0.0);
                        pos = Some(Open {
                            t: OpenTrade {
                                entry_price: fill,
                                stop,
                                initial_risk: risk,
                                high_since_entry: fill,
                                bars_held: 0,
                                partial_taken: false,
                            },
                            entry_ts: s.ts[t],
                            entry_cost,
                            weight,
                            regime: tracker.current,
                            remaining: 1.0,
                            realized_frac_ret: 0.0,
                        });
                        // Entry fee hits equity immediately.
                        bar_ret -= weight * fee;
                    }
                }
                Action::Exit { fraction, reason, .. } => {
                    if let Some(mut op) = pos.take() {
                        let px = costs.sell_price(o);
                        let frac = fraction.clamp(0.0, 1.0) * op.remaining;
                        let leg_ret = (px / prev_close - 1.0) * op.weight * frac - op.weight * frac * costs.taker_fee;
                        bar_ret += leg_ret;
                        op.realized_frac_ret += frac * ((px / op.t.entry_price) * (1.0 - costs.taker_fee) - 1.0);
                        op.remaining -= frac;
                        if op.remaining <= 1e-9 {
                            out.trades.push(close_trade(&op, s.ts[t], px, format!("{reason:?}"), costs));
                        } else {
                            op.t.partial_taken = true;
                            // Remaining part keeps tracking from this bar open.
                            pos = Some(op);
                            let rem = pos.as_ref().map(|x| x.remaining * x.weight).unwrap_or(0.0);
                            bar_ret += (c / o - 1.0) * rem;
                            finish_bar(&mut pos, h, &mut out);
                            equity *= 1.0 + bar_ret;
                            out.bar_returns.push(bar_ret);
                            update_regime(&mut tracker, s, t, p.bars_per_year);
                            pending = decide(s, strategy, &tracker, &pos, t);
                            continue;
                        }
                    }
                }
                Action::UpdateStop { stop } => {
                    if let Some(op) = pos.as_mut() {
                        if stop > op.t.stop {
                            op.t.stop = stop;
                        }
                    }
                }
                _ => {}
            }
        }
        // 2) Intrabar: protective stop (gap-aware), mark to market.
        if let Some(mut op) = pos.take() {
            let just_entered = op.entry_ts == s.ts[t];
            let base = if just_entered { op.t.entry_price } else { prev_close };
            let stop = op.t.stop;
            if o <= stop || l <= stop {
                let gap = o <= stop;
                let a14 = atr(&s.high[..=t], &s.low[..=t], &s.close[..=t], 14).unwrap_or(0.0);
                let raw = if gap { o.min(stop) - 0.25 * a14 } else { stop };
                let px = costs.sell_price(raw.max(0.0));
                bar_ret += (px / base - 1.0) * op.weight * op.remaining - op.weight * op.remaining * costs.taker_fee;
                op.realized_frac_ret += op.remaining * ((px / op.t.entry_price) * (1.0 - costs.taker_fee) - 1.0);
                op.remaining = 0.0;
                out.trades.push(close_trade(&op, s.ts[t], px, if gap { "StopGap".into() } else { "Stop".into() }, costs));
            } else {
                bar_ret += (c / base - 1.0) * op.weight * op.remaining;
                pos = Some(op);
            }
        }
        finish_bar(&mut pos, h, &mut out);
        equity *= 1.0 + bar_ret;
        out.bar_returns.push(bar_ret);
        // 3) Decide at close of bar t.
        update_regime(&mut tracker, s, t, p.bars_per_year);
        pending = if t + 1 < n { decide(s, strategy, &tracker, &pos, t) } else { None };
    }
    // Close any open position at the last close (end of window).
    if let Some(op) = pos.take() {
        let last = n.saturating_sub(1);
        let px = costs.sell_price(s.close[last]);
        let fee = op.weight * op.remaining * costs.taker_fee;
        if let Some(r) = out.bar_returns.last_mut() {
            *r += (px / s.close[last] - 1.0) * op.weight * op.remaining - fee;
        }
        let mut op = op;
        op.realized_frac_ret += op.remaining * ((px / op.t.entry_price) * (1.0 - costs.taker_fee) - 1.0);
        op.remaining = 0.0;
        out.trades.push(close_trade(&op, s.ts[last], px, "WindowEnd".into(), costs));
    }
    let _ = equity;
    out
}

fn finish_bar(pos: &mut Option<Open>, high: f64, out: &mut SimResult) {
    if let Some(op) = pos.as_mut() {
        op.t.bars_held += 1;
        if high > op.t.high_since_entry {
            op.t.high_since_entry = high;
        }
        out.exposure_bars += 1;
    }
}

fn decide(s: &Series, strategy: &dyn Strategy, tracker: &RegimeTracker, pos: &Option<Open>, t: usize) -> Option<Action> {
    let from = t.saturating_sub(700);
    let view = window(s, from, t + 1);
    match strategy.evaluate(&view, tracker.current, pos.as_ref().map(|p| &p.t)) {
        Action::None => None,
        a => Some(a),
    }
}

fn update_regime(tracker: &mut RegimeTracker, s: &Series, t: usize, bpy: f64) {
    let from = t.saturating_sub(700);
    let view = window(s, from, t + 1);
    if let Some(i) = regime::inputs(&view, bpy) {
        tracker.update(regime::classify(&i, false));
    }
}

fn window(s: &Series, from: usize, to: usize) -> Series {
    Series {
        ts: s.ts[from..to].to_vec(),
        open: s.open[from..to].to_vec(),
        high: s.high[from..to].to_vec(),
        low: s.low[from..to].to_vec(),
        close: s.close[from..to].to_vec(),
        volume: s.volume[from..to].to_vec(),
    }
}

fn close_trade(op: &Open, ts: DateTime<Utc>, px: f64, reason: String, costs: Costs) -> Trade {
    let gross = px / op.t.entry_price - 1.0;
    let net = op.realized_frac_ret - op.entry_cost;
    let r = if op.t.initial_risk > 0.0 { net * op.t.entry_price / op.t.initial_risk } else { 0.0 };
    let _ = costs;
    Trade {
        entry_ts: op.entry_ts,
        exit_ts: ts,
        entry_price: op.t.entry_price,
        exit_price: px,
        weight: op.weight,
        gross_ret: gross,
        net_ret: net,
        r_multiple: r,
        regime: op.regime,
        bars_held: op.t.bars_held,
        exit_reason: reason,
        cost_ret: gross - net,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WalkForward {
    pub is_bars: usize,
    pub oos_bars: usize,
    pub embargo_bars: usize,
    pub warmup_bars: usize,
}

impl WalkForward {
    /// 1d bars: 24-month IS, 6-month OOS, 6-month step.
    pub fn daily() -> Self {
        WalkForward { is_bars: 730, oos_bars: 182, embargo_bars: 20, warmup_bars: 200 }
    }

    /// 4h bars: 12-month IS, 3-month OOS.
    pub fn four_hour() -> Self {
        WalkForward { is_bars: 2190, oos_bars: 547, embargo_bars: 60, warmup_bars: 400 }
    }

    /// 1h bars (research only; spec rejects sub-1h).
    pub fn hourly() -> Self {
        WalkForward { is_bars: 4380, oos_bars: 1095, embargo_bars: 120, warmup_bars: 700 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FoldReport {
    pub is_from: DateTime<Utc>,
    pub oos_from: DateTime<Utc>,
    pub oos_to: DateTime<Utc>,
    pub chosen_param_index: usize,
    pub is_sharpe: f64,
    pub oos: Metrics,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegimeEdge {
    pub regime: Regime,
    pub trades: usize,
    pub mean_gross_bps: f64,
    pub se_bps: f64,
    pub win_rate: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub strategy: String,
    pub market: String,
    pub interval: String,
    pub bars: usize,
    pub trials: usize,
    pub folds: Vec<FoldReport>,
    pub oos: Metrics,
    pub oos_double_cost_profit_factor: f64,
    pub is_sharpe_mean: f64,
    pub deflated_sharpe: f64,
    pub regime_edges: Vec<RegimeEdge>,
    pub promotion: Promotion,
    pub oos_trades: Vec<Trade>,
}

/// Run walk-forward validation for one strategy family on one market.
#[allow(clippy::too_many_arguments)]
pub fn walk_forward(
    s: &Series,
    family: &str,
    market: &str,
    interval: &str,
    costs: Costs,
    sim: SimParams,
    wf: WalkForward,
    rules: &PromotionRules,
) -> Option<Report> {
    let grid = parameter_grid(family);
    if grid.is_empty() || s.len() < wf.warmup_bars + wf.is_bars + wf.embargo_bars + wf.oos_bars {
        return None;
    }
    let mut folds = Vec::new();
    let mut oos_trades = Vec::new();
    let mut oos_returns = Vec::new();
    let mut oos2_trades = Vec::new();
    let mut is_sharpes_all = Vec::new();
    let mut is_chosen = Vec::new();
    let mut is_start = wf.warmup_bars;
    while is_start + wf.is_bars + wf.embargo_bars + wf.oos_bars <= s.len() {
        let is_end = is_start + wf.is_bars;
        let oos_start = is_end + wf.embargo_bars;
        let oos_end = oos_start + wf.oos_bars;
        // In-sample: evaluate every pre-registered parameter set.
        let sharpes: Vec<f64> = grid
            .iter()
            .map(|st| metrics::sharpe(&simulate(s, st.as_ref(), costs, sim, is_start, is_end).bar_returns, sim.bars_per_year))
            .collect();
        is_sharpes_all.extend(sharpes.iter().copied());
        // Plateau, not peak: average each set with its grid neighbours.
        let plateau: Vec<f64> = (0..sharpes.len())
            .map(|i| {
                let lo = i.saturating_sub(1);
                let hi = (i + 1).min(sharpes.len() - 1);
                sharpes[lo..=hi].iter().sum::<f64>() / (hi - lo + 1) as f64
            })
            .collect();
        let chosen = plateau
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap_or(0);
        is_chosen.push(sharpes[chosen]);
        let r = simulate(s, grid[chosen].as_ref(), costs, sim, oos_start, oos_end);
        let r2 = simulate(s, grid[chosen].as_ref(), costs.doubled(), sim, oos_start, oos_end);
        folds.push(FoldReport {
            is_from: s.ts[is_start],
            oos_from: s.ts[oos_start],
            oos_to: s.ts[oos_end - 1],
            chosen_param_index: chosen,
            is_sharpe: sharpes[chosen],
            oos: Metrics::compute(&r.trades, &r.bar_returns, r.exposure_bars, sim.bars_per_year),
        });
        oos_trades.extend(r.trades);
        oos_returns.extend(r.bar_returns);
        oos2_trades.extend(r2.trades);
        is_start += wf.oos_bars;
    }
    let exposure: usize = folds.iter().map(|f| (f.oos.exposure * wf.oos_bars as f64) as usize).sum();
    let oos = Metrics::compute(&oos_trades, &oos_returns, exposure, sim.bars_per_year);
    let pf2 = Metrics::compute(&oos2_trades, &[], 0, sim.bars_per_year).profit_factor;
    let is_sharpe_mean = tc_strategy::stats::mean(&is_chosen);
    let dsr = deflated_sharpe(&oos_returns, &is_sharpes_all, grid.len(), sim.bars_per_year);
    let regime_edges = regime_edges(&oos_trades);
    let positive_folds = folds.iter().filter(|f| f.oos.net_return > 0.0).count() as f64 / folds.len().max(1) as f64;
    let promotion = rules.evaluate(&oos, pf2, dsr, positive_folds, is_sharpe_mean);
    Some(Report {
        strategy: family.into(),
        market: market.into(),
        interval: interval.into(),
        bars: s.len(),
        trials: grid.len(),
        folds,
        oos,
        oos_double_cost_profit_factor: pf2,
        is_sharpe_mean,
        deflated_sharpe: dsr,
        regime_edges,
        promotion,
        oos_trades,
    })
}

/// OOS gross edge per regime bucket (feeds live fusion: mu_i, s_i, p_i).
pub fn regime_edges(trades: &[Trade]) -> Vec<RegimeEdge> {
    Regime::ALL
        .iter()
        .filter_map(|r| {
            let x: Vec<f64> = trades.iter().filter(|t| t.regime == *r).map(|t| t.gross_ret * 1e4).collect();
            if x.is_empty() {
                return None;
            }
            let wins = trades.iter().filter(|t| t.regime == *r && t.net_ret > 0.0).count();
            Some(RegimeEdge {
                regime: *r,
                trades: x.len(),
                mean_gross_bps: tc_strategy::stats::mean(&x),
                se_bps: if x.len() > 1 { tc_strategy::stats::sd(&x) / (x.len() as f64).sqrt() } else { f64::INFINITY },
                win_rate: wins as f64 / x.len() as f64,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;

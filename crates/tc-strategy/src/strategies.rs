//! Long-only spot strategies S1-S3 (docs/decisions/0005-strategies-validation.md §1).
//! Signals are computed on the close of the last bar; fills happen on the next bar.

use serde::{Deserialize, Serialize};

use crate::indicators::{atr, log_returns, log_zscore, min_last, prior_max, sma, stdev};
use crate::regime::Regime;
use crate::Series;

/// State of an open trade that a strategy needs to manage exits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenTrade {
    pub entry_price: f64,
    pub stop: f64,
    /// Initial risk per unit (entry − initial stop), defines 1R.
    pub initial_risk: f64,
    pub high_since_entry: f64,
    pub bars_held: u32,
    pub partial_taken: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    Signal,
    Regime,
    TimeStop,
    TakeProfit,
    ChannelBreak,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    None,
    Enter {
        stop: f64,
        /// Limit price for maker-only entries (S3); `None` = market/taker.
        limit: Option<f64>,
        strength: f64,
        rationale: String,
    },
    Exit {
        reason: ExitReason,
        /// Fraction of the position to sell (1.0 = all).
        fraction: f64,
        rationale: String,
    },
    /// Raise the protective stop (never lowered).
    UpdateStop {
        stop: f64,
    },
}

pub trait Strategy: Send + Sync {
    fn name(&self) -> &'static str;
    fn version(&self) -> u32;
    /// Bars needed before the strategy may act.
    fn min_bars(&self) -> usize;
    /// Uses maker (post-only limit) entries.
    fn maker_entries(&self) -> bool {
        false
    }
    fn evaluate(&self, s: &Series, regime: Regime, pos: Option<&OpenTrade>) -> Action;
}

fn raise_stop(pos: &OpenTrade, candidate: f64) -> Action {
    if candidate.is_finite() && candidate > pos.stop {
        Action::UpdateStop { stop: candidate }
    } else {
        Action::None
    }
}

/// S1: multi-horizon trend score with volatility normalisation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrendVol {
    pub entry_threshold: f64,
    pub stop_atr: f64,
    pub trail_atr: f64,
}

impl Default for TrendVol {
    fn default() -> Self {
        TrendVol {
            entry_threshold: 0.3,
            stop_atr: 2.5,
            trail_atr: 3.0,
        }
    }
}

impl TrendVol {
    pub fn score(c: &[f64]) -> Option<f64> {
        let r = log_returns(c);
        let sigma = stdev(&r, 60)?;
        if sigma <= 0.0 {
            return None;
        }
        let last = *c.last()?;
        let mut acc = 0.0;
        for l in [20usize, 60, 120] {
            let past = *c.get(c.len().checked_sub(l + 1)?)?;
            if past <= 0.0 {
                return None;
            }
            acc += ((last / past).ln() / (sigma * (l as f64).sqrt())).clamp(-1.0, 1.0);
        }
        Some(acc / 3.0)
    }
}

impl Strategy for TrendVol {
    fn name(&self) -> &'static str {
        "s1_trend_vol"
    }
    fn version(&self) -> u32 {
        1
    }
    fn min_bars(&self) -> usize {
        130
    }
    fn evaluate(&self, s: &Series, regime: Regime, pos: Option<&OpenTrade>) -> Action {
        let c = &s.close;
        if c.len() < self.min_bars() {
            return Action::None;
        }
        let (Some(score), Some(a20), Some(sma100), Some(&close)) = (
            Self::score(c),
            atr(&s.high, &s.low, c, 20),
            sma(c, 100),
            c.last(),
        ) else {
            return Action::None;
        };
        match pos {
            None => {
                if regime == Regime::TrendUp && score >= self.entry_threshold && close > sma100 {
                    Action::Enter {
                        stop: close - self.stop_atr * a20,
                        limit: None,
                        strength: score.clamp(0.0, 1.0),
                        rationale: format!(
                            "trend score {score:.2} >= {:.2}, close above SMA100, regime TREND_UP",
                            self.entry_threshold
                        ),
                    }
                } else {
                    Action::None
                }
            }
            Some(p) => {
                if matches!(regime, Regime::TrendDown | Regime::Crisis) {
                    return Action::Exit {
                        reason: ExitReason::Regime,
                        fraction: 1.0,
                        rationale: format!("regime {}", regime.as_str()),
                    };
                }
                if score < 0.0 {
                    return Action::Exit {
                        reason: ExitReason::Signal,
                        fraction: 1.0,
                        rationale: format!("trend score {score:.2} < 0"),
                    };
                }
                let mult = if regime == Regime::Range {
                    2.0
                } else {
                    self.trail_atr
                };
                raise_stop(p, p.high_since_entry - mult * a20)
            }
        }
    }
}

/// S2: Donchian breakout with volume and volatility-expansion filters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Donchian {
    pub n: usize,
    pub stop_atr: f64,
    pub trail_atr: f64,
    pub min_volume_z: f64,
}

impl Default for Donchian {
    fn default() -> Self {
        Donchian {
            n: 20,
            stop_atr: 2.0,
            trail_atr: 3.0,
            min_volume_z: 1.0,
        }
    }
}

impl Strategy for Donchian {
    fn name(&self) -> &'static str {
        "s2_donchian"
    }
    fn version(&self) -> u32 {
        1
    }
    fn min_bars(&self) -> usize {
        self.n.max(101) + 1
    }
    fn evaluate(&self, s: &Series, regime: Regime, pos: Option<&OpenTrade>) -> Action {
        let c = &s.close;
        if c.len() < self.min_bars() {
            return Action::None;
        }
        let (Some(a14), Some(a100), Some(&close)) = (
            atr(&s.high, &s.low, c, 14),
            atr(&s.high, &s.low, c, 100),
            c.last(),
        ) else {
            return Action::None;
        };
        match pos {
            None => {
                let Some(hh) = prior_max(&s.high, self.n) else {
                    return Action::None;
                };
                let vz = log_zscore(&s.volume, 30).unwrap_or(0.0);
                let expanding = a100 > 0.0 && a14 / a100 >= 1.0;
                if close > hh
                    && vz >= self.min_volume_z
                    && expanding
                    && matches!(regime, Regime::TrendUp | Regime::Transition)
                {
                    Action::Enter {
                        stop: close - self.stop_atr * a14,
                        limit: None,
                        strength: (vz / 3.0).clamp(0.2, 1.0),
                        rationale: format!(
                            "close {close:.4} broke {}-bar high {hh:.4}, volume z {vz:.2}, ATR14/ATR100 {:.2}",
                            self.n,
                            a14 / a100
                        ),
                    }
                } else {
                    Action::None
                }
            }
            Some(p) => {
                if matches!(regime, Regime::TrendDown | Regime::Crisis) {
                    return Action::Exit {
                        reason: ExitReason::Regime,
                        fraction: 1.0,
                        rationale: format!("regime {}", regime.as_str()),
                    };
                }
                if let Some(ll) = min_last(&s.low[..s.low.len() - 1], self.n / 2) {
                    if close < ll {
                        return Action::Exit {
                            reason: ExitReason::ChannelBreak,
                            fraction: 1.0,
                            rationale: format!("close below {}-bar low", self.n / 2),
                        };
                    }
                }
                let r_mult = if p.initial_risk > 0.0 {
                    (close - p.entry_price) / p.initial_risk
                } else {
                    0.0
                };
                if !p.partial_taken && r_mult >= 2.0 {
                    return Action::Exit {
                        reason: ExitReason::TakeProfit,
                        fraction: 0.5,
                        rationale: "partial take-profit at +2R".into(),
                    };
                }
                if p.bars_held >= 10 && r_mult < 0.5 {
                    return Action::Exit {
                        reason: ExitReason::TimeStop,
                        fraction: 1.0,
                        rationale: "time stop: 10 bars below +0.5R".into(),
                    };
                }
                if r_mult >= 1.0 {
                    raise_stop(p, p.high_since_entry - self.trail_atr * a14)
                } else {
                    Action::None
                }
            }
        }
    }
}

/// S3: range mean reversion with maker-only entries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeanReversion {
    pub entry_z: f64,
    pub stop_atr: f64,
    pub max_bars: u32,
}

impl Default for MeanReversion {
    fn default() -> Self {
        MeanReversion {
            entry_z: -2.0,
            stop_atr: 1.5,
            max_bars: 5,
        }
    }
}

impl MeanReversion {
    fn z(c: &[f64]) -> Option<f64> {
        let m = sma(c, 20)?;
        let sd = stdev(c, 20)?;
        if sd <= 0.0 {
            return None;
        }
        Some((c.last()? - m) / sd)
    }
}

impl Strategy for MeanReversion {
    fn name(&self) -> &'static str {
        "s3_mean_reversion"
    }
    fn version(&self) -> u32 {
        1
    }
    fn min_bars(&self) -> usize {
        40
    }
    fn maker_entries(&self) -> bool {
        true
    }
    fn evaluate(&self, s: &Series, regime: Regime, pos: Option<&OpenTrade>) -> Action {
        let c = &s.close;
        if c.len() < self.min_bars() {
            return Action::None;
        }
        let (Some(z), Some(a14), Some(&close)) =
            (Self::z(c), atr(&s.high, &s.low, c, 14), c.last())
        else {
            return Action::None;
        };
        match pos {
            None => {
                if regime == Regime::Range && z <= self.entry_z {
                    Action::Enter {
                        stop: close - self.stop_atr * a14,
                        limit: Some(close),
                        strength: (-z / 3.0).clamp(0.0, 1.0),
                        rationale: format!(
                            "z-score {z:.2} <= {:.1} in RANGE; maker limit at close",
                            self.entry_z
                        ),
                    }
                } else {
                    Action::None
                }
            }
            Some(p) => {
                if matches!(regime, Regime::TrendDown | Regime::Crisis) {
                    Action::Exit {
                        reason: ExitReason::Regime,
                        fraction: 1.0,
                        rationale: format!("regime {}", regime.as_str()),
                    }
                } else if z >= 0.0 {
                    Action::Exit {
                        reason: ExitReason::Signal,
                        fraction: 1.0,
                        rationale: format!("z-score {z:.2} reverted to mean"),
                    }
                } else if p.bars_held >= self.max_bars {
                    Action::Exit {
                        reason: ExitReason::TimeStop,
                        fraction: 1.0,
                        rationale: "time stop 5 bars".into(),
                    }
                } else {
                    Action::None
                }
            }
        }
    }
}

/// All registered strategies with default (pre-registered) parameters.
pub fn registry() -> Vec<Box<dyn Strategy>> {
    vec![
        Box::new(TrendVol::default()),
        Box::new(Donchian::default()),
        Box::new(MeanReversion::default()),
    ]
}

/// Pre-registered parameter grids (≤ 20 sets per family, ordered so neighbours are similar).
pub fn parameter_grid(name: &str) -> Vec<Box<dyn Strategy>> {
    let mut out: Vec<Box<dyn Strategy>> = Vec::new();
    match name {
        "s1_trend_vol" => {
            for th in [0.2, 0.3, 0.4] {
                for st in [2.0, 2.5, 3.0] {
                    out.push(Box::new(TrendVol {
                        entry_threshold: th,
                        stop_atr: st,
                        trail_atr: st + 0.5,
                    }));
                }
            }
        }
        "s2_donchian" => {
            for n in [20usize, 30, 55] {
                for st in [1.5, 2.0, 2.5] {
                    out.push(Box::new(Donchian {
                        n,
                        stop_atr: st,
                        trail_atr: 3.0,
                        min_volume_z: 1.0,
                    }));
                }
            }
        }
        "s3_mean_reversion" => {
            for z in [-1.5, -2.0, -2.5] {
                for st in [1.0, 1.5, 2.0] {
                    out.push(Box::new(MeanReversion {
                        entry_z: z,
                        stop_atr: st,
                        max_bars: 5,
                    }));
                }
            }
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};

    fn series(c: Vec<f64>) -> Series {
        let t0 = Utc
            .with_ymd_and_hms(2025, 1, 1, 0, 0, 0)
            .single()
            .expect("time");
        Series {
            ts: (0..c.len())
                .map(|i| t0 + Duration::days(i as i64))
                .collect(),
            open: c.clone(),
            high: c.iter().map(|x| x * 1.01).collect(),
            low: c.iter().map(|x| x * 0.99).collect(),
            volume: vec![100.0; c.len()],
            close: c,
        }
    }

    #[test]
    fn s1_enters_in_uptrend_only() {
        let c: Vec<f64> = (0..200)
            .map(|i| 100.0 * (1.0 + 0.004 * i as f64) + (i % 3) as f64 * 0.3)
            .collect();
        let s = series(c);
        let st = TrendVol::default();
        assert!(matches!(
            st.evaluate(&s, Regime::TrendUp, None),
            Action::Enter { .. }
        ));
        assert_eq!(st.evaluate(&s, Regime::Range, None), Action::None);
        let p = OpenTrade {
            entry_price: 150.0,
            stop: 140.0,
            initial_risk: 10.0,
            high_since_entry: 180.0,
            bars_held: 5,
            partial_taken: false,
        };
        assert!(matches!(
            st.evaluate(&s, Regime::Crisis, Some(&p)),
            Action::Exit {
                reason: ExitReason::Regime,
                ..
            }
        ));
    }

    #[test]
    fn s1_stop_is_below_entry() {
        let c: Vec<f64> = (0..200)
            .map(|i| 100.0 + i as f64 * 0.5 + (i % 4) as f64)
            .collect();
        let s = series(c.clone());
        if let Action::Enter { stop, .. } = TrendVol::default().evaluate(&s, Regime::TrendUp, None)
        {
            assert!(stop < *c.last().expect("last"));
        }
    }

    #[test]
    fn s3_mean_reversion_entry_and_exit() {
        let mut c: Vec<f64> = (0..60)
            .map(|i| 100.0 + if i % 2 == 0 { 0.5 } else { -0.5 })
            .collect();
        c.push(96.0);
        let s = series(c);
        let st = MeanReversion::default();
        match st.evaluate(&s, Regime::Range, None) {
            Action::Enter { limit, stop, .. } => {
                assert_eq!(limit, Some(96.0));
                assert!(stop < 96.0);
            }
            other => panic!("expected entry, got {other:?}"),
        }
        assert_eq!(st.evaluate(&s, Regime::TrendUp, None), Action::None);
    }

    #[test]
    fn stops_only_move_up() {
        let p = OpenTrade {
            entry_price: 10.0,
            stop: 9.0,
            initial_risk: 1.0,
            high_since_entry: 10.0,
            bars_held: 1,
            partial_taken: false,
        };
        assert_eq!(raise_stop(&p, 8.0), Action::None);
        assert_eq!(raise_stop(&p, 9.5), Action::UpdateStop { stop: 9.5 });
    }

    #[test]
    fn grids_are_bounded() {
        for s in registry() {
            let g = parameter_grid(s.name());
            assert!(!g.is_empty() && g.len() <= 20);
        }
    }
}

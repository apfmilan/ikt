//! Deterministic market-regime detection (docs/decisions/0005-strategies-validation.md §2).

use serde::{Deserialize, Serialize};

use crate::indicators::{
    adx, ewma_vol, log_returns, percentile_rank, rolling_vol, sma, trend_tstat,
};
use crate::Series;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Regime {
    Crisis,
    TrendUp,
    TrendDown,
    Range,
    Transition,
}

impl Regime {
    pub fn as_str(&self) -> &'static str {
        match self {
            Regime::Crisis => "CRISIS",
            Regime::TrendUp => "TREND_UP",
            Regime::TrendDown => "TREND_DOWN",
            Regime::Range => "RANGE",
            Regime::Transition => "TRANSITION",
        }
    }

    pub const ALL: [Regime; 5] = [
        Regime::Crisis,
        Regime::TrendUp,
        Regime::TrendDown,
        Regime::Range,
        Regime::Transition,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegimeInputs {
    pub adx: f64,
    pub t_beta: f64,
    pub p_sigma: f64,
    /// |last return| / EWMA sigma.
    pub shock: f64,
    pub above_sma100: bool,
}

/// Compute regime inputs from a series; `None` until enough history exists.
pub fn inputs(s: &Series, bars_per_year: f64) -> Option<RegimeInputs> {
    let n = s.close.len();
    if n < 160 {
        return None;
    }
    let adx = adx(&s.high, &s.low, &s.close, 14)?;
    let t_beta = trend_tstat(&s.close, 50)?;
    let vols = rolling_vol(&s.close, 20, bars_per_year);
    let window = vols.len().min(365);
    let p_sigma = percentile_rank(&vols, window)?;
    let r = log_returns(&s.close);
    let last_r = *r.last()?;
    let ev = ewma_vol(&s.close)?;
    let shock = if ev > 0.0 { last_r.abs() / ev } else { 0.0 };
    let sma100 = sma(&s.close, 100)?;
    Some(RegimeInputs {
        adx,
        t_beta,
        p_sigma,
        shock,
        above_sma100: *s.close.last()? > sma100,
    })
}

/// Raw classification for one bar, priority order per spec.
pub fn classify(i: &RegimeInputs, breadth_stress: bool) -> Regime {
    if i.p_sigma >= 0.95 || i.shock >= 4.0 || breadth_stress {
        Regime::Crisis
    } else if i.adx >= 25.0 && i.t_beta >= 2.0 && i.above_sma100 {
        Regime::TrendUp
    } else if i.adx >= 25.0 && i.t_beta <= -2.0 {
        Regime::TrendDown
    } else if i.adx <= 20.0 && i.t_beta.abs() < 1.0 && i.p_sigma <= 0.80 {
        Regime::Range
    } else {
        Regime::Transition
    }
}

/// Hysteresis: a new state must hold 2 closes; CRISIS starts immediately and ends after
/// 3 consecutive non-crisis closes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegimeTracker {
    pub current: Regime,
    candidate: Option<Regime>,
    candidate_count: u32,
    calm_count: u32,
}

impl Default for RegimeTracker {
    fn default() -> Self {
        RegimeTracker {
            current: Regime::Transition,
            candidate: None,
            candidate_count: 0,
            calm_count: 0,
        }
    }
}

impl RegimeTracker {
    pub fn update(&mut self, raw: Regime) -> Regime {
        if raw == Regime::Crisis {
            self.current = Regime::Crisis;
            self.candidate = None;
            self.candidate_count = 0;
            self.calm_count = 0;
            return self.current;
        }
        if self.current == Regime::Crisis {
            self.calm_count += 1;
            if self.calm_count < 3 {
                return self.current;
            }
            self.current = raw;
            self.calm_count = 0;
            return self.current;
        }
        if raw == self.current {
            self.candidate = None;
            self.candidate_count = 0;
            return self.current;
        }
        if self.candidate == Some(raw) {
            self.candidate_count += 1;
        } else {
            self.candidate = Some(raw);
            self.candidate_count = 1;
        }
        if self.candidate_count >= 2 {
            self.current = raw;
            self.candidate = None;
            self.candidate_count = 0;
        }
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_priority() {
        let base = RegimeInputs {
            adx: 30.0,
            t_beta: 3.0,
            p_sigma: 0.5,
            shock: 1.0,
            above_sma100: true,
        };
        assert_eq!(classify(&base, false), Regime::TrendUp);
        assert_eq!(
            classify(
                &RegimeInputs {
                    p_sigma: 0.96,
                    ..base
                },
                false
            ),
            Regime::Crisis
        );
        assert_eq!(classify(&base, true), Regime::Crisis);
        assert_eq!(
            classify(
                &RegimeInputs {
                    t_beta: -3.0,
                    ..base
                },
                false
            ),
            Regime::TrendDown
        );
        assert_eq!(
            classify(
                &RegimeInputs {
                    adx: 15.0,
                    t_beta: 0.2,
                    ..base
                },
                false
            ),
            Regime::Range
        );
        assert_eq!(
            classify(&RegimeInputs { adx: 22.0, ..base }, false),
            Regime::Transition
        );
    }

    #[test]
    fn hysteresis() {
        let mut t = RegimeTracker::default();
        assert_eq!(t.update(Regime::TrendUp), Regime::Transition);
        assert_eq!(t.update(Regime::TrendUp), Regime::TrendUp);
        assert_eq!(t.update(Regime::Crisis), Regime::Crisis);
        assert_eq!(t.update(Regime::Range), Regime::Crisis);
        assert_eq!(t.update(Regime::Range), Regime::Crisis);
        assert_eq!(t.update(Regime::Range), Regime::Range);
    }
}

//! Deterministic trading intelligence: indicators, regime detection, strategies, signal
//! fusion and position sizing. No LLM output is trusted here; everything is reproducible
//! from candles and validated by `tc-backtest` before it may trade.

pub mod fusion;
pub mod indicators;
pub mod regime;
pub mod sizing;
pub mod stats;
pub mod strategies;

use chrono::{DateTime, Utc};
use rust_decimal::prelude::ToPrimitive;
use tc_core::Candle;

pub use fusion::{fuse, AbstainReason, Fused, FusionParams, Opinion};
pub use regime::{Regime, RegimeInputs, RegimeTracker};
pub use strategies::{registry, Action, ExitReason, OpenTrade, Strategy};

/// Column-oriented OHLCV series used by indicators (oldest first).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Series {
    pub ts: Vec<DateTime<Utc>>,
    pub open: Vec<f64>,
    pub high: Vec<f64>,
    pub low: Vec<f64>,
    pub close: Vec<f64>,
    pub volume: Vec<f64>,
}

impl Series {
    pub fn from_candles(c: &[Candle]) -> Self {
        let f = |d: rust_decimal::Decimal| d.to_f64().unwrap_or(f64::NAN);
        Series {
            ts: c.iter().map(|x| x.ts).collect(),
            open: c.iter().map(|x| f(x.open)).collect(),
            high: c.iter().map(|x| f(x.high)).collect(),
            low: c.iter().map(|x| f(x.low)).collect(),
            close: c.iter().map(|x| f(x.close)).collect(),
            volume: c.iter().map(|x| f(x.volume)).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.close.len()
    }

    pub fn is_empty(&self) -> bool {
        self.close.is_empty()
    }

    /// Prefix view `[0, end)` (used by the backtester to avoid look-ahead).
    pub fn prefix(&self, end: usize) -> Series {
        let e = end.min(self.len());
        Series {
            ts: self.ts[..e].to_vec(),
            open: self.open[..e].to_vec(),
            high: self.high[..e].to_vec(),
            low: self.low[..e].to_vec(),
            close: self.close[..e].to_vec(),
            volume: self.volume[..e].to_vec(),
        }
    }
}

/// Volume / return shock detector (veto-only input to fusion).
pub fn shock(s: &Series) -> Option<(f64, f64)> {
    let vz = indicators::log_zscore(&s.volume, 30)?;
    let r = indicators::log_returns(&s.close);
    let ev = indicators::ewma_vol(&s.close)?;
    let zr = if ev > 0.0 { r.last()? / ev } else { 0.0 };
    Some((vz, zr))
}

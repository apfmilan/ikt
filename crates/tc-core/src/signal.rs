//! Signals from runtime agents and the fused, explainable decision.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::InstrumentId;

/// Runtime intelligence agents. Every agent is deterministic Rust code; none may place
/// orders. Only the execution path (after the risk engine) can.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Scanner,
    Technical,
    Regime,
    Volatility,
    News,
    Listings,
    Ranker,
    Portfolio,
    Exit,
    Risk,
    Execution,
    Reconciler,
    LongTermInvest,
    RevolutWatch,
}

impl AgentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentKind::Scanner => "scanner",
            AgentKind::Technical => "technical",
            AgentKind::Regime => "regime",
            AgentKind::Volatility => "volatility",
            AgentKind::News => "news",
            AgentKind::Listings => "listings",
            AgentKind::Ranker => "ranker",
            AgentKind::Portfolio => "portfolio",
            AgentKind::Exit => "exit",
            AgentKind::Risk => "risk",
            AgentKind::Execution => "execution",
            AgentKind::Reconciler => "reconciler",
            AgentKind::LongTermInvest => "long_term_invest",
            AgentKind::RevolutWatch => "revolut_watch",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Long,
    Flat,
    Exit,
}

/// One agent's opinion on one instrument at one point in time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    pub agent: AgentKind,
    pub strategy: String,
    pub instrument: InstrumentId,
    pub direction: Direction,
    /// Signed strength in [-1, 1].
    pub strength: f64,
    /// Agent's own confidence in [0, 1] (calibrated from backtest hit rates where available).
    pub confidence: f64,
    /// Expected gross edge over the horizon, in basis points (before costs).
    pub expected_edge_bps: f64,
    /// Standard error of the edge estimate, in basis points.
    pub edge_stderr_bps: f64,
    pub horizon_bars: u32,
    pub generated_at: DateTime<Utc>,
    /// Data timestamp the signal is based on (last closed candle).
    pub data_as_of: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
    pub rationale: String,
}

impl Signal {
    pub fn is_stale(&self, now: DateTime<Utc>) -> bool {
        now > self.valid_until
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAction {
    Enter,
    Exit,
    Hold,
    Abstain,
}

impl DecisionAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            DecisionAction::Enter => "enter",
            DecisionAction::Exit => "exit",
            DecisionAction::Hold => "hold",
            DecisionAction::Abstain => "abstain",
        }
    }
}

/// Probability-based estimate. Always presented as a forecast, never as a result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Forecast {
    /// Estimated probability that the trade closes with positive net P&L.
    pub p_win: f64,
    /// Expected net return in bps after estimated costs.
    pub expected_net_bps: f64,
    /// Estimated round-trip costs in bps (fees + spread + slippage).
    pub cost_bps: f64,
    /// Uncertainty penalty applied (k * stderr), bps.
    pub uncertainty_bps: f64,
    /// 5th / 95th percentile return estimate, bps.
    pub p05_bps: f64,
    pub p95_bps: f64,
}

/// Explainable decision produced by signal fusion, before the risk engine verdict.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub id: Uuid,
    pub instrument: InstrumentId,
    pub action: DecisionAction,
    pub strategy: String,
    pub confidence: f64,
    pub forecast: Forecast,
    pub signals: Vec<Signal>,
    /// Human-readable reasons, ordered by importance.
    pub rationale: Vec<String>,
    pub created_at: DateTime<Utc>,
}

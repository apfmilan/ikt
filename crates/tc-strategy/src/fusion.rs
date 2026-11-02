//! Signal fusion and abstention (docs/decisions/0005-strategies-validation.md §3).
//!
//! Precision-weighted edge with a correlation-adjusted uncertainty. A trade is proposed only
//! if `mu_hat - cost - k*sigma_hat > 0` and weighted agreement >= 0.65. The deterministic risk
//! engine still has the final veto.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::stats::norm_cdf;

/// One agent's opinion on an instrument.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Opinion {
    pub agent: String,
    /// +1 enter, -1 exit, 0 neutral (ignored).
    pub direction: i8,
    /// Gross edge in bps over the agent's horizon (OOS mean of the strategy×regime bucket).
    pub mu_bps: f64,
    /// Standard error of `mu_bps` (SD / sqrt(n)).
    pub se_bps: f64,
    /// Reliability in [0, 1] (0.5 for new agents).
    pub reliability: f64,
    /// Calibrated win probability (from OOS), if known.
    pub p_win: Option<f64>,
    pub as_of: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
    /// Veto-only agents (news, shocks) can only block or reduce, never create entries.
    pub veto_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AbstainReason {
    NoSignal,
    Stale,
    Conflict,
    LowAgreement,
    EdgeBelowCost,
    Vetoed,
    InsufficientData,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fused {
    pub mu_hat_bps: f64,
    pub sigma_hat_bps: f64,
    pub cost_bps: f64,
    pub net_edge_bps: f64,
    pub agreement: f64,
    /// Phi((mu_hat - C) / sigma_hat)
    pub confidence: f64,
    pub p_win: f64,
    pub enter: bool,
    pub abstain: Option<AbstainReason>,
    pub notes: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FusionParams {
    pub k: f64,
    pub min_agreement: f64,
    pub error_correlation: f64,
}

impl Default for FusionParams {
    fn default() -> Self {
        FusionParams {
            k: 1.0,
            min_agreement: 0.65,
            error_correlation: 0.5,
        }
    }
}

pub fn fuse(ops: &[Opinion], cost_bps: f64, now: DateTime<Utc>, p: FusionParams) -> Fused {
    let mut notes = Vec::new();
    let abstain = |reason: AbstainReason, notes: Vec<String>| Fused {
        mu_hat_bps: 0.0,
        sigma_hat_bps: 0.0,
        cost_bps,
        net_edge_bps: 0.0,
        agreement: 0.0,
        confidence: 0.0,
        p_win: 0.0,
        enter: false,
        abstain: Some(reason),
        notes,
    };
    let live: Vec<&Opinion> = ops.iter().filter(|o| o.direction != 0).collect();
    let stale = live.iter().filter(|o| now > o.valid_until).count();
    let fresh: Vec<&Opinion> = live
        .iter()
        .copied()
        .filter(|o| now <= o.valid_until)
        .collect();
    if live.is_empty() {
        return abstain(AbstainReason::NoSignal, notes);
    }
    if stale > 0 {
        notes.push(format!("{stale} stale opinion(s) ignored"));
    }
    // Any exit opinion (including veto agents) blocks entries: enter-vs-exit conflict.
    if fresh.iter().any(|o| o.direction < 0) {
        let who: Vec<&str> = fresh
            .iter()
            .filter(|o| o.direction < 0)
            .map(|o| o.agent.as_str())
            .collect();
        notes.push(format!("exit/veto from {}", who.join(", ")));
        let reason = if fresh.iter().any(|o| o.direction < 0 && o.veto_only) {
            AbstainReason::Vetoed
        } else {
            AbstainReason::Conflict
        };
        return abstain(reason, notes);
    }
    let entries: Vec<&Opinion> = fresh
        .iter()
        .copied()
        .filter(|o| o.direction > 0 && !o.veto_only)
        .collect();
    if entries.is_empty() {
        return abstain(
            if stale > 0 {
                AbstainReason::Stale
            } else {
                AbstainReason::NoSignal
            },
            notes,
        );
    }
    let mut sw = 0.0;
    let mut swmu = 0.0;
    let mut swp = 0.0;
    for o in &entries {
        if !(o.se_bps.is_finite() && o.se_bps > 0.0 && o.mu_bps.is_finite()) {
            return abstain(AbstainReason::InsufficientData, notes);
        }
        let w = o.reliability.clamp(0.0, 1.0) / (o.se_bps * o.se_bps);
        sw += w;
        swmu += w * o.mu_bps;
        swp += w * o.p_win.unwrap_or(0.5);
    }
    if sw <= 0.0 {
        return abstain(AbstainReason::InsufficientData, notes);
    }
    let n = entries.len() as f64;
    let mu_hat = swmu / sw;
    let sigma_hat = ((1.0 / sw) * (1.0 + (n - 1.0) * p.error_correlation)).sqrt();
    // Agreement among all fresh directional opinions (only entries remain at this point).
    let agreement = 1.0;
    let net = mu_hat - cost_bps;
    let confidence = if sigma_hat > 0.0 {
        norm_cdf(net / sigma_hat)
    } else {
        0.0
    };
    let p_win = swp / sw;
    let mut out = Fused {
        mu_hat_bps: mu_hat,
        sigma_hat_bps: sigma_hat,
        cost_bps,
        net_edge_bps: net,
        agreement,
        confidence,
        p_win,
        enter: false,
        abstain: None,
        notes,
    };
    if agreement < p.min_agreement {
        out.abstain = Some(AbstainReason::LowAgreement);
    } else if net - p.k * sigma_hat <= 0.0 {
        out.abstain = Some(AbstainReason::EdgeBelowCost);
        out.notes.push(format!(
            "{mu_hat:.0} - {cost_bps:.0} - {:.1}*{sigma_hat:.0} <= 0 bps",
            p.k
        ));
    } else {
        out.enter = true;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn op(mu: f64, sd: f64, n: f64) -> Opinion {
        let now = Utc::now();
        Opinion {
            agent: "s1".into(),
            direction: 1,
            mu_bps: mu,
            se_bps: sd / n.sqrt(),
            reliability: 1.0,
            p_win: Some(0.4),
            as_of: now,
            valid_until: now + Duration::hours(1),
            veto_only: false,
        }
    }

    #[test]
    fn spec_example_trade_vs_abstain() {
        // mu=250, C=60, n=30, SD=800 -> sigma≈146 -> +44 bps: trade.
        let f = fuse(
            &[op(250.0, 800.0, 30.0)],
            60.0,
            Utc::now(),
            FusionParams::default(),
        );
        assert!(f.enter, "{f:?}");
        assert!((f.sigma_hat_bps - 146.06).abs() < 0.5);
        // n=15 -> sigma≈207 -> -17 bps: abstain.
        let f = fuse(
            &[op(250.0, 800.0, 15.0)],
            60.0,
            Utc::now(),
            FusionParams::default(),
        );
        assert_eq!(f.abstain, Some(AbstainReason::EdgeBelowCost));
    }

    #[test]
    fn exit_or_veto_blocks_entry() {
        let mut v = op(0.0, 100.0, 10.0);
        v.direction = -1;
        v.veto_only = true;
        v.agent = "news".into();
        let f = fuse(
            &[op(400.0, 300.0, 50.0), v],
            60.0,
            Utc::now(),
            FusionParams::default(),
        );
        assert_eq!(f.abstain, Some(AbstainReason::Vetoed));
    }

    #[test]
    fn stale_signals_are_ignored() {
        let mut o = op(400.0, 300.0, 50.0);
        o.valid_until = Utc::now() - Duration::minutes(1);
        let f = fuse(&[o], 60.0, Utc::now(), FusionParams::default());
        assert_eq!(f.abstain, Some(AbstainReason::Stale));
    }

    #[test]
    fn correlated_agents_do_not_shrink_uncertainty_naively() {
        let one = fuse(
            &[op(300.0, 800.0, 30.0)],
            60.0,
            Utc::now(),
            FusionParams::default(),
        );
        let two = fuse(
            &[op(300.0, 800.0, 30.0), op(300.0, 800.0, 30.0)],
            60.0,
            Utc::now(),
            FusionParams::default(),
        );
        // Independent agents would give sigma/sqrt(2) ≈ 0.707x; with rho=0.5 we get ≈0.866x.
        let ratio = two.sigma_hat_bps / one.sigma_hat_bps;
        assert!((ratio - 0.866).abs() < 0.01, "{ratio}");
    }

    #[test]
    fn veto_only_agents_cannot_create_entries() {
        let mut v = op(500.0, 100.0, 100.0);
        v.veto_only = true;
        let f = fuse(&[v], 10.0, Utc::now(), FusionParams::default());
        assert!(!f.enter);
    }
}

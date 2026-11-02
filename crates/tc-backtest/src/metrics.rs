use serde::{Deserialize, Serialize};
use tc_strategy::stats::{mean, norm_cdf, norm_inv, sd, skew_kurt};

use crate::Trade;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub trades: usize,
    pub net_return: f64,
    pub cagr: f64,
    pub sharpe: f64,
    pub sortino: f64,
    pub max_drawdown: f64,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub expectancy_r: f64,
    pub expectancy_bps: f64,
    pub avg_bars_held: f64,
    pub exposure: f64,
    pub cost_share: f64,
}

pub fn sharpe(r: &[f64], bars_per_year: f64) -> f64 {
    let s = sd(r);
    if s == 0.0 || r.len() < 2 {
        0.0
    } else {
        mean(r) / s * bars_per_year.sqrt()
    }
}

fn sortino(r: &[f64], bars_per_year: f64) -> f64 {
    let downside: Vec<f64> = r.iter().map(|x| x.min(0.0)).collect();
    let dd = (downside.iter().map(|x| x * x).sum::<f64>() / r.len().max(1) as f64).sqrt();
    if dd == 0.0 {
        0.0
    } else {
        mean(r) / dd * bars_per_year.sqrt()
    }
}

fn max_drawdown(r: &[f64]) -> f64 {
    let mut eq = 1.0_f64;
    let mut peak = 1.0_f64;
    let mut mdd = 0.0_f64;
    for x in r {
        eq *= 1.0 + x;
        peak = peak.max(eq);
        mdd = mdd.max(1.0 - eq / peak);
    }
    mdd
}

impl Metrics {
    pub fn compute(
        trades: &[Trade],
        bar_returns: &[f64],
        exposure_bars: usize,
        bars_per_year: f64,
    ) -> Metrics {
        let n = trades.len();
        let gains: f64 = trades
            .iter()
            .filter(|t| t.net_ret > 0.0)
            .map(|t| t.net_ret * t.weight)
            .sum();
        let losses: f64 = trades
            .iter()
            .filter(|t| t.net_ret < 0.0)
            .map(|t| -t.net_ret * t.weight)
            .sum();
        let pf = if losses > 0.0 {
            gains / losses
        } else if gains > 0.0 {
            f64::INFINITY
        } else {
            0.0
        };
        let net_return = bar_returns.iter().fold(1.0, |e, x| e * (1.0 + x)) - 1.0;
        let years = bar_returns.len() as f64 / bars_per_year;
        let cagr = if years > 0.0 && net_return > -1.0 {
            (1.0 + net_return).powf(1.0 / years) - 1.0
        } else {
            0.0
        };
        let gross_total: f64 = trades.iter().map(|t| (t.gross_ret * t.weight).abs()).sum();
        let cost_total: f64 = trades.iter().map(|t| t.cost_ret * t.weight).sum();
        Metrics {
            trades: n,
            net_return,
            cagr,
            sharpe: sharpe(bar_returns, bars_per_year),
            sortino: sortino(bar_returns, bars_per_year),
            max_drawdown: max_drawdown(bar_returns),
            win_rate: if n > 0 {
                trades.iter().filter(|t| t.net_ret > 0.0).count() as f64 / n as f64
            } else {
                0.0
            },
            profit_factor: pf,
            expectancy_r: mean(&trades.iter().map(|t| t.r_multiple).collect::<Vec<_>>()),
            expectancy_bps: mean(&trades.iter().map(|t| t.net_ret * 1e4).collect::<Vec<_>>()),
            avg_bars_held: mean(
                &trades
                    .iter()
                    .map(|t| t.bars_held as f64)
                    .collect::<Vec<_>>(),
            ),
            exposure: if bar_returns.is_empty() {
                0.0
            } else {
                exposure_bars as f64 / bar_returns.len() as f64
            },
            cost_share: if gross_total > 0.0 {
                cost_total / gross_total
            } else {
                0.0
            },
        }
    }
}

/// Deflated Sharpe Ratio (Bailey & López de Prado). `returns` are OOS per-bar returns,
/// `trial_sharpes` are annualised IS Sharpes of all tried parameter sets.
pub fn deflated_sharpe(
    returns: &[f64],
    trial_sharpes: &[f64],
    n_trials: usize,
    bars_per_year: f64,
) -> f64 {
    let t = returns.len() as f64;
    if t < 3.0 || n_trials == 0 {
        return 0.0;
    }
    let s = sd(returns);
    if s == 0.0 {
        return 0.0;
    }
    let sr = mean(returns) / s; // per-bar
    let per_bar: Vec<f64> = trial_sharpes
        .iter()
        .map(|x| x / bars_per_year.sqrt())
        .collect();
    let v = sd(&per_bar).powi(2);
    let g = 0.577_215_664_9_f64;
    let n = n_trials.max(2) as f64;
    let sr0 = v.sqrt()
        * ((1.0 - g) * norm_inv(1.0 - 1.0 / n)
            + g * norm_inv(1.0 - 1.0 / (n * std::f64::consts::E)));
    let (g3, g4) = skew_kurt(returns);
    let denom = (1.0 - g3 * sr + (g4 - 1.0) / 4.0 * sr * sr)
        .max(1e-12)
        .sqrt();
    norm_cdf((sr - sr0) * (t - 1.0).sqrt() / denom)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PromotionRules {
    pub min_trades: usize,
    pub min_profit_factor: f64,
    pub min_expectancy_r: f64,
    pub min_sharpe: f64,
    pub max_drawdown: f64,
    pub min_positive_folds: f64,
    pub min_oos_is_sharpe_ratio: f64,
    pub min_double_cost_pf: f64,
    pub min_dsr: f64,
}

impl Default for PromotionRules {
    fn default() -> Self {
        PromotionRules {
            min_trades: 30,
            min_profit_factor: 1.3,
            min_expectancy_r: 0.15,
            min_sharpe: 0.7,
            max_drawdown: 0.25,
            min_positive_folds: 0.6,
            min_oos_is_sharpe_ratio: 0.5,
            min_double_cost_pf: 1.05,
            min_dsr: 0.95,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Promotion {
    pub eligible: bool,
    pub failed: Vec<String>,
}

impl PromotionRules {
    pub fn evaluate(
        &self,
        oos: &Metrics,
        pf2: f64,
        dsr: f64,
        positive_folds: f64,
        is_sharpe: f64,
    ) -> Promotion {
        let mut failed = Vec::new();
        let mut req = |ok: bool, msg: String| {
            if !ok {
                failed.push(msg);
            }
        };
        req(
            oos.trades >= self.min_trades,
            format!("trades {} < {}", oos.trades, self.min_trades),
        );
        req(
            oos.profit_factor >= self.min_profit_factor,
            format!("PF {:.2} < {}", oos.profit_factor, self.min_profit_factor),
        );
        req(
            oos.expectancy_r >= self.min_expectancy_r,
            format!(
                "expectancy {:.2}R < {}R",
                oos.expectancy_r, self.min_expectancy_r
            ),
        );
        req(
            oos.sharpe >= self.min_sharpe,
            format!("Sharpe {:.2} < {}", oos.sharpe, self.min_sharpe),
        );
        req(
            oos.max_drawdown <= self.max_drawdown,
            format!(
                "max DD {:.1}% > {:.0}%",
                oos.max_drawdown * 100.0,
                self.max_drawdown * 100.0
            ),
        );
        req(
            positive_folds >= self.min_positive_folds,
            format!(
                "positive folds {:.0}% < {:.0}%",
                positive_folds * 100.0,
                self.min_positive_folds * 100.0
            ),
        );
        let ratio = if is_sharpe > 0.0 {
            oos.sharpe / is_sharpe
        } else {
            0.0
        };
        req(
            ratio >= self.min_oos_is_sharpe_ratio,
            format!(
                "OOS/IS Sharpe {ratio:.2} < {}",
                self.min_oos_is_sharpe_ratio
            ),
        );
        req(
            pf2 >= self.min_double_cost_pf,
            format!("PF at 2x costs {pf2:.2} < {}", self.min_double_cost_pf),
        );
        req(
            dsr >= self.min_dsr,
            format!("DSR {dsr:.3} < {}", self.min_dsr),
        );
        Promotion {
            eligible: failed.is_empty(),
            failed,
        }
    }
}

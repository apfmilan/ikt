//! The user's financial plan and the accounting that keeps deposits separate from profit.
//!
//! * [`PlanConfig`] holds the configurable starting assumptions (defaults = the plan agreed
//!   for Nov 2026 – May 2027+).
//! * [`UnitLedger`] unitizes the account: external cash flows buy/sell units at the current
//!   NAV per unit, so performance and drawdown are never distorted by deposits.
//! * [`daily_summary_hu`] renders the Hungarian push text from *actual* net P&L.
//! * Withdrawals are never initiated by the system; [`ProfitSplit`] only reports how much
//!   is available to withdraw manually.

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tc_core::format_huf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct YearMonth {
    pub year: i32,
    pub month: u32,
}

impl YearMonth {
    pub fn new(year: i32, month: u32) -> Self {
        YearMonth { year, month }
    }

    pub fn of(d: NaiveDate) -> Self {
        YearMonth {
            year: d.year(),
            month: d.month(),
        }
    }

    pub fn next(self) -> Self {
        if self.month == 12 {
            YearMonth {
                year: self.year + 1,
                month: 1,
            }
        } else {
            YearMonth {
                year: self.year,
                month: self.month + 1,
            }
        }
    }

    pub fn label(&self) -> String {
        format!("{:04}-{:02}", self.year, self.month)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EtfTarget {
    /// Instrument id, e.g. `trading212:<ticker>` chosen during onboarding.
    pub instrument: String,
    /// Weight fraction (all weights sum to 1).
    pub weight: Decimal,
}

/// Configurable plan assumptions (all HUF).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanConfig {
    pub start: YearMonth,
    pub initial_huf: Decimal,
    /// Monthly contribution after the start month, up to and including `phase1_until`.
    pub phase1_monthly_huf: Decimal,
    pub phase1_until: YearMonth,
    /// Combined monthly amount from `phase2_from` (trading + long-term investing).
    pub phase2_monthly_huf: Decimal,
    pub phase2_from: YearMonth,
    /// Share of the phase-2 amount that goes to long-term investing.
    pub long_term_share: Decimal,
    pub long_term_start: YearMonth,
    /// Monthly realized trading profit split; must sum to 1.
    pub reinvest_frac: Decimal,
    pub withdraw_frac: Decimal,
    pub etf_targets: Vec<EtfTarget>,
}

impl Default for PlanConfig {
    fn default() -> Self {
        PlanConfig {
            start: YearMonth::new(2026, 11),
            initial_huf: Decimal::new(25_000, 0),
            phase1_monthly_huf: Decimal::new(50_000, 0),
            phase1_until: YearMonth::new(2027, 4),
            phase2_monthly_huf: Decimal::new(100_000, 0),
            phase2_from: YearMonth::new(2027, 5),
            long_term_share: Decimal::new(5, 1),
            long_term_start: YearMonth::new(2027, 5),
            reinvest_frac: Decimal::ONE,
            withdraw_frac: Decimal::ZERO,
            etf_targets: vec![],
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PlanError(pub String);

impl PlanConfig {
    pub fn validate(&self) -> Result<(), PlanError> {
        let neg = |d: Decimal| d.is_sign_negative() && !d.is_zero();
        if neg(self.initial_huf) || neg(self.phase1_monthly_huf) || neg(self.phase2_monthly_huf) {
            return Err(PlanError("amounts cannot be negative".into()));
        }
        if self.long_term_share < Decimal::ZERO || self.long_term_share > Decimal::ONE {
            return Err(PlanError("long_term_share must be within 0..1".into()));
        }
        if self.reinvest_frac < Decimal::ZERO
            || self.withdraw_frac < Decimal::ZERO
            || self.reinvest_frac + self.withdraw_frac != Decimal::ONE
        {
            return Err(PlanError(
                "reinvest_frac + withdraw_frac must equal 1".into(),
            ));
        }
        if !self.etf_targets.is_empty() {
            let sum: Decimal = self.etf_targets.iter().map(|t| t.weight).sum();
            if sum != Decimal::ONE || self.etf_targets.iter().any(|t| t.weight <= Decimal::ZERO) {
                return Err(PlanError(
                    "ETF weights must be positive and sum to 1".into(),
                ));
            }
        }
        if self.phase2_from <= self.start {
            return Err(PlanError("phase 2 must start after the plan start".into()));
        }
        Ok(())
    }

    /// Planned contributions for one month.
    pub fn month(&self, ym: YearMonth) -> MonthPlan {
        let zero = Decimal::ZERO;
        let (total, lt) = if ym < self.start {
            (zero, zero)
        } else if ym == self.start {
            (self.initial_huf, zero)
        } else if ym <= self.phase1_until && ym < self.phase2_from {
            (self.phase1_monthly_huf, zero)
        } else if ym >= self.phase2_from {
            let lt = if ym >= self.long_term_start {
                self.phase2_monthly_huf * self.long_term_share
            } else {
                zero
            };
            (self.phase2_monthly_huf, lt)
        } else {
            (zero, zero)
        };
        MonthPlan {
            month: ym,
            trading_huf: total - lt,
            long_term_huf: lt,
        }
    }

    /// Schedule from the plan start for `months` months.
    pub fn schedule(&self, months: usize) -> Vec<MonthPlan> {
        let mut out = Vec::with_capacity(months);
        let mut ym = self.start;
        for _ in 0..months {
            out.push(self.month(ym));
            ym = ym.next();
        }
        out
    }

    /// Total planned contributions up to and including `until`.
    pub fn planned_total(&self, until: YearMonth) -> Decimal {
        let mut ym = self.start;
        let mut sum = Decimal::ZERO;
        while ym <= until {
            let m = self.month(ym);
            sum += m.trading_huf + m.long_term_huf;
            ym = ym.next();
        }
        sum
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonthPlan {
    pub month: YearMonth,
    pub trading_huf: Decimal,
    pub long_term_huf: Decimal,
}

/// Unitized NAV accounting. Deposits buy units at the current NAV; withdrawals redeem units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnitLedger {
    pub units: Decimal,
    pub net_deposits_huf: Decimal,
    pub nav_peak: Decimal,
}

impl Default for UnitLedger {
    fn default() -> Self {
        UnitLedger {
            units: Decimal::ZERO,
            net_deposits_huf: Decimal::ZERO,
            nav_peak: Decimal::ONE,
        }
    }
}

impl UnitLedger {
    /// NAV per unit for a given equity. 1.0 before the first deposit.
    pub fn nav(&self, equity_huf: Decimal) -> Decimal {
        if self.units.is_zero() {
            Decimal::ONE
        } else {
            equity_huf / self.units
        }
    }

    /// Record an external flow (+deposit / −withdrawal). `equity_before` is the
    /// mark-to-market equity immediately before the flow.
    pub fn apply_flow(&mut self, amount_huf: Decimal, equity_before: Decimal) {
        let nav = self.nav(equity_before);
        if nav > Decimal::ZERO {
            self.units += amount_huf / nav;
        }
        self.net_deposits_huf += amount_huf;
        if self.units < Decimal::ZERO {
            self.units = Decimal::ZERO;
        }
    }

    /// Update the peak NAV; returns current drawdown fraction.
    pub fn mark(&mut self, equity_huf: Decimal) -> Decimal {
        let nav = self.nav(equity_huf);
        if nav > self.nav_peak {
            self.nav_peak = nav;
        }
        if self.nav_peak.is_zero() {
            Decimal::ZERO
        } else {
            Decimal::ONE - nav / self.nav_peak
        }
    }

    /// Profit attributable to trading (equity minus everything deposited).
    pub fn net_profit(&self, equity_huf: Decimal) -> Decimal {
        equity_huf - self.net_deposits_huf
    }
}

/// Net P&L over a period from actual balances: change in equity minus external flows.
pub fn net_pnl(equity_start: Decimal, equity_end: Decimal, net_flows: Decimal) -> Decimal {
    equity_end - equity_start - net_flows
}

/// Monthly profit split report. Withdrawals stay manual: this only computes amounts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfitSplit {
    pub month_profit_huf: Decimal,
    pub reinvest_huf: Decimal,
    pub withdrawable_huf: Decimal,
}

pub fn split_profit(cfg: &PlanConfig, month_profit_huf: Decimal) -> ProfitSplit {
    if month_profit_huf <= Decimal::ZERO {
        return ProfitSplit {
            month_profit_huf,
            reinvest_huf: Decimal::ZERO,
            withdrawable_huf: Decimal::ZERO,
        };
    }
    let w = (month_profit_huf * cfg.withdraw_frac).floor();
    ProfitSplit {
        month_profit_huf,
        reinvest_huf: month_profit_huf - w,
        withdrawable_huf: w,
    }
}

/// Hungarian daily push summary from actual net P&L, e.g. `💰 Mai eredmény: +1 000 Ft`.
pub fn daily_summary_hu(net_pnl_huf: Decimal) -> String {
    let rounded = net_pnl_huf.round();
    let icon = if rounded.is_sign_negative() && !rounded.is_zero() {
        "📉"
    } else {
        "💰"
    };
    format!("{icon} Mai eredmény: {}", format_huf(rounded, true)).replace('\u{a0}', " ")
}

/// Allocate a long-term contribution to ETF targets (amounts in HUF, floored to whole forints).
pub fn allocate_long_term(cfg: &PlanConfig, amount_huf: Decimal) -> Vec<(String, Decimal)> {
    cfg.etf_targets
        .iter()
        .map(|t| (t.instrument.clone(), (amount_huf * t.weight).floor()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn default_plan_matches_agreed_assumptions() {
        let p = PlanConfig::default();
        p.validate().unwrap();
        let s = p.schedule(8);
        assert_eq!(s[0].month, YearMonth::new(2026, 11));
        assert_eq!(s[0].trading_huf, dec!(25000));
        for m in &s[1..6] {
            assert_eq!(m.trading_huf, dec!(50000), "{:?}", m.month);
            assert_eq!(m.long_term_huf, dec!(0));
        }
        assert_eq!(s[5].month, YearMonth::new(2027, 4));
        assert_eq!(s[6].month, YearMonth::new(2027, 5));
        assert_eq!(s[6].trading_huf + s[6].long_term_huf, dec!(100000));
        assert_eq!(s[6].long_term_huf, dec!(50000));
        assert_eq!(p.planned_total(YearMonth::new(2027, 5)), dec!(375000));
    }

    #[test]
    fn deposits_do_not_count_as_profit() {
        let mut l = UnitLedger::default();
        l.apply_flow(dec!(25000), dec!(0));
        assert_eq!(l.nav(dec!(25000)), dec!(1));
        // Lose 10 %: equity 22,500.
        assert_eq!(l.mark(dec!(22500)), dec!(0.1));
        // Deposit 50,000 at NAV 0.9: NAV stays 0.9, drawdown stays 10 %.
        l.apply_flow(dec!(50000), dec!(22500));
        assert_eq!(l.nav(dec!(72500)).round_dp(6), dec!(0.9));
        assert_eq!(l.mark(dec!(72500)).round_dp(6), dec!(0.1));
        assert_eq!(l.net_profit(dec!(72500)), dec!(-2500));
        assert_eq!(net_pnl(dec!(22500), dec!(72500), dec!(50000)), dec!(0));
    }

    #[test]
    fn hungarian_summary() {
        assert_eq!(daily_summary_hu(dec!(1000)), "💰 Mai eredmény: +1 000 Ft");
        assert_eq!(
            daily_summary_hu(dec!(-1000.4)),
            "📉 Mai eredmény: -1 000 Ft"
        );
        assert_eq!(daily_summary_hu(dec!(0)), "💰 Mai eredmény: +0 Ft");
    }

    #[test]
    fn profit_split_never_withdraws_automatically() {
        let cfg = PlanConfig {
            reinvest_frac: dec!(0.7),
            withdraw_frac: dec!(0.3),
            ..PlanConfig::default()
        };
        cfg.validate().unwrap();
        let s = split_profit(&cfg, dec!(10001));
        assert_eq!(s.withdrawable_huf, dec!(3000));
        assert_eq!(s.reinvest_huf, dec!(7001));
        assert_eq!(split_profit(&cfg, dec!(-500)).withdrawable_huf, dec!(0));
    }

    #[test]
    fn invalid_plans_rejected() {
        let p = PlanConfig {
            reinvest_frac: dec!(0.5),
            withdraw_frac: dec!(0.6),
            ..PlanConfig::default()
        };
        assert!(p.validate().is_err());
        let p = PlanConfig {
            etf_targets: vec![EtfTarget {
                instrument: "trading212:X".into(),
                weight: dec!(0.5),
            }],
            ..PlanConfig::default()
        };
        assert!(p.validate().is_err());
    }
}

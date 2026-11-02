//! Risk limits. `HARD_MAX` values are compiled constants: configuration can tighten them
//! but never loosen beyond them (see `RiskConfig::validate`).

use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Percent values are fractions (0.01 = 1 %). Basis-point values are plain bps.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RiskConfig {
    pub risk_per_trade: Decimal,
    pub position_cap: Decimal,
    pub crypto_cap: Decimal,
    pub equity_cap: Decimal,
    pub total_cap: Decimal,
    pub cluster_cap: Decimal,
    pub max_positions_per_cluster: u32,
    pub daily_loss: Decimal,
    pub weekly_loss: Decimal,
    pub drawdown_reduce: Decimal,
    pub drawdown_halt: Decimal,
    pub max_positions: u32,
    pub max_entries_per_hour: u32,
    pub max_entries_per_day: u32,
    pub loss_streak: u32,
    pub loss_streak_cooldown_hours: i64,
    pub revenge_reentry_hours: i64,
    pub duplicate_window_secs: i64,
    pub edge_buffer_bps: Decimal,
    pub min_expected_net_huf: Decimal,
    pub max_spread_bps_crypto: Decimal,
    pub max_spread_bps_equity: Decimal,
    pub min_volume_24h_eur_crypto: Decimal,
    pub min_adv_usd_equity: Decimal,
    pub max_volume_participation: Decimal,
    pub new_listing_days: i64,
    pub quote_max_age_secs_crypto: i64,
    pub quote_max_age_secs_equity: i64,
    pub account_max_age_secs: i64,
    pub fx_max_age_days: i64,
    pub max_unknown_age_secs: i64,
    pub cash_floor_frac: Decimal,
    pub cash_floor_min_huf: Decimal,
    pub cash_buffer_frac: Decimal,
    pub market_open_buffer_mins: i64,
    pub fx_fee_bps: Decimal,
    pub slippage_bps: Decimal,
}

impl Default for RiskConfig {
    /// Conservative defaults sized for a 25,000 HUF account (docs/decisions/proposals/risk-engine.md).
    fn default() -> Self {
        RiskConfig {
            risk_per_trade: d("0.01"),
            position_cap: d("0.25"),
            crypto_cap: d("0.40"),
            equity_cap: d("0.80"),
            total_cap: d("0.80"),
            cluster_cap: d("0.40"),
            max_positions_per_cluster: 2,
            daily_loss: d("0.03"),
            weekly_loss: d("0.06"),
            drawdown_reduce: d("0.06"),
            drawdown_halt: d("0.10"),
            max_positions: 3,
            max_entries_per_hour: 4,
            max_entries_per_day: 12,
            loss_streak: 3,
            loss_streak_cooldown_hours: 24,
            revenge_reentry_hours: 4,
            duplicate_window_secs: 120,
            edge_buffer_bps: d("20"),
            min_expected_net_huf: d("25"),
            max_spread_bps_crypto: d("30"),
            max_spread_bps_equity: d("15"),
            min_volume_24h_eur_crypto: d("2000000"),
            min_adv_usd_equity: d("5000000"),
            max_volume_participation: d("0.0005"),
            new_listing_days: 90,
            quote_max_age_secs_crypto: 5,
            quote_max_age_secs_equity: 15,
            account_max_age_secs: 30,
            fx_max_age_days: 4,
            max_unknown_age_secs: 600,
            cash_floor_frac: d("0.05"),
            cash_floor_min_huf: d("1000"),
            cash_buffer_frac: d("0.01"),
            market_open_buffer_mins: 15,
            fx_fee_bps: d("15"),
            slippage_bps: d("5"),
        }
    }
}

/// Absolute ceilings (or floors) that configuration can never cross.
pub struct HardMax;

impl HardMax {
    pub const RISK_PER_TRADE: &'static str = "0.02";
    pub const POSITION_CAP: &'static str = "0.40";
    pub const CRYPTO_CAP: &'static str = "0.60";
    pub const EQUITY_CAP: &'static str = "0.95";
    pub const TOTAL_CAP: &'static str = "0.95";
    pub const DAILY_LOSS: &'static str = "0.05";
    pub const WEEKLY_LOSS: &'static str = "0.10";
    pub const DRAWDOWN_HALT: &'static str = "0.20";
    pub const MAX_POSITIONS: u32 = 10;
    pub const MAX_ENTRIES_PER_HOUR: u32 = 12;
    pub const MAX_ENTRIES_PER_DAY: u32 = 40;
    pub const LOSS_STREAK: u32 = 5;
    pub const MAX_SPREAD_BPS: &'static str = "100";
    pub const MAX_VOLUME_PARTICIPATION: &'static str = "0.01";
    pub const MIN_EDGE_BUFFER_BPS: &'static str = "10";
    pub const MIN_VOLUME_24H_EUR_CRYPTO: &'static str = "250000";
    pub const MIN_NEW_LISTING_DAYS: i64 = 30;
    pub const QUOTE_MAX_AGE_SECS_CRYPTO: i64 = 30;
    pub const QUOTE_MAX_AGE_SECS_EQUITY: i64 = 30;
    pub const MIN_CASH_FLOOR_FRAC: &'static str = "0.02";
}

fn d(s: &str) -> Decimal {
    s.parse().unwrap_or(Decimal::ZERO)
}

/// Convert an `f64` statistic to `Decimal` at the risk boundary. Non-finite → `None`.
pub fn dec_from_f64(x: f64) -> Option<Decimal> {
    if x.is_finite() {
        Decimal::from_f64(x)
    } else {
        None
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct LimitViolation(pub String);

impl RiskConfig {
    /// Reject any configuration that loosens a hard limit.
    pub fn validate(&self) -> Result<(), LimitViolation> {
        let mut errs = Vec::new();
        let mut le = |name: &str, v: Decimal, max: &str| {
            if v > d(max) || v.is_sign_negative() {
                errs.push(format!("{name}={v} exceeds hard limit {max}"));
            }
        };
        le(
            "risk_per_trade",
            self.risk_per_trade,
            HardMax::RISK_PER_TRADE,
        );
        le("position_cap", self.position_cap, HardMax::POSITION_CAP);
        le("crypto_cap", self.crypto_cap, HardMax::CRYPTO_CAP);
        le("equity_cap", self.equity_cap, HardMax::EQUITY_CAP);
        le("total_cap", self.total_cap, HardMax::TOTAL_CAP);
        le("daily_loss", self.daily_loss, HardMax::DAILY_LOSS);
        le("weekly_loss", self.weekly_loss, HardMax::WEEKLY_LOSS);
        le("drawdown_halt", self.drawdown_halt, HardMax::DRAWDOWN_HALT);
        le(
            "max_spread_bps_crypto",
            self.max_spread_bps_crypto,
            HardMax::MAX_SPREAD_BPS,
        );
        le(
            "max_spread_bps_equity",
            self.max_spread_bps_equity,
            HardMax::MAX_SPREAD_BPS,
        );
        le(
            "max_volume_participation",
            self.max_volume_participation,
            HardMax::MAX_VOLUME_PARTICIPATION,
        );
        if self.max_positions > HardMax::MAX_POSITIONS {
            errs.push(format!(
                "max_positions={} exceeds {}",
                self.max_positions,
                HardMax::MAX_POSITIONS
            ));
        }
        if self.max_entries_per_hour > HardMax::MAX_ENTRIES_PER_HOUR {
            errs.push("max_entries_per_hour exceeds hard limit".into());
        }
        if self.max_entries_per_day > HardMax::MAX_ENTRIES_PER_DAY {
            errs.push("max_entries_per_day exceeds hard limit".into());
        }
        if self.loss_streak == 0 || self.loss_streak > HardMax::LOSS_STREAK {
            errs.push("loss_streak outside 1..=5".into());
        }
        if self.edge_buffer_bps < d(HardMax::MIN_EDGE_BUFFER_BPS) {
            errs.push("edge_buffer_bps below floor 10".into());
        }
        if self.min_volume_24h_eur_crypto < d(HardMax::MIN_VOLUME_24H_EUR_CRYPTO) {
            errs.push("min_volume_24h_eur_crypto below floor".into());
        }
        if self.new_listing_days < HardMax::MIN_NEW_LISTING_DAYS {
            errs.push("new_listing_days below floor 30".into());
        }
        if self.quote_max_age_secs_crypto > HardMax::QUOTE_MAX_AGE_SECS_CRYPTO
            || self.quote_max_age_secs_equity > HardMax::QUOTE_MAX_AGE_SECS_EQUITY
        {
            errs.push("quote max age exceeds hard limit 30s".into());
        }
        if self.cash_floor_frac < d(HardMax::MIN_CASH_FLOOR_FRAC) {
            errs.push("cash_floor_frac below floor 0.02".into());
        }
        if self.drawdown_reduce > self.drawdown_halt {
            errs.push("drawdown_reduce must be <= drawdown_halt".into());
        }
        if self.slippage_bps.is_sign_negative() || self.fx_fee_bps.is_sign_negative() {
            errs.push("cost assumptions cannot be negative".into());
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(LimitViolation(errs.join("; ")))
        }
    }

    /// True if `new` loosens any limit compared to `self` (loosening needs passkey + 24 h delay
    /// and revokes live authorization).
    pub fn is_loosening(&self, new: &RiskConfig) -> bool {
        new.risk_per_trade > self.risk_per_trade
            || new.position_cap > self.position_cap
            || new.crypto_cap > self.crypto_cap
            || new.equity_cap > self.equity_cap
            || new.total_cap > self.total_cap
            || new.cluster_cap > self.cluster_cap
            || new.daily_loss > self.daily_loss
            || new.weekly_loss > self.weekly_loss
            || new.drawdown_halt > self.drawdown_halt
            || new.drawdown_reduce > self.drawdown_reduce
            || new.max_positions > self.max_positions
            || new.max_entries_per_hour > self.max_entries_per_hour
            || new.max_entries_per_day > self.max_entries_per_day
            || new.loss_streak > self.loss_streak
            || new.max_spread_bps_crypto > self.max_spread_bps_crypto
            || new.max_spread_bps_equity > self.max_spread_bps_equity
            || new.max_volume_participation > self.max_volume_participation
            || new.edge_buffer_bps < self.edge_buffer_bps
            || new.min_expected_net_huf < self.min_expected_net_huf
            || new.min_volume_24h_eur_crypto < self.min_volume_24h_eur_crypto
            || new.new_listing_days < self.new_listing_days
            || new.cash_floor_frac < self.cash_floor_frac
            || new.quote_max_age_secs_crypto > self.quote_max_age_secs_crypto
            || new.quote_max_age_secs_equity > self.quote_max_age_secs_equity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        RiskConfig::default().validate().unwrap();
    }

    #[test]
    fn hard_limits_cannot_be_exceeded() {
        let c = RiskConfig {
            risk_per_trade: d("0.05"),
            ..RiskConfig::default()
        };
        assert!(c.validate().is_err());
        let c = RiskConfig {
            max_positions: 50,
            ..RiskConfig::default()
        };
        assert!(c.validate().is_err());
        let c = RiskConfig {
            edge_buffer_bps: d("0"),
            ..RiskConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn loosening_is_detected() {
        let base = RiskConfig::default();
        let tighter = RiskConfig {
            daily_loss: d("0.02"),
            ..base.clone()
        };
        assert!(!base.is_loosening(&tighter));
        let looser = RiskConfig {
            daily_loss: d("0.04"),
            ..base.clone()
        };
        assert!(base.is_loosening(&looser));
    }
}

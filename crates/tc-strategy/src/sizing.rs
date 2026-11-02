//! Position sizing for tiny accounts (docs/decisions/0005-strategies-validation.md §5).
//! Money/quantities in `Decimal`. The risk engine re-checks and may only shrink the result.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SizingInput {
    /// Trading equity in the instrument's quote currency (excludes the long-term ETF core).
    pub equity: Decimal,
    pub price: Decimal,
    pub stop: Decimal,
    /// Risk per trade fraction (0.01) and its ceiling for min-order bumps (0.0125).
    pub risk_frac: Decimal,
    pub risk_frac_max: Decimal,
    /// Max notional fraction of equity for one position (0.5 per spec; risk engine caps tighter).
    pub max_position_frac: Decimal,
    /// Volatility-target notional cap (`E*(0.20/K)/sigma_i`), already computed; `None` = no cap.
    pub vol_cap_notional: Option<Decimal>,
    pub cash: Decimal,
    pub min_notional: Decimal,
    pub min_base: Decimal,
    pub quantity_decimals: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Sizing {
    Trade {
        quantity: Decimal,
        notional: Decimal,
        risk_frac: Decimal,
        bumped_to_minimum: bool,
    },
    Abstain {
        reason: String,
    },
}

pub fn size(i: &SizingInput) -> Sizing {
    if i.price <= Decimal::ZERO
        || i.stop <= Decimal::ZERO
        || i.stop >= i.price
        || i.equity <= Decimal::ZERO
    {
        return Sizing::Abstain {
            reason: "invalid price/stop/equity".into(),
        };
    }
    let d = i.price - i.stop;
    let d_frac = d / i.price;
    let n_risk = i.risk_frac * i.equity * i.price / d;
    let mut n = n_risk.min(i.max_position_frac * i.equity).min(i.cash);
    if let Some(v) = i.vol_cap_notional {
        n = n.min(v);
    }
    let n_min = i.min_notional.max(i.min_base * i.price);
    // The exit at the stop must also clear the venue minimum.
    let n_min_eff = n_min * Decimal::new(105, 2) / (Decimal::ONE - d_frac);
    let (notional, bumped) = if n >= n_min_eff {
        (n, false)
    } else {
        let f_needed = n_min_eff * d_frac / i.equity;
        if f_needed <= i.risk_frac_max && n_min_eff <= i.cash {
            (n_min_eff, true)
        } else {
            return Sizing::Abstain {
                reason: format!(
                    "MIN_ORDER_RISK: minimum order {} needs {}% risk > {}%",
                    n_min_eff.round_dp(2),
                    (f_needed * Decimal::ONE_HUNDRED).round_dp(2),
                    i.risk_frac_max * Decimal::ONE_HUNDRED
                ),
            };
        }
    };
    let mut quantity = (notional / i.price)
        .round_dp_with_strategy(i.quantity_decimals, rust_decimal::RoundingStrategy::ToZero);
    if bumped {
        // Rounding down may fall below the minimum; step up one increment if needed.
        let step = Decimal::new(1, i.quantity_decimals);
        while quantity * i.price < n_min_eff && quantity * i.price <= i.cash {
            quantity += step;
        }
    }
    if quantity <= Decimal::ZERO || quantity * i.price < n_min {
        return Sizing::Abstain {
            reason: "quantity below venue minimum after rounding".into(),
        };
    }
    let notional = quantity * i.price;
    Sizing::Trade {
        quantity,
        notional,
        risk_frac: notional * d_frac / i.equity,
        bumped_to_minimum: bumped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn base(price: Decimal, stop: Decimal) -> SizingInput {
        SizingInput {
            equity: dec!(63),
            price,
            stop,
            risk_frac: dec!(0.01),
            risk_frac_max: dec!(0.0125),
            max_position_frac: dec!(0.5),
            vol_cap_notional: None,
            cash: dec!(60),
            min_notional: dec!(5),
            min_base: dec!(0),
            quantity_decimals: 8,
        }
    }

    #[test]
    fn spec_example_btc_6pct_stop_trades() {
        // N_risk = 0.01*63/0.06 = 10.50 EUR
        match size(&base(dec!(70000), dec!(65800))) {
            Sizing::Trade {
                notional,
                bumped_to_minimum,
                ..
            } => {
                assert!(
                    notional <= dec!(10.50) && notional > dec!(10.49),
                    "{notional}"
                );
                assert!(!bumped_to_minimum);
            }
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn spec_example_alt_15pct_stop_abstains() {
        match size(&base(dec!(1), dec!(0.85))) {
            Sizing::Abstain { reason } => assert!(reason.starts_with("MIN_ORDER_RISK"), "{reason}"),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn small_bump_to_minimum_allowed_within_ceiling() {
        // 10 % stop: N_risk = 6.30, N_min_eff = 5.25/0.9 = 5.83 -> trade without bump.
        assert!(matches!(
            size(&base(dec!(1), dec!(0.9))),
            Sizing::Trade { .. }
        ));
        // 12% stop: N_risk = 5.25 < N_min_eff 5.966 -> f' = 0.716/63 = 1.14% <= 1.25%: bump.
        match size(&base(dec!(1), dec!(0.88))) {
            Sizing::Trade {
                bumped_to_minimum,
                risk_frac,
                ..
            } => {
                assert!(bumped_to_minimum);
                assert!(risk_frac <= dec!(0.0125));
            }
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn invalid_inputs_abstain() {
        assert!(matches!(
            size(&base(dec!(10), dec!(11))),
            Sizing::Abstain { .. }
        ));
    }
}

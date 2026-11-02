//! Money and currency types. Money is always `rust_decimal::Decimal`, never `f64`.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::CoreError;

/// ISO 4217 currency code (3 upper-case ASCII letters). Trading 212 can also report
/// `GBX` (pence) which is normalised by the connector before it reaches the domain.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Currency([u8; 3]);

impl Currency {
    pub const HUF: Currency = Currency(*b"HUF");
    pub const EUR: Currency = Currency(*b"EUR");
    pub const USD: Currency = Currency(*b"USD");
    pub const GBP: Currency = Currency(*b"GBP");

    pub fn new(code: &str) -> Result<Self, CoreError> {
        let b = code.as_bytes();
        if b.len() != 3 || !b.iter().all(|c| c.is_ascii_uppercase()) {
            return Err(CoreError::InvalidCurrency(code.to_owned()));
        }
        Ok(Currency([b[0], b[1], b[2]]))
    }

    pub fn as_str(&self) -> &str {
        // Constructed only from validated ASCII upper-case bytes.
        std::str::from_utf8(&self.0).unwrap_or("???")
    }

    /// Number of minor-unit decimals used when presenting/rounding cash amounts.
    pub fn decimals(&self) -> u32 {
        match &self.0 {
            b"HUF" => 0,
            b"JPY" => 0,
            _ => 2,
        }
    }
}

impl TryFrom<String> for Currency {
    type Error = CoreError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Currency::new(&s)
    }
}

impl From<Currency> for String {
    fn from(c: Currency) -> String {
        c.as_str().to_owned()
    }
}

impl FromStr for Currency {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Currency::new(s)
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An amount in a specific currency.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Money {
    pub amount: Decimal,
    pub currency: Currency,
}

impl Money {
    pub fn new(amount: Decimal, currency: Currency) -> Self {
        Money { amount, currency }
    }

    pub fn zero(currency: Currency) -> Self {
        Money {
            amount: Decimal::ZERO,
            currency,
        }
    }

    pub fn huf(amount: Decimal) -> Self {
        Money::new(amount, Currency::HUF)
    }

    pub fn eur(amount: Decimal) -> Self {
        Money::new(amount, Currency::EUR)
    }

    pub fn checked_add(self, other: Money) -> Result<Money, CoreError> {
        self.same_currency(&other)?;
        let amount = self
            .amount
            .checked_add(other.amount)
            .ok_or(CoreError::Overflow)?;
        Ok(Money::new(amount, self.currency))
    }

    pub fn checked_sub(self, other: Money) -> Result<Money, CoreError> {
        self.same_currency(&other)?;
        let amount = self
            .amount
            .checked_sub(other.amount)
            .ok_or(CoreError::Overflow)?;
        Ok(Money::new(amount, self.currency))
    }

    pub fn checked_mul(self, factor: Decimal) -> Result<Money, CoreError> {
        let amount = self.amount.checked_mul(factor).ok_or(CoreError::Overflow)?;
        Ok(Money::new(amount, self.currency))
    }

    pub fn is_negative(&self) -> bool {
        self.amount.is_sign_negative() && !self.amount.is_zero()
    }

    /// Round to the currency's minor unit (banker's rounding is avoided: half away from zero).
    pub fn rounded(self) -> Money {
        Money::new(
            self.amount.round_dp_with_strategy(
                self.currency.decimals(),
                rust_decimal::RoundingStrategy::MidpointAwayFromZero,
            ),
            self.currency,
        )
    }

    fn same_currency(&self, other: &Money) -> Result<(), CoreError> {
        if self.currency != other.currency {
            return Err(CoreError::CurrencyMismatch(self.currency, other.currency));
        }
        Ok(())
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.rounded().amount, self.currency)
    }
}

/// Parse a decimal that may be written in scientific notation.
/// Bitvavo's live ticker returns values such as `"6.95E+4"` (verified 2026-11-02).
pub fn parse_decimal(s: &str) -> Result<Decimal, CoreError> {
    let t = s.trim();
    if t.is_empty() {
        return Err(CoreError::InvalidDecimal(s.to_owned()));
    }
    if t.contains(['e', 'E']) {
        return Decimal::from_scientific(t).map_err(|_| CoreError::InvalidDecimal(s.to_owned()));
    }
    Decimal::from_str(t).map_err(|_| CoreError::InvalidDecimal(s.to_owned()))
}

/// Format HUF the Hungarian way: `+1 000 Ft`, `-12 500 Ft` (non-breaking space separators).
pub fn format_huf(amount: Decimal, with_sign: bool) -> String {
    let rounded =
        amount.round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    let neg = rounded.is_sign_negative() && !rounded.is_zero();
    let digits = rounded.abs().trunc().to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push('\u{a0}');
        }
        grouped.push(ch);
    }
    let sign = if neg {
        "-"
    } else if with_sign {
        "+"
    } else {
        ""
    };
    format!("{sign}{grouped}\u{a0}Ft")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn scientific_notation_is_parsed() {
        assert_eq!(parse_decimal("6.95E+4").unwrap(), dec!(69500));
        assert_eq!(parse_decimal("7.014E+4").unwrap(), dec!(70140));
        assert_eq!(parse_decimal("0.00007294").unwrap(), dec!(0.00007294));
        assert!(parse_decimal("abc").is_err());
        assert!(parse_decimal("").is_err());
    }

    #[test]
    fn money_rejects_currency_mismatch() {
        let a = Money::huf(dec!(100));
        let b = Money::eur(dec!(1));
        assert!(a.checked_add(b).is_err());
        assert_eq!(
            a.checked_add(Money::huf(dec!(5))).unwrap().amount,
            dec!(105)
        );
    }

    #[test]
    fn huf_formatting_matches_push_summary_style() {
        assert_eq!(format_huf(dec!(1000), true), "+1\u{a0}000\u{a0}Ft");
        assert_eq!(format_huf(dec!(-12500.4), true), "-12\u{a0}500\u{a0}Ft");
        assert_eq!(format_huf(dec!(0), true), "+0\u{a0}Ft");
        assert_eq!(format_huf(dec!(999), false), "999\u{a0}Ft");
        assert_eq!(
            format_huf(dec!(1234567), false),
            "1\u{a0}234\u{a0}567\u{a0}Ft"
        );
    }

    #[test]
    fn currency_validation() {
        assert!(Currency::new("huf").is_err());
        assert!(Currency::new("HUFF").is_err());
        assert_eq!(Currency::new("HUF").unwrap(), Currency::HUF);
        assert_eq!(Currency::HUF.decimals(), 0);
    }
}

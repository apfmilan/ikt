use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::{CoreError, Currency};

/// Supported venues. Revolut is signal-only: no connector can place orders there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Broker {
    Trading212,
    Bitvavo,
    Revolut,
}

impl Broker {
    pub fn as_str(&self) -> &'static str {
        match self {
            Broker::Trading212 => "trading212",
            Broker::Bitvavo => "bitvavo",
            Broker::Revolut => "revolut",
        }
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        match s {
            "trading212" | "t212" => Ok(Broker::Trading212),
            "bitvavo" => Ok(Broker::Bitvavo),
            "revolut" => Ok(Broker::Revolut),
            other => Err(CoreError::Invalid(format!("unknown broker {other}"))),
        }
    }

    /// Whether automated order placement is allowed for this venue at all.
    pub fn supports_automated_trading(&self) -> bool {
        !matches!(self, Broker::Revolut)
    }
}

impl fmt::Display for Broker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetClass {
    Crypto,
    Stock,
    Etf,
}

impl AssetClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            AssetClass::Crypto => "crypto",
            AssetClass::Stock => "stock",
            AssetClass::Etf => "etf",
        }
    }
}

/// Globally unique instrument key: `<broker>:<native symbol>`, e.g. `bitvavo:BTC-EUR`,
/// `trading212:AAPL_US_EQ`.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct InstrumentId(String);

impl InstrumentId {
    pub fn new(broker: Broker, symbol: &str) -> Self {
        InstrumentId(format!("{}:{}", broker.as_str(), symbol))
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        let (b, sym) = s
            .split_once(':')
            .ok_or_else(|| CoreError::InvalidInstrumentId(s.to_owned()))?;
        let broker = Broker::parse(b).map_err(|_| CoreError::InvalidInstrumentId(s.to_owned()))?;
        if sym.is_empty()
            || sym.len() > 64
            || !sym
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        {
            return Err(CoreError::InvalidInstrumentId(s.to_owned()));
        }
        Ok(InstrumentId::new(broker, sym))
    }

    pub fn broker(&self) -> Broker {
        // Invariant established by constructors.
        let b = self.0.split(':').next().unwrap_or_default();
        Broker::parse(b).unwrap_or(Broker::Revolut)
    }

    pub fn symbol(&self) -> &str {
        self.0.split_once(':').map(|(_, s)| s).unwrap_or_default()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for InstrumentId {
    type Error = CoreError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        InstrumentId::parse(&s)
    }
}

impl From<InstrumentId> for String {
    fn from(i: InstrumentId) -> String {
        i.0
    }
}

impl fmt::Display for InstrumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for InstrumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InstrumentId({})", self.0)
    }
}

/// Static trading rules of an instrument as reported by the broker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Instrument {
    pub id: InstrumentId,
    pub name: String,
    pub asset_class: AssetClass,
    pub quote_currency: Currency,
    pub isin: Option<String>,
    /// Minimum order notional in quote currency (Bitvavo `minOrderInQuoteAsset`).
    pub min_notional: Decimal,
    /// Minimum base quantity (Bitvavo `minOrderInBaseAsset`, T212 `minTradeQuantity`).
    pub min_quantity: Decimal,
    /// Max decimals allowed in an order quantity.
    pub quantity_decimals: u32,
    /// Minimum price increment.
    pub tick_size: Decimal,
    /// Venue says the instrument is currently tradable.
    pub tradable: bool,
    /// First time this system saw the instrument (new-listing detection).
    pub first_seen: DateTime<Utc>,
}

impl Instrument {
    /// Round a quantity down to the allowed precision (never up: never exceed intended risk).
    pub fn floor_quantity(&self, q: Decimal) -> Decimal {
        q.round_dp_with_strategy(
            self.quantity_decimals,
            rust_decimal::RoundingStrategy::ToZero,
        )
    }

    /// Round a price to the tick grid. Buys round down, sells round up (never worse than intended).
    pub fn round_price(&self, price: Decimal, side: crate::Side) -> Decimal {
        if self.tick_size.is_zero() {
            return price;
        }
        let ticks = price / self.tick_size;
        let ticks = match side {
            crate::Side::Buy => ticks.floor(),
            crate::Side::Sell => ticks.ceil(),
        };
        (ticks * self.tick_size).normalize()
    }
}

/// Candle interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Interval {
    #[serde(rename = "15m")]
    M15,
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "4h")]
    H4,
    #[serde(rename = "1d")]
    D1,
}

impl Interval {
    pub fn as_str(&self) -> &'static str {
        match self {
            Interval::M15 => "15m",
            Interval::H1 => "1h",
            Interval::H4 => "4h",
            Interval::D1 => "1d",
        }
    }

    pub fn seconds(&self) -> i64 {
        match self {
            Interval::M15 => 900,
            Interval::H1 => 3600,
            Interval::H4 => 14_400,
            Interval::D1 => 86_400,
        }
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        match s {
            "15m" => Ok(Interval::M15),
            "1h" => Ok(Interval::H1),
            "4h" => Ok(Interval::H4),
            "1d" => Ok(Interval::D1),
            o => Err(CoreError::Invalid(format!("unsupported interval {o}"))),
        }
    }

    /// Bars per year, used to annualise statistics (crypto trades 24/7; stocks ~252 days).
    pub fn bars_per_year(&self, asset_class: AssetClass) -> f64 {
        let per_day = 86_400.0 / self.seconds() as f64;
        match asset_class {
            AssetClass::Crypto => per_day * 365.0,
            _ => per_day.min(1.0).max(per_day) * 252.0 / if per_day > 1.0 { 3.7 } else { 1.0 },
        }
    }
}

/// OHLCV bar. `ts` is the bar open time (UTC).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candle {
    pub ts: DateTime<Utc>,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
}

impl Candle {
    pub fn validate(&self) -> Result<(), CoreError> {
        let ok = self.low <= self.high
            && self.low <= self.open
            && self.low <= self.close
            && self.high >= self.open
            && self.high >= self.close
            && !self.low.is_sign_negative()
            && !self.volume.is_sign_negative();
        if ok {
            Ok(())
        } else {
            Err(CoreError::Invalid(format!(
                "inconsistent candle at {}",
                self.ts
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn inst() -> Instrument {
        Instrument {
            id: InstrumentId::new(Broker::Bitvavo, "BTC-EUR"),
            name: "Bitcoin".into(),
            asset_class: AssetClass::Crypto,
            quote_currency: Currency::EUR,
            isin: None,
            min_notional: dec!(5),
            min_quantity: dec!(0.00007294),
            quantity_decimals: 8,
            tick_size: dec!(1),
            tradable: true,
            first_seen: Utc::now(),
        }
    }

    #[test]
    fn instrument_id_roundtrip() {
        let id = InstrumentId::parse("bitvavo:BTC-EUR").unwrap();
        assert_eq!(id.broker(), Broker::Bitvavo);
        assert_eq!(id.symbol(), "BTC-EUR");
        assert!(InstrumentId::parse("nobroker:X").is_err());
        assert!(InstrumentId::parse("bitvavo:").is_err());
        assert!(InstrumentId::parse("bitvavo:BTC EUR").is_err());
        let t = InstrumentId::parse("trading212:VUSAl_EQ").unwrap();
        assert_eq!(t.broker(), Broker::Trading212);
    }

    #[test]
    fn quantity_and_price_rounding_never_exceed_intent() {
        let i = inst();
        assert_eq!(i.floor_quantity(dec!(0.123456789)), dec!(0.12345678));
        assert_eq!(i.round_price(dec!(70140.7), crate::Side::Buy), dec!(70140));
        assert_eq!(i.round_price(dec!(70140.2), crate::Side::Sell), dec!(70141));
    }

    #[test]
    fn revolut_cannot_trade() {
        assert!(!Broker::Revolut.supports_automated_trading());
        assert!(Broker::Bitvavo.supports_automated_trading());
    }

    #[test]
    fn candle_validation() {
        let c = Candle {
            ts: Utc::now(),
            open: dec!(10),
            high: dec!(9),
            low: dec!(8),
            close: dec!(9),
            volume: dec!(1),
        };
        assert!(c.validate().is_err());
    }
}

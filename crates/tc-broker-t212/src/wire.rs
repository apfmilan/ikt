//! Trading 212 JSON shapes (OpenAPI bundle `https://docs.trading212.com/_bundle/api.json`,
//! checked 2026-11-02) and conversion into domain types.
//!
//! Numbers: Trading 212 sends JSON numbers (not strings). Without serde_json's
//! `arbitrary_precision` feature (deliberately not enabled workspace-wide because it changes
//! `serde_json::Value` behaviour for every crate) numbers are held as `f64`. We convert them via
//! their shortest round-trip decimal string (`Number`'s `Display`), which reproduces the literal
//! the server sent for any value with ≤15 significant digits — enough for prices/quantities.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use tc_broker::{BrokerError, BrokerFill, BrokerOrder, BrokerOrderStatus};
use tc_core::{parse_decimal, Currency, InstrumentId, OrderKind, Side};

pub(crate) fn decode<T>(msg: impl std::fmt::Display) -> Result<T, BrokerError> {
    Err(BrokerError::Decode(msg.to_string()))
}

/// Decimal from a JSON number (or numeric string).
pub(crate) fn num(v: &Value) -> Result<Decimal, BrokerError> {
    match v {
        Value::Number(n) => {
            parse_decimal(&n.to_string()).map_err(|e| BrokerError::Decode(e.to_string()))
        }
        Value::String(s) => parse_decimal(s).map_err(|e| BrokerError::Decode(e.to_string())),
        other => decode(format!("expected number, got {other}")),
    }
}

pub(crate) fn opt_num(v: Option<&Value>) -> Result<Option<Decimal>, BrokerError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => num(v).map(Some),
    }
}

pub(crate) fn field<'a>(o: &'a Value, k: &str) -> Result<&'a Value, BrokerError> {
    o.get(k)
        .ok_or_else(|| BrokerError::Decode(format!("missing field {k}")))
}

pub(crate) fn str_field<'a>(o: &'a Value, k: &str) -> Result<&'a str, BrokerError> {
    field(o, k)?
        .as_str()
        .ok_or_else(|| BrokerError::Decode(format!("field {k} is not a string")))
}

pub(crate) fn time(v: &Value) -> Result<DateTime<Utc>, BrokerError> {
    let s = v
        .as_str()
        .ok_or_else(|| BrokerError::Decode("timestamp is not a string".into()))?;
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| BrokerError::Decode(format!("bad timestamp {s}: {e}")))
}

pub(crate) fn id_string(v: &Value) -> Result<String, BrokerError> {
    match v {
        Value::Number(n) => Ok(n.to_string()),
        Value::String(s) => Ok(s.clone()),
        other => decode(format!("bad id {other}")),
    }
}

pub(crate) fn instrument_id(ticker: &str) -> Result<InstrumentId, BrokerError> {
    InstrumentId::parse(&format!("trading212:{ticker}"))
        .map_err(|e| BrokerError::Decode(e.to_string()))
}

/// Price unit of an instrument. LSE lines may be quoted in pence (`GBX`); the domain always
/// uses major units (GBP). Whether Trading 212 ever reports `GBX` is UNVERIFIED; handled
/// defensively.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PriceUnit {
    Major,
    Pence,
}

impl PriceUnit {
    pub(crate) fn from_currency_code(code: &str) -> Self {
        if code.eq_ignore_ascii_case("GBX") {
            PriceUnit::Pence
        } else {
            PriceUnit::Major
        }
    }

    /// Broker price → domain price.
    pub(crate) fn inbound(self, p: Decimal) -> Decimal {
        match self {
            PriceUnit::Major => p,
            PriceUnit::Pence => p / Decimal::ONE_HUNDRED,
        }
    }

    /// Domain price → broker price.
    pub(crate) fn outbound(self, p: Decimal) -> Decimal {
        match self {
            PriceUnit::Major => p,
            PriceUnit::Pence => p * Decimal::ONE_HUNDRED,
        }
    }
}

/// Normalised currency (GBX → GBP).
pub(crate) fn currency(code: &str) -> Option<Currency> {
    if code.eq_ignore_ascii_case("GBX") {
        Some(Currency::GBP)
    } else {
        Currency::new(code).ok()
    }
}

fn status(s: &str) -> Result<BrokerOrderStatus, BrokerError> {
    Ok(match s {
        "LOCAL" | "UNCONFIRMED" | "CONFIRMED" | "NEW" | "CANCELLING" | "REPLACING" => {
            BrokerOrderStatus::Open
        }
        "PARTIALLY_FILLED" => BrokerOrderStatus::PartiallyFilled,
        "FILLED" => BrokerOrderStatus::Filled,
        // REPLACED: superseded by another order; this one no longer works.
        "CANCELLED" | "REPLACED" => BrokerOrderStatus::Cancelled,
        "REJECTED" => BrokerOrderStatus::Rejected,
        other => return decode(format!("unknown order status {other}")),
    })
}

/// Map an `Order` object (pending orders, placement responses, history `order` part).
/// `unit` converts prices of pence-quoted instruments.
pub(crate) fn parse_order(
    o: &Value,
    unit: impl Fn(&str) -> PriceUnit,
) -> Result<BrokerOrder, BrokerError> {
    let ticker = match o.get("ticker").and_then(Value::as_str) {
        Some(t) => t.to_owned(),
        None => str_field(field(o, "instrument")?, "ticker")?.to_owned(),
    };
    let u = unit(&ticker);
    let quantity_signed = opt_num(o.get("quantity"))?.unwrap_or(Decimal::ZERO);
    let side = match o.get("side").and_then(Value::as_str) {
        Some("BUY") => Side::Buy,
        Some("SELL") => Side::Sell,
        Some(other) => return decode(format!("unknown side {other}")),
        None if quantity_signed.is_sign_negative() => Side::Sell,
        None => Side::Buy,
    };
    let limit = opt_num(o.get("limitPrice"))?.map(|p| u.inbound(p));
    let stop = opt_num(o.get("stopPrice"))?.map(|p| u.inbound(p));
    let kind = match (o.get("type").and_then(Value::as_str), limit, stop) {
        (Some("LIMIT"), Some(p), _) => OrderKind::Limit { price: p },
        (Some("STOP"), _, Some(s)) => OrderKind::Stop { stop: s },
        (Some("STOP_LIMIT"), Some(l), Some(s)) => OrderKind::StopLimit { stop: s, limit: l },
        (Some("MARKET"), _, _) | (None, _, _) => OrderKind::Market,
        (Some(t), _, _) => return decode(format!("order type {t} without required prices")),
    };
    let created_at = time(field(o, "createdAt")?)?;
    let st = match o.get("status").and_then(Value::as_str) {
        Some(s) => status(s)?,
        None => BrokerOrderStatus::Open,
    };
    Ok(BrokerOrder {
        broker_order_id: id_string(field(o, "id")?)?,
        client_order_id: None,
        instrument: instrument_id(&ticker)?,
        side,
        kind,
        quantity: quantity_signed.abs(),
        filled_quantity: opt_num(o.get("filledQuantity"))?
            .unwrap_or(Decimal::ZERO)
            .abs(),
        avg_fill_price: None,
        fee_paid: Decimal::ZERO,
        fee_currency: None,
        status: st,
        via_api: o.get("initiatedFrom").and_then(Value::as_str) == Some("API"),
        created_at,
        updated_at: created_at,
        fills: Vec::new(),
    })
}

/// Map a `HistoricalOrder` (`{order, fill}`). Only `TRADE` fills become [`BrokerFill`]s;
/// corporate-action fills (STOCK_SPLIT, ...) are logged and left to position reconciliation,
/// which halts on any unexplained quantity change.
pub(crate) fn parse_historical(
    h: &Value,
    unit: impl Fn(&str) -> PriceUnit + Copy,
) -> Result<BrokerOrder, BrokerError> {
    let mut order = parse_order(field(h, "order")?, unit)?;
    let u = unit(order.instrument.symbol());
    if let Some(f) = h.get("fill").filter(|f| !f.is_null()) {
        let fill_type = f.get("type").and_then(Value::as_str).unwrap_or("TRADE");
        let ts = time(field(f, "filledAt")?)?;
        order.updated_at = ts;
        if fill_type == "TRADE" {
            let mut fee = Decimal::ZERO;
            let mut fee_currency = None;
            if let Some(w) = f.get("walletImpact").filter(|w| !w.is_null()) {
                fee_currency = w.get("currency").and_then(Value::as_str).and_then(currency);
                if let Some(taxes) = w.get("taxes").and_then(Value::as_array) {
                    for t in taxes {
                        fee += opt_num(t.get("quantity"))?.unwrap_or(Decimal::ZERO).abs();
                    }
                }
            }
            let price = u.inbound(num(field(f, "price")?)?);
            let qty = num(field(f, "quantity")?)?.abs();
            order.avg_fill_price = Some(price);
            order.fee_paid = fee;
            order.fee_currency = fee_currency;
            order.fills.push(BrokerFill {
                fill_id: id_string(field(f, "id")?)?,
                quantity: qty,
                price,
                fee,
                fee_currency,
                ts,
            });
            if order.filled_quantity.is_zero() {
                order.filled_quantity = qty;
            }
        } else {
            tracing::warn!(
                ticker = order.instrument.symbol(),
                fill_type,
                "non-trade fill in order history"
            );
        }
    }
    Ok(order)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use serde_json::json;

    #[test]
    fn numbers_keep_their_decimal_literal() {
        assert_eq!(num(&json!(0.1)).unwrap(), dec!(0.1));
        assert_eq!(num(&json!(184.23)).unwrap(), dec!(184.23));
        assert_eq!(num(&json!(-10.5)).unwrap(), dec!(-10.5));
        assert_eq!(num(&json!(1e-7)).unwrap(), dec!(0.0000001));
        assert_eq!(num(&json!(12345678)).unwrap(), dec!(12345678));
    }

    #[test]
    fn gbx_prices_are_normalised() {
        let u = PriceUnit::from_currency_code("GBX");
        assert_eq!(u.inbound(dec!(1234)), dec!(12.34));
        assert_eq!(u.outbound(dec!(12.34)), dec!(1234));
        assert_eq!(currency("GBX"), Some(Currency::GBP));
        assert_eq!(PriceUnit::from_currency_code("USD"), PriceUnit::Major);
    }

    #[test]
    fn sell_orders_have_positive_quantity_in_domain() {
        let o = json!({"id": 9007199254740i64, "ticker": "AAPL_US_EQ", "side": "SELL", "type": "STOP",
            "quantity": -0.5, "stopPrice": 180.25, "status": "NEW", "initiatedFrom": "API",
            "createdAt": "2026-11-02T14:30:00.000Z", "filledQuantity": 0});
        let b = parse_order(&o, |_| PriceUnit::Major).unwrap();
        assert_eq!(b.side, Side::Sell);
        assert_eq!(b.quantity, dec!(0.5));
        assert_eq!(b.kind, OrderKind::Stop { stop: dec!(180.25) });
        assert!(b.via_api);
        assert_eq!(b.broker_order_id, "9007199254740");
    }

    #[test]
    fn historical_trade_fill_with_taxes() {
        let h = json!({"order": {"id": 1, "ticker": "VUSAl_EQ", "side": "BUY", "type": "MARKET",
            "quantity": 2, "filledQuantity": 2, "status": "FILLED", "initiatedFrom": "API",
            "createdAt": "2026-11-02T09:00:00Z"},
            "fill": {"id": 77, "filledAt": "2026-11-02T09:00:01Z", "price": 9512.5, "quantity": 2,
            "type": "TRADE", "tradingMethod": "TOTV",
            "walletImpact": {"currency": "EUR", "fxRate": 1.2, "netValue": 220.0, "realisedProfitLoss": 0,
            "taxes": [{"name": "CURRENCY_CONVERSION_FEE", "quantity": 0.33, "currency": "EUR"}]}}});
        let b = parse_historical(&h, |_| PriceUnit::Pence).unwrap();
        assert_eq!(b.fills.len(), 1);
        assert_eq!(b.fills[0].price, dec!(95.125));
        assert_eq!(b.fee_paid, dec!(0.33));
        assert_eq!(b.fee_currency, Some(Currency::EUR));
    }

    #[test]
    fn corporate_action_fill_is_not_a_trade() {
        let h = json!({"order": {"id": 2, "ticker": "AAPL_US_EQ", "side": "BUY", "type": "MARKET",
            "quantity": 3, "status": "FILLED", "createdAt": "2026-11-02T09:00:00Z"},
            "fill": {"id": 78, "filledAt": "2026-11-02T09:00:01Z", "price": 0, "quantity": 3, "type": "STOCK_SPLIT"}});
        let b = parse_historical(&h, |_| PriceUnit::Major).unwrap();
        assert!(b.fills.is_empty());
        assert!(!b.via_api);
    }
}

//! Raw Bitvavo JSON shapes and conversion into domain types.
//!
//! Decimal values arrive as strings, sometimes in scientific notation (`"6.95E+4"`, observed
//! live on `/ticker/24h` 2026-11-02); some integer fields are documented as strings but
//! returned as numbers (`quantityDecimals`). All parsing goes through [`dec`] / [`int`].

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;
use tc_broker::{BrokerError, BrokerFill, BrokerOrder, BrokerOrderStatus};
use tc_core::{
    parse_decimal, AssetClass, Broker, Candle, Currency, Instrument, InstrumentId, OrderKind, Side,
};
use uuid::Uuid;

pub(crate) fn decode<T>(msg: impl std::fmt::Display) -> Result<T, BrokerError> {
    Err(BrokerError::Decode(msg.to_string()))
}

/// Parse a decimal from a JSON string or number.
pub(crate) fn dec(v: &Value) -> Result<Decimal, BrokerError> {
    match v {
        Value::String(s) => parse_decimal(s).map_err(|e| BrokerError::Decode(e.to_string())),
        Value::Number(n) => {
            parse_decimal(&n.to_string()).map_err(|e| BrokerError::Decode(e.to_string()))
        }
        other => decode(format!("expected decimal, got {other}")),
    }
}

pub(crate) fn opt_dec(v: Option<&Value>) -> Result<Option<Decimal>, BrokerError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(v) => dec(v).map(Some),
    }
}

pub(crate) fn int(v: &Value) -> Result<i64, BrokerError> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .ok_or_else(|| BrokerError::Decode(format!("not an integer: {n}"))),
        Value::String(s) => s
            .trim()
            .parse::<i64>()
            .map_err(|_| BrokerError::Decode(format!("not an integer: {s}"))),
        other => decode(format!("expected integer, got {other}")),
    }
}

pub(crate) fn ts_ms(v: &Value) -> Result<DateTime<Utc>, BrokerError> {
    let ms = int(v)?;
    Utc.timestamp_millis_opt(ms)
        .single()
        .ok_or_else(|| BrokerError::Decode(format!("invalid timestamp {ms}")))
}

fn field<'a>(o: &'a Value, k: &str) -> Result<&'a Value, BrokerError> {
    o.get(k)
        .ok_or_else(|| BrokerError::Decode(format!("missing field {k}")))
}

fn str_field<'a>(o: &'a Value, k: &str) -> Result<&'a str, BrokerError> {
    field(o, k)?
        .as_str()
        .ok_or_else(|| BrokerError::Decode(format!("field {k} is not a string")))
}

/// Error body: `{"errorCode": 205, "error": "..."}`.
#[derive(Debug, Deserialize)]
pub(crate) struct ApiError {
    #[serde(rename = "errorCode")]
    pub error_code: Option<i64>,
    pub error: Option<String>,
}

/// Map a `/markets` entry to an [`Instrument`]. Returns `None` for non-EUR markets.
pub(crate) fn market_to_instrument(
    m: &Value,
    now: DateTime<Utc>,
) -> Result<Option<Instrument>, BrokerError> {
    let quote = str_field(m, "quote")?;
    if quote != "EUR" {
        return Ok(None);
    }
    let market = str_field(m, "market")?;
    let base = str_field(m, "base")?;
    let status = str_field(m, "status")?;
    let id = InstrumentId::parse(&format!("bitvavo:{market}"))
        .map_err(|e| BrokerError::Decode(e.to_string()))?;
    let quantity_decimals = int(field(m, "quantityDecimals")?)?;
    let quantity_decimals = u32::try_from(quantity_decimals)
        .map_err(|_| BrokerError::Decode("quantityDecimals".into()))?;
    Ok(Some(Instrument {
        id,
        name: base.to_owned(),
        asset_class: AssetClass::Crypto,
        quote_currency: Currency::EUR,
        isin: None,
        min_notional: dec(field(m, "minOrderInQuoteAsset")?)?,
        min_quantity: dec(field(m, "minOrderInBaseAsset")?)?,
        quantity_decimals,
        tick_size: opt_dec(m.get("tickSize"))?.unwrap_or(Decimal::ZERO),
        tradable: status == "trading",
        first_seen: now,
    }))
}

/// Parse candles `[[ts_ms,"o","h","l","c","v"], ...]` (newest first) into oldest-first
/// closed candles. A candle whose interval has not ended at `now` is dropped.
pub(crate) fn parse_candles(
    v: &Value,
    interval_secs: i64,
    now: DateTime<Utc>,
) -> Result<Vec<Candle>, BrokerError> {
    let arr = v
        .as_array()
        .ok_or_else(|| BrokerError::Decode("candles: expected array".into()))?;
    let mut out = Vec::with_capacity(arr.len());
    for row in arr {
        let r = row
            .as_array()
            .ok_or_else(|| BrokerError::Decode("candle row: expected array".into()))?;
        if r.len() < 6 {
            return decode("candle row: expected 6 elements");
        }
        let c = Candle {
            ts: ts_ms(&r[0])?,
            open: dec(&r[1])?,
            high: dec(&r[2])?,
            low: dec(&r[3])?,
            close: dec(&r[4])?,
            volume: dec(&r[5])?,
        };
        if c.ts + chrono::Duration::seconds(interval_secs) > now {
            continue; // still open
        }
        c.validate()
            .map_err(|e| BrokerError::Decode(e.to_string()))?;
        out.push(c);
    }
    out.sort_by_key(|c| c.ts);
    out.dedup_by_key(|c| c.ts);
    Ok(out)
}

fn status(s: &str) -> Result<BrokerOrderStatus, BrokerError> {
    Ok(match s {
        "new" | "awaitingTrigger" => BrokerOrderStatus::Open,
        "partiallyFilled" => BrokerOrderStatus::PartiallyFilled,
        "filled" => BrokerOrderStatus::Filled,
        "canceled" | "cancelled" => BrokerOrderStatus::Cancelled,
        "expired" => BrokerOrderStatus::Expired,
        "rejected" => BrokerOrderStatus::Rejected,
        other => return decode(format!("unknown order status {other}")),
    })
}

/// Map an order object (create/get/list responses share the shape).
pub(crate) fn parse_order(o: &Value) -> Result<BrokerOrder, BrokerError> {
    let market = str_field(o, "market")?;
    let instrument = InstrumentId::parse(&format!("bitvavo:{market}"))
        .map_err(|e| BrokerError::Decode(e.to_string()))?;
    let side = match str_field(o, "side")? {
        "buy" => Side::Buy,
        "sell" => Side::Sell,
        s => return decode(format!("unknown side {s}")),
    };
    let order_type = o
        .get("orderType")
        .and_then(Value::as_str)
        .unwrap_or("market");
    let price = opt_dec(o.get("price"))?;
    let trigger = opt_dec(o.get("triggerPrice"))?.or(opt_dec(o.get("triggerAmount"))?);
    let kind = match (order_type, price, trigger) {
        ("limit", Some(p), _) => OrderKind::Limit { price: p },
        ("stopLoss" | "takeProfit", _, Some(t)) => OrderKind::Stop { stop: t },
        ("stopLossLimit" | "takeProfitLimit", Some(p), Some(t)) => {
            OrderKind::StopLimit { stop: t, limit: p }
        }
        _ => OrderKind::Market,
    };
    let filled = opt_dec(o.get("filledAmount"))?.unwrap_or(Decimal::ZERO);
    let filled_quote = opt_dec(o.get("filledAmountQuote"))?.unwrap_or(Decimal::ZERO);
    let quantity = opt_dec(o.get("amount"))?.unwrap_or(filled);
    let fee_currency = o
        .get("feeCurrency")
        .and_then(Value::as_str)
        .and_then(|c| Currency::new(c).ok());
    let mut fills = Vec::new();
    if let Some(arr) = o.get("fills").and_then(Value::as_array) {
        for f in arr {
            fills.push(BrokerFill {
                fill_id: str_field(f, "id")?.to_owned(),
                quantity: dec(field(f, "amount")?)?,
                price: dec(field(f, "price")?)?,
                fee: opt_dec(f.get("fee"))?.unwrap_or(Decimal::ZERO),
                fee_currency: f
                    .get("feeCurrency")
                    .and_then(Value::as_str)
                    .and_then(|c| Currency::new(c).ok()),
                ts: ts_ms(field(f, "timestamp")?)?,
            });
        }
    }
    let created_at = ts_ms(field(o, "created")?)?;
    let updated_at = match o.get("updated") {
        Some(v) => ts_ms(v)?,
        None => created_at,
    };
    let status = match o.get("status").and_then(Value::as_str) {
        Some(s) => status(s)?,
        // responseRequired=false returns only ids; treat as acknowledged/open.
        None => BrokerOrderStatus::Open,
    };
    let client_order_id = match o.get("clientOrderId").and_then(Value::as_str) {
        Some(s) => Some(
            Uuid::parse_str(s)
                .map_err(|_| BrokerError::Decode(format!("bad clientOrderId {s}")))?,
        ),
        None => None,
    };
    Ok(BrokerOrder {
        broker_order_id: str_field(o, "orderId")?.to_owned(),
        client_order_id,
        instrument,
        side,
        kind,
        quantity,
        filled_quantity: filled,
        avg_fill_price: if filled > Decimal::ZERO {
            Some(filled_quote / filled)
        } else {
            None
        },
        fee_paid: opt_dec(o.get("feePaid"))?.unwrap_or(Decimal::ZERO),
        fee_currency,
        status,
        via_api: true,
        created_at,
        updated_at,
        fills,
    })
}

pub(crate) fn instrument_for(market: &str) -> Result<InstrumentId, BrokerError> {
    InstrumentId::parse(&format!("{}:{market}", Broker::Bitvavo.as_str()))
        .map_err(|e| BrokerError::Decode(e.to_string()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use serde_json::json;

    #[test]
    fn markets_handle_numeric_and_string_decimals() {
        let live = json!({"market":"BTC-EUR","status":"trading","base":"BTC","quote":"EUR","pricePrecision":null,
            "minOrderInBaseAsset":"0.00007294","minOrderInQuoteAsset":"5.00","maxOrderInBaseAsset":"14588.11369242",
            "maxOrderInQuoteAsset":"1000000000.00","quantityDecimals":8,"notionalDecimals":2,"tickSize":"1.00",
            "maxOpenOrders":400,"feeCategory":"A","orderTypes":["market","limit"]});
        let i = market_to_instrument(&live, Utc::now()).unwrap().unwrap();
        assert_eq!(i.quantity_decimals, 8);
        assert_eq!(i.min_notional, dec!(5.00));
        assert_eq!(i.tick_size, dec!(1.00));
        assert!(i.tradable);
        let doc = json!({"market":"ETH-EUR","status":"halted","base":"ETH","quote":"EUR",
            "minOrderInBaseAsset":"0.001","minOrderInQuoteAsset":"5","quantityDecimals":"4","tickSize":"0.01"});
        let i = market_to_instrument(&doc, Utc::now()).unwrap().unwrap();
        assert_eq!(i.quantity_decimals, 4);
        assert!(!i.tradable);
        let usdc = json!({"market":"BTC-USDC","status":"trading","base":"BTC","quote":"USDC",
            "minOrderInBaseAsset":"0.001","minOrderInQuoteAsset":"5","quantityDecimals":8});
        assert!(market_to_instrument(&usdc, Utc::now()).unwrap().is_none());
    }

    #[test]
    fn candles_are_oldest_first_and_open_candle_dropped() {
        // Live sample shape from 2026-11-02.
        let v = json!([
            [
                1793584800000i64,
                "70135",
                "70293",
                "70076",
                "70154",
                "0.45073138"
            ],
            [
                1793581200000i64,
                "69913",
                "70315",
                "69876",
                "70127",
                "4.32182903"
            ]
        ]);
        let now = Utc
            .timestamp_millis_opt(1793584800000 + 1_800_000)
            .single()
            .unwrap();
        let c = parse_candles(&v, 3600, now).unwrap();
        assert_eq!(c.len(), 1, "the 1793584800000 candle is still open");
        assert_eq!(c[0].close, dec!(70127));
        let later = Utc
            .timestamp_millis_opt(1793584800000 + 3_600_000)
            .single()
            .unwrap();
        let c = parse_candles(&v, 3600, later).unwrap();
        assert_eq!(c.len(), 2);
        assert!(c[0].ts < c[1].ts);
    }

    #[test]
    fn scientific_notation_in_order_fields() {
        let o = json!({"orderId":"1be6d0df-d5dc-4b53-a250-3376f3b393e6","market":"BTC-EUR","created":1706100650751i64,
            "updated":1706100650751i64,"status":"filled","side":"buy","orderType":"market","amount":"1E-4",
            "filledAmount":"1E-4","filledAmountQuote":"7.014","feePaid":"0.0175","feeCurrency":"EUR",
            "fills":[{"id":"371c6bd3-d06d-4573-9f15-18697cd210e5","timestamp":1706100650751i64,"amount":"0.0001",
            "price":"7.014E+4","taker":true,"fee":"0.0175","feeCurrency":"EUR","settled":true}]});
        let b = parse_order(&o).unwrap();
        assert_eq!(b.quantity, dec!(0.0001));
        assert_eq!(b.fills[0].price, dec!(70140));
        assert_eq!(b.avg_fill_price, Some(dec!(70140)));
        assert_eq!(b.status, BrokerOrderStatus::Filled);
    }
}

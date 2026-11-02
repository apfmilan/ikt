//! EODHD end-of-day stock/ETF data (default provider, docs/decisions/proposals/market-data-research.md).
//!
//! Verified 2026-11-02:
//! * `GET /api/eod/{CODE}.{EXCH}?api_token=…&fmt=json&period=d&from=YYYY-MM-DD&to=YYYY-MM-DD`
//!   returns `[{date,open,high,low,close,adjusted_close,volume}]` (live call with the `demo`
//!   token on AAPL.US).
//! * `GET /api/real-time/{CODE}.{EXCH}?api_token=…&fmt=json` returns
//!   `{code,timestamp,gmtoffset,open,high,low,close,volume,previousClose,change,change_p}`;
//!   prices are ~15 minutes delayed, so the risk engine's quote-age check vetoes entries on them.
//!
//! UNVERIFIED: `GET /api/search/{ISIN}` — response fields (`Code, Exchange, Name, Type, Country,
//! Currency, ISIN, isPrimary, previousClose, previousCloseDate`) are taken from the official
//! docs page; the demo token cannot call it, so the shape has not been observed live. It is used
//! only to *propose* symbol mappings, never in the order path.
//!
//! EODHD states its prices are indicative and "not appropriate for trading purposes"; callers
//! must cross-check against Trading 212 prices before trading.
//!
//! The API token is passed as a query parameter, so every error message is produced without the
//! request URL (`reqwest::Error::without_url`) and response bodies are not echoed with URLs.

use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, TimeZone, Utc};
use reqwest::Url;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;
use tc_broker::ratelimit::{Limit, RateLimiter};
use tc_broker::{BrokerError, BrokerResult, MarketDataSource, Quote};
use tc_core::{parse_decimal, Candle, InstrumentId, Interval};

pub const EODHD_BASE_URL: &str = "https://eodhd.com/api";
const REQ_KEY: &str = "req";

#[derive(Clone)]
pub struct EodhdConfig {
    pub base_url: String,
    pub api_token: SecretString,
    pub timeout: Duration,
    /// Free tier: 20 requests/min, 20 calls/day; paid EOD plan: 1000/min.
    pub requests_per_minute: u32,
}

impl EodhdConfig {
    pub fn new(base_url: impl Into<String>, api_token: SecretString) -> Self {
        EodhdConfig {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_token,
            timeout: Duration::from_secs(15),
            requests_per_minute: 20,
        }
    }
}

impl fmt::Debug for EodhdConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EodhdConfig")
            .field("base_url", &self.base_url)
            .field("api_token", &"[REDACTED]")
            .field("requests_per_minute", &self.requests_per_minute)
            .finish()
    }
}

/// One listing returned by the search endpoint (UNVERIFIED shape, see module docs).
#[derive(Clone, Debug, PartialEq)]
pub struct EodhdListing {
    pub code: String,
    pub exchange: String,
    pub name: String,
    pub kind: String,
    pub currency: String,
    pub isin: Option<String>,
    pub is_primary: bool,
}

impl EodhdListing {
    pub fn symbol(&self) -> String {
        format!("{}.{}", self.code, self.exchange)
    }
}

/// Pick the unique listing on `exchange` quoted in `currency` (`GBX` matches `GBX` only).
/// Zero or several matches → `None`: the instrument stays unmapped and is not traded.
pub fn select_listing<'a>(
    listings: &'a [EodhdListing],
    exchange: &str,
    currency: &str,
) -> Option<&'a EodhdListing> {
    let mut it = listings.iter().filter(|l| {
        l.exchange.eq_ignore_ascii_case(exchange) && l.currency.eq_ignore_ascii_case(currency)
    });
    let first = it.next()?;
    if it.next().is_some() {
        return None;
    }
    Some(first)
}

fn valid_symbol(s: &str) -> bool {
    let mut parts = s.split('.');
    let (Some(code), Some(exch), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let ok = |p: &str| {
        !p.is_empty()
            && p.len() <= 20
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    ok(code) && ok(exch)
}

fn num(v: Option<&Value>) -> Option<Decimal> {
    match v? {
        Value::Number(n) => parse_decimal(&n.to_string()).ok(),
        Value::String(s) => parse_decimal(s).ok(), // "NA" → None
        _ => None,
    }
}

pub struct EodhdClient {
    cfg: EodhdConfig,
    http: reqwest::Client,
    limiter: RateLimiter,
    symbols: Mutex<HashMap<InstrumentId, String>>,
}

impl fmt::Debug for EodhdClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EodhdClient")
            .field("cfg", &self.cfg)
            .finish_non_exhaustive()
    }
}

impl EodhdClient {
    pub fn new(cfg: EodhdConfig) -> BrokerResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent("trading-center/2")
            .build()
            .map_err(|e| BrokerError::Network(e.without_url().to_string()))?;
        let limiter = RateLimiter::new(&[(
            REQ_KEY,
            Limit::per(cfg.requests_per_minute.max(1), Duration::from_secs(60)),
        )]);
        Ok(EodhdClient {
            cfg,
            http,
            limiter,
            symbols: Mutex::new(HashMap::new()),
        })
    }

    /// Register the EODHD symbol (`CODE.EXCH`, e.g. `AAPL.US`, `VUSA.LSE`) for an instrument.
    pub fn set_symbol(&self, id: InstrumentId, eod_symbol: &str) -> BrokerResult<()> {
        if !valid_symbol(eod_symbol) {
            return Err(BrokerError::Decode(format!(
                "invalid EODHD symbol {eod_symbol}"
            )));
        }
        if let Ok(mut m) = self.symbols.lock() {
            m.insert(id, eod_symbol.to_owned());
        }
        Ok(())
    }

    pub fn symbol_for(&self, id: &InstrumentId) -> Option<String> {
        self.symbols.lock().ok().and_then(|m| m.get(id).cloned())
    }

    async fn get(&self, path: &str, params: &[(&str, String)]) -> BrokerResult<Value> {
        self.limiter.acquire(REQ_KEY, 1).await;
        let mut url = Url::parse(&format!("{}{}", self.cfg.base_url, path))
            .map_err(|e| BrokerError::Decode(format!("bad url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("api_token", self.cfg.api_token.expose_secret());
            q.append_pair("fmt", "json");
            for (k, v) in params {
                q.append_pair(k, v);
            }
        }
        let resp = self.http.get(url).send().await.map_err(|e| {
            if e.is_timeout() {
                BrokerError::Timeout
            } else {
                BrokerError::Network(e.without_url().to_string())
            }
        })?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| BrokerError::Network(e.without_url().to_string()))?;
        if status.as_u16() == 429 {
            self.limiter.block_for(REQ_KEY, Duration::from_secs(60));
            return Err(BrokerError::RateLimited { retry_at: None });
        }
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(BrokerError::Auth(format!("eodhd http {}", status.as_u16())));
        }
        if !status.is_success() {
            let token = self.cfg.api_token.expose_secret();
            let mut message: String = body.chars().take(200).collect();
            if !token.is_empty() {
                message = message.replace(token, "[REDACTED]");
            }
            return Err(BrokerError::Rejected {
                status: status.as_u16(),
                code: "eodhd".into(),
                message,
            });
        }
        serde_json::from_str(&body).map_err(|e| BrokerError::Decode(format!("eodhd {path}: {e}")))
    }

    /// Daily bars between `from` and `to` (inclusive), oldest first, split/dividend adjusted
    /// (open/high/low scaled by `adjusted_close / close`). Bars dated today or later (UTC) are
    /// dropped because they may still change.
    pub async fn eod(
        &self,
        symbol: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> BrokerResult<Vec<Candle>> {
        if !valid_symbol(symbol) {
            return Err(BrokerError::Decode(format!(
                "invalid EODHD symbol {symbol}"
            )));
        }
        let v = self
            .get(
                &format!("/eod/{symbol}"),
                &[
                    ("period", "d".into()),
                    ("from", from.to_string()),
                    ("to", to.to_string()),
                ],
            )
            .await?;
        let arr = v
            .as_array()
            .ok_or_else(|| BrokerError::Decode("eod: expected array".into()))?;
        let today = Utc::now().date_naive();
        let mut out = Vec::with_capacity(arr.len());
        for row in arr {
            let Some(date) = row
                .get("date")
                .and_then(Value::as_str)
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            else {
                return Err(BrokerError::Decode("eod: bad date".into()));
            };
            if date >= today {
                continue;
            }
            let (Some(o), Some(h), Some(l), Some(c)) = (
                num(row.get("open")),
                num(row.get("high")),
                num(row.get("low")),
                num(row.get("close")),
            ) else {
                tracing::warn!(symbol, %date, "eod bar with missing prices skipped");
                continue;
            };
            let adj = num(row.get("adjusted_close")).unwrap_or(c);
            let f = if c > Decimal::ZERO {
                adj / c
            } else {
                Decimal::ONE
            };
            let ts = Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap_or_default());
            let candle = Candle {
                ts,
                open: (o * f).round_dp(10),
                high: (h * f).round_dp(10),
                low: (l * f).round_dp(10),
                close: adj.round_dp(10),
                volume: num(row.get("volume")).unwrap_or(Decimal::ZERO),
            };
            if candle.validate().is_err() {
                tracing::warn!(symbol, %date, "inconsistent eod bar skipped");
                continue;
            }
            out.push(candle);
        }
        out.sort_by_key(|c| c.ts);
        out.dedup_by_key(|c| c.ts);
        Ok(out)
    }

    /// Search listings by ISIN (UNVERIFIED response shape; see module docs).
    pub async fn search(&self, query: &str) -> BrokerResult<Vec<EodhdListing>> {
        if query.is_empty() || !query.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(BrokerError::Decode(
                "search query must be alphanumeric (ISIN or ticker)".into(),
            ));
        }
        let v = self
            .get(&format!("/search/{query}"), &[("limit", "50".into())])
            .await?;
        let arr = v
            .as_array()
            .ok_or_else(|| BrokerError::Decode("search: expected array".into()))?;
        let s = |o: &Value, k: &str| {
            o.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Ok(arr
            .iter()
            .map(|o| EodhdListing {
                code: s(o, "Code"),
                exchange: s(o, "Exchange"),
                name: s(o, "Name"),
                kind: s(o, "Type"),
                currency: s(o, "Currency"),
                isin: o.get("ISIN").and_then(Value::as_str).map(str::to_owned),
                is_primary: o.get("isPrimary").and_then(Value::as_bool).unwrap_or(false),
            })
            .filter(|l| !l.code.is_empty() && !l.exchange.is_empty())
            .collect())
    }
}

#[async_trait]
impl MarketDataSource for EodhdClient {
    fn name(&self) -> &'static str {
        "eodhd"
    }

    /// Daily candles only: EODHD intraday bars are finalised 2-3 h after the close and are not
    /// suitable for live signals.
    async fn candles(
        &self,
        instrument: &InstrumentId,
        interval: Interval,
        limit: usize,
        end: Option<DateTime<Utc>>,
    ) -> BrokerResult<Vec<Candle>> {
        if interval != Interval::D1 {
            return Err(BrokerError::Unsupported(
                "eodhd adapter serves daily bars only",
            ));
        }
        let symbol = self.symbol_for(instrument).ok_or(BrokerError::Unsupported(
            "instrument has no verified EODHD symbol mapping",
        ))?;
        let end_date = end.unwrap_or_else(Utc::now).date_naive();
        let to = end_date - ChronoDuration::days(1);
        let span = (limit as i64) * 7 / 5 + 10;
        let from = to - ChronoDuration::days(span);
        let mut c = self.eod(&symbol, from, to).await?;
        c.retain(|x| x.ts.date_naive() < end_date);
        if c.len() > limit {
            c.drain(..c.len() - limit);
        }
        Ok(c)
    }

    async fn quote(&self, instrument: &InstrumentId) -> BrokerResult<Quote> {
        let symbol = self.symbol_for(instrument).ok_or(BrokerError::Unsupported(
            "instrument has no verified EODHD symbol mapping",
        ))?;
        let v = self.get(&format!("/real-time/{symbol}"), &[]).await?;
        let ts = v
            .get("timestamp")
            .and_then(Value::as_i64)
            .and_then(|s| DateTime::<Utc>::from_timestamp(s, 0))
            .ok_or_else(|| BrokerError::Decode("real-time: missing timestamp".into()))?;
        Ok(Quote {
            instrument: instrument.clone(),
            bid: None,
            ask: None,
            last: num(v.get("close")),
            volume_quote_24h: None,
            ts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(code: &str, ex: &str, ccy: &str) -> EodhdListing {
        EodhdListing {
            code: code.into(),
            exchange: ex.into(),
            name: String::new(),
            kind: "ETF".into(),
            currency: ccy.into(),
            isin: Some("IE00B3XXRP09".into()),
            is_primary: false,
        }
    }

    #[test]
    fn listing_selection_requires_unique_match() {
        let ls = vec![
            l("VUSA", "LSE", "GBP"),
            l("VUSD", "LSE", "USD"),
            l("VUSA", "XETRA", "EUR"),
        ];
        assert_eq!(
            select_listing(&ls, "LSE", "GBP").map(EodhdListing::symbol),
            Some("VUSA.LSE".into())
        );
        assert_eq!(select_listing(&ls, "US", "USD"), None);
        let dup = vec![l("A", "LSE", "GBP"), l("B", "LSE", "GBP")];
        assert_eq!(
            select_listing(&dup, "LSE", "GBP"),
            None,
            "ambiguous mapping fails closed"
        );
    }

    #[test]
    fn symbol_validation() {
        assert!(valid_symbol("AAPL.US"));
        assert!(valid_symbol("BRK-B.US"));
        assert!(!valid_symbol("AAPL"));
        assert!(!valid_symbol("../eod.US"));
        assert!(!valid_symbol("A.B.C"));
    }
}

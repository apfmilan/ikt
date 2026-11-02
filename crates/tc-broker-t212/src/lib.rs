//! Trading 212 Public API v0 (beta) connector.
//!
//! Source of truth: https://docs.trading212.com/api and the OpenAPI bundle
//! `https://docs.trading212.com/_bundle/api.json` (checked 2026-11-02).
//!
//! Verified constraints that shape this code:
//! * HTTP Basic auth: API key = username, API secret = password.
//! * Only Invest / Stocks ISA accounts; orders only in the primary account currency.
//! * Orders are quantity-based; a SELL is a **negative** quantity.
//! * **Order endpoints are not idempotent** and there is no client order id. `place_order`
//!   therefore sends exactly once; timeouts, 408 and 5xx are reported as
//!   [`PlaceOutcome::Unknown`] and must be reconciled from pending orders / order history
//!   (matching ticker, signed quantity, `initiatedFrom == API`, `createdAt`), never re-sent.
//! * Per-endpoint rate limits (see [`endpoint_limits`]); `x-ratelimit-*` headers honoured.
//! * No market-data endpoints (prices come from a separate provider).
//!
//! UNVERIFIED (not stated in the docs): fractional quantity precision and minimum order value
//! per instrument. Both are configuration ([`Trading212Config::quantity_decimals`],
//! [`Trading212Config::min_notional`]); a wrong value produces a definitive 400 rejection, not
//! a duplicate order.

mod wire;

use async_trait::async_trait;
use base64::Engine as _;
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::{Method, StatusCode, Url};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;
use tc_broker::ratelimit::{Limit, RateLimiter};
use tc_broker::{
    live, AccountSnapshot, BrokerClient, BrokerError, BrokerOrder, BrokerPosition, BrokerResult,
    CashBalance, CashMovement, CashMovementKind, CredentialReport, HealthCounters, OrderRef,
    PlaceOutcome,
};
use tc_core::{AssetClass, Broker, Instrument, OrderIntent, OrderKind, Side, TimeInForce};

use wire::{field, num, opt_num, str_field, PriceUnit};

pub const DEFAULT_BASE_URL: &str = "https://live.trading212.com/api/v0";
const MAX_PAGES: usize = 40;

/// Rate limits per endpoint class, from the OpenAPI operation descriptions (2026-11-02).
pub fn endpoint_limits() -> Vec<(&'static str, Limit)> {
    let s = Duration::from_secs;
    vec![
        ("account_summary", Limit::per(1, s(5))),
        ("positions", Limit::per(1, s(1))),
        ("orders_list", Limit::per(1, s(5))),
        ("order_get", Limit::per(1, s(1))),
        ("order_market", Limit::per(50, s(60))),
        ("order_limit", Limit::per(1, s(2))),
        ("order_stop", Limit::per(1, s(2))),
        ("order_stop_limit", Limit::per(1, s(2))),
        ("order_cancel", Limit::per(50, s(60))),
        ("instruments", Limit::per(1, s(50))),
        ("exchanges", Limit::per(1, s(30))),
        ("history_orders", Limit::per(20, s(60))),
        ("history_transactions", Limit::per(20, s(60))),
        ("history_dividends", Limit::per(20, s(60))),
    ]
}

#[derive(Clone)]
pub struct Trading212Config {
    pub base_url: String,
    pub api_key: SecretString,
    pub api_secret: SecretString,
    pub timeout: Duration,
    /// Decimals allowed in order quantities (UNVERIFIED; default 4).
    pub quantity_decimals: u32,
    /// Minimum order notional in instrument currency (UNVERIFIED; default 1).
    pub min_notional: Decimal,
}

impl Trading212Config {
    pub fn new(
        base_url: impl Into<String>,
        api_key: SecretString,
        api_secret: SecretString,
    ) -> Self {
        Trading212Config {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key,
            api_secret,
            timeout: Duration::from_secs(10),
            quantity_decimals: 4,
            min_notional: Decimal::ONE,
        }
    }

    pub fn live(api_key: SecretString, api_secret: SecretString) -> Self {
        Trading212Config::new(DEFAULT_BASE_URL, api_key, api_secret)
    }
}

impl fmt::Debug for Trading212Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Trading212Config")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .field("api_secret", &"[REDACTED]")
            .field("quantity_decimals", &self.quantity_decimals)
            .field("min_notional", &self.min_notional)
            .finish()
    }
}

/// Trading 212-specific instrument metadata kept alongside the domain [`Instrument`].
#[derive(Clone, Debug, PartialEq)]
pub struct T212InstrumentMeta {
    pub ticker: String,
    pub kind: String,
    pub currency_code: String,
    pub working_schedule_id: Option<i64>,
    pub extended_hours: bool,
    pub max_open_quantity: Option<Decimal>,
}

#[derive(Default)]
struct HealthState {
    last_ok: Option<DateTime<Utc>>,
    last_error: Option<DateTime<Utc>>,
    last_rate_limited: Option<DateTime<Utc>>,
    rate_budget_remaining: Option<Decimal>,
    window: VecDeque<(DateTime<Utc>, bool)>,
}

impl HealthState {
    fn record(&mut self, ok: bool) {
        let now = Utc::now();
        if ok {
            self.last_ok = Some(now);
        } else {
            self.last_error = Some(now);
        }
        self.window.push_back((now, ok));
        let cutoff = now - chrono::Duration::minutes(5);
        while self.window.front().is_some_and(|(t, _)| *t < cutoff) {
            self.window.pop_front();
        }
    }
}

/// Working schedule id → (event time, event type), sorted by time.
type Schedules = HashMap<i64, Vec<(DateTime<Utc>, String)>>;

struct Raw {
    status: StatusCode,
    body: String,
    reset_at: Option<DateTime<Utc>>,
}

pub struct Trading212Client {
    cfg: Trading212Config,
    http: reqwest::Client,
    limiter: RateLimiter,
    health: Mutex<HealthState>,
    meta: Mutex<HashMap<String, T212InstrumentMeta>>,
    /// working schedule id → (event time, event type)
    schedules: Mutex<Schedules>,
}

impl fmt::Debug for Trading212Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Trading212Client")
            .field("cfg", &self.cfg)
            .finish_non_exhaustive()
    }
}

impl Trading212Client {
    pub fn new(cfg: Trading212Config) -> BrokerResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent("trading-center/2")
            .build()
            .map_err(|e| BrokerError::Network(e.without_url().to_string()))?;
        Ok(Trading212Client {
            cfg,
            http,
            limiter: RateLimiter::new(&endpoint_limits()),
            health: Mutex::new(HealthState::default()),
            meta: Mutex::new(HashMap::new()),
            schedules: Mutex::new(HashMap::new()),
        })
    }

    fn auth_header(&self) -> String {
        let raw = format!(
            "{}:{}",
            self.cfg.api_key.expose_secret(),
            self.cfg.api_secret.expose_secret()
        );
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(raw)
        )
    }

    fn origin(&self) -> String {
        match Url::parse(&self.cfg.base_url) {
            Ok(u) => {
                let mut o = format!("{}://{}", u.scheme(), u.host_str().unwrap_or_default());
                if let Some(p) = u.port() {
                    o.push_str(&format!(":{p}"));
                }
                o
            }
            Err(_) => self.cfg.base_url.clone(),
        }
    }

    /// Resolve a `nextPagePath`. Only relative paths under the API's history endpoints are
    /// accepted (no scheme, no host switch).
    fn next_page_url(&self, next: &str) -> BrokerResult<String> {
        if !next.starts_with('/') || next.starts_with("//") || next.contains("://") {
            return Err(BrokerError::Decode(format!(
                "unexpected nextPagePath {next}"
            )));
        }
        if next.starts_with("/api/") {
            Ok(format!("{}{}", self.origin(), next))
        } else {
            Ok(format!("{}{}", self.cfg.base_url, next))
        }
    }

    fn unit(&self, ticker: &str) -> PriceUnit {
        self.meta
            .lock()
            .ok()
            .and_then(|m| {
                m.get(ticker)
                    .map(|x| PriceUnit::from_currency_code(&x.currency_code))
            })
            .unwrap_or(PriceUnit::Major)
    }

    fn unit_map(&self) -> HashMap<String, PriceUnit> {
        self.meta
            .lock()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), PriceUnit::from_currency_code(&v.currency_code)))
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn send(
        &self,
        method: Method,
        url: &str,
        body: Option<String>,
        key: &'static str,
    ) -> BrokerResult<Raw> {
        self.limiter.acquire(key, 1).await;
        let mut req = self
            .http
            .request(method, url)
            .header("Authorization", self.auth_header());
        if let Some(b) = body {
            req = req.header("Content-Type", "application/json").body(b);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                self.record(false);
                return Err(if e.is_timeout() {
                    BrokerError::Timeout
                } else {
                    BrokerError::Network(e.without_url().to_string())
                });
            }
        };
        let status = resp.status();
        let reset_at = self.read_rate_headers(resp.headers(), status, key);
        let body = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                self.record(false);
                return Err(if e.is_timeout() {
                    BrokerError::Timeout
                } else {
                    BrokerError::Network(e.without_url().to_string())
                });
            }
        };
        self.record(status.is_success());
        Ok(Raw {
            status,
            body,
            reset_at,
        })
    }

    fn record(&self, ok: bool) {
        if let Ok(mut h) = self.health.lock() {
            h.record(ok);
        }
    }

    fn read_rate_headers(
        &self,
        headers: &reqwest::header::HeaderMap,
        status: StatusCode,
        key: &'static str,
    ) -> Option<DateTime<Utc>> {
        let get = |k: &str| {
            headers
                .get(k)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let remaining = get("x-ratelimit-remaining").and_then(|v| v.parse::<Decimal>().ok());
        let limit = get("x-ratelimit-limit").and_then(|v| v.parse::<Decimal>().ok());
        let reset = get("x-ratelimit-reset")
            .and_then(|v| v.parse::<i64>().ok())
            .and_then(|s| DateTime::<Utc>::from_timestamp(s, 0));
        if let Ok(mut h) = self.health.lock() {
            if let (Some(r), Some(l)) = (remaining, limit) {
                if l > Decimal::ZERO {
                    h.rate_budget_remaining = Some((r / l).clamp(Decimal::ZERO, Decimal::ONE));
                }
            }
            if status == StatusCode::TOO_MANY_REQUESTS {
                h.last_rate_limited = Some(Utc::now());
            }
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            let wait = reset
                .map(|t| (t - Utc::now()).num_milliseconds())
                .filter(|ms| *ms > 0)
                .map(|ms| Duration::from_millis(ms as u64 + 1_000))
                .unwrap_or(Duration::from_secs(60));
            self.limiter.block_for(key, wait);
        } else if remaining.is_some_and(|r| r.is_zero()) {
            if let Some(t) = reset {
                let ms = (t - Utc::now()).num_milliseconds();
                if ms > 0 {
                    self.limiter
                        .block_for(key, Duration::from_millis(ms as u64));
                }
            }
        }
        reset
    }

    fn error_from(raw: &Raw) -> BrokerError {
        let msg: String = raw.body.chars().take(300).collect();
        match raw.status {
            StatusCode::UNAUTHORIZED => BrokerError::Auth(msg),
            StatusCode::FORBIDDEN => BrokerError::Forbidden(msg),
            StatusCode::TOO_MANY_REQUESTS => BrokerError::RateLimited {
                retry_at: raw.reset_at,
            },
            s => BrokerError::Rejected {
                status: s.as_u16(),
                code: format!("http_{}", s.as_u16()),
                message: msg,
            },
        }
    }

    async fn get_json(&self, path: &str, key: &'static str) -> BrokerResult<Value> {
        let url = format!("{}{}", self.cfg.base_url, path);
        self.get_url(&url, key).await
    }

    async fn get_url(&self, url: &str, key: &'static str) -> BrokerResult<Value> {
        let raw = self.send(Method::GET, url, None, key).await?;
        if !raw.status.is_success() {
            return Err(Self::error_from(&raw));
        }
        serde_json::from_str(&raw.body).map_err(|e| BrokerError::Decode(format!("{e}")))
    }

    /// Follow cursor pagination (`nextPagePath`) until `stop(item)` is true for an item, the
    /// path is null, or [`MAX_PAGES`] is reached. Items are returned in server order.
    async fn paginate(
        &self,
        first: &str,
        key: &'static str,
        stop: impl Fn(&Value) -> bool,
    ) -> BrokerResult<Vec<Value>> {
        let mut url = format!("{}{}", self.cfg.base_url, first);
        let mut out = Vec::new();
        for _ in 0..MAX_PAGES {
            let v = self.get_url(&url, key).await?;
            let items = v
                .get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut done = false;
            for it in items {
                if stop(&it) {
                    done = true;
                    break;
                }
                out.push(it);
            }
            match v.get("nextPagePath").and_then(Value::as_str) {
                Some(next) if !done && !next.is_empty() => url = self.next_page_url(next)?,
                _ => break,
            }
        }
        Ok(out)
    }

    /// Load exchange working schedules (`GET /equity/metadata/exchanges`, 1 req / 30 s).
    pub async fn load_exchanges(&self) -> BrokerResult<usize> {
        let v = self
            .get_json("/equity/metadata/exchanges", "exchanges")
            .await?;
        let arr = v
            .as_array()
            .ok_or_else(|| BrokerError::Decode("exchanges: expected array".into()))?;
        let mut map = HashMap::new();
        for ex in arr {
            for ws in ex
                .get("workingSchedules")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                let Some(id) = ws.get("id").and_then(Value::as_i64) else {
                    continue;
                };
                let mut events = Vec::new();
                for te in ws
                    .get("timeEvents")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                {
                    let (Some(d), Some(t)) =
                        (te.get("date"), te.get("type").and_then(Value::as_str))
                    else {
                        continue;
                    };
                    if let Ok(ts) = wire::time(d) {
                        events.push((ts, t.to_owned()));
                    }
                }
                events.sort_by_key(|(t, _)| *t);
                map.insert(id, events);
            }
        }
        let n = map.len();
        if let Ok(mut s) = self.schedules.lock() {
            *s = map;
        }
        Ok(n)
    }

    pub fn meta(&self, ticker: &str) -> Option<T212InstrumentMeta> {
        self.meta.lock().ok().and_then(|m| m.get(ticker).cloned())
    }

    /// Regular session `(OPEN, CLOSE)` in UTC for `ticker` on `date` (UTC date of the OPEN
    /// event). `None` = no regular session that day or schedule unknown (callers fail closed).
    /// Requires [`Self::load_exchanges`] and `instruments()` to have run.
    pub fn session_for(
        &self,
        ticker: &str,
        date: NaiveDate,
    ) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let schedule = self.meta(ticker)?.working_schedule_id?;
        let schedules = self.schedules.lock().ok()?;
        let events = schedules.get(&schedule)?;
        let open = events
            .iter()
            .find(|(t, k)| k == "OPEN" && t.date_naive() == date)?
            .0;
        let close = events.iter().find(|(t, k)| k == "CLOSE" && *t > open)?.0;
        Some((open, close))
    }

    fn order_endpoint(kind: &OrderKind) -> (&'static str, &'static str) {
        match kind {
            OrderKind::Market => ("/equity/orders/market", "order_market"),
            OrderKind::Limit { .. } => ("/equity/orders/limit", "order_limit"),
            OrderKind::Stop { .. } => ("/equity/orders/stop", "order_stop"),
            OrderKind::StopLimit { .. } => ("/equity/orders/stop_limit", "order_stop_limit"),
        }
    }

    /// Build the JSON body by hand so quantities/prices are sent as exact decimal literals
    /// (never through `f64`).
    #[allow(clippy::result_large_err)] // private; the Err is the final local outcome, built once
    fn order_body(
        &self,
        intent: &OrderIntent,
        instrument: &Instrument,
    ) -> Result<String, PlaceOutcome> {
        let ticker = instrument.id.symbol();
        let qty = intent.quantity.round_dp_with_strategy(
            instrument.quantity_decimals.min(self.cfg.quantity_decimals),
            rust_decimal::RoundingStrategy::ToZero,
        );
        if qty <= Decimal::ZERO {
            return Err(PlaceOutcome::Rejected {
                code: "local_precision".into(),
                message: "quantity rounds to zero".into(),
            });
        }
        let signed = match intent.side {
            Side::Buy => qty,
            Side::Sell => -qty,
        };
        let unit = self.unit(ticker);
        let px = |p: Decimal| unit.outbound(p).normalize().to_string();
        // Market orders carry no time validity; IOC is only refused for priced orders.
        let validity = match intent.time_in_force {
            TimeInForce::Gtc => "GOOD_TILL_CANCEL",
            TimeInForce::Day => "DAY",
            TimeInForce::Ioc if intent.kind == OrderKind::Market => "GOOD_TILL_CANCEL",
            TimeInForce::Ioc => {
                return Err(PlaceOutcome::Rejected {
                    code: "local_unsupported".into(),
                    message: "Trading 212 has no IOC time validity".into(),
                })
            }
        };
        let t = serde_json::to_string(ticker).unwrap_or_else(|_| "\"\"".into());
        let q = signed.normalize().to_string();
        Ok(match intent.kind {
            OrderKind::Market => {
                format!(r#"{{"ticker":{t},"quantity":{q},"extendedHours":false}}"#)
            }
            OrderKind::Limit { price } => {
                format!(
                    r#"{{"ticker":{t},"quantity":{q},"limitPrice":{},"timeValidity":"{validity}"}}"#,
                    px(price)
                )
            }
            OrderKind::Stop { stop } => {
                format!(
                    r#"{{"ticker":{t},"quantity":{q},"stopPrice":{},"timeValidity":"{validity}"}}"#,
                    px(stop)
                )
            }
            OrderKind::StopLimit { stop, limit } => format!(
                r#"{{"ticker":{t},"quantity":{q},"stopPrice":{},"limitPrice":{},"timeValidity":"{validity}"}}"#,
                px(stop),
                px(limit)
            ),
        })
    }

    /// Scope probe for `orders:execute` (NOT called automatically): cancels order id 0, which
    /// cannot exist. 403 = scope missing; 404 = scope present. UNVERIFIED with a live key:
    /// the 404-for-unknown-id behaviour is documented as a response code but not described.
    /// Requires live trading to be armed (DELETE is a state-changing endpoint).
    pub async fn probe_trade_scope(&self) -> BrokerResult<Option<bool>> {
        live::check_live_allowed(&self.cfg.base_url)?;
        let url = format!("{}/equity/orders/0", self.cfg.base_url);
        let raw = self
            .send(Method::DELETE, &url, None, "order_cancel")
            .await?;
        Ok(match raw.status {
            StatusCode::FORBIDDEN => Some(false),
            StatusCode::NOT_FOUND => Some(true),
            _ => None,
        })
    }
}

#[async_trait]
impl BrokerClient for Trading212Client {
    fn broker(&self) -> Broker {
        Broker::Trading212
    }

    async fn verify_credentials(&self) -> BrokerResult<CredentialReport> {
        let mut notes = Vec::new();
        let summary = match self
            .get_json("/equity/account/summary", "account_summary")
            .await
        {
            Ok(v) => Some(v),
            Err(BrokerError::Auth(m)) => {
                notes.push(format!("invalid key/secret: {m}"));
                return Ok(CredentialReport {
                    authenticated: false,
                    can_read_account: false,
                    can_trade: false,
                    can_withdraw: Some(false),
                    account_currency: None,
                    notes,
                });
            }
            Err(BrokerError::Forbidden(m)) => {
                notes.push(format!("scope account missing: {m}"));
                None
            }
            Err(e) => return Err(e),
        };
        let account_currency = summary
            .as_ref()
            .and_then(|s| s.get("currency").and_then(Value::as_str))
            .and_then(wire::currency);
        let mut scope_ok = |name: &str, r: BrokerResult<Value>| -> BrokerResult<bool> {
            match r {
                Ok(_) => Ok(true),
                Err(BrokerError::Forbidden(m)) => {
                    notes.push(format!("scope {name} missing: {m}"));
                    Ok(false)
                }
                Err(e) => Err(e),
            }
        };
        let portfolio = scope_ok(
            "portfolio",
            self.get_json("/equity/positions", "positions").await,
        )?;
        let orders_read = scope_ok(
            "orders:read",
            self.get_json("/equity/orders", "orders_list").await,
        )?;
        let hist_orders = scope_ok(
            "history:orders",
            self.get_json("/equity/history/orders?limit=1", "history_orders")
                .await,
        )?;
        let hist_tx = scope_ok(
            "history:transactions",
            self.get_json(
                "/equity/history/transactions?limit=1",
                "history_transactions",
            )
            .await,
        )?;
        let metadata = scope_ok(
            "metadata",
            self.get_json("/equity/metadata/exchanges", "exchanges")
                .await,
        )?;
        notes.push(
            "orders:execute cannot be verified without an order; confirmed by probe_trade_scope() after arming or by the first order"
                .into(),
        );
        notes.push("Trading 212 Public API v0 exposes no withdrawal endpoint".into());
        if !(orders_read && hist_orders && hist_tx && metadata) {
            notes.push("required scopes: account, portfolio, orders:read, orders:execute, history:orders, history:transactions, metadata".into());
        }
        Ok(CredentialReport {
            authenticated: true,
            can_read_account: summary.is_some() && portfolio,
            can_trade: false,
            can_withdraw: Some(false),
            account_currency,
            notes,
        })
    }

    /// STOCK and ETF instruments only (other types are not traded by this system).
    async fn instruments(&self) -> BrokerResult<Vec<Instrument>> {
        let v = self
            .get_json("/equity/metadata/instruments", "instruments")
            .await?;
        let arr = v
            .as_array()
            .ok_or_else(|| BrokerError::Decode("instruments: expected array".into()))?;
        let mut out = Vec::new();
        let mut meta = HashMap::new();
        let min_qty = Decimal::new(1, self.cfg.quantity_decimals);
        for it in arr {
            let (Some(ticker), Some(kind)) = (
                it.get("ticker").and_then(Value::as_str),
                it.get("type").and_then(Value::as_str),
            ) else {
                continue;
            };
            let asset_class = match kind {
                "STOCK" => AssetClass::Stock,
                "ETF" => AssetClass::Etf,
                _ => continue,
            };
            let code = it
                .get("currencyCode")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let (Some(quote_currency), Ok(id)) =
                (wire::currency(code), wire::instrument_id(ticker))
            else {
                tracing::debug!(
                    ticker,
                    code,
                    "skipping instrument with unsupported currency or ticker"
                );
                continue;
            };
            let first_seen = it
                .get("addedOn")
                .and_then(|v| wire::time(v).ok())
                .unwrap_or_else(Utc::now);
            meta.insert(
                ticker.to_owned(),
                T212InstrumentMeta {
                    ticker: ticker.to_owned(),
                    kind: kind.to_owned(),
                    currency_code: code.to_owned(),
                    working_schedule_id: it.get("workingScheduleId").and_then(Value::as_i64),
                    extended_hours: it
                        .get("extendedHours")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    max_open_quantity: opt_num(it.get("maxOpenQuantity")).ok().flatten(),
                },
            );
            out.push(Instrument {
                id,
                name: it
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(ticker)
                    .to_owned(),
                asset_class,
                quote_currency,
                isin: it.get("isin").and_then(Value::as_str).map(str::to_owned),
                min_notional: self.cfg.min_notional,
                min_quantity: min_qty,
                quantity_decimals: self.cfg.quantity_decimals,
                tick_size: Decimal::ZERO,
                tradable: true,
                first_seen,
            });
        }
        if let Ok(mut m) = self.meta.lock() {
            *m = meta;
        }
        Ok(out)
    }

    async fn account(&self) -> BrokerResult<AccountSnapshot> {
        let v = self
            .get_json("/equity/account/summary", "account_summary")
            .await?;
        let code = str_field(&v, "currency")?;
        let currency =
            wire::currency(code).ok_or_else(|| BrokerError::Decode(format!("currency {code}")))?;
        let cash = field(&v, "cash")?;
        Ok(AccountSnapshot {
            broker: Broker::Trading212,
            account_currency: currency,
            cash: vec![CashBalance {
                currency,
                available: num(field(cash, "availableToTrade")?)?,
                reserved: opt_num(cash.get("reservedForOrders"))?.unwrap_or(Decimal::ZERO),
            }],
            total_value: opt_num(v.get("totalValue"))?,
            as_of: Utc::now(),
        })
    }

    async fn positions(&self) -> BrokerResult<Vec<BrokerPosition>> {
        let v = self.get_json("/equity/positions", "positions").await?;
        let arr = v
            .as_array()
            .ok_or_else(|| BrokerError::Decode("positions: expected array".into()))?;
        let mut out = Vec::new();
        for p in arr {
            let ticker = str_field(field(p, "instrument")?, "ticker")?;
            let u = self.unit(ticker);
            let quantity = num(field(p, "quantity")?)?;
            out.push(BrokerPosition {
                instrument: wire::instrument_id(ticker)?,
                quantity,
                free_quantity: opt_num(p.get("quantityAvailableForTrading"))?.unwrap_or(quantity),
                avg_price: opt_num(p.get("averagePricePaid"))?.map(|x| u.inbound(x)),
                current_price: opt_num(p.get("currentPrice"))?.map(|x| u.inbound(x)),
            });
        }
        Ok(out)
    }

    async fn open_orders(&self) -> BrokerResult<Vec<BrokerOrder>> {
        let v = self.get_json("/equity/orders", "orders_list").await?;
        let arr = v
            .as_array()
            .ok_or_else(|| BrokerError::Decode("orders: expected array".into()))?;
        let units = self.unit_map();
        arr.iter()
            .map(|o| wire::parse_order(o, |t| units.get(t).copied().unwrap_or(PriceUnit::Major)))
            .collect()
    }

    async fn order_history(&self, since: DateTime<Utc>) -> BrokerResult<Vec<BrokerOrder>> {
        let items = self
            .paginate("/equity/history/orders?limit=50", "history_orders", |it| {
                it.get("order")
                    .and_then(|o| o.get("createdAt"))
                    .and_then(|t| wire::time(t).ok())
                    .is_some_and(|t| t < since)
            })
            .await?;
        let units = self.unit_map();
        let unit = |t: &str| units.get(t).copied().unwrap_or(PriceUnit::Major);
        let mut out = items
            .iter()
            .map(|h| wire::parse_historical(h, unit))
            .collect::<BrokerResult<Vec<_>>>()?;
        out.sort_by_key(|o| std::cmp::Reverse(o.created_at));
        Ok(out)
    }

    /// Trading 212 has no client order ids: lookup requires `broker_order_id`. A pending order
    /// is read from `GET /equity/orders/{id}`; otherwise recent history pages are searched.
    async fn get_order(&self, r: &OrderRef) -> BrokerResult<Option<BrokerOrder>> {
        let Some(id) = r.broker_order_id.as_deref() else {
            return Err(BrokerError::Unsupported(
                "Trading 212 has no client order id; reconcile via order_history (ticker, quantity, createdAt, initiatedFrom=API)",
            ));
        };
        if !id.chars().all(|c| c.is_ascii_digit()) {
            return Err(BrokerError::Decode(format!(
                "invalid Trading 212 order id {id}"
            )));
        }
        let url = format!("{}/equity/orders/{id}", self.cfg.base_url);
        let raw = self.send(Method::GET, &url, None, "order_get").await?;
        if raw.status.is_success() {
            let v: Value =
                serde_json::from_str(&raw.body).map_err(|e| BrokerError::Decode(e.to_string()))?;
            let unit = self.unit_map();
            return wire::parse_order(&v, |t| unit.get(t).copied().unwrap_or(PriceUnit::Major))
                .map(Some);
        }
        if raw.status != StatusCode::NOT_FOUND {
            return Err(Self::error_from(&raw));
        }
        let hist = self
            .order_history(Utc::now() - chrono::Duration::days(7))
            .await?;
        Ok(hist.into_iter().find(|o| o.broker_order_id == id))
    }

    async fn place_order(
        &self,
        intent: &OrderIntent,
        instrument: &Instrument,
    ) -> BrokerResult<PlaceOutcome> {
        live::check_live_allowed(&self.cfg.base_url)?;
        if instrument.id != intent.instrument || intent.instrument.broker() != Broker::Trading212 {
            return Ok(PlaceOutcome::Rejected {
                code: "local_instrument".into(),
                message: "instrument mismatch".into(),
            });
        }
        if intent.kind != OrderKind::Market && self.meta(instrument.id.symbol()).is_none() {
            // Price units (GBX) must be known before sending a price.
            return Ok(PlaceOutcome::Rejected {
                code: "local_metadata".into(),
                message: "instrument metadata not loaded; call instruments() first".into(),
            });
        }
        let body = match self.order_body(intent, instrument) {
            Ok(b) => b,
            Err(o) => return Ok(o),
        };
        let (path, key) = Self::order_endpoint(&intent.kind);
        let url = format!("{}{}", self.cfg.base_url, path);
        let raw = match self.send(Method::POST, &url, Some(body), key).await {
            Ok(r) => r,
            Err(BrokerError::Timeout) => {
                return Ok(PlaceOutcome::Unknown {
                    reason: "timeout".into(),
                })
            }
            Err(BrokerError::Network(m)) => {
                return Ok(PlaceOutcome::Unknown {
                    reason: format!("network: {m}"),
                })
            }
            Err(e) => return Err(e),
        };
        if raw.status.is_success() {
            let unit = self.unit(instrument.id.symbol());
            return Ok(
                match serde_json::from_str::<Value>(&raw.body)
                    .map_err(|e| e.to_string())
                    .and_then(|v| wire::parse_order(&v, |_| unit).map_err(|e| e.to_string()))
                {
                    Ok(order) => PlaceOutcome::Accepted { order },
                    Err(e) => PlaceOutcome::Unknown {
                        reason: format!("accepted but unparseable response: {e}"),
                    },
                },
            );
        }
        if raw.status == StatusCode::REQUEST_TIMEOUT || raw.status.is_server_error() {
            return Ok(PlaceOutcome::Unknown {
                reason: format!("http {}", raw.status.as_u16()),
            });
        }
        Ok(PlaceOutcome::Rejected {
            code: format!("http_{}", raw.status.as_u16()),
            message: raw.body.chars().take(300).collect(),
        })
    }

    async fn cancel_order(&self, r: &OrderRef) -> BrokerResult<()> {
        live::check_live_allowed(&self.cfg.base_url)?;
        let Some(id) = r.broker_order_id.as_deref() else {
            return Err(BrokerError::Unsupported(
                "Trading 212 cancel requires the broker order id",
            ));
        };
        if !id.chars().all(|c| c.is_ascii_digit()) {
            return Err(BrokerError::Decode(format!(
                "invalid Trading 212 order id {id}"
            )));
        }
        let url = format!("{}/equity/orders/{id}", self.cfg.base_url);
        let raw = self
            .send(Method::DELETE, &url, None, "order_cancel")
            .await?;
        if raw.status.is_success() {
            Ok(())
        } else {
            Err(Self::error_from(&raw))
        }
    }

    /// Deposits, withdrawals, fees and interest from `history/transactions`, dividends from
    /// `history/dividends`. TRANSFER is mapped by sign (in = deposit, out = withdrawal).
    async fn cash_movements(&self, since: DateTime<Utc>) -> BrokerResult<Vec<CashMovement>> {
        let before = |key: &'static str| {
            move |it: &Value| {
                it.get(key)
                    .and_then(|t| wire::time(t).ok())
                    .is_some_and(|t| t < since)
            }
        };
        let tx = self
            .paginate(
                "/equity/history/transactions?limit=50",
                "history_transactions",
                before("dateTime"),
            )
            .await?;
        let mut out = Vec::new();
        for t in tx {
            let amount = num(field(&t, "amount")?)?;
            let kind = match t.get("type").and_then(Value::as_str) {
                Some("DEPOSIT") => CashMovementKind::Deposit,
                Some("WITHDRAW") => CashMovementKind::Withdrawal,
                Some("FEE") => CashMovementKind::Fee,
                Some("INTEREST_ON_FREE_CASH" | "LENDING_INTEREST") => CashMovementKind::Interest,
                Some("TRANSFER") if amount.is_sign_negative() => CashMovementKind::Withdrawal,
                Some("TRANSFER") => CashMovementKind::Deposit,
                other => {
                    tracing::warn!(?other, "unknown Trading 212 transaction type");
                    continue;
                }
            };
            let code = str_field(&t, "currency")?;
            let currency = wire::currency(code)
                .ok_or_else(|| BrokerError::Decode(format!("currency {code}")))?;
            let ts = wire::time(field(&t, "dateTime")?)?;
            let reference = t
                .get("reference")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{}-{amount}", ts.timestamp()));
            out.push(CashMovement {
                broker_ref: reference,
                kind,
                amount: amount.abs(),
                currency,
                ts,
            });
        }
        let divs = self
            .paginate(
                "/equity/history/dividends?limit=50",
                "history_dividends",
                before("paidOn"),
            )
            .await?;
        for d in divs {
            let Some(currency) = d
                .get("currency")
                .and_then(Value::as_str)
                .and_then(wire::currency)
            else {
                continue;
            };
            let ts = wire::time(field(&d, "paidOn")?)?;
            let amount = num(field(&d, "amount")?)?;
            let reference = d
                .get("reference")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("div-{}", ts.timestamp()));
            out.push(CashMovement {
                broker_ref: reference,
                kind: CashMovementKind::Dividend,
                amount,
                currency,
                ts,
            });
        }
        out.sort_by_key(|m| m.ts);
        Ok(out)
    }

    fn health(&self) -> HealthCounters {
        let Ok(h) = self.health.lock() else {
            return HealthCounters::default();
        };
        HealthCounters {
            last_ok: h.last_ok,
            last_error: h.last_error,
            last_rate_limited: h.last_rate_limited,
            rate_budget_remaining: h.rate_budget_remaining,
            requests_5m: h.window.len() as u32,
            errors_5m: h.window.iter().filter(|(_, ok)| !ok).count() as u32,
            clock_skew_ms: 0,
        }
    }
}

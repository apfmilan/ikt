//! Bitvavo REST API v2 connector.
//!
//! Source of truth: https://docs.bitvavo.com (pages checked 2026-11-02: rest-api/introduction,
//! create-order, get-order, get-orders, get-open-orders, cancel-order, get-account-balance,
//! get-account-fees, get-markets, get-candlestick-data, get-ticker-book, get-ticker-data-24-h,
//! get-deposit-history, get-withdrawal-history, withdraw-assets, rate-limits, errors).
//!
//! Safety properties:
//! * `place_order` sends exactly one request and never retries. Uncertain results (timeout,
//!   transport error, 5xx, `errorCode` 101/107/108/109/111/400/419, unparseable 2xx) are
//!   returned as [`PlaceOutcome::Unknown`]; the engine reconciles them by `clientOrderId`.
//! * Order requests use a short `Bitvavo-Access-Window` (default 5000 ms): Bitvavo refuses a
//!   request that arrives after `timestamp + window` (errorCode 304), which bounds how late an
//!   uncertain order can still execute.
//! * Every state-changing request calls [`tc_broker::live::check_live_allowed`] first.
//! * The API secret lives in [`secrecy::SecretString`]; `Debug` output is redacted.

mod sign;
mod wire;

pub use sign::sign;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use reqwest::{Method, StatusCode, Url};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};
use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;
use tc_broker::ratelimit::{Limit, RateLimiter};
use tc_broker::{
    live, AccountSnapshot, BrokerClient, BrokerError, BrokerOrder, BrokerPosition, BrokerResult,
    CashBalance, CashMovement, CashMovementKind, CredentialReport, HealthCounters,
    MarketDataSource, OrderRef, PlaceOutcome, Quote,
};
use tc_core::{
    Broker, Candle, Currency, Instrument, InstrumentId, Interval, OrderIntent, OrderKind,
    TimeInForce,
};

use wire::{dec, int, opt_dec, ts_ms, ApiError};

pub const DEFAULT_BASE_URL: &str = "https://api.bitvavo.com/v2";
const WEIGHT_KEY: &str = "weight";

/// Connector configuration. Construct with [`BitvavoConfig::new`] for public-only access or
/// [`BitvavoConfig::with_credentials`] for trading.
#[derive(Clone)]
pub struct BitvavoConfig {
    pub base_url: String,
    pub api_key: Option<SecretString>,
    pub api_secret: Option<SecretString>,
    /// Required by `POST /order` and `DELETE /order` ("identifier for the trader or the bot").
    pub operator_id: i64,
    /// Access window for read requests (ms, docs: default 10000, max 60000).
    pub access_window_ms: u32,
    /// Access window for order placement/cancellation (ms). Short on purpose.
    pub order_access_window_ms: u32,
    pub timeout: Duration,
    /// Weight budget per minute used locally (docs: 1000; keep headroom).
    pub weight_per_minute: u32,
}

impl BitvavoConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        BitvavoConfig {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key: None,
            api_secret: None,
            operator_id: 1,
            access_window_ms: 10_000,
            order_access_window_ms: 5_000,
            timeout: Duration::from_secs(10),
            weight_per_minute: 900,
        }
    }

    pub fn with_credentials(
        base_url: impl Into<String>,
        api_key: SecretString,
        api_secret: SecretString,
        operator_id: i64,
    ) -> Self {
        BitvavoConfig {
            api_key: Some(api_key),
            api_secret: Some(api_secret),
            operator_id,
            ..BitvavoConfig::new(base_url)
        }
    }
}

impl Default for BitvavoConfig {
    fn default() -> Self {
        BitvavoConfig::new(DEFAULT_BASE_URL)
    }
}

impl fmt::Debug for BitvavoConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BitvavoConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field(
                "api_secret",
                &self.api_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("operator_id", &self.operator_id)
            .field("access_window_ms", &self.access_window_ms)
            .field("order_access_window_ms", &self.order_access_window_ms)
            .finish()
    }
}

#[derive(Default)]
struct HealthState {
    last_ok: Option<DateTime<Utc>>,
    last_error: Option<DateTime<Utc>>,
    last_rate_limited: Option<DateTime<Utc>>,
    rate_budget_remaining: Option<Decimal>,
    window: VecDeque<(DateTime<Utc>, bool)>,
    clock_skew_ms: i64,
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

struct Raw {
    status: StatusCode,
    body: String,
    reset_at: Option<DateTime<Utc>>,
}

/// Bitvavo connector (trading + public market data).
pub struct BitvavoClient {
    cfg: BitvavoConfig,
    http: reqwest::Client,
    limiter: RateLimiter,
    health: Mutex<HealthState>,
    tracked_markets: Mutex<BTreeSet<String>>,
}

impl fmt::Debug for BitvavoClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BitvavoClient")
            .field("cfg", &self.cfg)
            .finish_non_exhaustive()
    }
}

/// Result of [`BitvavoClient::probe_withdrawal_permission`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WithdrawProbe {
    /// errorCode 312: the key has no withdraw permission (the only passing result).
    NoWithdrawPermission,
    /// Bitvavo accepted the request: the key CAN withdraw. Refuse the key.
    WithdrawPossible,
    /// Any other answer: permission could not be determined.
    Inconclusive(String),
}

impl BitvavoClient {
    pub fn new(cfg: BitvavoConfig) -> BrokerResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent("trading-center/2")
            .build()
            .map_err(|e| BrokerError::Network(e.without_url().to_string()))?;
        let limiter = RateLimiter::new(&[(
            WEIGHT_KEY,
            Limit::per(cfg.weight_per_minute, Duration::from_secs(60)),
        )]);
        Ok(BitvavoClient {
            cfg,
            http,
            limiter,
            health: Mutex::new(HealthState::default()),
            tracked_markets: Mutex::new(BTreeSet::new()),
        })
    }

    pub fn config(&self) -> &BitvavoConfig {
        &self.cfg
    }

    /// Markets whose order history must be scanned (Bitvavo `GET /orders` requires `market`).
    /// The engine registers every market it has traded or holds intents for.
    pub fn track_market(&self, market: &str) {
        if let Ok(mut m) = self.tracked_markets.lock() {
            m.insert(market.to_owned());
        }
    }

    pub fn set_tracked_markets(&self, markets: impl IntoIterator<Item = String>) {
        if let Ok(mut m) = self.tracked_markets.lock() {
            m.clear();
            m.extend(markets);
        }
    }

    fn creds(&self) -> BrokerResult<(&SecretString, &SecretString)> {
        match (&self.cfg.api_key, &self.cfg.api_secret) {
            (Some(k), Some(s)) => Ok((k, s)),
            _ => Err(BrokerError::NoCredentials),
        }
    }

    fn url(&self, endpoint: &str, query: &[(&str, String)]) -> BrokerResult<Url> {
        let mut url = Url::parse(&format!("{}{}", self.cfg.base_url, endpoint))
            .map_err(|e| BrokerError::Decode(format!("bad url: {e}")))?;
        if !query.is_empty() {
            let mut q = url.query_pairs_mut();
            for (k, v) in query {
                q.append_pair(k, v);
            }
        }
        Ok(url)
    }

    /// Send one request. Returns `Ok` for every HTTP response; transport failures map to
    /// `Timeout` / `Network`. Never retries.
    #[allow(clippy::too_many_arguments)] // private plumbing; every argument is part of the signature
    async fn send(
        &self,
        method: Method,
        endpoint: &str,
        query: &[(&str, String)],
        body: Option<String>,
        auth: bool,
        weight: u32,
        window_ms: u32,
    ) -> BrokerResult<Raw> {
        self.limiter.acquire(WEIGHT_KEY, weight).await;
        let url = self.url(endpoint, query)?;
        let body = body.unwrap_or_default();
        let mut req = self.http.request(method.clone(), url.clone());
        if auth {
            let (key, secret) = self.creds()?;
            let ts = Utc::now().timestamp_millis();
            let path = match url.query() {
                Some(q) => format!("{}?{}", url.path(), q),
                None => url.path().to_owned(),
            };
            let sig = sign(secret.expose_secret(), ts, method.as_str(), &path, &body);
            req = req
                .header("Bitvavo-Access-Key", key.expose_secret())
                .header("Bitvavo-Access-Signature", sig)
                .header("Bitvavo-Access-Timestamp", ts.to_string())
                .header("Bitvavo-Access-Window", window_ms.to_string());
        }
        if !body.is_empty() {
            req = req.header("Content-Type", "application/json").body(body);
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
        let reset_at = self.read_rate_headers(resp.headers(), status);
        let text = match resp.text().await {
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
        if status != StatusCode::TOO_MANY_REQUESTS && text.contains("\"errorCode\":105") {
            self.limiter.block_for(WEIGHT_KEY, Duration::from_secs(61));
        }
        Ok(Raw {
            status,
            body: text,
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
    ) -> Option<DateTime<Utc>> {
        let get = |k: &str| {
            headers
                .get(k)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let remaining = get("bitvavo-ratelimit-remaining").and_then(|v| v.parse::<Decimal>().ok());
        let limit = get("bitvavo-ratelimit-limit").and_then(|v| v.parse::<Decimal>().ok());
        let reset_at = get("bitvavo-ratelimit-resetat").and_then(|v| v.parse::<i64>().ok());
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
            let wait = reset_at
                .map(|ms| ms - Utc::now().timestamp_millis())
                .filter(|ms| *ms > 0)
                .map(|ms| Duration::from_millis(ms as u64 + 1_000))
                .unwrap_or(Duration::from_secs(61));
            self.limiter.block_for(WEIGHT_KEY, wait);
        }
        reset_at.and_then(|ms| Utc.timestamp_millis_opt(ms).single())
    }

    /// Convert a non-success response into a typed error.
    fn error_from(raw: &Raw) -> BrokerError {
        let parsed: Option<ApiError> = serde_json::from_str(&raw.body).ok();
        let code = parsed.as_ref().and_then(|e| e.error_code);
        let msg = parsed
            .as_ref()
            .and_then(|e| e.error.clone())
            .unwrap_or_else(|| raw.body.chars().take(200).collect());
        if raw.status == StatusCode::TOO_MANY_REQUESTS || code == Some(105) {
            return BrokerError::RateLimited {
                retry_at: raw.reset_at,
            };
        }
        match code {
            Some(300..=309 | 313 | 317 | 318 | 322) => {
                return BrokerError::Auth(format!("{} {msg}", code.unwrap_or(0)))
            }
            Some(310 | 311 | 312 | 319 | 320 | 511..=514) => {
                return BrokerError::Forbidden(format!("{} {msg}", code.unwrap_or(0)))
            }
            _ => {}
        }
        match raw.status {
            StatusCode::UNAUTHORIZED => BrokerError::Auth(msg),
            StatusCode::FORBIDDEN => BrokerError::Forbidden(msg),
            s => BrokerError::Rejected {
                status: s.as_u16(),
                code: code.map(|c| c.to_string()).unwrap_or_else(|| "http".into()),
                message: msg,
            },
        }
    }

    async fn get_json(
        &self,
        endpoint: &str,
        query: &[(&str, String)],
        auth: bool,
        weight: u32,
    ) -> BrokerResult<Value> {
        let raw = self
            .send(
                Method::GET,
                endpoint,
                query,
                None,
                auth,
                weight,
                self.cfg.access_window_ms,
            )
            .await?;
        if !raw.status.is_success() {
            return Err(Self::error_from(&raw));
        }
        serde_json::from_str(&raw.body).map_err(|e| BrokerError::Decode(format!("{endpoint}: {e}")))
    }

    /// Measure clock skew against `GET /time` (ms, positive = local clock ahead).
    pub async fn sync_time(&self) -> BrokerResult<i64> {
        let before = Utc::now().timestamp_millis();
        let v = self.get_json("/time", &[], false, 1).await?;
        let after = Utc::now().timestamp_millis();
        let server = int(v.get("time").unwrap_or(&Value::Null))?;
        let skew = (before + after) / 2 - server;
        if let Ok(mut h) = self.health.lock() {
            h.clock_skew_ms = skew;
        }
        Ok(skew)
    }

    async fn balances(&self) -> BrokerResult<Vec<(String, Decimal, Decimal)>> {
        let v = self.get_json("/balance", &[], true, 5).await?;
        let arr = v
            .as_array()
            .ok_or_else(|| BrokerError::Decode("balance: expected array".into()))?;
        let mut out = Vec::new();
        for b in arr {
            let sym = b
                .get("symbol")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let available = dec(b.get("available").unwrap_or(&Value::Null))?;
            let in_order = opt_dec(b.get("inOrder"))?.unwrap_or(Decimal::ZERO);
            out.push((sym, available, in_order));
        }
        Ok(out)
    }

    /// Full candle history between `start` and `end`, paging backwards 1440 candles at a time
    /// (public endpoint, weight 1 per page). Oldest first, closed candles only.
    pub async fn fetch_history(
        &self,
        market: &str,
        interval: Interval,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> BrokerResult<Vec<Candle>> {
        let mut out: Vec<Candle> = Vec::new();
        let mut cursor: Option<i64> = None;
        let now = Utc::now().min(end);
        for _ in 0..1000 {
            let mut q = vec![
                ("interval", interval.as_str().to_owned()),
                ("limit", "1440".to_owned()),
            ];
            if let Some(c) = cursor {
                q.push(("end", c.to_string()));
            }
            let v = self
                .get_json(&format!("/{market}/candles"), &q, false, 1)
                .await?;
            let page = wire::parse_candles(&v, interval.seconds(), now)?;
            let Some(oldest) = page.first().map(|c| c.ts) else {
                break;
            };
            let reached_start = oldest <= start;
            out.extend(page.into_iter().filter(|c| c.ts >= start && c.ts < end));
            if reached_start || cursor == Some(oldest.timestamp_millis()) {
                break;
            }
            cursor = Some(oldest.timestamp_millis());
        }
        out.sort_by_key(|c| c.ts);
        out.dedup_by_key(|c| c.ts);
        Ok(out)
    }

    /// Withdrawal-permission probe (NOT called automatically).
    ///
    /// Sends `POST /withdrawal` with a fixed zero EUR amount and a fixed invalid address so it can
    /// never move funds. Only errorCode 312 ("API key does not have the Withdraw digital assets
    /// permission", docs.bitvavo.com/docs/errors) proves the key cannot withdraw.
    ///
    /// UNVERIFIED: whether Bitvavo checks permissions before validating amount/address has not
    /// been confirmed with a live key. Until confirmed, any non-312 answer is `Inconclusive`
    /// and onboarding falls back to user attestation. Requires live trading to be armed.
    /// All markets' 24 h tickers in one call (`GET /ticker/24h` without `market`, weight 25 per
    /// docs.bitvavo.com rate-limits). Returns (market, last, volumeQuote) for the universe scanner.
    pub async fn tickers_24h(&self) -> BrokerResult<Vec<(String, Option<Decimal>, Option<Decimal>)>> {
        let v = self.get_json("/ticker/24h", &[], false, 25).await?;
        let arr = v.as_array().ok_or_else(|| BrokerError::Decode("ticker/24h: expected array".into()))?;
        let mut out = Vec::with_capacity(arr.len());
        for t in arr {
            let Some(m) = t.get("market").and_then(|m| m.as_str()) else { continue };
            out.push((m.to_owned(), opt_dec(t.get("last")).ok().flatten(), opt_dec(t.get("volumeQuote")).ok().flatten()));
        }
        Ok(out)
    }

    pub async fn probe_withdrawal_permission(&self) -> BrokerResult<WithdrawProbe> {
        live::check_live_allowed(&self.cfg.base_url)?;
        let body = json!({ "symbol": "EUR", "amount": "0", "address": "INVALID-PERMISSION-PROBE" })
            .to_string();
        let raw = self
            .send(
                Method::POST,
                "/withdrawal",
                &[],
                Some(body),
                true,
                1,
                self.cfg.order_access_window_ms,
            )
            .await?;
        if raw.status.is_success() {
            tracing::error!("bitvavo withdrawal probe was ACCEPTED: key has withdraw permission");
            return Ok(WithdrawProbe::WithdrawPossible);
        }
        let parsed: Option<ApiError> = serde_json::from_str(&raw.body).ok();
        match parsed.and_then(|e| e.error_code) {
            Some(312) => Ok(WithdrawProbe::NoWithdrawPermission),
            Some(c) => Ok(WithdrawProbe::Inconclusive(format!("errorCode {c}"))),
            None => Ok(WithdrawProbe::Inconclusive(format!(
                "http {}",
                raw.status.as_u16()
            ))),
        }
    }

    #[allow(clippy::result_large_err)] // private; the Err is the final local outcome, built once
    fn order_body(
        &self,
        intent: &OrderIntent,
        instrument: &Instrument,
    ) -> Result<Value, PlaceOutcome> {
        let qty = instrument.floor_quantity(intent.quantity);
        if qty <= Decimal::ZERO {
            return Err(PlaceOutcome::Rejected {
                code: "local_precision".into(),
                message: format!(
                    "quantity {} rounds to zero at {} decimals",
                    intent.quantity, instrument.quantity_decimals
                ),
            });
        }
        let side = intent.side.as_str();
        let px = |p: Decimal| {
            instrument
                .round_price(p, intent.side)
                .normalize()
                .to_string()
        };
        let tif = match intent.time_in_force {
            TimeInForce::Gtc => "GTC",
            TimeInForce::Ioc => "IOC",
            TimeInForce::Day => {
                return Err(PlaceOutcome::Rejected {
                    code: "local_unsupported".into(),
                    message: "Bitvavo has no DAY time-in-force".into(),
                })
            }
        };
        let mut body = json!({
            "market": instrument.id.symbol(),
            "side": side,
            "operatorId": self.cfg.operator_id,
            "clientOrderId": intent.id.to_string(),
            "amount": qty.normalize().to_string(),
            "responseRequired": true,
        });
        let obj = body.as_object_mut().ok_or_else(|| PlaceOutcome::Rejected {
            code: "local".into(),
            message: "body".into(),
        })?;
        match intent.kind {
            OrderKind::Market => {
                obj.insert("orderType".into(), json!("market"));
            }
            OrderKind::Limit { price } => {
                obj.insert("orderType".into(), json!("limit"));
                obj.insert("price".into(), json!(px(price)));
                obj.insert("timeInForce".into(), json!(tif));
            }
            OrderKind::Stop { stop } => {
                obj.insert("orderType".into(), json!("stopLoss"));
                obj.insert("triggerAmount".into(), json!(px(stop)));
                obj.insert("triggerType".into(), json!("price"));
                obj.insert("triggerReference".into(), json!("lastTrade"));
            }
            OrderKind::StopLimit { stop, limit } => {
                obj.insert("orderType".into(), json!("stopLossLimit"));
                obj.insert("price".into(), json!(px(limit)));
                obj.insert("triggerAmount".into(), json!(px(stop)));
                obj.insert("triggerType".into(), json!("price"));
                obj.insert("triggerReference".into(), json!("lastTrade"));
                obj.insert("timeInForce".into(), json!(tif));
            }
        }
        Ok(body)
    }
}

/// errorCodes for which "the operation may or may not have succeeded" (docs: errors page).
fn uncertain_code(code: i64) -> bool {
    matches!(code, 101 | 107 | 108 | 109 | 111 | 400 | 419)
}

fn ms(t: DateTime<Utc>) -> String {
    t.timestamp_millis().to_string()
}

#[async_trait]
impl BrokerClient for BitvavoClient {
    fn broker(&self) -> Broker {
        Broker::Bitvavo
    }

    async fn verify_credentials(&self) -> BrokerResult<CredentialReport> {
        let mut notes = Vec::new();
        match self.sync_time().await {
            Ok(skew) => notes.push(format!("clock skew {skew} ms")),
            Err(e) => notes.push(format!("time sync failed: {e}")),
        }
        let authenticated = match self.get_json("/account", &[], true, 1).await {
            Ok(_) => true,
            Err(BrokerError::Auth(m)) | Err(BrokerError::Forbidden(m)) => {
                notes.push(format!("GET /account refused: {m}"));
                false
            }
            Err(e) => return Err(e),
        };
        let can_read_account = authenticated
            && match self.balances().await {
                Ok(_) => true,
                Err(BrokerError::Auth(m)) | Err(BrokerError::Forbidden(m)) => {
                    notes.push(format!("GET /balance refused: {m}"));
                    false
                }
                Err(e) => return Err(e),
            };
        // Docs (introduction → Permissions): retrieving order information requires both the
        // Read-only and the Trade permission, so a successful GET /ordersOpen proves trading.
        let can_trade = authenticated
            && match self
                .get_json("/ordersOpen", &[("market", "BTC-EUR".to_owned())], true, 5)
                .await
            {
                Ok(_) => true,
                Err(BrokerError::Auth(m)) | Err(BrokerError::Forbidden(m)) => {
                    notes.push(format!("trade permission missing: {m}"));
                    false
                }
                Err(e) => return Err(e),
            };
        notes.push(
            "withdraw permission not determined automatically; run the withdrawal probe after arming or attest in onboarding"
                .into(),
        );
        Ok(CredentialReport {
            authenticated,
            can_read_account,
            can_trade,
            can_withdraw: None,
            account_currency: Some(Currency::EUR),
            notes,
        })
    }

    async fn instruments(&self) -> BrokerResult<Vec<Instrument>> {
        let v = self.get_json("/markets", &[], false, 1).await?;
        let arr = v
            .as_array()
            .ok_or_else(|| BrokerError::Decode("markets: expected array".into()))?;
        let now = Utc::now();
        let mut out = Vec::new();
        for m in arr {
            match wire::market_to_instrument(m, now) {
                Ok(Some(i)) => out.push(i),
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e, "skipping unparseable bitvavo market"),
            }
        }
        Ok(out)
    }

    async fn account(&self) -> BrokerResult<AccountSnapshot> {
        let balances = self.balances().await?;
        let cash = balances
            .iter()
            .filter(|(s, _, _)| s == "EUR")
            .map(|(_, a, r)| CashBalance {
                currency: Currency::EUR,
                available: *a,
                reserved: *r,
            })
            .collect::<Vec<_>>();
        let cash = if cash.is_empty() {
            vec![CashBalance {
                currency: Currency::EUR,
                available: Decimal::ZERO,
                reserved: Decimal::ZERO,
            }]
        } else {
            cash
        };
        Ok(AccountSnapshot {
            broker: Broker::Bitvavo,
            account_currency: Currency::EUR,
            cash,
            total_value: None,
            as_of: Utc::now(),
        })
    }

    /// Bitvavo has no positions concept: every non-EUR balance above zero is a spot position in
    /// `<SYMBOL>-EUR`. `avg_price` is not reported by Bitvavo (the engine keeps its own cost basis).
    async fn positions(&self) -> BrokerResult<Vec<BrokerPosition>> {
        let mut out = Vec::new();
        for (sym, available, in_order) in self.balances().await? {
            let total = available + in_order;
            if sym == "EUR" || total <= Decimal::ZERO {
                continue;
            }
            let instrument = match wire::instrument_for(&format!("{sym}-EUR")) {
                Ok(i) => i,
                Err(_) => continue,
            };
            out.push(BrokerPosition {
                instrument,
                quantity: total,
                free_quantity: available,
                avg_price: None,
                current_price: None,
            });
        }
        Ok(out)
    }

    async fn open_orders(&self) -> BrokerResult<Vec<BrokerOrder>> {
        let v = self.get_json("/ordersOpen", &[], true, 100).await?;
        let arr = v
            .as_array()
            .ok_or_else(|| BrokerError::Decode("ordersOpen: expected array".into()))?;
        arr.iter().map(wire::parse_order).collect()
    }

    /// `GET /orders` requires `market`, so this scans tracked markets plus markets of every
    /// non-EUR balance (weight 5 per market).
    async fn order_history(&self, since: DateTime<Utc>) -> BrokerResult<Vec<BrokerOrder>> {
        let mut markets: BTreeSet<String> = self
            .tracked_markets
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();
        for (sym, a, r) in self.balances().await? {
            if sym != "EUR" && a + r > Decimal::ZERO {
                markets.insert(format!("{sym}-EUR"));
            }
        }
        let mut out = Vec::new();
        for m in markets {
            let q = [
                ("market", m.clone()),
                ("start", ms(since)),
                ("limit", "1000".to_owned()),
            ];
            let v = self.get_json("/orders", &q, true, 5).await?;
            let arr = v
                .as_array()
                .ok_or_else(|| BrokerError::Decode("orders: expected array".into()))?;
            for o in arr {
                out.push(wire::parse_order(o)?);
            }
        }
        out.sort_by_key(|o| std::cmp::Reverse(o.created_at));
        Ok(out)
    }

    async fn get_order(&self, r: &OrderRef) -> BrokerResult<Option<BrokerOrder>> {
        let mut q = vec![("market", r.instrument.symbol().to_owned())];
        match (&r.client_order_id, &r.broker_order_id) {
            (Some(c), _) => q.push(("clientOrderId", c.to_string())),
            (None, Some(id)) => q.push(("orderId", id.clone())),
            (None, None) => return Err(BrokerError::Decode("order ref without ids".into())),
        }
        let raw = self
            .send(
                Method::GET,
                "/order",
                &q,
                None,
                true,
                1,
                self.cfg.access_window_ms,
            )
            .await?;
        if raw.status.is_success() {
            let v: Value =
                serde_json::from_str(&raw.body).map_err(|e| BrokerError::Decode(e.to_string()))?;
            return wire::parse_order(&v).map(Some);
        }
        let err = Self::error_from(&raw);
        match &err {
            BrokerError::Rejected { code, .. } if code == "240" => Ok(None),
            BrokerError::Rejected { status: 404, .. } => Ok(None),
            _ => Err(err),
        }
    }

    async fn place_order(
        &self,
        intent: &OrderIntent,
        instrument: &Instrument,
    ) -> BrokerResult<PlaceOutcome> {
        live::check_live_allowed(&self.cfg.base_url)?;
        self.creds()?;
        if instrument.id != intent.instrument || intent.instrument.broker() != Broker::Bitvavo {
            return Ok(PlaceOutcome::Rejected {
                code: "local_instrument".into(),
                message: "instrument mismatch".into(),
            });
        }
        let body = match self.order_body(intent, instrument) {
            Ok(b) => b.to_string(),
            Err(outcome) => return Ok(outcome),
        };
        self.track_market(instrument.id.symbol());
        let raw = match self
            .send(
                Method::POST,
                "/order",
                &[],
                Some(body),
                true,
                1,
                self.cfg.order_access_window_ms,
            )
            .await
        {
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
            return Ok(
                match serde_json::from_str::<Value>(&raw.body)
                    .map_err(|e| e.to_string())
                    .and_then(|v| wire::parse_order(&v).map_err(|e| e.to_string()))
                {
                    Ok(order) => PlaceOutcome::Accepted { order },
                    Err(e) => PlaceOutcome::Unknown {
                        reason: format!("accepted but unparseable response: {e}"),
                    },
                },
            );
        }
        let parsed: Option<ApiError> = serde_json::from_str(&raw.body).ok();
        let code = parsed.as_ref().and_then(|e| e.error_code);
        let message = parsed.and_then(|e| e.error).unwrap_or_default();
        if raw.status.is_server_error() || code.is_some_and(uncertain_code) {
            return Ok(PlaceOutcome::Unknown {
                reason: format!(
                    "http {} errorCode {:?}: {message}",
                    raw.status.as_u16(),
                    code
                ),
            });
        }
        Ok(PlaceOutcome::Rejected {
            code: code
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("http_{}", raw.status.as_u16())),
            message,
        })
    }

    async fn cancel_order(&self, r: &OrderRef) -> BrokerResult<()> {
        live::check_live_allowed(&self.cfg.base_url)?;
        let mut q = vec![("market", r.instrument.symbol().to_owned())];
        if let Some(id) = &r.broker_order_id {
            q.push(("orderId", id.clone()));
        }
        if let Some(c) = &r.client_order_id {
            q.push(("clientOrderId", c.to_string()));
        }
        if q.len() == 1 {
            return Err(BrokerError::Decode("order ref without ids".into()));
        }
        q.push(("operatorId", self.cfg.operator_id.to_string()));
        let raw = self
            .send(
                Method::DELETE,
                "/order",
                &q,
                None,
                true,
                1,
                self.cfg.order_access_window_ms,
            )
            .await?;
        if raw.status.is_success() {
            Ok(())
        } else {
            Err(Self::error_from(&raw))
        }
    }

    /// Deposits/withdrawals from `GET /depositHistory` and `GET /withdrawalHistory`. Movements in
    /// assets whose symbol is not a 3-letter code (cannot be a `Currency`) are logged and skipped.
    async fn cash_movements(&self, since: DateTime<Utc>) -> BrokerResult<Vec<CashMovement>> {
        let mut out = Vec::new();
        let q = [("start", ms(since)), ("limit", "1000".to_owned())];
        for (endpoint, kind) in [
            ("/depositHistory", CashMovementKind::Deposit),
            ("/withdrawalHistory", CashMovementKind::Withdrawal),
        ] {
            let v = self.get_json(endpoint, &q, true, 5).await?;
            let arr = v
                .as_array()
                .ok_or_else(|| BrokerError::Decode(format!("{endpoint}: expected array")))?;
            for m in arr {
                let status = m.get("status").and_then(Value::as_str);
                let counts = match kind {
                    CashMovementKind::Deposit => status.is_none_or(|s| s == "completed"),
                    _ => status.is_some_and(|s| {
                        matches!(
                            s,
                            "approved" | "sending" | "in_mempool" | "processed" | "completed"
                        )
                    }),
                };
                if !counts {
                    continue;
                }
                let sym = m.get("symbol").and_then(Value::as_str).unwrap_or_default();
                let Ok(currency) = Currency::new(sym) else {
                    tracing::warn!(symbol = sym, "skipping cash movement in non-ISO asset");
                    continue;
                };
                let ts = ts_ms(m.get("timestamp").unwrap_or(&Value::Null))?;
                let amount = dec(m.get("amount").unwrap_or(&Value::Null))?;
                let reference = m
                    .get("txId")
                    .or_else(|| m.get("paymentId"))
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_owned())
                    .unwrap_or_else(|| format!("{}-{sym}-{amount}", ts.timestamp_millis()));
                out.push(CashMovement {
                    broker_ref: reference.clone(),
                    kind,
                    amount: amount.abs(),
                    currency,
                    ts,
                });
                if let Some(fee) = opt_dec(m.get("fee"))?.filter(|f| *f > Decimal::ZERO) {
                    out.push(CashMovement {
                        broker_ref: format!("{reference}:fee"),
                        kind: CashMovementKind::Fee,
                        amount: fee,
                        currency,
                        ts,
                    });
                }
            }
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
            clock_skew_ms: h.clock_skew_ms,
        }
    }
}

#[async_trait]
impl MarketDataSource for BitvavoClient {
    fn name(&self) -> &'static str {
        "bitvavo"
    }

    async fn candles(
        &self,
        instrument: &InstrumentId,
        interval: Interval,
        limit: usize,
        end: Option<DateTime<Utc>>,
    ) -> BrokerResult<Vec<Candle>> {
        if instrument.broker() != Broker::Bitvavo {
            return Err(BrokerError::Unsupported(
                "bitvavo market data covers bitvavo instruments only",
            ));
        }
        let limit = limit.clamp(1, 1439);
        // Ask for one extra bar because the newest one may still be open.
        let mut q = vec![
            ("interval", interval.as_str().to_owned()),
            ("limit", (limit + 1).to_string()),
        ];
        let now = Utc::now();
        let cutoff = match end {
            Some(e) => {
                // `end` must align with an interval boundary (docs); align down.
                let step = interval.seconds() * 1000;
                let aligned = e.timestamp_millis() / step * step;
                q.push(("end", aligned.to_string()));
                Utc.timestamp_millis_opt(aligned)
                    .single()
                    .unwrap_or(now)
                    .min(now)
            }
            None => now,
        };
        let v = self
            .get_json(&format!("/{}/candles", instrument.symbol()), &q, false, 1)
            .await?;
        let mut c = wire::parse_candles(&v, interval.seconds(), cutoff)?;
        if c.len() > limit {
            c.drain(..c.len() - limit);
        }
        Ok(c)
    }

    async fn quote(&self, instrument: &InstrumentId) -> BrokerResult<Quote> {
        let market = instrument.symbol().to_owned();
        let book = self
            .get_json("/ticker/book", &[("market", market.clone())], false, 1)
            .await?;
        let day = self
            .get_json("/ticker/24h", &[("market", market)], false, 1)
            .await?;
        let ts = match day.get("timestamp") {
            Some(t) => ts_ms(t)?,
            None => Utc::now(),
        };
        Ok(Quote {
            instrument: instrument.clone(),
            bid: opt_dec(book.get("bid"))?,
            ask: opt_dec(book.get("ask"))?,
            last: opt_dec(day.get("last"))?,
            volume_quote_24h: opt_dec(day.get("volumeQuote"))?,
            ts,
        })
    }
}

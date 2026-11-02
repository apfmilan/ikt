//! ECB fixture parsing and EODHD adapter tests against wiremock. No real hosts are contacted.
#![allow(clippy::unwrap_used)]

use chrono::{Duration as ChronoDuration, NaiveDate, Utc};
use rust_decimal_macros::dec;
use secrecy::SecretString;
use serde_json::json;
use tc_broker::{BrokerError, MarketDataSource};
use tc_core::{Broker, Currency, InstrumentId, Interval};
use tc_marketdata::{parse_ecb_xml, EcbFx, EodhdClient, EodhdConfig};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE: &str = include_str!("fixtures/ecb-daily.xml");
const TOKEN: &str = "tok_SUPERSECRET_123";

#[test]
fn ecb_live_fixture_parses_with_huf_cross_rates() {
    let t = parse_ecb_xml(FIXTURE).unwrap();
    assert_eq!(t.as_of, NaiveDate::from_ymd_opt(2026, 10, 30).unwrap());
    assert_eq!(t.per_eur(Currency::HUF), Some(dec!(366.33)));
    assert_eq!(t.per_eur(Currency::EUR), Some(dec!(1)));
    // USD/HUF = 366.33 / 1.1172 ≈ 327.90 (research doc, 2026-10-30).
    assert_eq!(t.huf_per(Currency::USD).unwrap().round_dp(2), dec!(327.90));
    assert_eq!(t.to_huf(dec!(10), Currency::EUR), Some(dec!(3663.30)));
    assert_eq!(t.huf_per(Currency::GBP).unwrap().round_dp(2), dec!(439.35));
    assert_eq!(t.rate(Currency::HUF, Currency::HUF), Some(dec!(1)));
    assert!(t.per_eur.len() > 25);
    assert_eq!(t.age_days(NaiveDate::from_ymd_opt(2026, 11, 2).unwrap()), 3);
}

#[tokio::test]
async fn ecb_fetch_and_error_status() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/eurofxref-daily.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(FIXTURE))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/down.xml"))
        .respond_with(ResponseTemplate::new(503).set_body_string("maintenance"))
        .mount(&s)
        .await;
    let t = EcbFx::new(format!("{}/eurofxref-daily.xml", s.uri()))
        .unwrap()
        .fetch()
        .await
        .unwrap();
    assert_eq!(t.source, "ecb");
    let e = EcbFx::new(format!("{}/down.xml", s.uri()))
        .unwrap()
        .fetch()
        .await
        .unwrap_err();
    assert!(matches!(e, BrokerError::Rejected { status: 503, .. }));
}

fn eod_client(base: &str) -> EodhdClient {
    let mut cfg = EodhdConfig::new(base, SecretString::from(TOKEN));
    cfg.requests_per_minute = 1000;
    EodhdClient::new(cfg).unwrap()
}

fn aapl() -> InstrumentId {
    InstrumentId::new(Broker::Trading212, "AAPL_US_EQ")
}

#[tokio::test]
async fn eod_daily_candles_adjusted_oldest_first_today_dropped() {
    let s = MockServer::start().await;
    let today = Utc::now().date_naive();
    let d = |k: i64| (today - ChronoDuration::days(k)).to_string();
    Mock::given(method("GET"))
        .and(path("/eod/AAPL.US"))
        .and(query_param("api_token", TOKEN))
        .and(query_param("fmt", "json"))
        .and(query_param("period", "d"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"date": d(2), "open": 346.92, "high": 347.18, "low": 343.52, "close": 343.83, "adjusted_close": 343.83, "volume": 35714331},
            {"date": d(3), "open": 200.0, "high": 210.0, "low": 190.0, "close": 200.0, "adjusted_close": 100.0, "volume": 10},
            {"date": d(1), "open": 348, "high": 351.43, "low": 347.3, "close": 348.91, "adjusted_close": 348.91, "volume": 42582126},
            {"date": d(0), "open": 346.715, "high": 349.19, "low": 334.35, "close": 344.63, "adjusted_close": 344.63, "volume": 1}
        ])))
        .mount(&s)
        .await;
    let c = eod_client(&s.uri());
    // Unmapped instruments fail closed.
    assert!(matches!(
        c.candles(&aapl(), Interval::D1, 10, None)
            .await
            .unwrap_err(),
        BrokerError::Unsupported(_)
    ));
    c.set_symbol(aapl(), "AAPL.US").unwrap();
    assert!(matches!(
        c.candles(&aapl(), Interval::H1, 10, None)
            .await
            .unwrap_err(),
        BrokerError::Unsupported(_)
    ));
    let bars = c.candles(&aapl(), Interval::D1, 10, None).await.unwrap();
    assert_eq!(bars.len(), 3, "today's bar dropped");
    assert!(bars.windows(2).all(|w| w[0].ts < w[1].ts));
    // Split-adjusted bar: factor 0.5.
    assert_eq!(bars[0].open, dec!(100));
    assert_eq!(bars[0].high, dec!(105));
    assert_eq!(bars[0].close, dec!(100));
    assert_eq!(bars[2].close, dec!(348.91));
    let limited = c.candles(&aapl(), Interval::D1, 2, None).await.unwrap();
    assert_eq!(limited.len(), 2);
    assert_eq!(limited[1].close, dec!(348.91));
}

#[tokio::test]
async fn eod_quote_is_delayed_last_price_without_bid_ask() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/real-time/AAPL.US"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"code": "AAPL.US", "timestamp": 1793377800i64,
            "gmtoffset": 0, "open": 346.715, "high": 349.19, "low": 334.35, "close": 344.63, "volume": 67841555,
            "previousClose": 348.91, "change": -4.28, "change_p": "NA"})))
        .mount(&s)
        .await;
    let c = eod_client(&s.uri());
    c.set_symbol(aapl(), "AAPL.US").unwrap();
    let q = c.quote(&aapl()).await.unwrap();
    assert_eq!(q.last, Some(dec!(344.63)));
    assert_eq!(q.bid, None);
    assert_eq!(q.ts.timestamp(), 1793377800);
}

#[tokio::test]
async fn token_never_appears_in_errors() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/eod/MISSING.US"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string(format!("Ticker not found for api_token={TOKEN}")),
        )
        .mount(&s)
        .await;
    let c = eod_client(&s.uri());
    let today = Utc::now().date_naive();
    let e = c
        .eod("MISSING.US", today - ChronoDuration::days(5), today)
        .await
        .unwrap_err();
    assert!(!format!("{e} {e:?}").contains(TOKEN), "{e:?}");
    // Transport failure (closed port): reqwest errors include the URL unless stripped.
    let dead = eod_client("http://127.0.0.1:9");
    let e = dead
        .eod("AAPL.US", today - ChronoDuration::days(5), today)
        .await
        .unwrap_err();
    assert!(!format!("{e} {e:?}").contains(TOKEN), "{e:?}");
    assert!(!format!("{:?}", EodhdConfig::new("x", SecretString::from(TOKEN))).contains(TOKEN));
}

#[tokio::test]
async fn search_and_mapping_by_isin() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search/IE00B3XXRP09"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"Code":"VUSA","Exchange":"LSE","Name":"Vanguard S&P 500 UCITS ETF","Type":"ETF","Country":"UK","Currency":"GBP","ISIN":"IE00B3XXRP09","isPrimary":true},
            {"Code":"VUSA","Exchange":"XETRA","Name":"Vanguard S&P 500 UCITS ETF","Type":"ETF","Country":"Germany","Currency":"EUR","ISIN":"IE00B3XXRP09","isPrimary":false}
        ])))
        .mount(&s)
        .await;
    let c = eod_client(&s.uri());
    let ls = c.search("IE00B3XXRP09").await.unwrap();
    assert_eq!(ls.len(), 2);
    let pick = tc_marketdata::select_listing(&ls, "XETRA", "EUR").unwrap();
    assert_eq!(pick.symbol(), "VUSA.XETRA");
    assert!(c.search("../x").await.is_err());
    assert!(c.set_symbol(aapl(), "bad symbol").is_err());
}

#[tokio::test]
async fn rate_limit_and_auth_errors_are_typed() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/eod/A.US"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/eod/B.US"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&s)
        .await;
    let c = eod_client(&s.uri());
    let today = Utc::now().date_naive();
    assert!(matches!(
        c.eod("A.US", today, today).await.unwrap_err(),
        BrokerError::RateLimited { .. }
    ));
    let c = eod_client(&s.uri());
    assert!(matches!(
        c.eod("B.US", today, today).await.unwrap_err(),
        BrokerError::Auth(_)
    ));
}

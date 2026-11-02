//! Integration tests against a local wiremock server. These never reach api.bitvavo.com.
#![allow(clippy::unwrap_used)]

use chrono::{Duration as ChronoDuration, Utc};
use rust_decimal_macros::dec;
use secrecy::SecretString;
use serde_json::{json, Value};
use std::time::Duration;
use tc_broker::{BrokerClient, BrokerError, MarketDataSource, OrderRef, PlaceOutcome};
use tc_broker_bitvavo::{sign, BitvavoClient, BitvavoConfig};
use tc_core::{
    AssetClass, Broker, Currency, Instrument, InstrumentId, IntentReason, Interval, OrderIntent,
    OrderKind, Side, TimeInForce,
};
use uuid::Uuid;
use wiremock::matchers::{header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const SECRET: &str = "test-secret";

fn client(base: &str) -> BitvavoClient {
    let mut cfg = BitvavoConfig::with_credentials(
        format!("{base}/v2"),
        SecretString::from("test-key"),
        SecretString::from(SECRET),
        543462,
    );
    cfg.timeout = Duration::from_millis(400);
    BitvavoClient::new(cfg).unwrap()
}

fn btc() -> Instrument {
    Instrument {
        id: InstrumentId::new(Broker::Bitvavo, "BTC-EUR"),
        name: "BTC".into(),
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

fn intent(kind: OrderKind, side: Side) -> OrderIntent {
    OrderIntent {
        id: Uuid::new_v4(),
        instrument: btc().id,
        side,
        kind,
        quantity: dec!(0.000123456789),
        time_in_force: TimeInForce::Gtc,
        reason: IntentReason::Entry,
        decision_id: None,
        reference_price: dec!(70000),
        created_at: Utc::now(),
    }
}

fn order_json(client_id: Uuid, status: &str) -> Value {
    json!({"orderId":"1be6d0df-d5dc-4b53-a250-3376f3b393e6","clientOrderId":client_id.to_string(),"market":"BTC-EUR",
        "created":1706100650751i64,"updated":1706100650751i64,"status":status,"side":"buy","orderType":"market",
        "amount":"0.00012345","amountRemaining":"0","filledAmount":"0.00012345","filledAmountQuote":"8.6415",
        "feePaid":"0.0216","feeCurrency":"EUR","fills":[{"id":"371c6bd3-d06d-4573-9f15-18697cd210e5",
        "timestamp":1706100650751i64,"amount":"0.00012345","price":"7E+4","taker":true,"fee":"0.0216",
        "feeCurrency":"EUR","settled":true}],"operatorId":543462})
}

/// Recompute the signature from what the server received.
fn assert_signed(req: &Request, expected_window: &str) {
    let ts: i64 = req
        .headers
        .get("Bitvavo-Access-Timestamp")
        .unwrap()
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    let sig = req
        .headers
        .get("Bitvavo-Access-Signature")
        .unwrap()
        .to_str()
        .unwrap();
    let window = req
        .headers
        .get("Bitvavo-Access-Window")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(window, expected_window);
    let path = match req.url.query() {
        Some(q) => format!("{}?{}", req.url.path(), q),
        None => req.url.path().to_owned(),
    };
    let body = String::from_utf8(req.body.clone()).unwrap();
    assert_eq!(sig, sign(SECRET, ts, req.method.as_str(), &path, &body));
    assert_eq!(req.headers.get("Bitvavo-Access-Key").unwrap(), "test-key");
}

#[tokio::test]
async fn market_order_is_signed_and_mapped() {
    let server = MockServer::start().await;
    let i = intent(OrderKind::Market, Side::Buy);
    Mock::given(method("POST"))
        .and(path("/v2/order"))
        .and(header_exists("Bitvavo-Access-Signature"))
        .respond_with(ResponseTemplate::new(200).set_body_json(order_json(i.id, "filled")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server.uri());
    let out = c.place_order(&i, &btc()).await.unwrap();
    let PlaceOutcome::Accepted { order } = out else {
        panic!("expected accepted, got {out:?}")
    };
    assert_eq!(order.client_order_id, Some(i.id));
    assert_eq!(order.fills[0].price, dec!(70000));
    let reqs = server.received_requests().await.unwrap();
    assert_signed(&reqs[0], "5000");
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["market"], "BTC-EUR");
    assert_eq!(body["side"], "buy");
    assert_eq!(body["orderType"], "market");
    assert_eq!(body["operatorId"], 543462);
    assert_eq!(body["clientOrderId"], i.id.to_string());
    assert_eq!(body["amount"], "0.00012345", "floored to 8 decimals");
    assert!(body.get("price").is_none());
}

#[tokio::test]
async fn limit_and_stop_orders_map_to_bitvavo_types() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/order"))
        .respond_with(ResponseTemplate::new(200).set_body_json(order_json(Uuid::new_v4(), "new")))
        .mount(&server)
        .await;
    let c = client(&server.uri());
    c.place_order(
        &intent(
            OrderKind::Limit {
                price: dec!(70000.7),
            },
            Side::Buy,
        ),
        &btc(),
    )
    .await
    .unwrap();
    c.place_order(
        &intent(
            OrderKind::Stop {
                stop: dec!(67900.2),
            },
            Side::Sell,
        ),
        &btc(),
    )
    .await
    .unwrap();
    let reqs = server.received_requests().await.unwrap();
    let limit: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(limit["orderType"], "limit");
    assert_eq!(limit["price"], "70000", "buy price rounds down to tick");
    assert_eq!(limit["timeInForce"], "GTC");
    let stop: Value = serde_json::from_slice(&reqs[1].body).unwrap();
    assert_eq!(stop["orderType"], "stopLoss");
    assert_eq!(stop["side"], "sell");
    assert_eq!(
        stop["triggerAmount"], "67901",
        "sell trigger rounds up to tick"
    );
    assert_eq!(stop["triggerType"], "price");
    assert_eq!(stop["triggerReference"], "lastTrade");
}

#[tokio::test]
async fn timeout_is_unknown_never_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/order"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server.uri());
    let out = c
        .place_order(&intent(OrderKind::Market, Side::Buy), &btc())
        .await
        .unwrap();
    assert!(matches!(out, PlaceOutcome::Unknown { .. }), "{out:?}");
}

#[tokio::test]
async fn uncertain_error_codes_are_unknown_and_validation_errors_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/order"))
        .respond_with(
            ResponseTemplate::new(503).set_body_json(json!({"errorCode":109,"error":"timed out"})),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v2/order"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"errorCode":216,"error":"Insufficient balance"})),
        )
        .mount(&server)
        .await;
    let c = client(&server.uri());
    let a = c
        .place_order(&intent(OrderKind::Market, Side::Buy), &btc())
        .await
        .unwrap();
    assert!(matches!(a, PlaceOutcome::Unknown { .. }), "{a:?}");
    let b = c
        .place_order(&intent(OrderKind::Market, Side::Buy), &btc())
        .await
        .unwrap();
    assert_eq!(
        b,
        PlaceOutcome::Rejected {
            code: "216".into(),
            message: "Insufficient balance".into()
        }
    );
}

#[tokio::test]
async fn rate_limit_blocks_and_is_reported() {
    let server = MockServer::start().await;
    let reset = Utc::now().timestamp_millis() + 30_000;
    Mock::given(method("GET"))
        .and(path("/v2/markets"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("bitvavo-ratelimit-remaining", "0")
                .insert_header("bitvavo-ratelimit-limit", "1000")
                .insert_header("bitvavo-ratelimit-resetat", reset.to_string().as_str())
                .set_body_json(json!({"errorCode":105,"error":"Rate limit exceeded"})),
        )
        .mount(&server)
        .await;
    let c = client(&server.uri());
    let err = c.instruments().await.unwrap_err();
    match err {
        BrokerError::RateLimited { retry_at } => {
            assert_eq!(retry_at.unwrap().timestamp_millis(), reset)
        }
        other => panic!("expected rate limit, got {other:?}"),
    }
    let h = c.health();
    assert!(h.last_rate_limited.is_some());
    assert_eq!(h.rate_budget_remaining, Some(dec!(0)));
    assert_eq!(h.errors_5m, 1);
}

#[tokio::test]
async fn live_host_is_refused_without_arming() {
    let cfg = BitvavoConfig::with_credentials(
        "https://api.bitvavo.com/v2",
        SecretString::from("k"),
        SecretString::from("s"),
        1,
    );
    let c = BitvavoClient::new(cfg).unwrap();
    let e = c
        .place_order(&intent(OrderKind::Market, Side::Buy), &btc())
        .await
        .unwrap_err();
    assert!(matches!(e, BrokerError::LiveDisabled));
    let r = OrderRef {
        instrument: btc().id,
        broker_order_id: Some("x".into()),
        client_order_id: None,
    };
    assert!(matches!(
        c.cancel_order(&r).await.unwrap_err(),
        BrokerError::LiveDisabled
    ));
    assert!(matches!(
        c.probe_withdrawal_permission().await.unwrap_err(),
        BrokerError::LiveDisabled
    ));
}

#[tokio::test]
async fn get_order_by_client_id_and_missing_order() {
    let server = MockServer::start().await;
    let id = Uuid::new_v4();
    Mock::given(method("GET"))
        .and(path("/v2/order"))
        .and(query_param("clientOrderId", id.to_string()))
        .and(query_param("market", "BTC-EUR"))
        .respond_with(ResponseTemplate::new(200).set_body_json(order_json(id, "filled")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/order"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_json(json!({"errorCode":240,"error":"No order found."})),
        )
        .mount(&server)
        .await;
    let c = client(&server.uri());
    let found = c
        .get_order(&OrderRef {
            instrument: btc().id,
            broker_order_id: None,
            client_order_id: Some(id),
        })
        .await
        .unwrap();
    assert_eq!(found.unwrap().client_order_id, Some(id));
    let missing = c
        .get_order(&OrderRef {
            instrument: btc().id,
            broker_order_id: None,
            client_order_id: Some(Uuid::new_v4()),
        })
        .await
        .unwrap();
    assert!(missing.is_none());
    assert_signed(&server.received_requests().await.unwrap()[0], "10000");
}

#[tokio::test]
async fn candles_oldest_first_open_bar_dropped() {
    let server = MockServer::start().await;
    let hour = 3_600_000i64;
    let open_start = Utc::now().timestamp_millis() / hour * hour;
    let rows = json!([
        [open_start, "70135", "70293", "70076", "70154", "0.45"],
        [
            open_start - hour,
            "69913",
            "70315",
            "69876",
            "70127",
            "4.32"
        ],
        [
            open_start - 2 * hour,
            "69800",
            "69950",
            "69700",
            "69913",
            "3.1"
        ]
    ]);
    Mock::given(method("GET"))
        .and(path("/v2/BTC-EUR/candles"))
        .and(query_param("interval", "1h"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rows))
        .mount(&server)
        .await;
    let c = client(&server.uri());
    let candles = c.candles(&btc().id, Interval::H1, 5, None).await.unwrap();
    assert_eq!(candles.len(), 2);
    assert!(candles[0].ts < candles[1].ts);
    assert_eq!(candles[1].close, dec!(70127));
    assert!(
        server.received_requests().await.unwrap()[0]
            .headers
            .get("Bitvavo-Access-Key")
            .is_none(),
        "public call is unsigned"
    );
}

#[tokio::test]
async fn fetch_history_pages_backwards() {
    let server = MockServer::start().await;
    let hour = 3_600_000i64;
    let base = (Utc::now() - ChronoDuration::days(10)).timestamp_millis() / hour * hour;
    let page = |from: i64, n: i64| {
        Value::Array(
            (0..n)
                .rev()
                .map(|k| json!([from + k * hour, "1", "2", "0.5", "1.5", "10"]))
                .collect(),
        )
    };
    Mock::given(method("GET"))
        .and(path("/v2/BTC-EUR/candles"))
        .and(query_param("end", (base + 3 * hour).to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(base, 3)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/BTC-EUR/candles"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(base + 3 * hour, 3)))
        .mount(&server)
        .await;
    let c = client(&server.uri());
    let start = chrono::TimeZone::timestamp_millis_opt(&Utc, base)
        .single()
        .unwrap();
    let end = Utc::now();
    let all = c
        .fetch_history("BTC-EUR", Interval::H1, start, end)
        .await
        .unwrap();
    assert_eq!(all.len(), 6);
    assert!(all.windows(2).all(|w| w[0].ts < w[1].ts));
}

#[tokio::test]
async fn quote_parses_scientific_notation() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/ticker/book"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"market":"BTC-EUR","bid":"70170","bidSize":"0.005","ask":"70171","askSize":"0.016"}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/ticker/24h"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"market":"BTC-EUR",
            "timestamp":1793587009187i64,"open":"69532","high":"70315","low":"6.95E+4","last":"7.014E+4",
            "volume":"390.3","volumeQuote":"27272970.17"})))
        .mount(&server)
        .await;
    let c = client(&server.uri());
    let q = c.quote(&btc().id).await.unwrap();
    assert_eq!(q.last, Some(dec!(70140)));
    assert_eq!(q.bid, Some(dec!(70170)));
    assert_eq!(q.volume_quote_24h, Some(dec!(27272970.17)));
}

#[tokio::test]
async fn balances_map_to_account_and_positions_and_history_scans_markets() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/balance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"symbol":"EUR","available":"45.10","inOrder":"5.00"},
            {"symbol":"BTC","available":"0.0001","inOrder":"0.00005"},
            {"symbol":"ETH","available":"0","inOrder":"0"}
        ])))
        .mount(&server)
        .await;
    let id = Uuid::new_v4();
    Mock::given(method("GET"))
        .and(path("/v2/orders"))
        .and(query_param("market", "BTC-EUR"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([order_json(id, "filled")])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/orders"))
        .and(query_param("market", "SOL-EUR"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server.uri());
    let acc = c.account().await.unwrap();
    assert_eq!(acc.cash[0].available, dec!(45.10));
    assert_eq!(acc.cash[0].reserved, dec!(5.00));
    let pos = c.positions().await.unwrap();
    assert_eq!(pos.len(), 1);
    assert_eq!(pos[0].instrument.as_str(), "bitvavo:BTC-EUR");
    assert_eq!(pos[0].quantity, dec!(0.00015));
    assert_eq!(pos[0].free_quantity, dec!(0.0001));
    c.track_market("SOL-EUR");
    let hist = c
        .order_history(Utc::now() - ChronoDuration::days(1))
        .await
        .unwrap();
    assert_eq!(hist.len(), 1);
    assert_eq!(hist[0].client_order_id, Some(id));
}

#[tokio::test]
async fn verify_credentials_detects_missing_trade_permission() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/time"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"time": Utc::now().timestamp_millis()})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/account"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"fees":{"taker":"0.0025","maker":"0.0015","volume":"0"}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/balance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/ordersOpen"))
        .respond_with(ResponseTemplate::new(403).set_body_json(
            json!({"errorCode":310,"error":"This key does not allow trading actions."}),
        ))
        .mount(&server)
        .await;
    let c = client(&server.uri());
    let r = c.verify_credentials().await.unwrap();
    assert!(r.authenticated && r.can_read_account);
    assert!(!r.can_trade);
    assert_eq!(r.can_withdraw, None, "never claimed without probe");
    assert!(c.health().clock_skew_ms.abs() < 2_000);
}

#[tokio::test]
async fn invalid_key_reports_unauthenticated() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/time"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"time": Utc::now().timestamp_millis()})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/account"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(json!({"errorCode":309,"error":"The signature is invalid."})),
        )
        .mount(&server)
        .await;
    let c = client(&server.uri());
    let r = c.verify_credentials().await.unwrap();
    assert!(!r.authenticated && !r.can_trade && !r.can_read_account);
}

#[tokio::test]
async fn deposits_and_withdrawals_are_cash_movements() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/depositHistory"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"timestamp":1706609321000i64,"symbol":"EUR","amount":"500","fee":"0","status":"completed","address":"NL89BANK0123456789"},
            {"timestamp":1706609322000i64,"symbol":"EUR","amount":"20","fee":"0","status":"canceled","address":"NL89"},
            {"timestamp":1706609323000i64,"symbol":"SHIB","amount":"1000","txId":"abc"}
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/withdrawalHistory"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"timestamp":1706609400000i64,"symbol":"EUR","amount":"100","fee":"0.5","status":"completed","address":"NL89"},
            {"timestamp":1706609500000i64,"symbol":"EUR","amount":"50","fee":"0","status":"awaiting_processing","address":"NL89"}
        ])))
        .mount(&server)
        .await;
    let c = client(&server.uri());
    let m = c
        .cash_movements(Utc::now() - ChronoDuration::days(3650))
        .await
        .unwrap();
    let kinds: Vec<_> = m.iter().map(|x| (x.kind, x.amount)).collect();
    assert_eq!(
        kinds,
        vec![
            (tc_broker::CashMovementKind::Deposit, dec!(500)),
            (tc_broker::CashMovementKind::Withdrawal, dec!(100)),
            (tc_broker::CashMovementKind::Fee, dec!(0.5)),
        ]
    );
}

#[test]
fn config_debug_redacts_secrets() {
    let cfg = BitvavoConfig::with_credentials(
        "http://x/v2",
        SecretString::from("AKEY123"),
        SecretString::from("SECRET456"),
        1,
    );
    let s = format!("{cfg:?}");
    assert!(!s.contains("AKEY123") && !s.contains("SECRET456"));
    assert!(s.contains("REDACTED"));
}

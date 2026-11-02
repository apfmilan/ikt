//! Integration tests against a local wiremock server. Never reaches live.trading212.com.
#![allow(clippy::unwrap_used)]

use chrono::{Duration as ChronoDuration, NaiveDate, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use secrecy::SecretString;
use serde_json::{json, Value};
use std::time::Duration;
use tc_broker::{BrokerClient, BrokerError, CashMovementKind, OrderRef, PlaceOutcome};
use tc_broker_t212::{Trading212Client, Trading212Config};
use tc_core::{
    AssetClass, Broker, Currency, Instrument, InstrumentId, IntentReason, OrderIntent, OrderKind,
    Side, TimeInForce,
};
use uuid::Uuid;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(base: &str) -> Trading212Client {
    let mut cfg = Trading212Config::new(
        format!("{base}/api/v0"),
        SecretString::from("KEY"),
        SecretString::from("SECRET"),
    );
    cfg.timeout = Duration::from_millis(400);
    Trading212Client::new(cfg).unwrap()
}

fn inst(ticker: &str, ccy: Currency) -> Instrument {
    Instrument {
        id: InstrumentId::new(Broker::Trading212, ticker),
        name: ticker.into(),
        asset_class: AssetClass::Stock,
        quote_currency: ccy,
        isin: None,
        min_notional: dec!(1),
        min_quantity: dec!(0.0001),
        quantity_decimals: 4,
        tick_size: Decimal::ZERO,
        tradable: true,
        first_seen: Utc::now(),
    }
}

fn intent(i: &Instrument, kind: OrderKind, side: Side, tif: TimeInForce) -> OrderIntent {
    OrderIntent {
        id: Uuid::new_v4(),
        instrument: i.id.clone(),
        side,
        kind,
        quantity: dec!(0.123456),
        time_in_force: tif,
        reason: IntentReason::Entry,
        decision_id: None,
        reference_price: dec!(100),
        created_at: Utc::now(),
    }
}

fn order(id: i64, ticker: &str, side: &str, qty: f64, created: &str) -> Value {
    json!({"id": id, "ticker": ticker, "side": side, "type": "MARKET", "quantity": qty, "filledQuantity": 0,
        "status": "NEW", "initiatedFrom": "API", "createdAt": created, "currency": "USD", "strategy": "QUANTITY"})
}

// "KEY:SECRET" base64
const BASIC: &str = "Basic S0VZOlNFQ1JFVA==";

#[tokio::test]
async fn market_orders_use_basic_auth_and_signed_quantity() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v0/equity/orders/market"))
        .and(header("Authorization", BASIC))
        .respond_with(ResponseTemplate::new(200).set_body_json(order(
            11,
            "AAPL_US_EQ",
            "BUY",
            0.1234,
            "2026-11-02T14:31:00Z",
        )))
        .expect(2)
        .mount(&s)
        .await;
    let c = client(&s.uri());
    let aapl = inst("AAPL_US_EQ", Currency::USD);
    let buy = c
        .place_order(
            &intent(&aapl, OrderKind::Market, Side::Buy, TimeInForce::Gtc),
            &aapl,
        )
        .await
        .unwrap();
    assert!(
        matches!(buy, PlaceOutcome::Accepted { ref order } if order.broker_order_id == "11" && order.via_api)
    );
    c.place_order(
        &intent(&aapl, OrderKind::Market, Side::Sell, TimeInForce::Gtc),
        &aapl,
    )
    .await
    .unwrap();
    let reqs = s.received_requests().await.unwrap();
    let b0: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    let b1 = String::from_utf8(reqs[1].body.clone()).unwrap();
    assert_eq!(b0["ticker"], "AAPL_US_EQ");
    assert_eq!(b0["extendedHours"], false);
    assert_eq!(
        String::from_utf8(reqs[0].body.clone()).unwrap(),
        r#"{"ticker":"AAPL_US_EQ","quantity":0.1234,"extendedHours":false}"#
    );
    assert!(
        b1.contains(r#""quantity":-0.1234"#),
        "sell must be negative: {b1}"
    );
}

#[tokio::test]
async fn limit_order_needs_metadata_and_converts_gbx() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/metadata/instruments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"ticker":"VODl_EQ","type":"STOCK","isin":"GB00BH4HKS39","currencyCode":"GBX","name":"Vodafone",
             "shortName":"VOD","workingScheduleId":7,"extendedHours":false,"maxOpenQuantity":10000,"addedOn":"2018-01-01T00:00:00Z"},
            {"ticker":"BTC_CRYPTO","type":"CRYPTOCURRENCY","currencyCode":"USD","name":"x"}
        ])))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v0/equity/orders/limit"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 5, "ticker": "VODl_EQ", "side": "BUY",
            "type": "LIMIT", "quantity": 0.1234, "limitPrice": 7250, "status": "NEW", "initiatedFrom": "API",
            "createdAt": "2026-11-02T10:00:00Z"})))
        .expect(1)
        .mount(&s)
        .await;
    let c = client(&s.uri());
    let vod = inst("VODl_EQ", Currency::GBP);
    // Without metadata a priced order is refused locally (nothing sent).
    let r = c
        .place_order(
            &intent(
                &vod,
                OrderKind::Limit { price: dec!(72.50) },
                Side::Buy,
                TimeInForce::Day,
            ),
            &vod,
        )
        .await
        .unwrap();
    assert!(matches!(r, PlaceOutcome::Rejected { ref code, .. } if code == "local_metadata"));
    let instruments = c.instruments().await.unwrap();
    assert_eq!(instruments.len(), 1, "only STOCK/ETF");
    assert_eq!(instruments[0].quote_currency, Currency::GBP);
    assert_eq!(instruments[0].isin.as_deref(), Some("GB00BH4HKS39"));
    let r = c
        .place_order(
            &intent(
                &vod,
                OrderKind::Limit { price: dec!(72.50) },
                Side::Buy,
                TimeInForce::Day,
            ),
            &vod,
        )
        .await
        .unwrap();
    let PlaceOutcome::Accepted { order } = r else {
        panic!("{r:?}")
    };
    assert_eq!(
        order.kind,
        OrderKind::Limit { price: dec!(72.5) },
        "pence normalised inbound"
    );
    let reqs = s.received_requests().await.unwrap();
    let body = String::from_utf8(reqs.last().unwrap().body.clone()).unwrap();
    assert_eq!(
        body,
        r#"{"ticker":"VODl_EQ","quantity":0.1234,"limitPrice":7250,"timeValidity":"DAY"}"#
    );
}

#[tokio::test]
async fn uncertain_outcomes_are_unknown_and_never_retried() {
    for (status, delay) in [(408u16, 0u64), (500, 0), (200, 2000)] {
        let s = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v0/equity/orders/market"))
            .respond_with(ResponseTemplate::new(status).set_delay(Duration::from_millis(delay)))
            .expect(1)
            .mount(&s)
            .await;
        let c = client(&s.uri());
        let aapl = inst("AAPL_US_EQ", Currency::USD);
        let out = c
            .place_order(
                &intent(&aapl, OrderKind::Market, Side::Buy, TimeInForce::Gtc),
                &aapl,
            )
            .await
            .unwrap();
        assert!(
            matches!(out, PlaceOutcome::Unknown { .. }),
            "status {status}: {out:?}"
        );
    }
}

#[tokio::test]
async fn validation_error_is_rejected_and_429_blocks() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v0/equity/orders/market"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string(r#"{"code":"InsufficientFreeForStocksBuy"}"#),
        )
        .up_to_n_times(1)
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v0/equity/orders/market"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("x-ratelimit-limit", "50")
                .insert_header(
                    "x-ratelimit-reset",
                    (Utc::now().timestamp() + 30).to_string().as_str(),
                ),
        )
        .mount(&s)
        .await;
    let c = client(&s.uri());
    let aapl = inst("AAPL_US_EQ", Currency::USD);
    let a = c
        .place_order(
            &intent(&aapl, OrderKind::Market, Side::Buy, TimeInForce::Gtc),
            &aapl,
        )
        .await
        .unwrap();
    assert!(
        matches!(a, PlaceOutcome::Rejected { ref code, ref message } if code == "http_400" && message.contains("InsufficientFree"))
    );
    let b = c
        .place_order(
            &intent(&aapl, OrderKind::Market, Side::Buy, TimeInForce::Gtc),
            &aapl,
        )
        .await
        .unwrap();
    assert!(matches!(b, PlaceOutcome::Rejected { ref code, .. } if code == "http_429"));
    let h = c.health();
    assert!(h.last_rate_limited.is_some());
    assert_eq!(h.rate_budget_remaining, Some(dec!(0)));
}

#[tokio::test]
async fn live_host_refused_and_ioc_unsupported() {
    let c = Trading212Client::new(Trading212Config::live(
        SecretString::from("k"),
        SecretString::from("s"),
    ))
    .unwrap();
    let aapl = inst("AAPL_US_EQ", Currency::USD);
    let e = c
        .place_order(
            &intent(&aapl, OrderKind::Market, Side::Buy, TimeInForce::Gtc),
            &aapl,
        )
        .await
        .unwrap_err();
    assert!(matches!(e, BrokerError::LiveDisabled));
    let r = OrderRef {
        instrument: aapl.id.clone(),
        broker_order_id: Some("1".into()),
        client_order_id: None,
    };
    assert!(matches!(
        c.cancel_order(&r).await.unwrap_err(),
        BrokerError::LiveDisabled
    ));
    assert!(matches!(
        c.probe_trade_scope().await.unwrap_err(),
        BrokerError::LiveDisabled
    ));

    let s = MockServer::start().await;
    let c = client(&s.uri());
    let out = c
        .place_order(
            &intent(&aapl, OrderKind::Market, Side::Buy, TimeInForce::Ioc),
            &aapl,
        )
        .await
        .unwrap();
    // Market orders ignore time validity; IOC only matters for priced orders.
    assert!(!matches!(out, PlaceOutcome::Rejected { ref code, .. } if code == "local_unsupported"));
}

#[tokio::test]
async fn order_history_follows_next_page_path_until_since() {
    let s = MockServer::start().await;
    let now = Utc::now();
    let t = |mins: i64| (now - ChronoDuration::minutes(mins)).to_rfc3339();
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/history/orders"))
        .and(query_param("cursor", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [{"order": order(2, "AAPL_US_EQ", "SELL", -1.0, &t(30)),
                       "fill": {"id": 20, "filledAt": t(30), "price": 101.5, "quantity": -1.0, "type": "TRADE",
                                "walletImpact": {"currency": "EUR", "taxes": [{"name":"CURRENCY_CONVERSION_FEE","quantity":0.15,"currency":"EUR"}]}}},
                      {"order": order(1, "AAPL_US_EQ", "BUY", 1.0, &t(60 * 48)), "fill": null}],
            "nextPagePath": "/api/v0/equity/history/orders?limit=50&cursor=1"})))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/history/orders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [{"order": order(3, "MSFT_US_EQ", "BUY", 2.0, &t(5)), "fill": null}],
            "nextPagePath": "/api/v0/equity/history/orders?limit=50&cursor=2"})))
        .mount(&s)
        .await;
    let c = client(&s.uri());
    let h = c
        .order_history(now - ChronoDuration::hours(24))
        .await
        .unwrap();
    let ids: Vec<_> = h.iter().map(|o| o.broker_order_id.as_str()).collect();
    assert_eq!(ids, vec!["3", "2"], "newest first, stops at `since`");
    assert_eq!(h[1].fills[0].price, dec!(101.5));
    assert_eq!(h[1].fee_paid, dec!(0.15));
    assert_eq!(h[1].side, Side::Sell);
    assert_eq!(
        s.received_requests().await.unwrap().len(),
        2,
        "page with cursor=1 never requested"
    );
}

#[tokio::test]
async fn foreign_next_page_path_is_rejected() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/history/orders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [], "nextPagePath": "https://evil.example/steal"})))
        .mount(&s)
        .await;
    let c = client(&s.uri());
    // Empty page with a next path: still must not be followed to another host.
    let r = c.order_history(Utc::now() - ChronoDuration::hours(1)).await;
    assert!(matches!(r, Err(BrokerError::Decode(_))), "{r:?}");
}

#[tokio::test]
async fn account_positions_and_sessions() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/account/summary"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 1, "currency": "EUR", "totalValue": 312.45,
            "cash": {"availableToTrade": 120.5, "reservedForOrders": 10, "inPies": 0},
            "investments": {"currentValue": 182.0, "totalCost": 175.0, "realizedProfitLoss": 3.1, "unrealizedProfitLoss": 7.0}})))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/positions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"instrument": {"ticker": "VODl_EQ", "isin": "GB00BH4HKS39", "currency": "GBX", "name": "Vodafone"},
             "quantity": 10.5, "quantityAvailableForTrading": 10.0, "quantityInPies": 0, "averagePricePaid": 7012.0,
             "currentPrice": 7250.0, "createdAt": "2026-10-01T10:00:00Z"}])))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/metadata/instruments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"ticker":"VODl_EQ","type":"STOCK","currencyCode":"GBX","name":"Vodafone","workingScheduleId":7}])))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/metadata/exchanges"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!([{"id": 1, "name": "London Stock Exchange",
            "workingSchedules": [{"id": 7, "timeEvents": [
                {"date": "2026-11-02T08:00:00Z", "type": "OPEN"},
                {"date": "2026-11-02T16:30:00Z", "type": "CLOSE"},
                {"date": "2026-11-03T08:00:00Z", "type": "OPEN"}]}]}]),
        ))
        .mount(&s)
        .await;
    let c = client(&s.uri());
    let acc = c.account().await.unwrap();
    assert_eq!(acc.account_currency, Currency::EUR);
    assert_eq!(acc.cash[0].available, dec!(120.5));
    assert_eq!(acc.total_value, Some(dec!(312.45)));
    c.instruments().await.unwrap();
    let pos = c.positions().await.unwrap();
    assert_eq!(pos[0].quantity, dec!(10.5));
    assert_eq!(pos[0].free_quantity, dec!(10));
    assert_eq!(pos[0].avg_price, Some(dec!(70.12)));
    assert_eq!(pos[0].current_price, Some(dec!(72.5)));
    assert_eq!(c.load_exchanges().await.unwrap(), 1);
    let (open, close) = c
        .session_for("VODl_EQ", NaiveDate::from_ymd_opt(2026, 11, 2).unwrap())
        .unwrap();
    assert_eq!(open.to_rfc3339(), "2026-11-02T08:00:00+00:00");
    assert_eq!(close.to_rfc3339(), "2026-11-02T16:30:00+00:00");
    assert!(c
        .session_for("VODl_EQ", NaiveDate::from_ymd_opt(2026, 11, 1).unwrap())
        .is_none());
}

#[tokio::test]
async fn verify_credentials_reports_auth_failure_and_missing_scopes() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/account/summary"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Bad API key"))
        .mount(&s)
        .await;
    let r = client(&s.uri()).verify_credentials().await.unwrap();
    assert!(!r.authenticated && !r.can_trade);

    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/account/summary"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"currency":"EUR","cash":{"availableToTrade":1}})),
        )
        .mount(&s)
        .await;
    for p in [
        "/api/v0/equity/positions",
        "/api/v0/equity/orders",
        "/api/v0/equity/metadata/exchanges",
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&s)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/history/orders"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"items": [], "nextPagePath": null})),
        )
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/history/transactions"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_string("Scope( history:transactions ) missing for API key"),
        )
        .mount(&s)
        .await;
    let r = client(&s.uri()).verify_credentials().await.unwrap();
    assert!(r.authenticated && r.can_read_account);
    assert_eq!(r.account_currency, Some(Currency::EUR));
    assert_eq!(r.can_withdraw, Some(false));
    assert!(
        !r.can_trade,
        "orders:execute is never claimed without proof"
    );
    assert!(r.notes.iter().any(|n| n.contains("history:transactions")));
}

#[tokio::test]
async fn cash_movements_from_transactions_and_dividends() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/history/transactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [
            {"type": "DEPOSIT", "amount": 50.0, "currency": "EUR", "dateTime": "2026-11-01T10:00:00Z", "reference": "d1"},
            {"type": "WITHDRAW", "amount": -20.0, "currency": "EUR", "dateTime": "2026-10-31T10:00:00Z", "reference": "w1"},
            {"type": "INTEREST_ON_FREE_CASH", "amount": 0.02, "currency": "EUR", "dateTime": "2026-10-30T10:00:00Z", "reference": "i1"},
            {"type": "DEPOSIT", "amount": 999.0, "currency": "EUR", "dateTime": "2020-01-01T10:00:00Z", "reference": "old"}
        ], "nextPagePath": null})))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/history/dividends"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [
            {"ticker": "VUSAl_EQ", "amount": 0.31, "amountInEuro": 0.31, "currency": "EUR", "paidOn": "2026-10-15T00:00:00Z",
             "quantity": 1.2, "reference": "div1", "type": "ORDINARY"}], "nextPagePath": null})))
        .mount(&s)
        .await;
    let c = client(&s.uri());
    let m = c
        .cash_movements(Utc::now() - ChronoDuration::days(60))
        .await
        .unwrap();
    let v: Vec<_> = m
        .iter()
        .map(|x| (x.kind, x.amount, x.broker_ref.as_str()))
        .collect();
    assert_eq!(
        v,
        vec![
            (CashMovementKind::Dividend, dec!(0.31), "div1"),
            (CashMovementKind::Interest, dec!(0.02), "i1"),
            (CashMovementKind::Withdrawal, dec!(20), "w1"),
            (CashMovementKind::Deposit, dec!(50), "d1"),
        ]
    );
}

#[tokio::test]
async fn get_order_pending_then_history_fallback() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/orders/42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(order(
            42,
            "AAPL_US_EQ",
            "BUY",
            1.0,
            "2026-11-02T14:31:00Z",
        )))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/orders/43"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/history/orders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [
            {"order": order(43, "AAPL_US_EQ", "BUY", 1.0, &Utc::now().to_rfc3339()), "fill": null}], "nextPagePath": null})))
        .mount(&s)
        .await;
    let c = client(&s.uri());
    let id = InstrumentId::new(Broker::Trading212, "AAPL_US_EQ");
    let r = |x: &str| OrderRef {
        instrument: id.clone(),
        broker_order_id: Some(x.into()),
        client_order_id: None,
    };
    assert_eq!(
        c.get_order(&r("42"))
            .await
            .unwrap()
            .unwrap()
            .broker_order_id,
        "42"
    );
    assert_eq!(
        c.get_order(&r("43"))
            .await
            .unwrap()
            .unwrap()
            .broker_order_id,
        "43"
    );
    let no_id = OrderRef {
        instrument: id.clone(),
        broker_order_id: None,
        client_order_id: Some(Uuid::new_v4()),
    };
    assert!(matches!(
        c.get_order(&no_id).await.unwrap_err(),
        BrokerError::Unsupported(_)
    ));
    assert!(matches!(
        c.get_order(&r("../x")).await.unwrap_err(),
        BrokerError::Decode(_)
    ));
}

#[test]
fn config_debug_redacts() {
    let cfg = Trading212Config::live(
        SecretString::from("KEY123456"),
        SecretString::from("SECRET987"),
    );
    let s = format!("{cfg:?}");
    assert!(!s.contains("KEY123456") && !s.contains("SECRET987"));
}

#[tokio::test]
async fn ioc_priced_order_is_refused_locally() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v0/equity/metadata/instruments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"ticker":"AAPL_US_EQ","type":"STOCK","currencyCode":"USD","name":"Apple"}])))
        .mount(&s)
        .await;
    let c = client(&s.uri());
    c.instruments().await.unwrap();
    let aapl = inst("AAPL_US_EQ", Currency::USD);
    let out = c
        .place_order(
            &intent(
                &aapl,
                OrderKind::Limit { price: dec!(100) },
                Side::Buy,
                TimeInForce::Ioc,
            ),
            &aapl,
        )
        .await
        .unwrap();
    assert!(matches!(out, PlaceOutcome::Rejected { ref code, .. } if code == "local_unsupported"));
    assert_eq!(
        s.received_requests().await.unwrap().len(),
        1,
        "only the metadata call was sent"
    );
}

//! MySQL integration tests. Run with a disposable database:
//! `TC_TEST_DATABASE_URL=mysql://root:pw@127.0.0.1:3307/tc_test cargo test -p tc-store -- --include-ignored`
//! (`make db-test` does this with a throwaway mysql:8.4 container.)

use chrono::Duration;
use rust_decimal_macros::dec;
use tc_core::{
    Broker, InstrumentId, IntentReason, OrderIntent, OrderKind, OrderState, Side, TimeInForce,
};
use tc_risk::KillMode;
use tc_store::auth::TokenKind;
use tc_store::trading::IntentUpdate;
use tc_store::{now_micros, Store};
use uuid::Uuid;

async fn store() -> Option<Store> {
    let url = std::env::var("TC_TEST_DATABASE_URL").ok()?;
    let s = Store::connect(&url, 5).await.expect("connect");
    s.migrate().await.expect("migrate");
    Some(s)
}

fn intent(inst: &str) -> OrderIntent {
    OrderIntent {
        id: Uuid::new_v4(),
        instrument: InstrumentId::parse(inst).expect("id"),
        side: Side::Buy,
        kind: OrderKind::Market,
        quantity: dec!(0.0002),
        time_in_force: TimeInForce::Gtc,
        reason: IntentReason::Entry,
        decision_id: None,
        reference_price: dec!(70000),
        created_at: now_micros(),
    }
}

#[tokio::test]
#[ignore = "needs TC_TEST_DATABASE_URL"]
async fn intent_state_machine_is_enforced_in_db() {
    let Some(s) = store().await else { return };
    let i = intent("bitvavo:BTC-EUR");
    s.insert_intent(&i, "s1_trend_vol", &serde_json::json!({"verdict":"allow"}))
        .await
        .expect("insert");
    let r = s
        .transition_intent(i.id, OrderState::Submitting, &IntentUpdate::default())
        .await
        .expect("submit");
    assert_eq!(r.state, OrderState::Submitting);
    assert!(r.submitted_at.is_some());
    let r = s
        .transition_intent(
            i.id,
            OrderState::Unknown,
            &IntentUpdate {
                last_error: Some("timeout".into()),
                ..Default::default()
            },
        )
        .await
        .expect("unknown");
    assert_eq!(r.state, OrderState::Unknown);
    // Blind retry path is rejected by the state machine.
    assert!(s
        .transition_intent(i.id, OrderState::Submitting, &IntentUpdate::default())
        .await
        .is_err());
    let r = s
        .transition_intent(
            i.id,
            OrderState::Filled,
            &IntentUpdate {
                broker_order_id: Some("abc".into()),
                filled_quantity: Some(dec!(0.0002)),
                ..Default::default()
            },
        )
        .await
        .expect("filled");
    assert_eq!(r.broker_order_id.as_deref(), Some("abc"));
    assert_eq!(r.filled_quantity, dec!(0.0002));
    let active = s
        .active_and_recent_intents(now_micros() - Duration::minutes(5))
        .await
        .expect("list");
    assert!(active.iter().any(|x| x.intent.id == i.id));
}

#[tokio::test]
#[ignore = "needs TC_TEST_DATABASE_URL"]
async fn audit_chain_verifies_and_is_append_only() {
    let Some(s) = store().await else { return };
    for k in 0..5 {
        s.audit(
            "test",
            "event",
            &serde_json::json!({"k": k, "amount": "1.50", "nested": {"b": 1, "a": [1, 2.5]}}),
        )
        .await
        .expect("audit");
    }
    assert!(s.verify_audit_chain().await.expect("verify") >= 5);
    let upd = sqlx::query("UPDATE tc_audit_log SET actor = 'x' ORDER BY id DESC LIMIT 1")
        .execute(&s.pool)
        .await;
    assert!(upd.is_err(), "append-only trigger must reject UPDATE");
    let del = sqlx::query("DELETE FROM tc_ledger").execute(&s.pool).await;
    assert!(del.is_err() || del.map(|r| r.rows_affected()).unwrap_or(0) == 0);
}

#[tokio::test]
#[ignore = "needs TC_TEST_DATABASE_URL"]
async fn ledger_is_idempotent_and_flows_sum() {
    let Some(s) = store().await else { return };
    let r = format!("dep-{}", Uuid::new_v4());
    let t = now_micros();
    assert!(s
        .ledger_append(
            t,
            "bitvavo",
            "deposit",
            dec!(62.5),
            "EUR",
            dec!(25000),
            dec!(400),
            &r,
            None
        )
        .await
        .expect("append"));
    assert!(!s
        .ledger_append(
            t,
            "bitvavo",
            "deposit",
            dec!(62.5),
            "EUR",
            dec!(25000),
            dec!(400),
            &r,
            None
        )
        .await
        .expect("dup"));
    let flows = s
        .net_flows_huf(t - Duration::seconds(1), t + Duration::seconds(1))
        .await
        .expect("flows");
    assert!(flows >= dec!(25000));
}

#[tokio::test]
#[ignore = "needs TC_TEST_DATABASE_URL"]
async fn sessions_tokens_and_kill_switch() {
    let Some(s) = store().await else { return };
    let tok = Uuid::new_v4().as_bytes().to_vec();
    s.insert_token(
        &tok,
        TokenKind::Setup,
        None,
        None,
        Some(Duration::minutes(5)),
    )
    .await
    .expect("token");
    assert!(
        s.consume_token(&tok, TokenKind::Pairing, None)
            .await
            .is_err(),
        "kind mismatch"
    );
    s.consume_token(&tok, TokenKind::Setup, None)
        .await
        .expect("consume");
    assert!(
        s.consume_token(&tok, TokenKind::Setup, None).await.is_err(),
        "single use"
    );

    let pk = s
        .insert_passkey(Uuid::new_v4().as_bytes(), "{\"x\":1}", "iPhone")
        .await
        .expect("passkey");
    let sess = Uuid::new_v4().as_bytes().to_vec();
    s.create_session(
        &sess,
        pk,
        Duration::minutes(60),
        Duration::hours(12),
        Some("test"),
    )
    .await
    .expect("session");
    assert!(s
        .touch_session(&sess, Duration::minutes(60))
        .await
        .expect("touch")
        .is_some());
    s.revoke_session(&sess).await.expect("revoke");
    assert!(s
        .touch_session(&sess, Duration::minutes(60))
        .await
        .expect("touch")
        .is_none());

    s.set_kill("broker:bitvavo", KillMode::ReduceOnly, "test", "test")
        .await
        .expect("kill");
    let k = s.kill_state().await.expect("state");
    assert_eq!(k.brokers.get(&Broker::Bitvavo), Some(&KillMode::ReduceOnly));
    s.set_kill("broker:bitvavo", KillMode::Normal, "test", "test")
        .await
        .expect("kill");
}

#[tokio::test]
#[ignore = "needs TC_TEST_DATABASE_URL"]
async fn commands_are_claimed_once() {
    let Some(s) = store().await else { return };
    let id = s
        .enqueue_command(
            "verify_credentials",
            &serde_json::json!({"broker":"bitvavo"}),
            "test",
            None,
        )
        .await
        .expect("enqueue");
    let mut seen = false;
    while let Some(c) = s.claim_command().await.expect("claim") {
        if c.id == id {
            seen = true;
        }
        s.finish_command(c.id, true, &serde_json::json!({"ok": true}))
            .await
            .expect("finish");
    }
    assert!(seen);
    assert!(s.claim_command().await.expect("claim").is_none());
}

#[tokio::test]
#[ignore = "needs TC_TEST_DATABASE_URL"]
async fn engine_singleton_lock() {
    let Some(s) = store().await else { return };
    let a = s.acquire_engine_lock().await.expect("first");
    assert!(
        s.acquire_engine_lock().await.is_err(),
        "second engine must be refused"
    );
    drop(a);
}

#[tokio::test]
#[ignore = "needs TC_TEST_DATABASE_URL"]
async fn fill_booking_is_atomic_and_idempotent() {
    use tc_store::booking::{FillCommit, LedgerEntry, PositionChange};
    use tc_store::trading::PositionRow;
    let Some(s) = store().await else { return };
    let i = intent("bitvavo:ETH-EUR");
    s.insert_intent(&i, "s1_trend_vol", &serde_json::json!({})).await.expect("insert");
    s.transition_intent(i.id, OrderState::Submitting, &IntentUpdate::default()).await.expect("submit");
    let pos = PositionRow {
        instrument: i.instrument.clone(),
        quantity: dec!(0.0001),
        avg_price: dec!(3000),
        opened_at: now_micros(),
        stop_price: Some(dec!(2900)),
        take_profit: None,
        initial_risk: Some(dec!(100)),
        high_watermark: dec!(3000),
        strategy: "s1_trend_vol".into(),
        long_term: false,
        bars_held: 0,
        partial_taken: false,
        stop_intent_id: None,
        decision_id: None,
    };
    let fill_id = format!("ord-{}@0.0001", Uuid::new_v4());
    let commit = FillCommit {
        broker: Broker::Bitvavo,
        fill_id: fill_id.clone(),
        intent_id: i.id,
        instrument: i.instrument.clone(),
        side: Side::Buy,
        quantity: dec!(0.0001),
        price: dec!(3000),
        fee: dec!(0.00075),
        fee_currency: Some("EUR".into()),
        ts: now_micros(),
        intent_to: Some(OrderState::PartiallyFilled),
        intent_update: IntentUpdate { filled_quantity: Some(dec!(0.0001)), avg_fill_price: Some(dec!(3000)), ..Default::default() },
        position: PositionChange::Upsert(pos),
        trade: None,
        ledger: vec![LedgerEntry {
            ts: now_micros(),
            broker: "bitvavo".into(),
            kind: "fee".into(),
            amount: dec!(-0.00075),
            currency: "EUR".into(),
            amount_huf: dec!(-0.3),
            fx_rate: dec!(400),
            r#ref: format!("{fill_id}:fee"),
        }],
    };
    // Submitting → PartiallyFilled is valid; booking writes everything at once.
    assert!(s.commit_fill(&commit).await.expect("commit"));
    // Same fill id again: no-op (no double booking).
    assert!(!s.commit_fill(&commit).await.expect("dup"));
    let p = s.positions().await.expect("positions").into_iter().find(|p| p.instrument == i.instrument).expect("position");
    assert_eq!(p.quantity, dec!(0.0001));
    let row = s.intent(i.id).await.expect("intent").expect("row");
    assert_eq!(row.state, OrderState::PartiallyFilled);
    assert_eq!(row.filled_quantity, dec!(0.0001));
    // An invalid transition rolls back the whole booking (fill row included).
    let bad = FillCommit { fill_id: format!("{fill_id}-x"), intent_to: Some(OrderState::Submitting), ..commit.clone() };
    assert!(s.commit_fill(&bad).await.is_err());
    assert!(s.commit_fill(&FillCommit { intent_to: None, ..bad.clone() }).await.expect("retry ok"));
    s.delete_position(&i.instrument).await.expect("cleanup");
}

#[tokio::test]
#[ignore = "needs TC_TEST_DATABASE_URL"]
async fn stale_pending_is_aborted_and_ledger_cursor_counts_late_deposits() {
    let Some(s) = store().await else { return };
    let i = intent("bitvavo:ADA-EUR");
    s.insert_intent(&i, "x", &serde_json::json!({})).await.expect("insert");
    let aborted = s.abort_stale_pending(now_micros() + Duration::seconds(1)).await.expect("abort");
    assert!(aborted.contains(&i.id));
    assert_eq!(s.intent(i.id).await.expect("i").expect("row").state, OrderState::Aborted);

    let before = s.max_ledger_id().await.expect("max");
    // A deposit whose broker timestamp is a week old still counts (cursor is by ledger id).
    let old_ts = now_micros() - Duration::days(7);
    s.ledger_append(old_ts, "bitvavo", "deposit", dec!(125), "EUR", dec!(50000), dec!(400), &format!("late-{}", Uuid::new_v4()), None)
        .await
        .expect("append");
    let (flows, max_id) = s.flows_after_ledger_id(before).await.expect("flows");
    assert_eq!(flows, dec!(50000));
    assert!(max_id > before);
    let (none, same) = s.flows_after_ledger_id(max_id).await.expect("flows2");
    assert_eq!(none, dec!(0));
    assert_eq!(same, max_id);
}

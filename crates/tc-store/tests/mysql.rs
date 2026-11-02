//! MySQL integration tests. Run with a disposable database:
//! `TC_TEST_DATABASE_URL=mysql://root:pw@127.0.0.1:3307/tc_test cargo test -p tc-store -- --include-ignored`
//! (`make db-test` does this with a throwaway mysql:8.4 container.)

use chrono::Duration;
use rust_decimal_macros::dec;
use tc_core::{Broker, InstrumentId, IntentReason, OrderIntent, OrderKind, OrderState, Side, TimeInForce};
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
    s.insert_intent(&i, "s1_trend_vol", &serde_json::json!({"verdict":"allow"})).await.expect("insert");
    let r = s.transition_intent(i.id, OrderState::Submitting, &IntentUpdate::default()).await.expect("submit");
    assert_eq!(r.state, OrderState::Submitting);
    assert!(r.submitted_at.is_some());
    let r = s.transition_intent(i.id, OrderState::Unknown, &IntentUpdate { last_error: Some("timeout".into()), ..Default::default() }).await.expect("unknown");
    assert_eq!(r.state, OrderState::Unknown);
    // Blind retry path is rejected by the state machine.
    assert!(s.transition_intent(i.id, OrderState::Submitting, &IntentUpdate::default()).await.is_err());
    let r = s
        .transition_intent(i.id, OrderState::Filled, &IntentUpdate { broker_order_id: Some("abc".into()), filled_quantity: Some(dec!(0.0002)), ..Default::default() })
        .await
        .expect("filled");
    assert_eq!(r.broker_order_id.as_deref(), Some("abc"));
    assert_eq!(r.filled_quantity, dec!(0.0002));
    let active = s.active_and_recent_intents(now_micros() - Duration::minutes(5)).await.expect("list");
    assert!(active.iter().any(|x| x.intent.id == i.id));
}

#[tokio::test]
#[ignore = "needs TC_TEST_DATABASE_URL"]
async fn audit_chain_verifies_and_is_append_only() {
    let Some(s) = store().await else { return };
    for k in 0..5 {
        s.audit("test", "event", &serde_json::json!({"k": k, "amount": "1.50", "nested": {"b": 1, "a": [1, 2.5]}})).await.expect("audit");
    }
    assert!(s.verify_audit_chain().await.expect("verify") >= 5);
    let upd = sqlx::query("UPDATE tc_audit_log SET actor = 'x' ORDER BY id DESC LIMIT 1").execute(&s.pool).await;
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
    assert!(s.ledger_append(t, "bitvavo", "deposit", dec!(62.5), "EUR", dec!(25000), dec!(400), &r, None).await.expect("append"));
    assert!(!s.ledger_append(t, "bitvavo", "deposit", dec!(62.5), "EUR", dec!(25000), dec!(400), &r, None).await.expect("dup"));
    let flows = s.net_flows_huf(t - Duration::seconds(1), t + Duration::seconds(1)).await.expect("flows");
    assert!(flows >= dec!(25000));
}

#[tokio::test]
#[ignore = "needs TC_TEST_DATABASE_URL"]
async fn sessions_tokens_and_kill_switch() {
    let Some(s) = store().await else { return };
    let tok = Uuid::new_v4().as_bytes().to_vec();
    s.insert_token(&tok, TokenKind::Setup, None, None, Some(Duration::minutes(5))).await.expect("token");
    assert!(s.consume_token(&tok, TokenKind::Pairing, None).await.is_err(), "kind mismatch");
    s.consume_token(&tok, TokenKind::Setup, None).await.expect("consume");
    assert!(s.consume_token(&tok, TokenKind::Setup, None).await.is_err(), "single use");

    let pk = s.insert_passkey(Uuid::new_v4().as_bytes(), "{\"x\":1}", "iPhone").await.expect("passkey");
    let sess = Uuid::new_v4().as_bytes().to_vec();
    s.create_session(&sess, pk, Duration::minutes(60), Duration::hours(12), Some("test")).await.expect("session");
    assert!(s.touch_session(&sess, Duration::minutes(60)).await.expect("touch").is_some());
    s.revoke_session(&sess).await.expect("revoke");
    assert!(s.touch_session(&sess, Duration::minutes(60)).await.expect("touch").is_none());

    s.set_kill("broker:bitvavo", KillMode::ReduceOnly, "test", "test").await.expect("kill");
    let k = s.kill_state().await.expect("state");
    assert_eq!(k.brokers.get(&Broker::Bitvavo), Some(&KillMode::ReduceOnly));
    s.set_kill("broker:bitvavo", KillMode::Normal, "test", "test").await.expect("kill");
}

#[tokio::test]
#[ignore = "needs TC_TEST_DATABASE_URL"]
async fn commands_are_claimed_once() {
    let Some(s) = store().await else { return };
    let id = s.enqueue_command("verify_credentials", &serde_json::json!({"broker":"bitvavo"}), "test", None).await.expect("enqueue");
    let mut seen = false;
    while let Some(c) = s.claim_command().await.expect("claim") {
        if c.id == id {
            seen = true;
        }
        s.finish_command(c.id, true, &serde_json::json!({"ok": true})).await.expect("finish");
    }
    assert!(seen);
    assert!(s.claim_command().await.expect("claim").is_none());
}

#[tokio::test]
#[ignore = "needs TC_TEST_DATABASE_URL"]
async fn engine_singleton_lock() {
    let Some(s) = store().await else { return };
    let a = s.acquire_engine_lock().await.expect("first");
    assert!(s.acquire_engine_lock().await.is_err(), "second engine must be refused");
    drop(a);
}

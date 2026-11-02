//! Router tests that need no database: CSRF origin guard and auth extractor rejection.
//! Database-backed API flows are covered by the MySQL integration suite.

use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

fn state() -> Arc<AppState> {
    let store = Store::connect_lazy("mysql://u:p@127.0.0.1:1/none").expect("lazy pool");
    let cfg = Config {
        database_url: String::new(),
        bind: String::new(),
        rp_id: "localhost".into(),
        rp_origin: "http://localhost:8080".into(),
        engine_public_key: None,
        engine_secret_key_file: None,
        vapid_public_key: None,
        vapid_private_key_file: None,
        vapid_subject: String::new(),
        live_trading: None,
        dev_insecure_cookies: true,
    };
    Arc::new(AppState::new(store, &cfg).expect("state"))
}

#[tokio::test]
async fn post_without_origin_is_rejected() {
    let app = router(state());
    let resp = app
        .oneshot(
            Request::post("/api/control/kill")
                .header("content-type", "application/json")
                .body(Body::from("{\"mode\":\"freeze\"}"))
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn post_with_foreign_origin_is_rejected() {
    let app = router(state());
    let resp = app
        .oneshot(
            Request::post("/api/auth/login/start")
                .header("origin", "https://evil.example")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn protected_get_without_session_is_401() {
    let app = router(state());
    let resp = app
        .oneshot(
            Request::get("/api/overview")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        resp.headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
}

#[tokio::test]
async fn ceremony_ids_are_single_use() {
    let st = state();
    let (_, r) = st
        .webauthn
        .start_passkey_registration(Uuid::new_v4(), "u", "u", None)
        .expect("reg");
    let id = st.put_ceremony(Ceremony::Register {
        state: r,
        device_name: "x".into(),
        purpose: RegisterPurpose::Pairing,
    });
    assert!(st.take_ceremony(&id).is_some());
    assert!(
        st.take_ceremony(&id).is_none(),
        "challenge must be single-use"
    );
    assert!(st.take_ceremony("unknown").is_none());
}

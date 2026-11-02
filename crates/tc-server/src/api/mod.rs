//! Public HTTP API (role `api`). Holds no broker secrets: credentials submitted during
//! onboarding are sealed with the engine's public key immediately (ADR-0004).

mod auth;
mod routes;
mod ws;

use anyhow::Result;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::{header, request::Parts, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use axum_extra::extract::CookieJar;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine;
use rand::RngCore;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tc_store::Store;
use tc_vault::SealingKey;
use tower_http::limit::RequestBodyLimitLayer;
use url::Url;
use webauthn_rs::prelude::*;

use crate::config::Config;

pub const SESSION_COOKIE: &str = "__Host-tc_session";
pub const DEV_SESSION_COOKIE: &str = "tc_session";
pub const SESSION_IDLE: chrono::Duration = chrono::Duration::minutes(60);
pub const SESSION_ABSOLUTE: chrono::Duration = chrono::Duration::hours(12);

/// Pending WebAuthn ceremony state, kept in memory for at most 5 minutes.
pub enum Ceremony {
    Register {
        state: PasskeyRegistration,
        device_name: String,
        purpose: RegisterPurpose,
    },
    Login {
        state: PasskeyAuthentication,
    },
    StepUp {
        state: PasskeyAuthentication,
        action: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterPurpose {
    FirstSetup,
    Pairing,
    Recovery,
}

pub struct AppState {
    pub store: Store,
    pub webauthn: Webauthn,
    pub origin: String,
    pub sealing_key: Option<SealingKey>,
    pub vapid_public_key: Option<String>,
    pub secure_cookies: bool,
    pub ceremonies: Mutex<HashMap<String, (Instant, Ceremony)>>,
    pub auth_limiter: Mutex<(Instant, u32)>,
    pub events: tokio::sync::broadcast::Sender<String>,
}

impl AppState {
    pub fn new(store: Store, cfg: &Config) -> Result<Self> {
        let origin = Url::parse(&cfg.rp_origin)?;
        let webauthn = WebauthnBuilder::new(&cfg.rp_id, &origin)?
            .rp_name("Trading Center")
            .build()?;
        let sealing_key = match &cfg.engine_public_key {
            Some(k) => Some(
                SealingKey::from_base64(k)
                    .map_err(|e| anyhow::anyhow!("engine public key: {e}"))?,
            ),
            None => None,
        };
        let (tx, _) = tokio::sync::broadcast::channel(256);
        Ok(AppState {
            store,
            webauthn,
            origin: cfg.rp_origin.trim_end_matches('/').to_owned(),
            sealing_key,
            vapid_public_key: cfg.vapid_public_key.clone(),
            secure_cookies: !cfg.dev_insecure_cookies,
            ceremonies: Mutex::new(HashMap::new()),
            auth_limiter: Mutex::new((Instant::now(), 0)),
            events: tx,
        })
    }

    pub fn put_ceremony(&self, c: Ceremony) -> String {
        let id = random_token(24);
        if let Ok(mut m) = self.ceremonies.lock() {
            m.retain(|_, (t, _)| t.elapsed() < Duration::from_secs(300));
            m.insert(id.clone(), (Instant::now(), c));
        }
        id
    }

    pub fn take_ceremony(&self, id: &str) -> Option<Ceremony> {
        let mut m = self.ceremonies.lock().ok()?;
        let (t, c) = m.remove(id)?;
        (t.elapsed() < Duration::from_secs(300)).then_some(c)
    }

    /// Global auth endpoint limiter: 30 attempts per minute (single-user system).
    pub fn auth_rate_ok(&self) -> bool {
        let Ok(mut g) = self.auth_limiter.lock() else {
            return false;
        };
        if g.0.elapsed() > Duration::from_secs(60) {
            *g = (Instant::now(), 0);
        }
        g.1 += 1;
        g.1 <= 30
    }

    pub fn cookie_name(&self) -> &'static str {
        if self.secure_cookies {
            SESSION_COOKIE
        } else {
            DEV_SESSION_COOKIE
        }
    }
}

pub fn random_token(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut b);
    B64URL.encode(b)
}

// ---------- errors ----------
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        ApiError {
            status,
            code,
            message: message.into(),
        }
    }
    pub fn bad_request(m: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", m)
    }
    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication required",
        )
    }
    pub fn forbidden(m: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", m)
    }
    pub fn stepup() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "stepup_required",
            "passkey confirmation required for this action",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if self.status.is_server_error() {
            tracing::error!(code = self.code, message = %self.message, "api error");
        }
        let msg = if self.status.is_server_error() {
            "internal error".to_owned()
        } else {
            self.message
        };
        (
            self.status,
            Json(serde_json::json!({ "error": { "code": self.code, "message": msg } })),
        )
            .into_response()
    }
}

impl From<tc_store::StoreError> for ApiError {
    fn from(e: tc_store::StoreError) -> Self {
        match e {
            tc_store::StoreError::NotFound => {
                ApiError::new(StatusCode::NOT_FOUND, "not_found", "not found")
            }
            tc_store::StoreError::Conflict(m) => ApiError::new(StatusCode::CONFLICT, "conflict", m),
            tc_store::StoreError::Invalid(m) => {
                ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid", m)
            }
            other => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                other.to_string(),
            ),
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

// ---------- authenticated session extractor ----------
#[derive(Clone, Debug)]
pub struct Session {
    pub passkey_id: i64,
    pub token: Vec<u8>,
}

impl Session {
    pub fn actor(&self) -> String {
        format!("passkey:{}", self.passkey_id)
    }
}

impl FromRequestParts<Arc<AppState>> for Session {
    type Rejection = ApiError;
    async fn from_request_parts(
        parts: &mut Parts,
        st: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let raw = jar
            .get(st.cookie_name())
            .map(|c| c.value().to_owned())
            .ok_or_else(ApiError::unauthorized)?;
        let token = B64URL
            .decode(raw.as_bytes())
            .map_err(|_| ApiError::unauthorized())?;
        let row = st
            .store
            .touch_session(&token, SESSION_IDLE)
            .await?
            .ok_or_else(ApiError::unauthorized)?;
        Ok(Session {
            passkey_id: row.passkey_id.unwrap_or_default(),
            token,
        })
    }
}

/// Consume a step-up token bound to `action` from the `X-Stepup-Token` header.
pub async fn require_stepup(st: &AppState, headers: &HeaderMap, action: &str) -> ApiResult<i64> {
    let raw = headers
        .get("x-stepup-token")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(ApiError::stepup)?;
    let tok = B64URL
        .decode(raw.as_bytes())
        .map_err(|_| ApiError::stepup())?;
    match st
        .store
        .consume_token(&tok, tc_store::auth::TokenKind::Stepup, Some(action))
        .await
    {
        Ok(Some(pk)) => Ok(pk),
        _ => Err(ApiError::stepup()),
    }
}

// ---------- middleware ----------
/// CSRF defence: state-changing requests must carry `Origin` equal to the configured origin.
async fn origin_guard(State(st): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let m = req.method().clone();
    if m != Method::GET && m != Method::HEAD && m != Method::OPTIONS {
        let ok = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|o| o.trim_end_matches('/') == st.origin);
        if !ok {
            return ApiError::forbidden("origin check failed").into_response();
        }
    }
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    resp
}

pub fn router(st: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/health", get(routes::health))
        .route("/auth/status", get(auth::status))
        .route("/auth/setup/start", post(auth::setup_start))
        .route("/auth/setup/finish", post(auth::register_finish))
        .route("/auth/login/start", post(auth::login_start))
        .route("/auth/login/finish", post(auth::login_finish))
        .route("/auth/recover", post(auth::recover_start))
        .route("/auth/stepup/start", post(auth::stepup_start))
        .route("/auth/stepup/finish", post(auth::stepup_finish))
        .route("/auth/pair", post(auth::pair_code))
        .route("/auth/pair/start", post(auth::pair_start))
        .route("/auth/devices", get(auth::devices))
        .route("/auth/devices/{id}", delete(auth::device_remove))
        .route("/auth/logout", post(auth::logout))
        .route("/overview", get(routes::overview))
        .route("/agents", get(routes::agents))
        .route("/positions", get(routes::positions))
        .route("/positions/sell", post(routes::manual_sell))
        .route("/orders", get(routes::orders))
        .route("/trades", get(routes::trades))
        .route("/transactions", get(routes::transactions))
        .route("/decisions", get(routes::decisions))
        .route("/decisions/{id}", get(routes::decision))
        .route("/instruments/{id}/chart", get(routes::chart))
        .route("/performance", get(routes::performance))
        .route("/strategies", get(routes::strategies))
        .route("/plan", get(routes::plan_get).put(routes::plan_put))
        .route("/risk", get(routes::risk_get).put(routes::risk_put))
        .route("/control/kill", post(routes::kill))
        .route("/control/live", post(routes::live))
        .route("/onboarding", get(routes::onboarding))
        .route("/onboarding/credentials", post(routes::credentials))
        .route("/onboarding/verify", post(routes::verify))
        .route("/onboarding/attest", post(routes::attest))
        .route("/onboarding/risk", post(routes::onboarding_risk))
        .route("/onboarding/authorize", post(routes::authorize))
        .route("/push/vapid", get(routes::vapid))
        .route(
            "/push/subscribe",
            post(routes::push_subscribe).delete(routes::push_unsubscribe),
        )
        .route(
            "/notifications/settings",
            get(routes::notif_get).put(routes::notif_put),
        )
        .route("/ws", get(ws::handler));
    Router::new()
        .nest("/api", api)
        .layer(middleware::from_fn_with_state(st.clone(), origin_guard))
        .layer(RequestBodyLimitLayer::new(64 * 1024))
        .with_state(st)
}

pub async fn serve(
    st: Arc<AppState>,
    bind: &str,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    tokio::spawn(ws::pump_events(st.clone()));
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "api listening");
    axum::serve(listener, router(st))
        .with_graceful_shutdown(async move {
            let _ = shutdown.changed().await;
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;

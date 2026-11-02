//! WebAuthn passkey authentication (ADR-0004). User verification is required by
//! webauthn-rs passkey ceremonies (Face ID / Windows Hello).
//!
//! * First passkey: single-use setup token from `tc setup-token` (only while no passkey exists).
//! * Further devices: pairing code issued by an authenticated session.
//! * Recovery: one of the 10 single-use recovery codes shown at setup.
//! * Step-up: a fresh assertion bound to one action yields a 120 s single-use token.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use axum_extra::extract::cookie::{Cookie, SameSite};
use axum_extra::extract::CookieJar;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tc_store::auth::TokenKind;
use webauthn_rs::prelude::*;

use super::{
    random_token, require_stepup, ApiError, ApiResult, AppState, Ceremony, RegisterPurpose,
    Session, SESSION_ABSOLUTE, SESSION_IDLE,
};

pub const STEPUP_ACTIONS: &[&str] = &[
    "live_enable",
    "risk_change",
    "manual_sell",
    "kill_release",
    "credentials_change",
    "device_remove",
    "plan_change",
    "flatten",
    "device_add",
];

fn limited(st: &AppState) -> ApiResult<()> {
    if st.auth_rate_ok() {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "too many authentication attempts",
        ))
    }
}

fn decode_token(s: &str) -> ApiResult<Vec<u8>> {
    B64URL
        .decode(s.trim().as_bytes())
        .map_err(|_| ApiError::forbidden("invalid token"))
}

fn device_name(s: &str) -> String {
    let n: String = s.chars().filter(|c| !c.is_control()).take(60).collect();
    if n.trim().is_empty() {
        "Eszköz".into()
    } else {
        n
    }
}

pub async fn status(State(st): State<Arc<AppState>>, jar: CookieJar) -> ApiResult<Json<Value>> {
    let count = st.store.passkey_count().await?;
    let authed = match jar
        .get(st.cookie_name())
        .and_then(|c| B64URL.decode(c.value().as_bytes()).ok())
    {
        Some(t) => st.store.touch_session(&t, SESSION_IDLE).await?.is_some(),
        None => false,
    };
    let (_, name) = st.store.ensure_user("Trading Center").await?;
    Ok(Json(json!({
        "setup_required": count == 0,
        "authenticated": authed,
        "user": if authed { json!({ "display_name": name }) } else { Value::Null },
        "devices": if authed { json!(count) } else { Value::Null },
    })))
}

async fn start_registration(
    st: &AppState,
    device: &str,
    purpose: RegisterPurpose,
) -> ApiResult<Json<Value>> {
    let (uuid, name) = st.store.ensure_user("Trading Center").await?;
    let user_id = Uuid::parse_str(&uuid).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let existing: Vec<CredentialID> = st
        .store
        .active_passkeys()
        .await?
        .iter()
        .filter_map(|p| serde_json::from_str::<Passkey>(&p.passkey_json).ok())
        .map(|p| p.cred_id().clone())
        .collect();
    let (ccr, state) = st
        .webauthn
        .start_passkey_registration(user_id, &name, &name, Some(existing))
        .map_err(|e| ApiError::bad_request(format!("webauthn: {e}")))?;
    let id = st.put_ceremony(Ceremony::Register {
        state,
        device_name: device_name(device),
        purpose,
    });
    Ok(Json(json!({ "challenge_id": id, "options": ccr })))
}

#[derive(Deserialize)]
pub struct SetupStart {
    setup_token: String,
    device_name: String,
}

pub async fn setup_start(
    State(st): State<Arc<AppState>>,
    Json(b): Json<SetupStart>,
) -> ApiResult<Json<Value>> {
    limited(&st)?;
    if st.store.passkey_count().await? > 0 {
        return Err(ApiError::forbidden(
            "setup already completed; use pairing or recovery",
        ));
    }
    st.store
        .consume_token(&decode_token(&b.setup_token)?, TokenKind::Setup, None)
        .await
        .map_err(|_| ApiError::forbidden("setup token invalid, used or expired"))?;
    start_registration(&st, &b.device_name, RegisterPurpose::FirstSetup).await
}

#[derive(Deserialize)]
pub struct PairStart {
    pairing_code: String,
    device_name: String,
}

pub async fn pair_start(
    State(st): State<Arc<AppState>>,
    Json(b): Json<PairStart>,
) -> ApiResult<Json<Value>> {
    limited(&st)?;
    st.store
        .consume_token(&decode_token(&b.pairing_code)?, TokenKind::Pairing, None)
        .await
        .map_err(|_| ApiError::forbidden("pairing code invalid, used or expired"))?;
    start_registration(&st, &b.device_name, RegisterPurpose::Pairing).await
}

#[derive(Deserialize)]
pub struct RecoverStart {
    recovery_code: String,
    device_name: String,
}

pub async fn recover_start(
    State(st): State<Arc<AppState>>,
    Json(b): Json<RecoverStart>,
) -> ApiResult<Json<Value>> {
    limited(&st)?;
    let code: String = b
        .recovery_code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    st.store
        .consume_token(code.as_bytes(), TokenKind::Recovery, None)
        .await
        .map_err(|_| ApiError::forbidden("recovery code invalid or used"))?;
    st.store
        .audit("recovery", "recovery_code_used", &json!({}))
        .await?;
    start_registration(&st, &b.device_name, RegisterPurpose::Recovery).await
}

#[derive(Deserialize)]
pub struct Finish<T> {
    challenge_id: String,
    credential: T,
    #[serde(default)]
    action: Option<String>,
}

fn session_cookie(st: &AppState, token: &[u8]) -> Cookie<'static> {
    let mut c = Cookie::new(st.cookie_name(), B64URL.encode(token));
    c.set_http_only(true);
    c.set_secure(st.secure_cookies);
    c.set_same_site(SameSite::Strict);
    c.set_path("/");
    c
}

async fn new_session(
    st: &AppState,
    jar: CookieJar,
    passkey_id: i64,
    headers: &HeaderMap,
) -> ApiResult<CookieJar> {
    let mut token = vec![0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut token);
    let ua = headers.get("user-agent").and_then(|v| v.to_str().ok());
    st.store
        .create_session(&token, passkey_id, SESSION_IDLE, SESSION_ABSOLUTE, ua)
        .await?;
    Ok(jar.add(session_cookie(st, &token)))
}

pub async fn register_finish(
    State(st): State<Arc<AppState>>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(b): Json<Finish<RegisterPublicKeyCredential>>,
) -> ApiResult<(CookieJar, Json<Value>)> {
    limited(&st)?;
    let Some(Ceremony::Register {
        state,
        device_name,
        purpose,
    }) = st.take_ceremony(&b.challenge_id)
    else {
        return Err(ApiError::forbidden("unknown or expired challenge"));
    };
    let pk = st
        .webauthn
        .finish_passkey_registration(&b.credential, &state)
        .map_err(|e| ApiError::forbidden(format!("registration failed: {e}")))?;
    let pj = serde_json::to_string(&pk).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let id = st
        .store
        .insert_passkey(pk.cred_id().as_ref(), &pj, &device_name)
        .await?;
    if purpose != RegisterPurpose::FirstSetup {
        let title = if purpose == RegisterPurpose::Recovery {
            "Helyreállító kóddal új passkey"
        } else {
            "Új eszköz párosítva"
        };
        let _ = st
            .store
            .enqueue_command(
                "notify",
                &json!({ "category": "critical_risk", "title": title, "body": device_name }),
                &format!("passkey:{id}"),
                None,
            )
            .await;
    }
    if purpose == RegisterPurpose::Recovery {
        // Recovery may mean a lost or stolen device: end every session and stop new entries.
        st.store.revoke_all_sessions().await?;
        st.store
            .set_kill(
                "global",
                tc_risk::KillMode::ReduceOnly,
                "account recovery",
                &format!("passkey:{id}"),
            )
            .await?;
    }
    let mut recovery_codes = Vec::new();
    if purpose == RegisterPurpose::FirstSetup {
        st.store.invalidate_tokens(TokenKind::Setup).await?;
        for _ in 0..10 {
            let code = recovery_code();
            st.store
                .insert_token(
                    code.replace('-', "").as_bytes(),
                    TokenKind::Recovery,
                    None,
                    None,
                    None,
                )
                .await?;
            recovery_codes.push(code);
        }
    }
    st.store
        .audit(
            &format!("passkey:{id}"),
            "passkey_registered",
            &json!({ "device": device_name, "purpose": format!("{purpose:?}") }),
        )
        .await?;
    let jar = new_session(&st, jar, id, &headers).await?;
    Ok((
        jar,
        Json(json!({ "ok": true, "recovery_codes": recovery_codes })),
    ))
}

/// 20 chars from an unambiguous alphabet (~100 bits), grouped for readability.
fn recovery_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut b = [0u8; 20];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut b);
    let s: String = b
        .iter()
        .map(|x| ALPHABET[(*x as usize) % ALPHABET.len()] as char)
        .collect();
    format!(
        "{}-{}-{}-{}-{}",
        &s[0..4],
        &s[4..8],
        &s[8..12],
        &s[12..16],
        &s[16..20]
    )
}

async fn passkeys(st: &AppState) -> ApiResult<Vec<(i64, Passkey)>> {
    Ok(st
        .store
        .active_passkeys()
        .await?
        .into_iter()
        .filter_map(|p| {
            serde_json::from_str::<Passkey>(&p.passkey_json)
                .ok()
                .map(|k| (p.id, k))
        })
        .collect())
}

pub async fn login_start(State(st): State<Arc<AppState>>) -> ApiResult<Json<Value>> {
    limited(&st)?;
    let keys: Vec<Passkey> = passkeys(&st).await?.into_iter().map(|(_, k)| k).collect();
    if keys.is_empty() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "setup_required",
            "no passkey registered yet",
        ));
    }
    let (rcr, state) = st
        .webauthn
        .start_passkey_authentication(&keys)
        .map_err(|e| ApiError::bad_request(format!("webauthn: {e}")))?;
    let id = st.put_ceremony(Ceremony::Login { state });
    Ok(Json(json!({ "challenge_id": id, "options": rcr })))
}

/// Verify an assertion, persist counter updates; returns the passkey row id.
async fn verify_assertion(
    st: &AppState,
    state: &PasskeyAuthentication,
    cred: &PublicKeyCredential,
) -> ApiResult<i64> {
    let res = st
        .webauthn
        .finish_passkey_authentication(cred, state)
        .map_err(|e| ApiError::forbidden(format!("authentication failed: {e}")))?;
    let keys = passkeys(st).await?;
    let Some((id, mut pk)) = keys.into_iter().find(|(_, k)| k.cred_id() == res.cred_id()) else {
        return Err(ApiError::forbidden("unknown credential"));
    };
    if pk.update_credential(&res).is_some() {
        let pj = serde_json::to_string(&pk).map_err(|e| ApiError::bad_request(e.to_string()))?;
        st.store.update_passkey(id, &pj).await?;
    }
    Ok(id)
}

pub async fn login_finish(
    State(st): State<Arc<AppState>>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(b): Json<Finish<PublicKeyCredential>>,
) -> ApiResult<(CookieJar, Json<Value>)> {
    limited(&st)?;
    let Some(Ceremony::Login { state }) = st.take_ceremony(&b.challenge_id) else {
        return Err(ApiError::forbidden("unknown or expired challenge"));
    };
    let id = verify_assertion(&st, &state, &b.credential).await?;
    st.store
        .audit(&format!("passkey:{id}"), "login", &json!({}))
        .await?;
    let jar = new_session(&st, jar, id, &headers).await?;
    Ok((jar, Json(json!({ "ok": true }))))
}

#[derive(Deserialize)]
pub struct StepStart {
    action: String,
}

pub async fn stepup_start(
    State(st): State<Arc<AppState>>,
    _s: Session,
    Json(b): Json<StepStart>,
) -> ApiResult<Json<Value>> {
    limited(&st)?;
    if !STEPUP_ACTIONS.contains(&b.action.as_str()) {
        return Err(ApiError::bad_request("unknown step-up action"));
    }
    let keys: Vec<Passkey> = passkeys(&st).await?.into_iter().map(|(_, k)| k).collect();
    let (rcr, state) = st
        .webauthn
        .start_passkey_authentication(&keys)
        .map_err(|e| ApiError::bad_request(format!("webauthn: {e}")))?;
    let id = st.put_ceremony(Ceremony::StepUp {
        state,
        action: b.action,
    });
    Ok(Json(json!({ "challenge_id": id, "options": rcr })))
}

pub async fn stepup_finish(
    State(st): State<Arc<AppState>>,
    _s: Session,
    Json(b): Json<Finish<PublicKeyCredential>>,
) -> ApiResult<Json<Value>> {
    limited(&st)?;
    let Some(Ceremony::StepUp { state, action }) = st.take_ceremony(&b.challenge_id) else {
        return Err(ApiError::forbidden("unknown or expired challenge"));
    };
    if b.action.as_deref().is_some_and(|a| a != action) {
        return Err(ApiError::forbidden("step-up action mismatch"));
    }
    let id = verify_assertion(&st, &state, &b.credential).await?;
    let mut tok = vec![0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut tok);
    st.store
        .insert_token(
            &tok,
            TokenKind::Stepup,
            Some(&action),
            Some(id),
            Some(chrono::Duration::seconds(120)),
        )
        .await?;
    st.store
        .audit(
            &format!("passkey:{id}"),
            "stepup",
            &json!({ "action": action }),
        )
        .await?;
    Ok(Json(
        json!({ "stepup_token": B64URL.encode(tok), "expires_at": chrono::Utc::now() + chrono::Duration::seconds(120) }),
    ))
}

pub async fn pair_code(
    State(st): State<Arc<AppState>>,
    s: Session,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    // Adding a device grants full control, so it needs a fresh passkey assertion (SEC-07).
    require_stepup(&st, &headers, "device_add").await?;
    let code = random_token(12);
    st.store
        .insert_token(
            &decode_token(&code)?,
            TokenKind::Pairing,
            None,
            Some(s.passkey_id),
            Some(chrono::Duration::minutes(10)),
        )
        .await?;
    st.store
        .audit(&s.actor(), "pairing_code_issued", &json!({}))
        .await?;
    Ok(Json(
        json!({ "pairing_code": code, "expires_at": chrono::Utc::now() + chrono::Duration::minutes(10) }),
    ))
}

pub async fn devices(State(st): State<Arc<AppState>>, _s: Session) -> ApiResult<Json<Value>> {
    let rows = st.store.active_passkeys().await?;
    Ok(Json(json!(rows
        .iter()
        .map(|p| json!({ "id": p.id, "name": p.device_name, "created_at": p.created_at, "last_used_at": p.last_used_at }))
        .collect::<Vec<_>>())))
}

pub async fn device_remove(
    State(st): State<Arc<AppState>>,
    s: Session,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    require_stepup(&st, &headers, "device_remove").await?;
    if st.store.passkey_count().await? <= 1 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "last_device",
            "cannot remove the last passkey",
        ));
    }
    st.store.revoke_passkey(id).await?;
    st.store
        .audit(&s.actor(), "passkey_revoked", &json!({ "id": id }))
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn logout(
    State(st): State<Arc<AppState>>,
    s: Session,
    jar: CookieJar,
) -> ApiResult<(CookieJar, Json<Value>)> {
    st.store.revoke_session(&s.token).await?;
    let mut c = Cookie::from(st.cookie_name());
    c.set_path("/");
    c.set_secure(st.secure_cookies);
    Ok((jar.remove(c), Json(json!({ "ok": true }))))
}

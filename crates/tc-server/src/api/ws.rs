//! Authenticated WebSocket fan-out of engine events (server → client only).
//! The `Origin` header must match the configured origin (CSWSH defence); the session is
//! re-validated every 60 s so revoked sessions are disconnected.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;

use super::{ApiError, AppState, Session, SESSION_IDLE};

pub async fn handler(
    State(st): State<Arc<AppState>>,
    s: Session,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let ok = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|o| o.trim_end_matches('/') == st.origin);
    if !ok {
        return Err(ApiError::forbidden("origin check failed"));
    }
    Ok(ws
        .max_message_size(4096)
        .on_upgrade(move |socket| run(st, s, socket)))
}

async fn run(st: Arc<AppState>, s: Session, socket: WebSocket) {
    let (mut tx, mut rx) = socket.split();
    let mut events = st.events.subscribe();
    let mut recheck = tokio::time::interval(Duration::from_secs(60));
    recheck.tick().await;
    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Ok(text) => {
                    if tx.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Client fell behind: tell it to refetch everything.
                    let msg = serde_json::json!({ "type": "overview", "data": { "refetch": true }, "ts": chrono::Utc::now() }).to_string();
                    if tx.send(Message::Text(msg.into())).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            },
            msg = rx.next() => match msg {
                Some(Ok(Message::Text(t))) if t.as_str().contains("\"ping\"") => {
                    let pong = serde_json::json!({ "type": "pong", "ts": chrono::Utc::now() }).to_string();
                    if tx.send(Message::Text(pong.into())).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            _ = recheck.tick() => {
                match st.store.touch_session(&s.token, SESSION_IDLE).await {
                    Ok(Some(_)) => {}
                    _ => break,
                }
            }
        }
    }
    let _ = tx.close().await;
}

/// Poll `tc_events` (written by the engine) and broadcast to connected sockets.
pub async fn pump_events(st: Arc<AppState>) {
    let mut last = st.store.last_event_id().await.unwrap_or(0);
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tick.tick().await;
        match st.store.events_after(last, 200).await {
            Ok(rows) => {
                for e in rows {
                    last = e.id;
                    let msg = serde_json::json!({ "type": e.r#type, "data": e.data, "ts": e.ts })
                        .to_string();
                    let _ = st.events.send(msg);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "event poll failed");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

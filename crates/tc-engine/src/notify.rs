//! Web Push notifications (VAPID, RFC 8291 aes128gcm via the `web-push` crate) sent with
//! reqwest. Categories follow the user's settings: trades, market_open, errors,
//! critical_risk, daily_summary. Push failures never affect trading.

use chrono::{NaiveTime, Utc};
use chrono_tz::Europe::Budapest;
use serde::Deserialize;
use serde_json::json;
use tracing::{info, warn};
use web_push::{ContentEncoding, SubscriptionInfo, VapidSignatureBuilder, WebPushMessageBuilder};

use crate::Engine;

#[derive(Clone)]
pub struct Push {
    private_key_b64: String,
    subject: String,
    http: reqwest::Client,
}

#[derive(Deserialize, Clone)]
struct Settings {
    trades: bool,
    market_open: bool,
    errors: bool,
    critical_risk: bool,
    daily_summary: bool,
    daily_summary_time: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            trades: true,
            market_open: false,
            errors: true,
            critical_risk: true,
            daily_summary: true,
            daily_summary_time: "21:00".into(),
        }
    }
}

impl Push {
    pub fn load(path: Option<&str>, subject: &str) -> Option<Push> {
        let key = std::fs::read_to_string(path?).ok()?.trim().to_owned();
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .ok()?;
        Some(Push {
            private_key_b64: key,
            subject: subject.to_owned(),
            http,
        })
    }

    async fn deliver(
        &self,
        sub: &tc_store::control::PushSub,
        payload: &[u8],
    ) -> Result<u16, String> {
        let info =
            SubscriptionInfo::new(sub.endpoint.clone(), sub.p256dh.clone(), sub.auth.clone());
        let mut sig = VapidSignatureBuilder::from_base64(&self.private_key_b64, &info)
            .map_err(|e| e.to_string())?;
        sig.add_claim("sub", self.subject.as_str());
        let sig = sig.build().map_err(|e| e.to_string())?;
        let mut b = WebPushMessageBuilder::new(&info);
        b.set_ttl(3600);
        b.set_payload(ContentEncoding::Aes128Gcm, payload);
        b.set_vapid_signature(sig);
        let msg = b.build().map_err(|e| e.to_string())?;
        let req = web_push::request_builder::build_request::<Vec<u8>>(msg);
        let (parts, body) = req.into_parts();
        let mut r = self.http.post(parts.uri.to_string());
        for (k, v) in parts.headers.iter() {
            if let Ok(v) = v.to_str() {
                r = r.header(k.as_str(), v);
            }
        }
        let resp = r.body(body).send().await.map_err(|e| e.to_string())?;
        Ok(resp.status().as_u16())
    }
}

async fn settings(eng: &Engine) -> Settings {
    eng.store
        .effective_config::<Settings>("notifications", Utc::now())
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Send a push in `category` if enabled. Also mirrored to the WebSocket stream.
pub async fn send(eng: &Engine, category: &str, title: &str, body: &str) {
    eng.event(
        "alert",
        json!({ "category": category, "title": title, "body": body }),
    )
    .await;
    let s = settings(eng).await;
    let enabled = match category {
        "trades" | "listing" => s.trades,
        "market_open" => s.market_open,
        "errors" => s.errors,
        "critical_risk" => s.critical_risk,
        "daily_summary" => s.daily_summary,
        _ => true,
    };
    let Some(push) = &eng.push else { return };
    if !enabled {
        return;
    }
    let payload =
        json!({ "title": title, "body": body, "category": category, "url": "/" }).to_string();
    // Delivery runs in the background so trading paths (execute, flatten) never wait on it.
    let subs = eng.store.push_subs().await.unwrap_or_default();
    let (push, store) = (push.clone(), eng.store.clone());
    tokio::spawn(async move {
        for sub in subs {
            match push.deliver(&sub, payload.as_bytes()).await {
                Ok(404) | Ok(410) => {
                    info!(device = %sub.device_name, "push subscription expired; removing");
                    let _ = store.remove_push_sub(&sub.endpoint).await;
                }
                Ok(code) if (200..300).contains(&code) => {}
                Ok(code) => warn!(code, device = %sub.device_name, "push rejected"),
                Err(e) => warn!(error = %e, "push failed"),
            }
        }
    });
}

/// Errors / critical risk alerts (also audited).
pub async fn alert(eng: &Engine, category: &str, title: &str, body: &str) {
    let _ = eng
        .store
        .audit(
            "engine",
            "alert",
            &json!({ "category": category, "title": title, "body": body }),
        )
        .await;
    send(eng, category, &format!("⚠️ {title}"), body).await;
}

/// Daily Hungarian summary from actual net P&L (equity change minus deposits/withdrawals).
pub async fn daily_summary_if_due(eng: &Engine) {
    let s = settings(eng).await;
    let now = Utc::now();
    let local = now.with_timezone(&Budapest);
    let Ok(at) = NaiveTime::parse_from_str(&s.daily_summary_time, "%H:%M") else {
        return;
    };
    if local.time() < at {
        return;
    }
    let today = local.date_naive();
    {
        let mut sent = eng.summary_sent_for.lock().await;
        if *sent == Some(today) {
            return;
        }
        *sent = Some(today);
    }
    let day0 = crate::decide::budapest_midnight(now);
    let (Ok(Some(end)), Ok(start)) = (
        eng.store.latest_snapshot().await,
        eng.store.snapshot_at(day0).await,
    ) else {
        return;
    };
    let start_eq = start.map(|s| s.equity_huf).unwrap_or(end.equity_huf);
    let flows = eng.store.net_flows_huf(day0, now).await.unwrap_or_default();
    let pnl = tc_plan::net_pnl(start_eq, end.equity_huf, flows);
    let text = tc_plan::daily_summary_hu(pnl);
    send(
        eng,
        "daily_summary",
        &text,
        "Tényleges nettó eredmény (befizetések nélkül)",
    )
    .await;
}

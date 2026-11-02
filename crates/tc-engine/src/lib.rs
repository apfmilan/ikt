//! Autonomous engine (role `engine`). One instance at a time (MySQL named lock).
//!
//! Boot is fail-closed: the engine starts in RECOVERY (no entries), reconciles broker state
//! against the database, and only then switches to RUNNING. Every order passes the
//! deterministic risk engine immediately before it is persisted, and the kill switch is
//! re-read from MySQL immediately before every send (ADR-0001).
//!
//! Runtime agents (each reports to `tc_agent_status`):
//! scanner · technical · regime · volatility · listings · ranker · portfolio · exit · risk ·
//! execution · reconciler · long_term_invest · revolut_watch · news (not configured).

mod creds;
mod decide;
mod exec;
mod market;
mod notify;

use anyhow::Result;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tc_core::{Broker, InstrumentId};
use tc_marketdata::FxTable;
use tc_risk::{RiskConfig, RiskEngine};
use tc_store::control::AgentStatusRow;
use tc_store::Store;
use tc_strategy::RegimeTracker;
use tc_vault::OpeningKey;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

pub use creds::Brokers;

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub secret_key_file: Option<String>,
    pub vapid_private_key_file: Option<String>,
    pub vapid_subject: String,
    pub live_trading_flag: Option<String>,
    pub health_bind: Option<String>,
    pub version: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Recovery,
    Running,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Recovery => "recovery",
            Mode::Running => "running",
        }
    }
}

pub struct Engine {
    pub store: Store,
    pub cfg: EngineConfig,
    pub key: Option<OpeningKey>,
    pub brokers: RwLock<Brokers>,
    pub public_bitvavo: Arc<tc_broker_bitvavo::BitvavoClient>,
    pub fx: RwLock<Option<FxTable>>,
    pub regimes: Mutex<HashMap<InstrumentId, RegimeTracker>>,
    pub listed_at: Mutex<HashMap<InstrumentId, DateTime<Utc>>>,
    pub universe: RwLock<Vec<InstrumentId>>,
    pub mode: RwLock<Mode>,
    pub recon_mismatch: RwLock<Option<String>>,
    pub last_account_sync: RwLock<HashMap<Broker, DateTime<Utc>>>,
    pub last_decision_bar: Mutex<HashMap<InstrumentId, DateTime<Utc>>>,
    pub push: Option<notify::Push>,
    pub summary_sent_for: Mutex<Option<NaiveDate>>,
    pub fees: RwLock<HashMap<Broker, Decimal>>,
    pub live_armed: bool,
}

impl Engine {
    pub async fn risk_engine(&self) -> (RiskEngine, RiskConfig, bool) {
        let (cfg, exists) = match self
            .store
            .effective_config::<RiskConfig>("risk", Utc::now())
            .await
        {
            Ok(Some(c)) => (c, true),
            _ => (RiskConfig::default(), false),
        };
        let engine = RiskEngine::new(cfg.clone()).unwrap_or_else(|_| {
            // Stored config violating hard limits is ignored in favour of defaults (fail safe).
            RiskEngine::new(RiskConfig::default())
                .unwrap_or_else(|_| unreachable!("default config is valid"))
        });
        (engine, cfg, exists)
    }

    pub async fn agent(
        &self,
        agent: &str,
        status: &str,
        started: std::time::Instant,
        summary: impl Into<String>,
        metrics: serde_json::Value,
    ) {
        let row = AgentStatusRow {
            agent: agent.to_owned(),
            status: status.to_owned(),
            last_run_at: tc_store::now_micros(),
            last_duration_ms: started.elapsed().as_millis() as i64,
            summary: summary.into(),
            metrics,
        };
        if let Err(e) = self.store.put_agent_status(&row).await {
            warn!(error = %e, agent, "agent status write failed");
        }
        let _ = self
            .store
            .push_event("agent", &json!({ "agent": agent, "status": status }))
            .await;
    }

    pub async fn event(&self, t: &str, data: serde_json::Value) {
        if let Err(e) = self.store.push_event(t, &data).await {
            warn!(error = %e, "event write failed");
        }
    }

    pub async fn mode(&self) -> Mode {
        *self.mode.read().await
    }
}

/// Engine entry point; returns only on shutdown or fatal error (Docker restarts it).
pub async fn run(
    store: Store,
    cfg: EngineConfig,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    // Single writer: wait for the lock (another engine may still be shutting down).
    let _lock = loop {
        match store.acquire_engine_lock().await {
            Ok(l) => break l,
            Err(e) => {
                warn!(error = %e, "engine lock busy; retrying in 10 s");
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {}
                    _ = shutdown.changed() => return Ok(()),
                }
            }
        }
    };
    let key = match &cfg.secret_key_file {
        Some(p) => match std::fs::read_to_string(p)
            .map_err(anyhow::Error::from)
            .and_then(|s| OpeningKey::from_base64(&s).map_err(|e| anyhow::anyhow!("{e}")))
        {
            Ok(k) => Some(k),
            Err(e) => {
                error!(error = %e, "engine secret key unreadable; broker credentials cannot be opened");
                None
            }
        },
        None => {
            warn!("TC_ENGINE_SECRET_KEY_FILE not set: running without broker credentials (market data only)");
            None
        }
    };
    let live_armed = tc_broker::live::arm(cfg.live_trading_flag.as_deref());
    info!(
        live_armed,
        "live order sending {}",
        if live_armed {
            "ARMED (TC_LIVE_TRADING=enabled)"
        } else {
            "disarmed"
        }
    );
    let push = notify::Push::load(cfg.vapid_private_key_file.as_deref(), &cfg.vapid_subject);
    let public_bitvavo = Arc::new(tc_broker_bitvavo::BitvavoClient::new(
        tc_broker_bitvavo::BitvavoConfig::new(tc_broker_bitvavo::DEFAULT_BASE_URL),
    )?);
    let eng = Arc::new(Engine {
        store: store.clone(),
        cfg: cfg.clone(),
        key,
        brokers: RwLock::new(Brokers::default()),
        public_bitvavo,
        fx: RwLock::new(None),
        regimes: Mutex::new(HashMap::new()),
        listed_at: Mutex::new(HashMap::new()),
        universe: RwLock::new(Vec::new()),
        mode: RwLock::new(Mode::Recovery),
        recon_mismatch: RwLock::new(None),
        last_account_sync: RwLock::new(HashMap::new()),
        last_decision_bar: Mutex::new(HashMap::new()),
        push,
        summary_sent_for: Mutex::new(None),
        fees: RwLock::new(HashMap::new()),
        live_armed,
    });

    if let Some(bind) = cfg.health_bind.clone() {
        tokio::spawn(health_server(eng.clone(), bind));
    }
    let failed = store.fail_stale_running_commands().await.unwrap_or(0);
    if failed > 0 {
        warn!(
            failed,
            "commands interrupted by a previous crash were marked failed (not re-run)"
        );
    }
    store
        .audit(
            "engine",
            "engine_start",
            &json!({ "version": cfg.version, "live_armed": live_armed }),
        )
        .await
        .ok();
    heartbeat(&eng).await;

    // Boot sequence in RECOVERY mode.
    creds::load_all(&eng).await;
    market::refresh_fx(&eng).await;
    market::refresh_instruments(&eng).await;
    let clean = exec::reconcile(&eng).await;
    if clean {
        *eng.mode.write().await = Mode::Running;
        info!("reconciliation clean: RUNNING");
    } else {
        warn!("reconciliation not clean: staying in RECOVERY (entries blocked)");
    }
    eng.event("mode", json!({ "engine": eng.mode().await.as_str() }))
        .await;

    let mut t_cmd = tokio::time::interval(std::time::Duration::from_millis(250));
    let mut t_heartbeat = tokio::time::interval(std::time::Duration::from_secs(15));
    let mut t_recon = tokio::time::interval(std::time::Duration::from_secs(60));
    let mut t_market = tokio::time::interval(std::time::Duration::from_secs(300));
    let mut t_exit = tokio::time::interval(std::time::Duration::from_secs(30));
    let mut t_slow = tokio::time::interval(std::time::Duration::from_secs(3600));
    for t in [
        &mut t_cmd,
        &mut t_heartbeat,
        &mut t_recon,
        &mut t_market,
        &mut t_exit,
        &mut t_slow,
    ] {
        t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    }
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                info!("engine shutting down");
                store.audit("engine", "engine_stop", &json!({})).await.ok();
                return Ok(());
            }
            _ = t_cmd.tick() => exec::process_commands(&eng).await,
            _ = t_heartbeat.tick() => {
                heartbeat(&eng).await;
                notify::daily_summary_if_due(&eng).await;
            }
            _ = t_recon.tick() => {
                let clean = exec::reconcile(&eng).await;
                let mut m = eng.mode.write().await;
                if clean && *m == Mode::Recovery {
                    *m = Mode::Running;
                    drop(m);
                    info!("reconciliation clean: RUNNING");
                    eng.event("mode", json!({ "engine": "running" })).await;
                }
            }
            _ = t_market.tick() => {
                market::scan(&eng).await;
                decide::cycle(&eng).await;
            }
            _ = t_exit.tick() => exec::manage_exits(&eng).await,
            _ = t_slow.tick() => {
                market::refresh_fx(&eng).await;
                market::refresh_instruments(&eng).await;
                creds::periodic_verify(&eng).await;
                decide::long_term_invest(&eng).await;
                let _ = store.prune_events(Utc::now() - Duration::days(7)).await;
            }
        }
    }
}

async fn heartbeat(eng: &Engine) {
    let mode = eng.mode().await;
    let detail =
        json!({ "live_armed": eng.live_armed, "recon": eng.recon_mismatch.read().await.clone() });
    if let Err(e) = eng
        .store
        .heartbeat("engine", mode.as_str(), &eng.cfg.version, &detail)
        .await
    {
        error!(error = %e, "heartbeat failed (DB unavailable?)");
    }
}

/// Minimal health endpoint for the container healthcheck (no framework needed).
async fn health_server(eng: Arc<Engine>, bind: String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Ok(listener) = tokio::net::TcpListener::bind(&bind).await else {
        error!(%bind, "engine health listener failed to bind");
        return;
    };
    loop {
        let Ok((mut sock, _)) = listener.accept().await else {
            continue;
        };
        let eng = eng.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(2), sock.read(&mut buf)).await;
            let ok = eng.store.ping().await;
            let body = json!({ "status": if ok { "ok" } else { "db_down" }, "mode": eng.mode().await.as_str() }).to_string();
            let status = if ok {
                "200 OK"
            } else {
                "503 Service Unavailable"
            };
            let resp = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = sock.write_all(resp.as_bytes()).await;
        });
    }
}

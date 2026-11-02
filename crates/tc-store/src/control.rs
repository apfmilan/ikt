//! Control plane: kill switch, live authorization, configs, api→engine commands,
//! engine→api events, heartbeats, sealed credentials, push subscriptions, agent status.

use chrono::{DateTime, Duration, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sqlx::Row;
use std::collections::BTreeMap;
use tc_core::Broker;
use tc_risk::KillMode;

use crate::{now_micros, sha256, Store, StoreError, StoreResult};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct KillState {
    pub global: KillMode,
    pub brokers: BTreeMap<Broker, KillMode>,
    pub strategies: BTreeMap<String, KillMode>,
}

pub fn parse_kill(s: &str) -> KillMode {
    match s {
        "normal" => KillMode::Normal,
        "reduce_only" => KillMode::ReduceOnly,
        "flatten" => KillMode::Flatten,
        // Unknown values fail safe.
        _ => KillMode::Freeze,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Command {
    pub id: i64,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_by: String,
    pub stepup_action: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventRow {
    pub id: i64,
    pub ts: DateTime<Utc>,
    pub r#type: String,
    pub data: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CredentialStatusRow {
    pub broker: String,
    pub authenticated: bool,
    pub can_trade: bool,
    pub can_withdraw: Option<bool>,
    pub attested_no_withdraw: bool,
    pub scopes_ok: bool,
    pub account_currency: Option<String>,
    pub notes: serde_json::Value,
    pub checked_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PushSub {
    pub id: i64,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub device_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentStatusRow {
    pub agent: String,
    pub status: String,
    pub last_run_at: DateTime<Utc>,
    pub last_duration_ms: i64,
    pub summary: String,
    pub metrics: serde_json::Value,
}

fn json_of(r: &sqlx::mysql::MySqlRow, col: &str) -> StoreResult<serde_json::Value> {
    let s: Option<String> = r.try_get(col)?;
    Ok(match s {
        Some(s) => serde_json::from_str(&s)?,
        None => serde_json::Value::Null,
    })
}

impl Store {
    // ---------- kill switch ----------
    pub async fn kill_state(&self) -> StoreResult<KillState> {
        let rows = sqlx::query("SELECT scope, mode FROM tc_control")
            .fetch_all(&self.pool)
            .await?;
        let mut st = KillState {
            global: KillMode::Normal,
            brokers: BTreeMap::new(),
            strategies: BTreeMap::new(),
        };
        for r in rows {
            let scope: String = r.try_get("scope")?;
            let mode = parse_kill(&r.try_get::<String, _>("mode")?);
            if scope == "global" {
                st.global = mode;
            } else if let Some(b) = scope.strip_prefix("broker:") {
                if let Ok(b) = Broker::parse(b) {
                    st.brokers.insert(b, mode);
                }
            } else if let Some(s) = scope.strip_prefix("strategy:") {
                st.strategies.insert(s.to_owned(), mode);
            }
        }
        Ok(st)
    }

    pub async fn set_kill(
        &self,
        scope: &str,
        mode: KillMode,
        reason: &str,
        by: &str,
    ) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO tc_control (scope, mode, reason, updated_at, updated_by) VALUES (?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE mode = VALUES(mode), reason = VALUES(reason), updated_at = VALUES(updated_at), updated_by = VALUES(updated_by)",
        )
        .bind(scope)
        .bind(mode.as_str())
        .bind(reason)
        .bind(now_micros())
        .bind(by)
        .execute(&self.pool)
        .await?;
        self.audit(
            by,
            "kill_switch",
            &serde_json::json!({ "scope": scope, "mode": mode.as_str(), "reason": reason }),
        )
        .await
    }

    // ---------- live authorization ----------
    pub async fn authorize_live(
        &self,
        broker: Broker,
        config_hash: &str,
        passkey_id: i64,
        days: i64,
    ) -> StoreResult<()> {
        let now = now_micros();
        sqlx::query("UPDATE tc_live_authorizations SET revoked_at = ?, revoke_reason = 'superseded' WHERE broker = ? AND revoked_at IS NULL")
            .bind(now)
            .bind(broker.as_str())
            .execute(&self.pool)
            .await?;
        sqlx::query("INSERT INTO tc_live_authorizations (broker, authorized_at, expires_at, config_hash, passkey_id) VALUES (?, ?, ?, ?, ?)")
            .bind(broker.as_str())
            .bind(now)
            .bind(now + Duration::days(days))
            .bind(config_hash)
            .bind(passkey_id)
            .execute(&self.pool)
            .await?;
        self.audit(&format!("passkey:{passkey_id}"), "live_authorized", &serde_json::json!({ "broker": broker.as_str(), "config_hash": config_hash, "days": days }))
            .await
    }

    pub async fn revoke_live(
        &self,
        broker: Option<Broker>,
        reason: &str,
        by: &str,
    ) -> StoreResult<()> {
        let now = now_micros();
        match broker {
            Some(b) => {
                sqlx::query("UPDATE tc_live_authorizations SET revoked_at = ?, revoke_reason = ? WHERE broker = ? AND revoked_at IS NULL")
                    .bind(now)
                    .bind(reason)
                    .bind(b.as_str())
                    .execute(&self.pool)
                    .await?;
            }
            None => {
                sqlx::query("UPDATE tc_live_authorizations SET revoked_at = ?, revoke_reason = ? WHERE revoked_at IS NULL")
                    .bind(now)
                    .bind(reason)
                    .execute(&self.pool)
                    .await?;
            }
        }
        self.audit(
            by,
            "live_revoked",
            &serde_json::json!({ "broker": broker.map(|b| b.as_str()), "reason": reason }),
        )
        .await
    }

    /// Brokers with a valid (unrevoked, unexpired) authorization bound to `config_hash`.
    pub async fn live_authorizations(
        &self,
        config_hash: &str,
    ) -> StoreResult<BTreeMap<Broker, bool>> {
        let rows = sqlx::query("SELECT broker, expires_at, config_hash FROM tc_live_authorizations WHERE revoked_at IS NULL")
            .fetch_all(&self.pool)
            .await?;
        let now = now_micros();
        let mut out = BTreeMap::new();
        for b in [Broker::Trading212, Broker::Bitvavo] {
            out.insert(b, false);
        }
        for r in rows {
            let b: String = r.try_get("broker")?;
            let exp: DateTime<Utc> = r.try_get("expires_at")?;
            let h: String = r.try_get("config_hash")?;
            if let Ok(b) = Broker::parse(&b) {
                out.insert(b, exp > now && h == config_hash);
            }
        }
        Ok(out)
    }

    // ---------- versioned configs ----------
    pub async fn put_config<T: Serialize>(
        &self,
        kind: &str,
        cfg: &T,
        effective_at: DateTime<Utc>,
        by: &str,
    ) -> StoreResult<()> {
        let s = serde_json::to_string(cfg)?;
        sqlx::query("INSERT INTO tc_config (kind, config_json, effective_at, created_at, created_by) VALUES (?, CAST(? AS JSON), ?, ?, ?)")
            .bind(kind)
            .bind(&s)
            .bind(effective_at)
            .bind(now_micros())
            .bind(by)
            .execute(&self.pool)
            .await?;
        self.audit(by, &format!("config_{kind}"), &serde_json::json!({ "effective_at": effective_at, "config": serde_json::from_str::<serde_json::Value>(&s)? }))
            .await
    }

    /// Latest config of `kind` that is effective at `now`.
    pub async fn effective_config<T: DeserializeOwned>(
        &self,
        kind: &str,
        now: DateTime<Utc>,
    ) -> StoreResult<Option<T>> {
        let s: Option<String> = sqlx::query_scalar(
            "SELECT CAST(config_json AS CHAR) FROM tc_config WHERE kind = ? AND effective_at <= ? ORDER BY effective_at DESC, id DESC LIMIT 1",
        )
        .bind(kind)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        Ok(match s {
            Some(s) => Some(serde_json::from_str(&s)?),
            None => None,
        })
    }

    /// Pending (future-dated) config of `kind`, if any.
    pub async fn pending_config<T: DeserializeOwned>(
        &self,
        kind: &str,
        now: DateTime<Utc>,
    ) -> StoreResult<Option<(T, DateTime<Utc>)>> {
        let r = sqlx::query("SELECT CAST(config_json AS CHAR) AS c, effective_at FROM tc_config WHERE kind = ? AND effective_at > ? ORDER BY effective_at DESC LIMIT 1")
            .bind(kind)
            .bind(now)
            .fetch_optional(&self.pool)
            .await?;
        match r {
            Some(r) => {
                let c: String = r.try_get("c")?;
                Ok(Some((
                    serde_json::from_str(&c)?,
                    r.try_get("effective_at")?,
                )))
            }
            None => Ok(None),
        }
    }

    // ---------- commands (api -> engine) ----------
    pub async fn enqueue_command(
        &self,
        kind: &str,
        payload: &serde_json::Value,
        by: &str,
        stepup_action: Option<&str>,
    ) -> StoreResult<i64> {
        let r = sqlx::query("INSERT INTO tc_commands (kind, payload, created_at, created_by, stepup_action, status) VALUES (?, CAST(? AS JSON), ?, ?, ?, 'pending')")
            .bind(kind)
            .bind(serde_json::to_string(payload)?)
            .bind(now_micros())
            .bind(by)
            .bind(stepup_action)
            .execute(&self.pool)
            .await?;
        let id = r.last_insert_id() as i64;
        self.audit(by, "command", &serde_json::json!({ "id": id, "kind": kind, "payload": payload, "stepup": stepup_action })).await?;
        Ok(id)
    }

    /// Claim the oldest pending command (engine only).
    pub async fn claim_command(&self) -> StoreResult<Option<Command>> {
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query(
            "SELECT id, kind, CAST(payload AS CHAR) AS payload, created_by, stepup_action, created_at FROM tc_commands WHERE status = 'pending' ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(r) = r else {
            tx.commit().await?;
            return Ok(None);
        };
        let id: i64 = r.try_get("id")?;
        sqlx::query("UPDATE tc_commands SET status = 'running' WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(Command {
            id,
            kind: r.try_get("kind")?,
            payload: json_of(&r, "payload")?,
            created_by: r.try_get("created_by")?,
            stepup_action: r.try_get("stepup_action")?,
            created_at: r.try_get("created_at")?,
        }))
    }

    pub async fn finish_command(
        &self,
        id: i64,
        ok: bool,
        result: &serde_json::Value,
    ) -> StoreResult<()> {
        sqlx::query("UPDATE tc_commands SET status = ?, result = CAST(? AS JSON), processed_at = ? WHERE id = ?")
            .bind(if ok { "done" } else { "failed" })
            .bind(serde_json::to_string(result)?)
            .bind(now_micros())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Commands left `running` by a crashed engine are marked failed (never re-run blindly).
    pub async fn fail_stale_running_commands(&self) -> StoreResult<u64> {
        Ok(sqlx::query("UPDATE tc_commands SET status = 'failed', result = JSON_OBJECT('error', 'engine restarted while running'), processed_at = ? WHERE status = 'running'")
            .bind(now_micros())
            .execute(&self.pool)
            .await?
            .rows_affected())
    }

    pub async fn command_result(
        &self,
        id: i64,
    ) -> StoreResult<Option<(String, serde_json::Value)>> {
        let r = sqlx::query(
            "SELECT status, CAST(result AS CHAR) AS result FROM tc_commands WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        match r {
            Some(r) => Ok(Some((r.try_get("status")?, json_of(&r, "result")?))),
            None => Ok(None),
        }
    }

    // ---------- events (engine -> api) ----------
    pub async fn push_event(&self, r#type: &str, data: &serde_json::Value) -> StoreResult<i64> {
        let r =
            sqlx::query("INSERT INTO tc_events (ts, type, data) VALUES (?, ?, CAST(? AS JSON))")
                .bind(now_micros())
                .bind(r#type)
                .bind(serde_json::to_string(data)?)
                .execute(&self.pool)
                .await?;
        Ok(r.last_insert_id() as i64)
    }

    pub async fn events_after(&self, after_id: i64, limit: i64) -> StoreResult<Vec<EventRow>> {
        let rows = sqlx::query("SELECT id, ts, type, CAST(data AS CHAR) AS data FROM tc_events WHERE id > ? ORDER BY id LIMIT ?")
            .bind(after_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|r| {
                Ok(EventRow {
                    id: r.try_get("id")?,
                    ts: r.try_get("ts")?,
                    r#type: r.try_get("type")?,
                    data: json_of(r, "data")?,
                })
            })
            .collect()
    }

    pub async fn last_event_id(&self) -> StoreResult<i64> {
        Ok(
            sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM tc_events")
                .fetch_one(&self.pool)
                .await?
                .unwrap_or(0),
        )
    }

    pub async fn prune_events(&self, older_than: DateTime<Utc>) -> StoreResult<u64> {
        Ok(sqlx::query("DELETE FROM tc_events WHERE ts < ?")
            .bind(older_than)
            .execute(&self.pool)
            .await?
            .rows_affected())
    }

    // ---------- heartbeat ----------
    pub async fn heartbeat(
        &self,
        component: &str,
        mode: &str,
        version: &str,
        detail: &serde_json::Value,
    ) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO tc_heartbeat (component, ts, mode, version, detail) VALUES (?, ?, ?, ?, CAST(? AS JSON))
             ON DUPLICATE KEY UPDATE ts = VALUES(ts), mode = VALUES(mode), version = VALUES(version), detail = VALUES(detail)",
        )
        .bind(component)
        .bind(now_micros())
        .bind(mode)
        .bind(version)
        .bind(serde_json::to_string(detail)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn heartbeat_of(
        &self,
        component: &str,
    ) -> StoreResult<Option<(DateTime<Utc>, String)>> {
        let r = sqlx::query("SELECT ts, mode FROM tc_heartbeat WHERE component = ?")
            .bind(component)
            .fetch_optional(&self.pool)
            .await?;
        match r {
            Some(r) => Ok(Some((r.try_get("ts")?, r.try_get("mode")?))),
            None => Ok(None),
        }
    }

    // ---------- sealed credentials ----------
    pub async fn put_credential(
        &self,
        broker: &str,
        field: &str,
        key_version: i32,
        envelope: &[u8],
    ) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO tc_credentials (broker, field, key_version, envelope, created_at) VALUES (?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE key_version = VALUES(key_version), envelope = VALUES(envelope), created_at = VALUES(created_at)",
        )
        .bind(broker)
        .bind(field)
        .bind(key_version)
        .bind(envelope)
        .bind(now_micros())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn credential(
        &self,
        broker: &str,
        field: &str,
    ) -> StoreResult<Option<(i32, Vec<u8>)>> {
        let r = sqlx::query(
            "SELECT key_version, envelope FROM tc_credentials WHERE broker = ? AND field = ?",
        )
        .bind(broker)
        .bind(field)
        .fetch_optional(&self.pool)
        .await?;
        match r {
            Some(r) => Ok(Some((r.try_get("key_version")?, r.try_get("envelope")?))),
            None => Ok(None),
        }
    }

    pub async fn credential_brokers(&self) -> StoreResult<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT DISTINCT broker FROM tc_credentials ORDER BY broker")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn put_credential_status(&self, s: &CredentialStatusRow) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO tc_credential_status (broker, authenticated, can_trade, can_withdraw, attested_no_withdraw, scopes_ok, account_currency, notes, checked_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, CAST(? AS JSON), ?)
             ON DUPLICATE KEY UPDATE authenticated = VALUES(authenticated), can_trade = VALUES(can_trade), can_withdraw = VALUES(can_withdraw),
               scopes_ok = VALUES(scopes_ok), account_currency = VALUES(account_currency), notes = VALUES(notes), checked_at = VALUES(checked_at)",
        )
        .bind(&s.broker)
        .bind(s.authenticated)
        .bind(s.can_trade)
        .bind(s.can_withdraw.map(|b| b as i8))
        .bind(s.attested_no_withdraw)
        .bind(s.scopes_ok)
        .bind(&s.account_currency)
        .bind(serde_json::to_string(&s.notes)?)
        .bind(s.checked_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn attest_no_withdraw(&self, broker: &str, by: &str) -> StoreResult<()> {
        let n = sqlx::query(
            "UPDATE tc_credential_status SET attested_no_withdraw = TRUE WHERE broker = ?",
        )
        .bind(broker)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        self.audit(
            by,
            "attest_no_withdraw",
            &serde_json::json!({ "broker": broker }),
        )
        .await
    }

    pub async fn credential_statuses(&self) -> StoreResult<Vec<CredentialStatusRow>> {
        let rows = sqlx::query(
            "SELECT broker, authenticated, can_trade, can_withdraw, attested_no_withdraw, scopes_ok, account_currency, CAST(notes AS CHAR) AS notes, checked_at FROM tc_credential_status",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                let cw: Option<i8> = r.try_get("can_withdraw")?;
                Ok(CredentialStatusRow {
                    broker: r.try_get("broker")?,
                    authenticated: r.try_get("authenticated")?,
                    can_trade: r.try_get("can_trade")?,
                    can_withdraw: cw.map(|v| v != 0),
                    attested_no_withdraw: r.try_get("attested_no_withdraw")?,
                    scopes_ok: r.try_get("scopes_ok")?,
                    account_currency: r.try_get("account_currency")?,
                    notes: json_of(r, "notes")?,
                    checked_at: r.try_get("checked_at")?,
                })
            })
            .collect()
    }

    // ---------- push ----------
    pub async fn add_push_sub(
        &self,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
        device: &str,
    ) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO tc_push_subscriptions (endpoint, endpoint_hash, p256dh, auth, device_name, created_at) VALUES (?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE p256dh = VALUES(p256dh), auth = VALUES(auth), device_name = VALUES(device_name)",
        )
        .bind(endpoint)
        .bind(sha256(endpoint.as_bytes()))
        .bind(p256dh)
        .bind(auth)
        .bind(device)
        .bind(now_micros())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove_push_sub(&self, endpoint: &str) -> StoreResult<()> {
        sqlx::query("DELETE FROM tc_push_subscriptions WHERE endpoint_hash = ?")
            .bind(sha256(endpoint.as_bytes()))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn push_subs(&self) -> StoreResult<Vec<PushSub>> {
        let rows = sqlx::query(
            "SELECT id, endpoint, p256dh, auth, device_name FROM tc_push_subscriptions",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(PushSub {
                    id: r.try_get("id")?,
                    endpoint: r.try_get("endpoint")?,
                    p256dh: r.try_get("p256dh")?,
                    auth: r.try_get("auth")?,
                    device_name: r.try_get("device_name")?,
                })
            })
            .collect()
    }

    // ---------- agent status ----------
    pub async fn put_agent_status(&self, a: &AgentStatusRow) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO tc_agent_status (agent, status, last_run_at, last_duration_ms, summary, metrics) VALUES (?, ?, ?, ?, ?, CAST(? AS JSON))
             ON DUPLICATE KEY UPDATE status = VALUES(status), last_run_at = VALUES(last_run_at), last_duration_ms = VALUES(last_duration_ms), summary = VALUES(summary), metrics = VALUES(metrics)",
        )
        .bind(&a.agent)
        .bind(&a.status)
        .bind(a.last_run_at)
        .bind(a.last_duration_ms)
        .bind(a.summary.chars().take(512).collect::<String>())
        .bind(serde_json::to_string(&a.metrics)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn agent_statuses(&self) -> StoreResult<Vec<AgentStatusRow>> {
        let rows = sqlx::query("SELECT agent, status, last_run_at, last_duration_ms, summary, CAST(metrics AS CHAR) AS metrics FROM tc_agent_status ORDER BY agent")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|r| {
                Ok(AgentStatusRow {
                    agent: r.try_get("agent")?,
                    status: r.try_get("status")?,
                    last_run_at: r.try_get("last_run_at")?,
                    last_duration_ms: r.try_get("last_duration_ms")?,
                    summary: r.try_get("summary")?,
                    metrics: json_of(r, "metrics")?,
                })
            })
            .collect()
    }
}

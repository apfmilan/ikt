//! Passkeys, sessions and one-time tokens. Tokens are stored only as SHA-256 hashes.

use chrono::{DateTime, Duration, Utc};
use sqlx::Row;

use crate::{now_micros, sha256, Store, StoreError, StoreResult};

#[derive(Clone, Debug)]
pub struct PasskeyRow {
    pub id: i64,
    pub passkey_json: String,
    pub device_name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct SessionRow {
    pub passkey_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub idle_expires_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Setup,
    Pairing,
    Recovery,
    Stepup,
}

impl TokenKind {
    fn as_str(&self) -> &'static str {
        match self {
            TokenKind::Setup => "setup",
            TokenKind::Pairing => "pairing",
            TokenKind::Recovery => "recovery",
            TokenKind::Stepup => "stepup",
        }
    }
}

impl Store {
    /// Single-user installation: returns (user_uuid, display_name), creating it on first call.
    pub async fn ensure_user(&self, display_name: &str) -> StoreResult<(String, String)> {
        if let Some(r) = sqlx::query("SELECT user_uuid, display_name FROM tc_user WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?
        {
            return Ok((r.try_get("user_uuid")?, r.try_get("display_name")?));
        }
        let uuid = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT IGNORE INTO tc_user (id, user_uuid, display_name, created_at) VALUES (1, ?, ?, ?)")
            .bind(&uuid)
            .bind(display_name)
            .bind(now_micros())
            .execute(&self.pool)
            .await?;
        let r = sqlx::query("SELECT user_uuid, display_name FROM tc_user WHERE id = 1")
            .fetch_one(&self.pool)
            .await?;
        Ok((r.try_get("user_uuid")?, r.try_get("display_name")?))
    }

    pub async fn passkey_count(&self) -> StoreResult<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM tc_passkeys WHERE revoked_at IS NULL")
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn insert_passkey(
        &self,
        cred_id: &[u8],
        passkey_json: &str,
        device_name: &str,
    ) -> StoreResult<i64> {
        let r = sqlx::query(
            "INSERT INTO tc_passkeys (cred_id, cred_id_hash, passkey_json, device_name, created_at) VALUES (?, ?, CAST(? AS JSON), ?, ?)",
        )
        .bind(cred_id)
        .bind(sha256(cred_id))
        .bind(passkey_json)
        .bind(device_name)
        .bind(now_micros())
        .execute(&self.pool)
        .await?;
        Ok(r.last_insert_id() as i64)
    }

    pub async fn active_passkeys(&self) -> StoreResult<Vec<PasskeyRow>> {
        let rows = sqlx::query(
            "SELECT id, CAST(passkey_json AS CHAR) AS pj, device_name, created_at, last_used_at FROM tc_passkeys WHERE revoked_at IS NULL ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(PasskeyRow {
                    id: r.try_get("id")?,
                    passkey_json: r.try_get("pj")?,
                    device_name: r.try_get("device_name")?,
                    created_at: r.try_get("created_at")?,
                    last_used_at: r.try_get("last_used_at")?,
                })
            })
            .collect()
    }

    pub async fn passkey_id_by_cred(&self, cred_id: &[u8]) -> StoreResult<Option<i64>> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM tc_passkeys WHERE cred_id_hash = ? AND revoked_at IS NULL",
        )
        .bind(sha256(cred_id))
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Persist the updated passkey (signature counter / backup state) after authentication.
    pub async fn update_passkey(&self, id: i64, passkey_json: &str) -> StoreResult<()> {
        sqlx::query(
            "UPDATE tc_passkeys SET passkey_json = CAST(? AS JSON), last_used_at = ? WHERE id = ?",
        )
        .bind(passkey_json)
        .bind(now_micros())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn revoke_passkey(&self, id: i64) -> StoreResult<()> {
        let n = sqlx::query(
            "UPDATE tc_passkeys SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL",
        )
        .bind(now_micros())
        .bind(id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        // Revoke sessions created with that passkey.
        sqlx::query(
            "UPDATE tc_sessions SET revoked_at = ? WHERE passkey_id = ? AND revoked_at IS NULL",
        )
        .bind(now_micros())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create_session(
        &self,
        token: &[u8],
        passkey_id: i64,
        idle: Duration,
        absolute: Duration,
        user_agent: Option<&str>,
    ) -> StoreResult<()> {
        let now = now_micros();
        sqlx::query(
            "INSERT INTO tc_sessions (token_hash, passkey_id, created_at, last_seen_at, idle_expires_at, absolute_expires_at, user_agent) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(sha256(token))
        .bind(passkey_id)
        .bind(now)
        .bind(now)
        .bind(now + idle)
        .bind(now + absolute)
        .bind(user_agent.map(|s| s.chars().take(255).collect::<String>()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Validate a session token, sliding the idle expiry. Returns `None` if invalid/expired.
    pub async fn touch_session(
        &self,
        token: &[u8],
        idle: Duration,
    ) -> StoreResult<Option<SessionRow>> {
        let now = now_micros();
        let h = sha256(token);
        let Some(r) = sqlx::query(
            "SELECT passkey_id, created_at, idle_expires_at, absolute_expires_at FROM tc_sessions WHERE token_hash = ? AND revoked_at IS NULL",
        )
        .bind(&h)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let s = SessionRow {
            passkey_id: r.try_get("passkey_id")?,
            created_at: r.try_get("created_at")?,
            idle_expires_at: r.try_get("idle_expires_at")?,
            absolute_expires_at: r.try_get("absolute_expires_at")?,
        };
        if now >= s.idle_expires_at || now >= s.absolute_expires_at {
            return Ok(None);
        }
        let new_idle = (now + idle).min(s.absolute_expires_at);
        sqlx::query(
            "UPDATE tc_sessions SET last_seen_at = ?, idle_expires_at = ? WHERE token_hash = ?",
        )
        .bind(now)
        .bind(new_idle)
        .bind(&h)
        .execute(&self.pool)
        .await?;
        Ok(Some(SessionRow {
            idle_expires_at: new_idle,
            ..s
        }))
    }

    pub async fn revoke_session(&self, token: &[u8]) -> StoreResult<()> {
        sqlx::query("UPDATE tc_sessions SET revoked_at = ? WHERE token_hash = ?")
            .bind(now_micros())
            .bind(sha256(token))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn insert_token(
        &self,
        token: &[u8],
        kind: TokenKind,
        action: Option<&str>,
        passkey_id: Option<i64>,
        ttl: Option<Duration>,
    ) -> StoreResult<()> {
        let now = now_micros();
        sqlx::query("INSERT INTO tc_one_time_tokens (token_hash, kind, action, passkey_id, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(sha256(token))
            .bind(kind.as_str())
            .bind(action)
            .bind(passkey_id)
            .bind(now)
            .bind(ttl.map(|t| now + t))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Atomically consume a one-time token of `kind` (and `action`, if given).
    /// Returns the bound passkey id (if any). Fails if unknown, used, expired or mismatched.
    pub async fn consume_token(
        &self,
        token: &[u8],
        kind: TokenKind,
        action: Option<&str>,
    ) -> StoreResult<Option<i64>> {
        let now = now_micros();
        let h = sha256(token);
        let mut tx = self.pool.begin().await?;
        let Some(r) = sqlx::query("SELECT kind, action, passkey_id, expires_at, used_at FROM tc_one_time_tokens WHERE token_hash = ? FOR UPDATE")
            .bind(&h)
            .fetch_optional(&mut *tx)
            .await?
        else {
            return Err(StoreError::NotFound);
        };
        let k: String = r.try_get("kind")?;
        let a: Option<String> = r.try_get("action")?;
        let used: Option<DateTime<Utc>> = r.try_get("used_at")?;
        let exp: Option<DateTime<Utc>> = r.try_get("expires_at")?;
        if k != kind.as_str()
            || used.is_some()
            || exp.is_some_and(|e| now >= e)
            || (action.is_some() && a.as_deref() != action)
        {
            return Err(StoreError::Conflict(
                "token invalid, used or expired".into(),
            ));
        }
        sqlx::query("UPDATE tc_one_time_tokens SET used_at = ? WHERE token_hash = ?")
            .bind(now)
            .bind(&h)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(r.try_get("passkey_id")?)
    }

    /// Invalidate all unused tokens of a kind (e.g. old setup tokens once setup completes).
    pub async fn invalidate_tokens(&self, kind: TokenKind) -> StoreResult<()> {
        sqlx::query("UPDATE tc_one_time_tokens SET used_at = ? WHERE kind = ? AND used_at IS NULL")
            .bind(now_micros())
            .bind(kind.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

impl Store {
    /// Revoke every session (account recovery, suspected compromise).
    pub async fn revoke_all_sessions(&self) -> StoreResult<u64> {
        Ok(
            sqlx::query("UPDATE tc_sessions SET revoked_at = ? WHERE revoked_at IS NULL")
                .bind(now_micros())
                .execute(&self.pool)
                .await?
                .rows_affected(),
        )
    }
}

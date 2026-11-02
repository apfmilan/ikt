//! MySQL persistence for Trading Center (SQLx, runtime-checked queries).
//!
//! Invariants enforced here:
//! * Order intents change state only through [`Store::transition_intent`], which validates the
//!   transition with [`tc_core::OrderState::transition`] inside a transaction and appends an
//!   `tc_order_events` row atomically.
//! * The audit log is a SHA-256 hash chain (`prev_hash` → `hash`), written under a MySQL named
//!   lock so concurrent writers (api + engine) cannot fork the chain. Append-only triggers
//!   (migration 0002) reject UPDATE/DELETE.
//! * Ledger rows are idempotent on `(broker, kind, ref)`.

pub mod auth;
pub mod control;
pub mod trading;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions};
use sqlx::{MySqlPool, Row};
use std::str::FromStr;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid data: {0}")]
    Invalid(String),
}

impl From<tc_core::CoreError> for StoreError {
    fn from(e: tc_core::CoreError) -> Self {
        StoreError::Invalid(e.to_string())
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        StoreError::Invalid(e.to_string())
    }
}

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Clone)]
pub struct Store {
    pub pool: MySqlPool,
}

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

impl Store {
    pub async fn connect(url: &str, max_connections: u32) -> StoreResult<Self> {
        let opts = MySqlConnectOptions::from_str(url)?
            .timezone(Some("+00:00".into()))
            .charset("utf8mb4");
        let pool = MySqlPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(opts)
            .await?;
        Ok(Store { pool })
    }

    pub async fn migrate(&self) -> StoreResult<()> {
        MIGRATOR.run(&self.pool).await?;
        Ok(())
    }

    pub async fn ping(&self) -> bool {
        sqlx::query("SELECT 1").execute(&self.pool).await.is_ok()
    }

    /// Append to the hash-chained audit log.
    pub async fn audit(
        &self,
        actor: &str,
        action: &str,
        detail: &serde_json::Value,
    ) -> StoreResult<()> {
        let mut conn = self.pool.acquire().await?;
        let locked: Option<i64> = sqlx::query_scalar("SELECT GET_LOCK('tc_audit_chain', 10)")
            .fetch_one(&mut *conn)
            .await?;
        if locked != Some(1) {
            return Err(StoreError::Conflict("audit chain lock timeout".into()));
        }
        let result = async {
            let prev: Option<Vec<u8>> = sqlx::query_scalar("SELECT hash FROM tc_audit_log ORDER BY id DESC LIMIT 1")
                .fetch_optional(&mut *conn)
                .await?;
            let prev = prev.unwrap_or_else(|| vec![0u8; 32]);
            let ts = now_micros();
            let detail_s = serde_json::to_string(detail)?;
            let hash = chain_hash(&prev, ts, actor, action, &detail_s);
            sqlx::query(
                "INSERT INTO tc_audit_log (ts, actor, action, detail, prev_hash, hash) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(ts)
            .bind(actor)
            .bind(action)
            .bind(&detail_s)
            .bind(&prev)
            .bind(hash.to_vec())
            .execute(&mut *conn)
            .await?;
            Ok::<(), StoreError>(())
        }
        .await;
        let _ = sqlx::query("SELECT RELEASE_LOCK('tc_audit_chain')")
            .execute(&mut *conn)
            .await;
        result
    }

    /// Verify the whole audit chain. Returns the number of rows checked.
    pub async fn verify_audit_chain(&self) -> StoreResult<u64> {
        let rows = sqlx::query(
            "SELECT ts, actor, action, detail, prev_hash, hash FROM tc_audit_log ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut prev = vec![0u8; 32];
        for (i, r) in rows.iter().enumerate() {
            let p: Vec<u8> = r.try_get("prev_hash")?;
            let h: Vec<u8> = r.try_get("hash")?;
            if p != prev {
                return Err(StoreError::Invalid(format!(
                    "audit chain broken at row {i}: prev_hash mismatch"
                )));
            }
            // `detail` is stored as the exact text that was hashed (MEDIUMTEXT, not JSON).
            let detail: String = r.try_get("detail")?;
            let actor: String = r.try_get("actor")?;
            let action: String = r.try_get("action")?;
            let expect = chain_hash(&p, r.try_get("ts")?, &actor, &action, &detail);
            if h != expect.to_vec() {
                return Err(StoreError::Invalid(format!(
                    "audit chain broken at row {i}: hash mismatch"
                )));
            }
            prev = h;
        }
        Ok(rows.len() as u64)
    }

    /// Single-writer guard for the engine. Holds a MySQL named lock on a dedicated connection
    /// for the life of the returned guard.
    pub async fn acquire_engine_lock(&self) -> StoreResult<EngineLock> {
        let mut conn = self.pool.acquire().await?.detach();
        let got: Option<i64> = sqlx::query_scalar("SELECT GET_LOCK('tc_engine_singleton', 0)")
            .fetch_one(&mut conn)
            .await?;
        if got != Some(1) {
            return Err(StoreError::Conflict(
                "another engine instance holds the lock".into(),
            ));
        }
        Ok(EngineLock { _conn: conn })
    }
}

/// Holds the engine singleton lock; dropping the connection releases it.
pub struct EngineLock {
    _conn: sqlx::MySqlConnection,
}

/// Current time truncated to microseconds (DATETIME(6) precision) so values round-trip exactly.
pub fn now_micros() -> DateTime<Utc> {
    let n = Utc::now();
    DateTime::from_timestamp_micros(n.timestamp_micros()).unwrap_or(n)
}

fn chain_hash(prev: &[u8], ts: DateTime<Utc>, actor: &str, action: &str, detail: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(prev);
    h.update(ts.timestamp_micros().to_be_bytes());
    h.update(actor.as_bytes());
    h.update([0]);
    h.update(action.as_bytes());
    h.update([0]);
    h.update(detail.as_bytes());
    h.finalize().into()
}

/// SHA-256 helper used for token and endpoint hashing.
pub fn sha256(data: &[u8]) -> Vec<u8> {
    Sha256::digest(data).to_vec()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

impl Store {
    /// Pool that connects on first use (tests that never touch the DB).
    pub fn connect_lazy(url: &str) -> StoreResult<Self> {
        let opts = MySqlConnectOptions::from_str(url)?;
        Ok(Store {
            pool: MySqlPoolOptions::new()
                .max_connections(1)
                .connect_lazy_with(opts),
        })
    }
}

/// Names of all tables in the current schema (operator inspection before migrating).
pub async fn sqlx_query_tables(store: &Store) -> StoreResult<Vec<String>> {
    Ok(sqlx::query_scalar("SELECT CAST(table_name AS CHAR) FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY table_name")
        .fetch_all(&store.pool)
        .await?)
}

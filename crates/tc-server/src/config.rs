//! Runtime configuration from environment variables and Docker secret files.
//! Secret values are only ever read from files (`*_FILE`), never from plain env vars in prod.

use anyhow::{bail, Context, Result};
use std::path::Path;

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub bind: String,
    pub rp_id: String,
    pub rp_origin: String,
    pub engine_public_key: Option<String>,
    pub engine_secret_key_file: Option<String>,
    pub vapid_public_key: Option<String>,
    pub vapid_private_key_file: Option<String>,
    pub vapid_subject: String,
    pub live_trading: Option<String>,
    pub dev_insecure_cookies: bool,
}

fn read_file(path: &str) -> Result<String> {
    let s = std::fs::read_to_string(Path::new(path)).with_context(|| format!("reading {path}"))?;
    Ok(s.trim().to_owned())
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// `NAME_FILE` (preferred) or `NAME`.
fn env_or_file(name: &str) -> Result<Option<String>> {
    if let Some(p) = env(&format!("{name}_FILE")) {
        return Ok(Some(read_file(&p)?));
    }
    Ok(env(name))
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let database_url = env_or_file("TC_DATABASE_URL")?
            .context("TC_DATABASE_URL_FILE (or TC_DATABASE_URL) is required")?;
        let rp_origin = env("TC_RP_ORIGIN").unwrap_or_else(|| "http://localhost:8080".into());
        let rp_id = env("TC_RP_ID").unwrap_or_else(|| "localhost".into());
        if !rp_origin.starts_with("https://") && rp_id != "localhost" {
            bail!("TC_RP_ORIGIN must be https:// outside localhost");
        }
        let engine_public_key = env_or_file("TC_ENGINE_PUBLIC_KEY")?;
        let vapid_public_key = env_or_file("TC_VAPID_PUBLIC_KEY")?;
        Ok(Config {
            database_url,
            bind: env("TC_BIND").unwrap_or_else(|| "0.0.0.0:8080".into()),
            rp_id,
            dev_insecure_cookies: rp_origin.starts_with("http://localhost"),
            rp_origin,
            engine_public_key,
            engine_secret_key_file: env("TC_ENGINE_SECRET_KEY_FILE"),
            vapid_public_key,
            vapid_private_key_file: env("TC_VAPID_PRIVATE_KEY_FILE"),
            vapid_subject: env("TC_VAPID_SUBJECT")
                .unwrap_or_else(|| "mailto:admin@localhost".into()),
            live_trading: env("TC_LIVE_TRADING"),
        })
    }
}

impl std::fmt::Debug for Config {
    /// Redacts the database URL (contains the password) and never prints key material.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("database_url", &"<redacted>")
            .field("bind", &self.bind)
            .field("rp_id", &self.rp_id)
            .field("rp_origin", &self.rp_origin)
            .field("engine_public_key", &self.engine_public_key.is_some())
            .field("vapid_public_key", &self.vapid_public_key.is_some())
            .field("live_trading", &self.live_trading)
            .finish()
    }
}

//! `tc-server --role api|engine|all` — see docs/decisions/0001-architecture.md.

mod api;
mod config;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::sync::Arc;
use tc_store::Store;
use tracing::info;

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Role {
    Api,
    Engine,
    All,
}

#[derive(Parser)]
#[command(name = "tc-server", version, about = "Trading Center server")]
struct Cli {
    #[arg(long, value_enum, env = "TC_ROLE", default_value = "all")]
    role: Role,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Exit 0 if the health URL answers 2xx (for container healthchecks).
    Healthcheck {
        #[arg(long)]
        url: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(Cmd::Healthcheck { url }) = cli.cmd {
        return healthcheck(&url).await;
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("TC_LOG").unwrap_or_else(|_| "info,sqlx=warn".into()))
        .json()
        .init();
    let cfg = config::Config::from_env()?;
    let store = Store::connect(&cfg.database_url, 10).await.context("connecting to MySQL")?;
    info!(role = ?cli.role, version = env!("CARGO_PKG_VERSION"), "starting");

    let shutdown = tokio::sync::watch::channel(false);
    let mut tasks = tokio::task::JoinSet::new();
    if matches!(cli.role, Role::Engine | Role::All) {
        let ecfg = tc_engine::EngineConfig {
            secret_key_file: cfg.engine_secret_key_file.clone(),
            vapid_private_key_file: cfg.vapid_private_key_file.clone(),
            vapid_subject: cfg.vapid_subject.clone(),
            live_trading_flag: cfg.live_trading.clone(),
            // In the engine container `TC_BIND` is the internal health listener (127.0.0.1:9090).
            health_bind: if cli.role == Role::Engine { Some(cfg.bind.clone()) } else { None },
            version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        let st = store.clone();
        let rx = shutdown.1.clone();
        tasks.spawn(async move { tc_engine::run(st, ecfg, rx).await });
    }
    if matches!(cli.role, Role::Api | Role::All) {
        let state = api::AppState::new(store.clone(), &cfg)?;
        let bind = cfg.bind.clone();
        let rx = shutdown.1.clone();
        tasks.spawn(async move { api::serve(Arc::new(state), &bind, rx).await });
    }
    let mut crashed = false;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => info!("shutdown signal"),
        r = tasks.join_next() => {
            crashed = true;
            if let Some(r) = r {
                match r {
                    Ok(Err(e)) => tracing::error!(error = %e, "component exited with error"),
                    Err(e) => tracing::error!(error = %e, "component panicked"),
                    Ok(Ok(())) => info!("component exited"),
                }
            }
        }
    }
    let _ = shutdown.0.send(true);
    while tasks.join_next().await.is_some() {}
    // Non-zero exit lets Docker restart the container (automatic recovery).
    std::process::exit(if crashed { 1 } else { 0 });
}

async fn healthcheck(url: &str) -> Result<()> {
    let r = reqwest::Client::builder().timeout(std::time::Duration::from_secs(4)).build()?.get(url).send().await?;
    if r.status().is_success() {
        Ok(())
    } else {
        anyhow::bail!("unhealthy: {}", r.status())
    }
}

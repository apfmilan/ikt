//! `tc` — operator CLI. Never prints secrets except the one-time setup URL token (by design,
//! shown once to the operator) and never touches broker order endpoints.

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine;
use chrono::{Duration, Utc};
use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::{Path, PathBuf};
use tc_backtest::{walk_forward, Costs, PromotionRules, Report, SimParams, WalkForward};
use tc_core::{Candle, Interval};
use tc_store::Store;
use tc_strategy::Series;

#[derive(Parser)]
#[command(name = "tc", version, about = "Trading Center operator CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Generate engine X25519 + VAPID key pairs into DIR (refuses to overwrite).
    Keygen {
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Create a single-use first-passkey setup token (valid 30 min) and print the setup URL.
    SetupToken {
        #[arg(long, env = "TC_RP_ORIGIN", default_value = "http://localhost:8080")]
        origin: String,
    },
    /// Apply database migrations (additive only).
    Migrate,
    /// Print table names of the configured database (pre-migration inspection).
    DbInspect,
    /// Download public Bitvavo candles to CSV (no API key needed).
    FetchCandles {
        #[arg(long)]
        market: String,
        #[arg(long, default_value = "1d")]
        interval: String,
        #[arg(long, default_value_t = 2000)]
        days: i64,
        #[arg(long)]
        out: PathBuf,
    },
    /// Walk-forward backtest with out-of-sample validation; optional promotion into the DB.
    Backtest {
        /// Strategy family (s1_trend_vol | s2_donchian | s3_mean_reversion) or "all".
        #[arg(long, default_value = "all")]
        strategy: String,
        /// One or more CSV files written by fetch-candles (market name taken from file stem).
        #[arg(long, num_args = 1..)]
        data: Vec<PathBuf>,
        #[arg(long, default_value = "1d")]
        interval: String,
        #[arg(long, default_value = "target/backtests")]
        out_dir: PathBuf,
        /// Write status + regime edges to the database (requires TC_DATABASE_URL[_FILE]).
        #[arg(long)]
        promote: bool,
        /// Edge-statistics group the data represents (`bitvavo_eur` or `t212_us_equity`).
        #[arg(long, default_value = "bitvavo_eur")]
        market_group: String,
        /// Asset class recorded for the strategy (`crypto`, `stock`, `etf`).
        #[arg(long, default_value = "crypto")]
        asset_class: String,
    },
}

fn database_url() -> Result<String> {
    if let Ok(p) = std::env::var("TC_DATABASE_URL_FILE") {
        return Ok(std::fs::read_to_string(&p)
            .with_context(|| format!("reading {p}"))?
            .trim()
            .to_owned());
    }
    std::env::var("TC_DATABASE_URL").context("TC_DATABASE_URL_FILE or TC_DATABASE_URL required")
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Keygen { out_dir } => keygen(&out_dir),
        Cmd::SetupToken { origin } => {
            let store = Store::connect(&database_url()?, 2).await?;
            if store.passkey_count().await? > 0 {
                bail!("a passkey already exists; add devices with pairing or use a recovery code");
            }
            let mut b = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut b);
            store
                .insert_token(
                    &b,
                    tc_store::auth::TokenKind::Setup,
                    None,
                    None,
                    Some(Duration::minutes(30)),
                )
                .await?;
            store
                .audit("cli", "setup_token_issued", &serde_json::json!({}))
                .await?;
            println!(
                "Open within 30 minutes on your iPhone (single use):\n{}/#token={}",
                origin.trim_end_matches('/'),
                B64URL.encode(b)
            );
            Ok(())
        }
        Cmd::Migrate => {
            let store = Store::connect(&database_url()?, 2).await?;
            store.migrate().await?;
            println!("migrations applied");
            Ok(())
        }
        Cmd::DbInspect => {
            let store = Store::connect(&database_url()?, 2).await?;
            let rows: Vec<(String, Option<i64>)> = sqlx_tables(&store).await?;
            for (t, n) in rows {
                println!(
                    "{t}\t{}",
                    n.map(|n| n.to_string()).unwrap_or_else(|| "?".into())
                );
            }
            Ok(())
        }
        Cmd::FetchCandles {
            market,
            interval,
            days,
            out,
        } => fetch(&market, &interval, days, &out).await,
        Cmd::Backtest {
            strategy,
            data,
            interval,
            out_dir,
            promote,
            market_group,
            asset_class,
        } => {
            backtest(
                &strategy,
                &data,
                &interval,
                &out_dir,
                promote,
                &market_group,
                &asset_class,
            )
            .await
        }
    }
}

async fn sqlx_tables(store: &Store) -> Result<Vec<(String, Option<i64>)>> {
    let names: Vec<String> = tc_store::sqlx_query_tables(store).await?;
    Ok(names.into_iter().map(|n| (n, None)).collect())
}

fn write_new(path: &Path, content: &str) -> Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("{} exists or is not writable", path.display()))?;
    f.write_all(content.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o400))?;
    }
    Ok(())
}

fn keygen(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    for f in [
        "engine_secret_key",
        "engine_public_key",
        "vapid_private_key",
        "vapid_public_key",
    ] {
        if dir.join(f).exists() {
            bail!(
                "{} already exists; refusing to overwrite",
                dir.join(f).display()
            );
        }
    }
    let sk = tc_vault::OpeningKey::generate();
    write_new(&dir.join("engine_secret_key"), &sk.to_base64_secret())?;
    write_new(
        &dir.join("engine_public_key"),
        &sk.sealing_key().to_base64(),
    )?;
    // VAPID (P-256): private = raw 32-byte scalar, public = uncompressed point; both base64url.
    let vapid = p256::SecretKey::random(&mut rand::rngs::OsRng);
    let pubkey = vapid.public_key();
    let point = p256::EncodedPoint::from(pubkey);
    write_new(
        &dir.join("vapid_private_key"),
        &B64URL.encode(vapid.to_bytes()),
    )?;
    write_new(
        &dir.join("vapid_public_key"),
        &B64URL.encode(point.as_bytes()),
    )?;
    println!(
        "wrote engine_secret_key, engine_public_key, vapid_private_key, vapid_public_key to {}",
        dir.display()
    );
    Ok(())
}

async fn fetch(market: &str, interval: &str, days: i64, out: &Path) -> Result<()> {
    let iv = Interval::parse(interval)?;
    let c = tc_broker_bitvavo::BitvavoClient::new(tc_broker_bitvavo::BitvavoConfig::new(
        tc_broker_bitvavo::DEFAULT_BASE_URL,
    ))?;
    let end = Utc::now();
    let candles = c
        .fetch_history(market, iv, end - Duration::days(days), end)
        .await?;
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p)?;
    }
    let mut s = String::from("ts,open,high,low,close,volume\n");
    for k in &candles {
        s.push_str(&format!(
            "{},{},{},{},{},{}\n",
            k.ts.timestamp_millis(),
            k.open,
            k.high,
            k.low,
            k.close,
            k.volume
        ));
    }
    std::fs::write(out, s)?;
    println!("{} {} candles → {}", candles.len(), market, out.display());
    Ok(())
}

fn read_csv(path: &Path) -> Result<Vec<Candle>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 6 {
            continue;
        }
        let ms: i64 = f[0].parse()?;
        out.push(Candle {
            ts: chrono::DateTime::from_timestamp_millis(ms).context("bad ts")?,
            open: f[1].parse()?,
            high: f[2].parse()?,
            low: f[3].parse()?,
            close: f[4].parse()?,
            volume: f[5].parse()?,
        });
    }
    out.sort_by_key(|c| c.ts);
    Ok(out)
}

async fn backtest(
    strategy: &str,
    data: &[PathBuf],
    interval: &str,
    out_dir: &Path,
    promote: bool,
    market_group: &str,
    asset_class: &str,
) -> Result<()> {
    if data.is_empty() {
        bail!("--data required");
    }
    let iv = Interval::parse(interval)?;
    let (wf, bpy) = match iv {
        Interval::D1 => (WalkForward::daily(), 365.0),
        Interval::H4 => (WalkForward::four_hour(), 365.0 * 6.0),
        _ => (WalkForward::hourly(), 365.0 * 24.0),
    };
    let families: Vec<String> = if strategy == "all" {
        tc_strategy::registry()
            .iter()
            .map(|s| s.name().to_owned())
            .collect()
    } else {
        vec![strategy.to_owned()]
    };
    std::fs::create_dir_all(out_dir)?;
    let rules = PromotionRules::default();
    let mut summary = Vec::new();
    for fam in &families {
        let mut reports: Vec<Report> = Vec::new();
        for path in data {
            let market = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("market")
                .to_owned();
            let candles = read_csv(path)?;
            let s = Series::from_candles(&candles);
            let sim = SimParams {
                bars_per_year: bpy,
                ..SimParams::default()
            };
            match walk_forward(
                &s,
                fam,
                &market,
                interval,
                Costs::bitvavo_default(),
                sim,
                wf,
                &rules,
            ) {
                Some(r) => {
                    std::fs::write(
                        out_dir.join(format!("{fam}-{market}.json")),
                        serde_json::to_string_pretty(&r)?,
                    )?;
                    println!(
                        "{fam:<18} {market:<14} folds {:>2} OOS trades {:>3} net {:>7.1}% Sharpe {:>5.2} PF {:>5.2} maxDD {:>5.1}% DSR {:.2} eligible={}",
                        r.folds.len(),
                        r.oos.trades,
                        r.oos.net_return * 100.0,
                        r.oos.sharpe,
                        r.oos.profit_factor,
                        r.oos.max_drawdown * 100.0,
                        r.deflated_sharpe,
                        r.promotion.eligible
                    );
                    reports.push(r);
                }
                None => println!("{fam:<18} {market:<14} not enough data for walk-forward"),
            }
        }
        if reports.is_empty() {
            continue;
        }
        // Pool OOS trades across markets (daily strategies need ≥30 pooled trades).
        let pooled: Vec<tc_backtest::Trade> =
            reports.iter().flat_map(|r| r.oos_trades.clone()).collect();
        let mut m = tc_backtest::Metrics::compute(&pooled, &[], 0, bpy);
        m.sharpe = reports.iter().map(|r| r.oos.sharpe).sum::<f64>() / reports.len() as f64;
        m.max_drawdown = reports
            .iter()
            .map(|r| r.oos.max_drawdown)
            .fold(0.0, f64::max);
        let pf2 = reports
            .iter()
            .map(|r| r.oos_double_cost_profit_factor)
            .fold(f64::INFINITY, f64::min);
        let dsr = reports.iter().map(|r| r.deflated_sharpe).sum::<f64>() / reports.len() as f64;
        let folds: Vec<&tc_backtest::FoldReport> =
            reports.iter().flat_map(|r| r.folds.iter()).collect();
        let pos = folds.iter().filter(|f| f.oos.net_return > 0.0).count() as f64
            / folds.len().max(1) as f64;
        let is_sharpe =
            reports.iter().map(|r| r.is_sharpe_mean).sum::<f64>() / reports.len() as f64;
        let promo = rules.evaluate(&m, pf2, dsr, pos, is_sharpe);
        println!(
            "{fam:<18} POOLED         trades {:>3} PF {:.2} expectancy {:.2}R mean Sharpe {:.2} → eligible={} {}",
            m.trades,
            m.profit_factor,
            m.expectancy_r,
            m.sharpe,
            promo.eligible,
            if promo.failed.is_empty() { String::new() } else { format!("({})", promo.failed.join("; ")) }
        );
        let edges = tc_backtest::regime_edges(&pooled);
        summary.push(serde_json::json!({ "strategy": fam, "pooled": m, "promotion": promo, "regime_edges": edges, "markets": reports.iter().map(|r| r.market.clone()).collect::<Vec<_>>() }));
        if promote {
            let store = Store::connect(&database_url()?, 2).await?;
            let status = if promo.eligible {
                "live_half"
            } else {
                "disabled"
            };
            store
                .upsert_strategy(&tc_store::trading::StrategyRow {
                    name: fam.clone(),
                    version: 1,
                    status: status.into(),
                    asset_class: asset_class.into(),
                    report: serde_json::json!({ "oos": { "trades": m.trades, "sharpe": m.sharpe, "profit_factor": m.profit_factor, "expectancy_r": m.expectancy_r, "max_dd_pct": m.max_drawdown * 100.0 }, "promotion": promo, "dsr": dsr }),
                    updated_at: Utc::now(),
                })
                .await?;
            let rows: Vec<tc_store::trading::EdgeRow> = edges
                .iter()
                .map(|e| tc_store::trading::EdgeRow {
                    strategy: fam.clone(),
                    market_group: market_group.into(),
                    regime: e.regime.as_str().into(),
                    trades: e.trades as i32,
                    mean_gross_bps: e.mean_gross_bps,
                    se_bps: e.se_bps,
                    win_rate: e.win_rate,
                })
                .collect();
            store.put_edges(&rows).await?;
            store
                .audit(
                    "cli",
                    "strategy_status",
                    &serde_json::json!({ "strategy": fam, "status": status }),
                )
                .await?;
            println!("{fam}: status → {status}");
        }
    }
    std::fs::write(
        out_dir.join("summary.json"),
        serde_json::to_string_pretty(&summary)?,
    )?;
    Ok(())
}

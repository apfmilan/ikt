//! Market agents: FX, instrument metadata + new-listing detection, universe scanner,
//! candle refresh, regime and volatility-shock detection.

use chrono::{Duration, Utc};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde_json::json;
use std::time::Instant;
use tc_broker::{BrokerClient, MarketDataSource};
use tc_core::{AssetClass, Broker, Instrument, InstrumentId, Interval};
use tc_strategy::{regime, RegimeTracker, Series};
use tracing::warn;

use crate::Engine;

/// Liquid US large caps tradable on Trading 212 (`*_US_EQ` tickers map to EODHD `<SYM>.US`).
/// Only those present and tradable in the account's instrument list are used.
pub const STOCK_UNIVERSE: &[&str] = &[
    "AAPL", "MSFT", "NVDA", "AMZN", "GOOGL", "META", "JPM", "V", "JNJ", "XOM",
];

pub const CRYPTO_UNIVERSE_SIZE: usize = 12;

/// Fiat-pegged assets have no directional edge to trade; excluded from the scanner universe.
pub const STABLECOINS: &[&str] = &[
    "USDC", "USDT", "EURC", "EURT", "DAI", "TUSD", "PYUSD", "FDUSD", "USDE", "USDS", "EURS",
    "EUROC",
];

pub async fn refresh_fx(eng: &Engine) {
    let t = Instant::now();
    let fx = match tc_marketdata::EcbFx::official() {
        Ok(c) => c.fetch().await,
        Err(e) => Err(e),
    };
    match fx {
        Ok(table) => {
            let rates: Vec<(String, Decimal)> = table
                .per_eur
                .iter()
                .map(|(c, r)| (c.to_string(), *r))
                .collect();
            if let Err(e) = eng.store.put_fx(table.as_of, &rates, table.source).await {
                warn!(error = %e, "fx store failed");
            }
            let huf = table.huf_per(tc_core::Currency::EUR);
            *eng.fx.write().await = Some(table);
            eng.agent(
                "fx",
                "active",
                t,
                format!(
                    "ECB EUR/HUF {}",
                    huf.map(|h| h.round_dp(2).to_string()).unwrap_or_default()
                ),
                json!({}),
            )
            .await;
        }
        Err(e) => {
            // Fall back to the last stored table (risk engine vetoes entries if it gets too old).
            if eng.fx.read().await.is_none() {
                if let Ok(Some((date, rates))) = eng.store.latest_fx().await {
                    let per_eur = rates
                        .into_iter()
                        .filter_map(|(c, r)| tc_core::Currency::new(&c).ok().map(|c| (c, r)))
                        .collect();
                    *eng.fx.write().await = Some(tc_marketdata::FxTable {
                        as_of: date,
                        source: "stored",
                        per_eur,
                    });
                }
            }
            eng.agent(
                "fx",
                "degraded",
                t,
                format!("ECB fetch failed: {e}"),
                json!({}),
            )
            .await;
        }
    }
}

/// Refresh instrument metadata; detect Bitvavo listings not seen before.
pub async fn refresh_instruments(eng: &Engine) {
    let t = Instant::now();
    let known: std::collections::HashSet<String> = eng
        .store
        .instruments(Some(Broker::Bitvavo))
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|i| i.id.as_str().to_owned())
        .collect();
    match eng.public_bitvavo.instruments().await {
        Ok(list) => {
            let mut new = Vec::new();
            for i in &list {
                if !known.is_empty() && !known.contains(i.id.as_str()) {
                    new.push(i.id.as_str().to_owned());
                }
                let _ = eng.store.upsert_instrument(i, &json!({})).await;
            }
            if !new.is_empty() {
                eng.event(
                    "alert",
                    json!({ "kind": "new_listing", "instruments": new }),
                )
                .await;
                crate::notify::send(
                    eng,
                    "listing",
                    "🆕 Új Bitvavo listázás",
                    &format!("{} (90 napig nem kereskedünk vele)", new.join(", ")),
                )
                .await;
            }
            eng.agent(
                "listings",
                "active",
                t,
                format!("{} markets, {} new", list.len(), new.len()),
                json!({ "markets": list.len(), "new": new }),
            )
            .await;
        }
        Err(e) => {
            eng.agent(
                "listings",
                "degraded",
                t,
                format!("markets fetch failed: {e}"),
                json!({}),
            )
            .await
        }
    }
    let t212 = eng.brokers.read().await.t212.clone();
    if let Some(c) = t212 {
        match c.instruments().await {
            Ok(list) => {
                for i in list.iter().filter(|i| {
                    i.id.symbol().ends_with("_US_EQ") || i.asset_class == AssetClass::Etf
                }) {
                    let _ = eng.store.upsert_instrument(i, &json!({})).await;
                }
            }
            Err(e) => warn!(error = %e, "trading212 instruments fetch failed"),
        }
    }
}

/// Scanner: pick the liquid crypto universe, refresh daily candles, update regimes.
pub async fn scan(eng: &Engine) {
    let t = Instant::now();
    let (_, rc, _) = eng.risk_engine().await;
    let mut universe: Vec<(InstrumentId, Decimal)> = Vec::new();
    match eng.public_bitvavo.tickers_24h().await {
        Ok(tickers) => {
            let instruments = eng
                .store
                .instruments(Some(Broker::Bitvavo))
                .await
                .unwrap_or_default();
            for (m, _last, vq) in tickers {
                let id = InstrumentId::new(Broker::Bitvavo, &m);
                let Some(inst) = instruments.iter().find(|i| i.id == id) else {
                    continue;
                };
                let base = m.split('-').next().unwrap_or_default();
                if STABLECOINS.contains(&base) {
                    continue;
                }
                if !inst.tradable || inst.quote_currency != tc_core::Currency::EUR {
                    continue;
                }
                if let Some(v) = vq {
                    if v >= rc.min_volume_24h_eur_crypto {
                        universe.push((id, v));
                    }
                }
            }
        }
        Err(e) => {
            eng.agent(
                "scanner",
                "degraded",
                t,
                format!("ticker scan failed: {e}"),
                json!({}),
            )
            .await;
            return;
        }
    }
    universe.sort_by_key(|x| std::cmp::Reverse(x.1));
    universe.truncate(CRYPTO_UNIVERSE_SIZE);
    let ids: Vec<InstrumentId> = universe.iter().map(|(i, _)| i.clone()).collect();
    *eng.universe.write().await = ids.clone();
    eng.agent(
        "scanner",
        "active",
        t,
        format!("{} liquid EUR markets (24h volume ≥ €{})", ids.len(), rc.min_volume_24h_eur_crypto),
        json!({ "universe": universe.iter().map(|(i, v)| json!({ "instrument": i.as_str(), "volume_24h_eur": v.round_dp(0) })).collect::<Vec<_>>() }),
    )
    .await;

    // Candles (daily) and regimes.
    let t2 = Instant::now();
    let mut regimes = Vec::new();
    let mut shocks = Vec::new();
    for id in &ids {
        if let Err(e) = refresh_candles(eng, id, Interval::D1).await {
            warn!(instrument = %id, error = %e, "candle refresh failed");
            continue;
        }
        if let Some((r, shock)) = update_regime(eng, id).await {
            regimes.push(json!({ "instrument": id.as_str(), "regime": r.as_str() }));
            if let Some((vz, zr)) = shock {
                if vz >= 3.0 || zr.abs() >= 4.0 {
                    shocks
                        .push(json!({ "instrument": id.as_str(), "volume_z": vz, "return_z": zr }));
                }
            }
        }
    }
    eng.agent(
        "regime",
        "active",
        t2,
        format!("{} instruments classified", regimes.len()),
        json!({ "regimes": regimes }),
    )
    .await;
    eng.agent(
        "volatility",
        if shocks.is_empty() {
            "active"
        } else {
            "degraded"
        },
        t2,
        if shocks.is_empty() {
            "no volume/return shocks".to_owned()
        } else {
            format!("{} shock(s): entries blocked for 2 bars", shocks.len())
        },
        json!({ "shocks": shocks }),
    )
    .await;
    eng.agent(
        "news",
        "idle",
        t2,
        "No news source configured: news/sentiment is not used for decisions",
        json!({}),
    )
    .await;

    stock_scan(eng).await;
}

/// Fetch closed candles from the right source and store them. Returns the number of new bars.
pub async fn refresh_candles(
    eng: &Engine,
    id: &InstrumentId,
    interval: Interval,
) -> anyhow::Result<usize> {
    let existing = eng.store.candles(id, interval, 1).await?;
    if let Some(last) = existing.last() {
        // The next bar is closed once `last.ts + 2*interval` has passed.
        if Utc::now() < last.ts + Duration::seconds(2 * interval.seconds()) {
            return Ok(0);
        }
    }
    let limit = if existing.is_empty() { 400 } else { 10 };
    let candles = match id.broker() {
        Broker::Bitvavo => {
            eng.public_bitvavo
                .candles(id, interval, limit, None)
                .await?
        }
        Broker::Trading212 => {
            let md = eng
                .brokers
                .read()
                .await
                .eodhd
                .clone()
                .ok_or_else(|| anyhow::anyhow!("no EODHD key"))?;
            md.candles(id, interval, limit, None).await?
        }
        Broker::Revolut => anyhow::bail!("no market data for revolut ids"),
    };
    if existing.is_empty() {
        if let Some(first) = candles.first() {
            // Fewer bars than requested means the market is younger than the window.
            let listed = if candles.len() < limit {
                first.ts
            } else {
                first.ts - Duration::days(365)
            };
            eng.listed_at.lock().await.insert(id.clone(), listed);
        }
    }
    eng.store.upsert_candles(id, interval, &candles).await?;
    Ok(candles.len())
}

/// Update the hysteresis regime tracker for an instrument; returns (regime, shock z-scores).
pub async fn update_regime(
    eng: &Engine,
    id: &InstrumentId,
) -> Option<(regime::Regime, Option<(f64, f64)>)> {
    let candles = eng.store.candles(id, Interval::D1, 700).await.ok()?;
    let s = Series::from_candles(&candles);
    if s.len() < 160 {
        return None;
    }
    let last_ts = *s.ts.last()?;
    let mut map = eng.regimes.lock().await;
    let fresh = !map.contains_key(id);
    {
        let mut bars = eng.regime_bar.lock().await;
        if !fresh && bars.get(id) == Some(&last_ts) {
            // Same closed bar as last scan: hysteresis must not advance again.
            return Some((
                map.get(id)
                    .map(|t| t.current)
                    .unwrap_or(regime::Regime::Transition),
                tc_strategy::shock(&s),
            ));
        }
        bars.insert(id.clone(), last_ts);
    }
    let tracker = map.entry(id.clone()).or_insert_with(RegimeTracker::default);
    // Replay the last bars for a new tracker so hysteresis state is meaningful after restart.
    let from = if fresh {
        s.len().saturating_sub(6)
    } else {
        s.len() - 1
    };
    for end in from..s.len() {
        if let Some(i) = regime::inputs(&s.prefix(end + 1), 365.0) {
            tracker.update(regime::classify(&i, false));
        }
    }
    Some((tracker.current, tc_strategy::shock(&s)))
}

/// Stocks: requires an EODHD key and Trading 212 instruments; daily bars after the US close.
async fn stock_scan(eng: &Engine) {
    let t = Instant::now();
    let b = eng.brokers.read().await.clone();
    let (Some(eod), Some(_t212)) = (b.eodhd, b.t212) else {
        eng.agent(
            "bot_stocks",
            "blocked",
            t,
            "Részvény adatforrás (EODHD) vagy Trading 212 kulcs hiányzik",
            json!({}),
        )
        .await;
        return;
    };
    let instruments = eng
        .store
        .instruments(Some(Broker::Trading212))
        .await
        .unwrap_or_default();
    let mut n = 0;
    for sym in STOCK_UNIVERSE {
        let id = InstrumentId::new(Broker::Trading212, &format!("{sym}_US_EQ"));
        let Some(inst) = instruments.iter().find(|i| i.id == id) else {
            continue;
        };
        if !inst.tradable {
            continue;
        }
        let _ = eod.set_symbol(id.clone(), &format!("{sym}.US"));
        match refresh_candles(eng, &id, Interval::D1).await {
            Ok(_) => {
                n += 1;
                let _ = update_regime(eng, &id).await;
            }
            Err(e) => warn!(instrument = %id, error = %e, "stock candles failed"),
        }
    }
    eng.agent(
        "technical",
        "active",
        t,
        format!("{n} stock(s) with fresh daily bars"),
        json!({ "stocks": n }),
    )
    .await;
}

/// Latest close for an instrument from stored candles (H1 preferred, then D1).
pub async fn last_price(eng: &Engine, id: &InstrumentId) -> Option<Decimal> {
    for iv in [Interval::H1, Interval::D1] {
        if let Ok(c) = eng.store.candles(id, iv, 1).await {
            if let Some(c) = c.last() {
                return Some(c.close);
            }
        }
    }
    None
}

pub fn to_f64(d: Decimal) -> f64 {
    d.to_f64().unwrap_or(f64::NAN)
}

/// Instrument metadata lookup.
pub async fn instrument(eng: &Engine, id: &InstrumentId) -> Option<Instrument> {
    eng.store.instrument(id).await.ok().flatten()
}

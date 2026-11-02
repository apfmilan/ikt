//! Opening sealed credentials (engine only) and verifying them.

use chrono::Utc;
use secrecy::SecretString;
use serde_json::json;
use std::sync::Arc;
use tc_broker::BrokerClient;
use tc_broker_bitvavo::{BitvavoClient, BitvavoConfig};
use tc_broker_t212::{Trading212Client, Trading212Config};
use tc_marketdata::{EodhdClient, EodhdConfig, EODHD_BASE_URL};
use tc_store::control::CredentialStatusRow;
use tracing::{info, warn};

use crate::Engine;

#[derive(Default, Clone)]
pub struct Brokers {
    pub bitvavo: Option<Arc<BitvavoClient>>,
    pub t212: Option<Arc<Trading212Client>>,
    pub eodhd: Option<Arc<EodhdClient>>,
}

async fn open(eng: &Engine, broker: &str, field: &str) -> Option<SecretString> {
    let key = eng.key.as_ref()?;
    let (kv, env) = eng.store.credential(broker, field).await.ok().flatten()?;
    match key.open_string(&env, &tc_vault::context(broker, field, kv.max(0) as u32)) {
        Ok(s) => Some(s),
        Err(e) => {
            warn!(broker, field, error = %e, "credential could not be opened");
            None
        }
    }
}

/// (Re)build broker clients from sealed credentials.
pub async fn load_all(eng: &Engine) {
    let mut b = Brokers::default();
    if let (Some(k), Some(s), Some(op)) = (
        open(eng, "bitvavo", "api_key").await,
        open(eng, "bitvavo", "api_secret").await,
        open(eng, "bitvavo", "operator_id").await,
    ) {
        let op: i64 = secrecy::ExposeSecret::expose_secret(&op)
            .parse()
            .unwrap_or(0);
        match BitvavoClient::new(BitvavoConfig::with_credentials(
            tc_broker_bitvavo::DEFAULT_BASE_URL,
            k,
            s,
            op,
        )) {
            Ok(c) => b.bitvavo = Some(Arc::new(c)),
            Err(e) => warn!(error = %e, "bitvavo client init failed"),
        }
    }
    if let (Some(k), Some(s)) = (
        open(eng, "trading212", "api_key").await,
        open(eng, "trading212", "api_secret").await,
    ) {
        match Trading212Client::new(Trading212Config::live(k, s)) {
            Ok(c) => {
                if let Err(e) = c.load_exchanges().await {
                    warn!(error = %e, "trading212 exchange metadata unavailable");
                }
                b.t212 = Some(Arc::new(c));
            }
            Err(e) => warn!(error = %e, "trading212 client init failed"),
        }
    }
    if let Some(tok) = open(eng, "eodhd", "api_key").await {
        match EodhdClient::new(EodhdConfig::new(EODHD_BASE_URL, tok)) {
            Ok(c) => b.eodhd = Some(Arc::new(c)),
            Err(e) => warn!(error = %e, "eodhd client init failed"),
        }
    }
    info!(
        bitvavo = b.bitvavo.is_some(),
        trading212 = b.t212.is_some(),
        eodhd = b.eodhd.is_some(),
        "broker clients loaded"
    );
    *eng.brokers.write().await = b;
}

/// Verify one broker's credentials and persist the status. Called on onboarding command.
pub async fn verify(eng: &Engine, broker: &str) -> serde_json::Value {
    load_all(eng).await;
    let brokers = eng.brokers.read().await.clone();
    let existing = eng.store.credential_statuses().await.unwrap_or_default();
    let attested = existing
        .iter()
        .find(|c| c.broker == broker)
        .is_some_and(|c| c.attested_no_withdraw);
    let mut status = CredentialStatusRow {
        broker: broker.to_owned(),
        authenticated: false,
        can_trade: false,
        can_withdraw: None,
        attested_no_withdraw: attested,
        scopes_ok: false,
        account_currency: None,
        notes: json!([]),
        checked_at: tc_store::now_micros(),
    };
    let mut notes: Vec<String> = Vec::new();
    let result = match broker {
        "bitvavo" => match &brokers.bitvavo {
            None => Err("credentials missing or unreadable".to_owned()),
            Some(c) => match c.verify_credentials().await {
                Ok(r) => {
                    status.authenticated = r.authenticated;
                    status.can_trade = r.can_trade;
                    status.can_withdraw = r.can_withdraw;
                    status.scopes_ok = r.authenticated && r.can_read_account;
                    status.account_currency = r.account_currency.map(|c| c.to_string());
                    notes.extend(r.notes);
                    if let Ok(acc) = c.account().await {
                        notes.push(format!("{} balance line(s) read", acc.cash.len()));
                    }
                    Ok(())
                }
                Err(e) => Err(e.to_string()),
            },
        },
        "trading212" => match &brokers.t212 {
            None => Err("credentials missing or unreadable".to_owned()),
            Some(c) => match c.verify_credentials().await {
                Ok(r) => {
                    status.authenticated = r.authenticated;
                    status.can_trade = r.can_trade;
                    status.can_withdraw = r.can_withdraw;
                    // Read scopes verified; the orders:execute scope is confirmed by the first order
                    // (a 403 then disables the broker and alerts).
                    status.scopes_ok = r.authenticated && r.can_read_account;
                    status.account_currency = r.account_currency.map(|c| c.to_string());
                    notes.extend(r.notes);
                    Ok(())
                }
                Err(e) => Err(e.to_string()),
            },
        },
        "eodhd" => match &brokers.eodhd {
            None => Err("API key missing or unreadable".to_owned()),
            Some(c) => {
                let id = tc_core::InstrumentId::new(tc_core::Broker::Trading212, "AAPL_US_EQ");
                let _ = c.set_symbol(id.clone(), "AAPL.US");
                match tc_broker::MarketDataSource::candles(
                    c.as_ref(),
                    &id,
                    tc_core::Interval::D1,
                    5,
                    None,
                )
                .await
                {
                    Ok(v) => {
                        status.authenticated = !v.is_empty();
                        status.scopes_ok = status.authenticated;
                        status.can_withdraw = Some(false);
                        notes.push(format!("{} daily bars received for AAPL.US", v.len()));
                        Ok(())
                    }
                    Err(e) => Err(e.to_string()),
                }
            }
        },
        _ => Err("unknown broker".to_owned()),
    };
    if let Err(e) = &result {
        notes.push(format!("error: {e}"));
    }
    if status.can_withdraw == Some(true) {
        notes.push(
            "API key can WITHDRAW funds: refused. Create a key without withdrawal permission."
                .into(),
        );
        let _ = eng
            .store
            .revoke_live(
                tc_core::Broker::parse(broker).ok(),
                "withdraw-capable key",
                "engine",
            )
            .await;
    }
    status.notes = json!(notes);
    if let Err(e) = eng.store.put_credential_status(&status).await {
        warn!(error = %e, "credential status write failed");
    }
    eng.event(
        "onboarding",
        json!({ "broker": broker, "authenticated": status.authenticated, "at": Utc::now() }),
    )
    .await;
    json!({ "broker": broker, "authenticated": status.authenticated, "can_trade": status.can_trade, "can_withdraw": status.can_withdraw, "notes": notes })
}

/// Re-verify configured brokers once a day (permissions can change).
pub async fn periodic_verify(eng: &Engine) {
    let last = eng.store.credential_statuses().await.unwrap_or_default();
    for b in eng.store.credential_brokers().await.unwrap_or_default() {
        let stale = last
            .iter()
            .find(|c| c.broker == b)
            .is_none_or(|c| Utc::now() - c.checked_at > chrono::Duration::hours(24));
        if stale {
            verify(eng, &b).await;
        }
    }
}

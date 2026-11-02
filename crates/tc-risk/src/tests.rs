//! Test matrix from docs/decisions/proposals/risk-engine.md §7 (E = 25,000 HUF, €1 = 400 HUF).

use super::*;
use chrono::TimeZone;
use rust_decimal_macros::dec;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 2, 14, 0, 0)
        .single()
        .expect("valid time")
}

fn btc() -> InstrumentId {
    InstrumentId::new(Broker::Bitvavo, "BTC-EUR")
}

fn engine() -> RiskEngine {
    RiskEngine::new(RiskConfig::default()).expect("valid config")
}

fn entry() -> RiskRequest {
    RiskRequest {
        instrument: btc(),
        asset_class: AssetClass::Crypto,
        side: Side::Buy,
        class: OrderClass::Entry,
        strategy: "trend_vol".into(),
        quantity: dec!(0.0002),
        price: dec!(70000),
        stop_price: Some(dec!(67900)),
        gap_p99: dec!(0.02),
        edge_bps: dec!(150),
        edge_se_bps: dec!(30),
        size_multiplier: dec!(1),
        cluster: "crypto".into(),
        created_at: now(),
    }
}

fn exit(class: OrderClass) -> RiskRequest {
    RiskRequest {
        side: Side::Sell,
        class,
        stop_price: None,
        edge_bps: dec!(0),
        edge_se_bps: dec!(0),
        ..entry()
    }
}

fn snap() -> RiskSnapshot {
    let n = now();
    RiskSnapshot {
        live_authorized: [(Broker::Bitvavo, true), (Broker::Trading212, true)]
            .into_iter()
            .collect(),
        strategy_approved: true,
        kill_global: KillMode::Normal,
        kill_broker: BTreeMap::new(),
        kill_strategy: BTreeMap::new(),
        credentials: [Broker::Bitvavo, Broker::Trading212]
            .into_iter()
            .map(|b| {
                (
                    b,
                    CredentialStatus {
                        withdraw_disabled_verified: true,
                        withdraw_disabled_attested: false,
                        scopes_ok: true,
                        checked_at: n,
                    },
                )
            })
            .collect(),
        health: [Broker::Bitvavo, Broker::Trading212]
            .into_iter()
            .map(|b| {
                (
                    b,
                    BrokerHealth {
                        last_ok: Some(n - Duration::seconds(5)),
                        last_rate_limited: None,
                        rate_budget_remaining: dec!(0.9),
                        error_rate_5m: dec!(0),
                        clock_skew_ms: 20,
                    },
                )
            })
            .collect(),
        market: Some(MarketData {
            quote_ts: n - Duration::seconds(1),
            bid: Some(dec!(69995)),
            ask: Some(dec!(70005)),
            liquidity_notional: Some(dec!(27000000)),
            candle_ts: n - Duration::minutes(30),
            candle_interval_secs: 3600,
        }),
        instrument: Some(InstrumentStatus {
            tradable: true,
            eligible: true,
            ineligible_reason: None,
            listed_at: n - Duration::days(800),
            min_notional: dec!(5),
            min_quantity: dec!(0.00007),
            quantity_decimals: 8,
            session: None,
            long_term_whitelisted: false,
        }),
        fx_to_huf: Some((dec!(400), n - Duration::days(1))),
        account_ts: n - Duration::seconds(5),
        recon_mismatch: false,
        nav_unit: dec!(1),
        nav_unit_day_start: dec!(1),
        nav_unit_week_start: dec!(1),
        nav_unit_peak: dec!(1),
        drawdown_halt_latched: false,
        equity_huf: dec!(25000),
        cash_available_huf: dec!(20000),
        cash_reserved_local_huf: dec!(0),
        taker_fee: dec!(0.0025),
        positions: vec![],
        intents: vec![],
        closed_trades: vec![],
        entries_last_hour: 0,
        entries_today: 0,
    }
}

fn with_btc_position(mut s: RiskSnapshot, free: Decimal) -> RiskSnapshot {
    s.positions.push(PositionExposure {
        instrument: btc(),
        asset_class: AssetClass::Crypto,
        cluster: "crypto".into(),
        value_huf: free * dec!(70000) * dec!(400),
        free_quantity: free,
        long_term: false,
    });
    s
}

fn codes(v: &Verdict) -> Vec<VetoCode> {
    v.codes()
}

#[test]
fn t24_all_checks_pass() {
    let v = engine().evaluate(&entry(), &snap(), now());
    assert_eq!(
        v,
        Verdict::Allow {
            quantity: dec!(0.0002)
        }
    );
}

#[test]
fn t01_live_not_authorized() {
    let mut s = snap();
    s.live_authorized.insert(Broker::Bitvavo, false);
    assert!(codes(&engine().evaluate(&entry(), &s, now())).contains(&VetoCode::LiveNotAuthorized));
}

#[test]
fn t02_reduce_only_blocks_entries_not_exits() {
    let mut s = with_btc_position(snap(), dec!(0.0002));
    s.kill_global = KillMode::ReduceOnly;
    assert!(codes(&engine().evaluate(&entry(), &s, now())).contains(&VetoCode::KillSwitch));
    let v = engine().evaluate(&exit(OrderClass::ExitProtective), &s, now());
    assert_eq!(
        v,
        Verdict::Allow {
            quantity: dec!(0.0002)
        }
    );
}

#[test]
fn freeze_blocks_everything_engine_sent() {
    let mut s = with_btc_position(snap(), dec!(0.0002));
    s.kill_broker.insert(Broker::Bitvavo, KillMode::Freeze);
    assert!(
        codes(&engine().evaluate(&exit(OrderClass::ExitProtective), &s, now()))
            .contains(&VetoCode::KillSwitch)
    );
}

#[test]
fn t03_withdraw_capable_key_vetoes() {
    let mut s = snap();
    if let Some(c) = s.credentials.get_mut(&Broker::Bitvavo) {
        c.withdraw_disabled_verified = false;
    }
    assert!(codes(&engine().evaluate(&entry(), &s, now())).contains(&VetoCode::CredWithdrawCapable));
}

#[test]
fn t04_stale_quote_blocks_entry_not_protective_exit() {
    let mut s = with_btc_position(snap(), dec!(0.0002));
    if let Some(m) = s.market.as_mut() {
        m.quote_ts = now() - Duration::seconds(9);
    }
    assert!(codes(&engine().evaluate(&entry(), &s, now())).contains(&VetoCode::DataStale));
    assert!(!engine()
        .evaluate(&exit(OrderClass::ExitProtective), &s, now())
        .is_veto());
}

#[test]
fn t05_missing_spread_vetoes() {
    let mut s = snap();
    if let Some(m) = s.market.as_mut() {
        m.bid = None;
    }
    assert!(codes(&engine().evaluate(&entry(), &s, now())).contains(&VetoCode::DataMissing));
}

#[test]
fn t06_daily_loss() {
    let mut s = snap();
    s.nav_unit = dec!(0.9696); // -3.04 % (= -760 HUF on 25,000)
    assert!(codes(&engine().evaluate(&entry(), &s, now())).contains(&VetoCode::DailyLoss));
}

#[test]
fn t07_oversized_order_is_resized_down() {
    let r = RiskRequest {
        quantity: dec!(0.001),
        ..entry()
    };
    match engine().evaluate(&r, &snap(), now()) {
        Verdict::AllowResized {
            quantity,
            requested,
            ..
        } => {
            assert!(quantity < requested);
            // Risk budget: qty * ((70000-67900)*400 + 70000*400*0.005) <= 250 HUF
            let risk = quantity * (dec!(2100) * dec!(400) + dec!(70000) * dec!(400) * dec!(0.005));
            assert!(risk <= dec!(250), "risk {risk}");
            assert!(quantity * dec!(70000) * dec!(400) <= dec!(6250));
        }
        other => panic!("expected resize, got {other:?}"),
    }
}

#[test]
fn t08_position_cap_aggregates_pending() {
    let mut s = with_btc_position(snap(), dec!(0.00017857)); // ≈5,000 HUF
    s.intents.push(IntentSummary {
        instrument: btc(),
        side: Side::Buy,
        state: OrderState::Open,
        class: OrderClass::Entry,
        created_at: now() - Duration::hours(2),
        notional_huf: dec!(1000),
    });
    let c = codes(&engine().evaluate(&entry(), &s, now()));
    assert!(c.contains(&VetoCode::PositionCap), "{c:?}");
}

#[test]
fn t09_class_cap() {
    let mut s = snap();
    s.positions.push(PositionExposure {
        instrument: InstrumentId::new(Broker::Bitvavo, "ETH-EUR"),
        asset_class: AssetClass::Crypto,
        cluster: "crypto_eth".into(),
        value_huf: dec!(9500),
        free_quantity: dec!(1),
        long_term: false,
    });
    let r = RiskRequest {
        cluster: "crypto_btc".into(),
        ..entry()
    };
    let c = codes(&engine().evaluate(&r, &s, now()));
    assert!(c.contains(&VetoCode::ClassCap), "{c:?}");
}

#[test]
fn t10_illiquid() {
    let mut s = snap();
    if let Some(m) = s.market.as_mut() {
        m.liquidity_notional = Some(dec!(800000));
    }
    assert!(codes(&engine().evaluate(&entry(), &s, now())).contains(&VetoCode::Illiquid));
}

#[test]
fn t11_wide_spread_blocks_entry_not_protective_exit() {
    let mut s = with_btc_position(snap(), dec!(0.0002));
    if let Some(m) = s.market.as_mut() {
        m.bid = Some(dec!(69842));
        m.ask = Some(dec!(70158)); // ≈45 bps
    }
    assert!(codes(&engine().evaluate(&entry(), &s, now())).contains(&VetoCode::SpreadWide));
    assert!(!engine()
        .evaluate(&exit(OrderClass::ExitProtective), &s, now())
        .is_veto());
    assert!(
        codes(&engine().evaluate(&exit(OrderClass::ExitSignal), &s, now()))
            .contains(&VetoCode::SpreadWide)
    );
}

#[test]
fn t12_edge_below_cost() {
    let r = RiskRequest {
        edge_bps: dec!(60),
        edge_se_bps: dec!(20),
        ..entry()
    };
    assert!(codes(&engine().evaluate(&r, &snap(), now())).contains(&VetoCode::EdgeBelowCost));
}

#[test]
fn t13_below_min_order() {
    let r = RiskRequest {
        quantity: dec!(0.00006),
        ..entry()
    }; // €4.20
    assert!(codes(&engine().evaluate(&r, &snap(), now())).contains(&VetoCode::BelowMinOrder));
}

#[test]
fn t14_duplicate_within_window() {
    let mut s = snap();
    s.intents.push(IntentSummary {
        instrument: btc(),
        side: Side::Buy,
        state: OrderState::Filled,
        class: OrderClass::Entry,
        created_at: now() - Duration::seconds(60),
        notional_huf: dec!(0),
    });
    assert!(codes(&engine().evaluate(&entry(), &s, now())).contains(&VetoCode::Duplicate));
}

#[test]
fn t15_unknown_intent_blocks_instrument() {
    let mut s = with_btc_position(snap(), dec!(0.0002));
    s.intents.push(IntentSummary {
        instrument: btc(),
        side: Side::Buy,
        state: OrderState::Unknown,
        class: OrderClass::Entry,
        created_at: now() - Duration::minutes(3),
        notional_huf: dec!(5000),
    });
    assert!(codes(&engine().evaluate(&entry(), &s, now())).contains(&VetoCode::InFlight));
    // Exits too: an uncertain order must be reconciled before anything else is sent.
    assert!(
        codes(&engine().evaluate(&exit(OrderClass::ExitProtective), &s, now()))
            .contains(&VetoCode::InFlight)
    );
}

#[test]
fn t16_loss_streak_cooldown() {
    let mut s = snap();
    for h in [6, 10, 20] {
        s.closed_trades.push(ClosedTrade {
            instrument: InstrumentId::new(Broker::Bitvavo, "SOL-EUR"),
            closed_at: now() - Duration::hours(h),
            net_pnl_huf: dec!(-100),
            stopped_out: false,
        });
    }
    assert!(codes(&engine().evaluate(&entry(), &s, now())).contains(&VetoCode::LossStreak));
}

#[test]
fn size_up_after_loss_and_revenge_reentry() {
    let mut s = snap();
    s.closed_trades.push(ClosedTrade {
        instrument: btc(),
        closed_at: now() - Duration::hours(1),
        net_pnl_huf: dec!(-100),
        stopped_out: true,
    });
    let r = RiskRequest {
        size_multiplier: dec!(1.5),
        ..entry()
    };
    let c = codes(&engine().evaluate(&r, &s, now()));
    assert!(c.contains(&VetoCode::SizeUpAfterLoss));
    assert!(c.contains(&VetoCode::RevengeReentry));
}

#[test]
fn t17_market_hours_buffer() {
    let mut s = snap();
    let open = now() - Duration::minutes(5);
    if let Some(i) = s.instrument.as_mut() {
        i.session = Some(Some((open, open + Duration::hours(6))));
    }
    let r = RiskRequest {
        instrument: InstrumentId::new(Broker::Trading212, "AAPL_US_EQ"),
        asset_class: AssetClass::Stock,
        cluster: "us_tech".into(),
        ..entry()
    };
    assert!(codes(&engine().evaluate(&r, &s, now())).contains(&VetoCode::MarketClosed));
}

#[test]
fn t18_new_listing() {
    let mut s = snap();
    if let Some(i) = s.instrument.as_mut() {
        i.listed_at = now() - Duration::days(20);
    }
    assert!(codes(&engine().evaluate(&entry(), &s, now())).contains(&VetoCode::NewListing));
}

#[test]
fn t19_insufficient_cash_after_floor() {
    let mut s = snap();
    s.cash_available_huf = dec!(3000);
    s.cash_reserved_local_huf = dec!(1000);
    let c = codes(&engine().evaluate(&entry(), &s, now()));
    assert!(c.contains(&VetoCode::InsufficientCash), "{c:?}");
}

#[test]
fn t20_oversell() {
    let s = with_btc_position(snap(), dec!(0.0015));
    let r = RiskRequest {
        quantity: dec!(0.002),
        ..exit(OrderClass::ManualSell)
    };
    assert!(codes(&engine().evaluate(&r, &s, now())).contains(&VetoCode::Oversell));
}

#[test]
fn t21_drawdown_not_hidden_by_deposit() {
    // Unitized NAV: a deposit buys units at current NAV, so nav_unit stays at 0.895.
    let mut s = snap();
    s.nav_unit = dec!(0.895);
    s.equity_huf = dec!(72375); // 22,375 + 50,000 deposit
    assert!(codes(&engine().evaluate(&entry(), &s, now())).contains(&VetoCode::DrawdownHalt));
    let mut s = snap();
    s.drawdown_halt_latched = true;
    assert!(codes(&engine().evaluate(&entry(), &s, now())).contains(&VetoCode::DrawdownHalt));
}

#[test]
fn drawdown_reduce_halves_risk_budget() {
    let mut s = snap();
    s.nav_unit = dec!(0.93);
    let r = RiskRequest {
        quantity: dec!(0.001),
        ..entry()
    };
    let q = engine()
        .evaluate(&r, &s, now())
        .approved_quantity()
        .unwrap_or_default();
    let risk = q * (dec!(2100) * dec!(400) + dec!(70000) * dec!(400) * dec!(0.005));
    assert!(risk <= dec!(125) + dec!(0.01), "risk {risk}");
}

#[test]
fn t23_ineligible_instrument() {
    let mut s = snap();
    if let Some(i) = s.instrument.as_mut() {
        i.eligible = false;
        i.ineligible_reason = Some("leveraged ETP".into());
    }
    assert!(
        codes(&engine().evaluate(&entry(), &s, now())).contains(&VetoCode::InstrumentIneligible)
    );
}

#[test]
fn revolut_is_signal_only() {
    let r = RiskRequest {
        instrument: InstrumentId::new(Broker::Revolut, "AAPL"),
        ..entry()
    };
    assert_eq!(
        codes(&engine().evaluate(&r, &snap(), now())),
        vec![VetoCode::VenueNotTradable]
    );
}

#[test]
fn entry_requires_stop_and_no_shorting() {
    let r = RiskRequest {
        stop_price: None,
        ..entry()
    };
    assert!(codes(&engine().evaluate(&r, &snap(), now())).contains(&VetoCode::RiskPerTrade));
    let r = RiskRequest {
        side: Side::Sell,
        ..entry()
    };
    assert!(codes(&engine().evaluate(&r, &snap(), now())).contains(&VetoCode::Leverage));
}

#[test]
fn unhealthy_broker_blocks_exits_too() {
    let mut s = with_btc_position(snap(), dec!(0.0002));
    if let Some(h) = s.health.get_mut(&Broker::Bitvavo) {
        h.last_rate_limited = Some(now() - Duration::seconds(30));
    }
    assert!(
        codes(&engine().evaluate(&exit(OrderClass::ExitProtective), &s, now()))
            .contains(&VetoCode::BrokerUnhealthy)
    );
}

#[test]
fn long_term_requires_whitelist() {
    let r = RiskRequest {
        class: OrderClass::LongTerm,
        stop_price: None,
        ..entry()
    };
    assert!(codes(&engine().evaluate(&r, &snap(), now())).contains(&VetoCode::InstrumentIneligible));
}

#[test]
fn engine_refuses_invalid_config() {
    let cfg = RiskConfig {
        total_cap: dec!(1.5),
        ..RiskConfig::default()
    };
    assert!(RiskEngine::new(cfg).is_err());
}

#[test]
fn verdict_is_deterministic() {
    let e = engine();
    let a = e.evaluate(&entry(), &snap(), now());
    let b = e.evaluate(&entry(), &snap(), now());
    assert_eq!(a, b);
}

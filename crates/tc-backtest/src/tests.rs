use super::*;
use chrono::{Duration, TimeZone};
use tc_strategy::strategies::TrendVol;

fn synthetic(n: usize, drift: f64, seed: u64) -> Series {
    // Deterministic LCG noise; offline test data only (never shown to users).
    let mut x = seed;
    let mut next = || {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((x >> 33) as f64 / (1u64 << 31) as f64) - 0.5
    };
    let t0 = Utc
        .with_ymd_and_hms(2020, 1, 1, 0, 0, 0)
        .single()
        .expect("time");
    let mut c = 100.0;
    let mut s = Series::default();
    for i in 0..n {
        let o = c;
        c *= 1.0 + drift + 0.02 * next();
        let h = o.max(c) * (1.0 + 0.005 * (next() + 0.5));
        let l = o.min(c) * (1.0 - 0.005 * (next() + 0.5));
        s.ts.push(t0 + Duration::days(i as i64));
        s.open.push(o);
        s.high.push(h);
        s.low.push(l);
        s.close.push(c);
        s.volume.push(1000.0 * (1.0 + next()));
    }
    s
}

#[test]
fn no_trades_means_flat_equity() {
    let s = synthetic(400, 0.0, 7);
    // Threshold impossible to reach: no trades.
    let st = TrendVol {
        entry_threshold: 5.0,
        stop_atr: 2.5,
        trail_atr: 3.0,
    };
    let r = simulate(
        &s,
        &st,
        Costs::bitvavo_default(),
        SimParams::default(),
        200,
        400,
    );
    assert!(r.trades.is_empty());
    assert!(r.bar_returns.iter().all(|x| *x == 0.0));
}

#[test]
fn costs_reduce_returns() {
    let s = synthetic(900, 0.002, 11);
    let st = TrendVol::default();
    let cheap = Costs {
        taker_fee: 0.0,
        maker_fee: 0.0,
        half_spread_bps: 0.0,
        slippage_bps: 0.0,
    };
    let a = simulate(&s, &st, cheap, SimParams::default(), 200, 900);
    let b = simulate(
        &s,
        &st,
        Costs::bitvavo_default().doubled(),
        SimParams::default(),
        200,
        900,
    );
    let ra = a.bar_returns.iter().fold(1.0, |e, x| e * (1.0 + x));
    let rb = b.bar_returns.iter().fold(1.0, |e, x| e * (1.0 + x));
    if !a.trades.is_empty() {
        assert!(rb < ra, "costs must hurt: {rb} vs {ra}");
    }
    for t in &b.trades {
        assert!(t.net_ret < t.gross_ret + 1e-12, "net must be below gross");
    }
}

#[test]
fn no_lookahead_signals_fill_next_bar() {
    let s = synthetic(600, 0.003, 3);
    let r = simulate(
        &s,
        &TrendVol::default(),
        Costs::bitvavo_default(),
        SimParams::default(),
        200,
        600,
    );
    for t in &r.trades {
        assert!(t.exit_ts >= t.entry_ts);
        // Entry happens at a bar open, which is the previous close in this synthetic data,
        // adjusted by spread+slippage (never better than the open).
        let idx =
            s.ts.iter()
                .position(|x| *x == t.entry_ts)
                .expect("entry bar");
        assert!(t.entry_price >= s.open[idx] - 1e-9);
    }
}

#[test]
fn walk_forward_produces_oos_report_and_respects_gates() {
    let s = synthetic(1800, 0.0005, 99);
    let rep = walk_forward(
        &s,
        "s1_trend_vol",
        "SYNTH",
        "1d",
        Costs::bitvavo_default(),
        SimParams::default(),
        WalkForward::daily(),
        &PromotionRules::default(),
    )
    .expect("enough data");
    assert!(!rep.folds.is_empty());
    assert_eq!(rep.trials, 9);
    // Random-walk data must not be promoted.
    assert!(!rep.promotion.eligible, "{:?}", rep.promotion);
    assert!(rep.deflated_sharpe >= 0.0 && rep.deflated_sharpe <= 1.0);
}

#[test]
fn promotion_rules_require_everything() {
    let m = Metrics {
        trades: 40,
        net_return: 0.3,
        cagr: 0.1,
        sharpe: 1.0,
        sortino: 1.5,
        max_drawdown: 0.1,
        win_rate: 0.45,
        profit_factor: 1.6,
        expectancy_r: 0.3,
        expectancy_bps: 120.0,
        avg_bars_held: 12.0,
        exposure: 0.4,
        cost_share: 0.2,
    };
    let r = PromotionRules::default();
    assert!(r.evaluate(&m, 1.2, 0.97, 0.7, 1.2).eligible);
    assert!(!r.evaluate(&m, 1.0, 0.97, 0.7, 1.2).eligible); // fails 2x cost
    assert!(
        !r.evaluate(
            &Metrics {
                trades: 10,
                ..m.clone()
            },
            1.2,
            0.97,
            0.7,
            1.2
        )
        .eligible
    );
    assert!(!r.evaluate(&m, 1.2, 0.5, 0.7, 1.2).eligible); // DSR
}

#[test]
fn dsr_penalises_many_trials() {
    let mut r = vec![];
    for i in 0..500 {
        r.push(if i % 3 == 0 { -0.004 } else { 0.004 });
    }
    let few = deflated_sharpe(&r, &[0.5, 0.6], 2, 365.0);
    let many = deflated_sharpe(
        &r,
        &(0..20).map(|i| i as f64 * 0.2).collect::<Vec<_>>(),
        20,
        365.0,
    );
    assert!(many < few, "{many} < {few}");
}

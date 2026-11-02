---
name: backtest
description: Run and evaluate a historical walk-forward backtest of a Trading Center strategy with out-of-sample validation and realistic costs. Use before enabling or changing any strategy.
---
# Backtest a strategy

1. Fetch real history (public data, no keys): `cargo run -p tc-cli -- fetch-candles --market BTC-EUR --interval 1h --days 365 --out data/candles/BTC-EUR-1h.csv`
2. Run walk-forward: `cargo run --release -p tc-cli -- backtest --strategy <name> --data data/candles/BTC-EUR-1h.csv --folds 5 --train-frac 0.7`
3. Read the JSON report (`target/backtests/<strategy>-<market>.json`). Check per fold OOS: trades >= 30 total, expectancy after costs > 0, max drawdown <= configured ceiling, profit factor > 1.1, haircut Sharpe > 0.
4. Promotion: only `promotion.eligible == true` strategies may be enabled for live trading. Record results in `docs/backtests/<date>-<strategy>.md`.
5. Report failures honestly; never tune parameters on the OOS folds.

Costs model: taker fee from broker (Bitvavo default 0.25%), half spread from recent order book (default 5 bps if unknown), slippage 5 bps. Never set costs to zero.

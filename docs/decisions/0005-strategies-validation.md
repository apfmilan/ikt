# 0005. Strategies, signal fusion and validation before live trading

Status: Accepted · Date: 2026-11-02

## Context
`proposals/quant-research.md`. Tiny account (≈63 EUR), Bitvavo taker 0.25 %/side. Requirement:
quantitative validation, not LLM predictions; abstain when no edge; no paper/demo mode.

## Options
- **A. Hand-tuned rules, go live immediately** — unvalidated.
- **B. Walk-forward OOS validation with deflated Sharpe, promotion/demotion gates (chosen)**.
- **C. ML/LLM signal generation** — opaque, overfit-prone, not explainable.

## Scores
| Option | Security | Correctness | Performance | Maintainability | UX | Cost | Total |
|---|---|---|---|---|---|---|---|
| A | 6 | 3 | 8 | 7 | 6 | 9 | 39 |
| B | 8 | 8 | 7 | 7 | 7 | 9 | 46 |
| C | 5 | 4 | 5 | 4 | 5 | 5 | 28 |

## Decision
- Strategies S1 trend/vol, S2 Donchian, S3 mean reversion (`tc-strategy`), regime gating with
  hysteresis, precision-weighted fusion with correlated-error uncertainty, trade only if
  `mu − cost − 1·sigma > 0` and agreement ≥ 0.65; shock/news agents are veto-only.
- `tc backtest`: walk-forward (24 m IS / 6 m OOS / embargo), ≤ 20 pre-registered parameter sets,
  plateau selection, realistic costs, deflated Sharpe; promotion only if every gate passes; promoted
  strategies start at half risk (`live_half`).
- **No shadow/paper trading**: non-promoted strategies never trade or simulate fills; their
  candidates are recorded only as explainable *abstain* decisions.

## Evidence (2026-11-02, real Bitvavo daily candles, 6 markets, ~5.5 years)
No strategy passed the gates (S1 pooled PF 0.75; S2 27 trades < 30, PF@2×cost 0.31; S3 6 trades).
All strategies stay `disabled`; the engine therefore abstains. See `docs/backtests/2026-11-02.md`.

## Consequences
The system is honest by construction: it will not trade crypto until evidence exists. Stock
strategies additionally require an EODHD key.

# Proposal: Deterministic Risk Engine (Risk Analyst, 2026-11-02)

## 0. Principles

- `verdict = f(intent, snapshot, config, hard_limits, now)`: pure function, `rust_decimal` arithmetic, no I/O, no LLM. A snapshot field that is missing, stale or unparseable produces a veto (fail closed).
- Only the order gateway holds credentials; agents submit intents.
- All checks run and every veto code is logged with the snapshot hash. ALLOW only with zero vetoes. The engine may lower quantity, never raise it.
- Order classes: `ENTRY`, `EXIT_PROTECTIVE` (stop, take-profit, time stop), `EXIT_SIGNAL`, `MANUAL_SELL` (passkey session only), `FLATTEN`.
- E = mark-to-market HUF equity (ECB FX). NAV is unitized: deposits buy units at current NAV, so they never hide losses or reset drawdown.
- Defaults assume E = 25,000 HUF (≈€60).

## 1. Pre-trade checks (evaluation order)

Notation: `default (hard limit)`. Exit: A = also exits, E = entries only.

| # | Check | Input | Rule: default (hard limit) | Code | Exit |
|---|---|---|---|---|---|
| 1 | Live authorization | per-broker auth record | WebAuthn-signed, binds config hash and engine version. Default false. Expires after 30 days or on any loosening. | `LIVE_NOT_AUTHORIZED` | E |
| 2 | Kill switch | mode {NORMAL, REDUCE_ONLY, FLATTEN, FREEZE} per global/broker/strategy | Entries need NORMAL at all levels. FREEZE also blocks engine-sent exits; broker-resting stops stay. | `KILL_SWITCH` | A (FREEZE) |
| 3 | Credential permissions | key probe, every 24h and on any 403 | Bitvavo: probe `POST /withdrawal` with amount `"0"` and an invalid address. Only 403/`312` passes; else disable broker. T212 v0 has no withdrawal endpoint; require scopes `orders:execute`, `orders:read`, `account`, `portfolio`, `metadata`, `history:*`. | `CRED_WITHDRAW_CAPABLE`, `CRED_SCOPE` | A |
| 4 | Connector health | call log, rate headers, WS heartbeat | Authenticated success ≤60s ago. No 429/`105` in 120s. Rate budget ≥20%. WS heartbeat ≤10s. 5-min error rate <20%. | `BROKER_UNHEALTHY` | A |
| 5 | Data freshness | timestamps | Quote ≤5s crypto, ≤15s equities (30s). Candle ≤2 intervals. Account ≤30s. ECB FX ≤4 days. Metadata ≤24h. Clock skew vs Bitvavo `/time` ≤1s. | `DATA_STALE`, `DATA_MISSING` | E |
| 6 | Reconciliation | recon status, intents | No open mismatch, no `UNKNOWN` intent >10 min. | `RECON_HALT` | E |
| 7 | Instrument eligibility | T212 `type`, `issues`; Bitvavo `status` | T212 STOCK/ETF only. No leveraged/inverse ETP (`ETN_*` issues, blocklist), no DELISTED/SUSPENDED. Bitvavo `trading`, EUR spot. | `INSTRUMENT_INELIGIBLE` | E |
| 8 | Market hours | T212 `/exchanges` `timeEvents` | Entries in [OPEN+15m, CLOSE−15m], outside breaks, `extendedHours=false`. | `MARKET_CLOSED` | E |
| 9 | New listing | T212 `addedOn`; Bitvavo first candle | Veto if listed <90 days (30). | `NEW_LISTING` | E |
| 10 | Daily/weekly loss | NAV at 00:00 Europe/Budapest, Monday | Day ≤−3% (5%), week ≤−6% (10%), incl. unrealized, fees, FX. | `DAILY_LOSS`, `WEEKLY_LOSS` | E |
| 11 | Drawdown | NAV peak | DD ≥6%: risk ×0.5. DD ≥10% (20%): halt. | `DRAWDOWN_HALT` | E |
| 12 | Loss-chasing | closed trades | 3 (5) consecutive net losses: 24h cool-down. Size multiplier ≤1.0 after any loss in 24h. No re-entry within 4h of a stop-out. | `LOSS_STREAK`, `SIZE_UP_AFTER_LOSS`, `REVENGE_REENTRY` | E |
| 13 | Order rate | entry counters | ≤4/h (12), ≤12/day (40). | `ORDER_RATE` | E |
| 14 | Duplicate / in-flight | intent table | Unique `intent_id = H(strategy, version, instrument, side, signal_bar_ts)`. Same instrument and side within 120s, or any `IN_FLIGHT`/`UNKNOWN` intent on the instrument: veto. | `DUPLICATE`, `IN_FLIGHT` | A |
| 15 | Open positions | positions | ≤3 (10). | `MAX_POSITIONS` | E |
| 16 | Strategy approval | registry | `LIVE_APPROVED`, version hash matches authorization. | `STRATEGY_NOT_APPROVED` | E |
| 17 | Fee-aware edge | edge mean, standard error (bps), fees (`GET /account`) | `edge − 1·se ≥ cost_rt + 20bps` (floor 10). `cost_rt = 2·fee + spread + 2·slippage + 2·FX fee`. Expected net ≥25 HUF. | `EDGE_BELOW_COST` | E |
| 18 | Spread | bid/ask | ≤30bps crypto, ≤15bps equities (100). | `SPREAD_WIDE` | E, EXIT_SIGNAL |
| 19 | Liquidity | 24h volume / 20-day ADV | Crypto ≥€2M (min €250k). Equities ADV ≥$5M. Notional ≤0.05% of volume (1%). | `ILLIQUID` | E |
| 20 | Per-trade risk | entry, stop, quantity | `qty·(entry − stop_eff) + fees ≤ 1%·E` (2%) = 250 HUF. `stop_eff` = farther of stop and 99th-percentile gap. Clamp size down. | `RISK_PER_TRADE` | E |
| 21 | Minimum size | `minOrderInQuoteAsset`, `minOrderInBaseAsset`, `quantityDecimals` | Floor quantity. Notional ≥1.02 × broker minimum. Never round up past checks 20–23. | `BELOW_MIN_ORDER` | A |
| 22 | Position cap | position + open orders + in-flight + order | ≤25%·E (40%) = 6,250 HUF. | `POSITION_CAP` | E |
| 23 | Class/total exposure | same aggregate | Crypto ≤40% (60%), equities ≤80% (95%), total ≤80% (95%). | `CLASS_CAP`, `TOTAL_CAP` | E |
| 24 | Correlation | clusters | All crypto = one cluster. Equities by sector or ρ≥0.7 (60d). ≤2 positions and ≤40%·E per cluster. All positions −15% must lose ≤5%·E. | `CONCENTRATION` | E |
| 25 | Cash | available, local reservations | `notional·(1 + fee + FX fee + 1%) ≤ available − reserved_local − max(5%·E, 1,000 HUF)`. | `INSUFFICIENT_CASH` | E |
| 26 | No short/leverage | free quantity, pending sells, order type | SELL ≤ free − pending sells. Whitelisted order types. No margin, lending, staking. | `OVERSELL`, `LEVERAGE` | A |

## 2. Exit-side rules

- Entry-only checks never block a reducing order. Exits pass checks 2 (FREEZE only), 3, 4, 14, 21 and 26. EXIT_SIGNAL also passes check 18: if the spread is wide, it waits up to 30 min, then becomes protective.
- Stale data never blocks a protective exit. It forces a collared marketable limit (crypto bestBid −2%, IOC) or a broker-resting stop.
- Every entry fill gets a broker-resting stop within 5s (Bitvavo `stopLoss`, T212 `stop` GTC). If placement fails, flatten that position. This covers engine downtime.
- Loop guard: ≤3 exit attempts per position per 10 min, then alert. A remainder below the minimum sells in full.
- MANUAL_SELL follows exit rules.
- FLATTEN (user passkey only; auto-flatten off by default):
  1. Set mode FLATTEN.
  2. Cancel entry orders and confirm.
  3. Reconcile.
  4. Per position: cancel the stop, confirm, sell free quantity (Bitvavo IOC limit; T212 market if open, else queue for the open and keep the stop).
  5. Report realized HUF P&L.

## 3. Post-trade checks and reconciliation

- Run on every fill or WS event, every 60s and on boot. Compare positions (exact), cash (±0.5% or ±50 HUF), open orders and fees (≤1.5× expected).
- T212 is not idempotent. A timeout or 5xx marks the intent `UNKNOWN`. Resolve via `GET /equity/orders` and `/equity/history/orders` (ticker, signed quantity, `createdAt ±60s`, `initiatedFrom=API`). Never re-POST first.
- Bitvavo: resolve by `clientOrderId`; errorCode 109 also means `UNKNOWN`.
- Any mismatch, foreign order or unrecognised corporate-action fill (`STOCK_SPLIT` etc.): `RECON_HALT` (REDUCE_ONLY), push alert, snapshot frozen.
- Median slippage over 20 fills >2× model: disable the strategy.

## 4. Circuit breakers and recovery

| Breaker | Effect | Resume |
|---|---|---|
| Unhealthy broker, stale data, clock skew | Entries off (broker) | Auto after 5 min healthy |
| 429 | Pause | Auto at `resetat` +30s |
| Daily loss / loss streak | Entries off / 24h cool-down | Auto; second trigger in 7 days needs user |
| Weekly loss | Entries off | User |
| Drawdown 10% | Halt | User, ≥72h. Peak resets; threshold halved for 30 days. |
| Drawdown 20%, recon mismatch, UNKNOWN >10 min, withdraw-capable key | REDUCE_ONLY | User, after root cause fixed |
| Kill switch, FLATTEN, FREEZE | As set | User only |
| Engine restart | REDUCE_ONLY | Auto to prior mode after clean reconciliation and valid authorization |
| Edge or slippage decay | Strategy off | User |

## 5. Configurability

- Hard limits are compiled constants, not DB values. Onboarding sets values between default and hard limit. Floors (edge buffer, liquidity, listing age, cash floor) can only rise.
- Tightening applies immediately. Loosening needs a passkey, applies after 24h and revokes live authorization. This blocks tilt-driven edits.
- Never configurable: no short, no leverage, no withdraw-capable keys, protective stop per entry, duplicate guard, fail-closed data.

## 6. Challenge: how money is still lost

1. Overfit edge. Use `min(claimed, live edge + 1·se)`; disable if live expectancy <0 after 30 trades.
2. Gaps and flash crashes jump stops. `stop_eff` uses gap percentiles, plus caps and the stress test.
3. Fee churn. Turnover ≤2×E/day, fees ≤1%·E/month, prefer postOnly maker.
4. Correlation breaks in a crisis. Crypto is one cluster; the stress test assumes ρ=1.
5. HUF/EUR/USD drift and T212 FX fee. Fee in `cost_rt`; FX P&L reported separately.
6. Bad third-party quote (T212 has no market data). Cross-check T212 `currentPrice` (≤1%), price collars, outlier filter.
7. Duplicates from retries or two gateways. MySQL `GET_LOCK` single writer, unique `intent_id`, UNKNOWN locks the instrument.
8. Agents splitting orders or editing config. Caps use aggregates, config needs a passkey, agents hold no credentials, MANUAL class needs a WebAuthn session.

## 7. Test matrix (E = 25,000 HUF, €1 = 400 HUF)

| # | Input | Expected |
|---|---|---|
| 1 | Auth false, ENTRY | VETO `LIVE_NOT_AUTHORIZED` |
| 2 | REDUCE_ONLY, ENTRY / EXIT_PROTECTIVE | VETO `KILL_SWITCH` / ALLOW |
| 3 | Bitvavo probe returns `406` | VETO `CRED_WITHDRAW_CAPABLE` |
| 4 | Crypto quote 9s old, ENTRY / EXIT_PROTECTIVE | VETO `DATA_STALE` / ALLOW (collared IOC) |
| 5 | Spread null | VETO `DATA_MISSING` |
| 6 | Day P&L −760 HUF, ENTRY | VETO `DAILY_LOSS` |
| 7 | Notional 6,000, stop −10% (risk 630) | ALLOW_RESIZED, risk ≤250 |
| 8 | BTC 5,000 + pending 1,000 + new 1,000 | VETO `POSITION_CAP` |
| 9 | Crypto 9,500 + new 1,000 | VETO `CLASS_CAP` |
| 10 | 24h volume €800k | VETO `ILLIQUID` |
| 11 | Spread 45bps, ENTRY / EXIT_PROTECTIVE | VETO `SPREAD_WIDE` / ALLOW |
| 12 | Edge 60±20bps, cost 70bps | VETO `EDGE_BELOW_COST` (40 < 90) |
| 13 | Sized €4.20, minimum €5 | VETO `BELOW_MIN_ORDER` |
| 14 | Same BUY BTC-EUR after 60s | VETO `DUPLICATE` |
| 15 | AAPL intent UNKNOWN, new BUY AAPL | VETO `IN_FLIGHT` |
| 16 | 3 consecutive losses 6h ago | VETO `LOSS_STREAK` |
| 17 | NYSE ENTRY at OPEN+5m | VETO `MARKET_CLOSED` |
| 18 | `addedOn` 20 days ago | VETO `NEW_LISTING` |
| 19 | Free 3,000, reserved 1,000, floor 1,250, order 2,000 | VETO `INSUFFICIENT_CASH` |
| 20 | SELL 0.002 BTC, holding 0.0015 | VETO `OVERSELL` |
| 21 | DD 10.5%, then 50,000 HUF deposit | Still `DRAWDOWN_HALT` |
| 22 | Re-authorization 50h after DD halt | Rejected |
| 23 | Leveraged ETP (`ETN_*` issue) | VETO `INSTRUMENT_INELIGIBLE` |
| 24 | All checks pass | ALLOW, quantity ≤ requested |

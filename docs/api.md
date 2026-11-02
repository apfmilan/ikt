# Trading Center API contract (v2)

Base path `/api`. JSON only (`Content-Type: application/json`), money as **decimal strings**
(e.g. `"1250.50"`), timestamps RFC 3339 UTC. HUF values are suffixed `_huf`.
Auth: `__Host-tc_session` cookie (HttpOnly, Secure, SameSite=Strict). State-changing requests
must send `Origin` equal to the configured origin. Errors: `{ "error": { "code": "...", "message": "..." } }`
with HTTP 400/401/403/404/409/422/429/500.

Every figure is either **realized** (from broker fills/balances) or **forecast**
(model estimate); responses keep them in separate objects named `realized` / `forecast`.

## Auth (WebAuthn passkeys)
| Method | Path | Body → Response |
|---|---|---|
| GET | `/auth/status` | → `{ setup_required, authenticated, user?: {display_name}, devices?: number }` |
| POST | `/auth/setup/start` | `{ setup_token, device_name }` → `{ challenge_id, options }` (`options` = WebAuthn `PublicKeyCredentialCreationOptions` JSON, base64url) |
| POST | `/auth/setup/finish` | `{ challenge_id, credential }` → `{ ok, recovery_codes: string[] }` (shown once) + session cookie |
| POST | `/auth/login/start` | `{}` → `{ challenge_id, options }` (`PublicKeyCredentialRequestOptions`) |
| POST | `/auth/login/finish` | `{ challenge_id, credential }` → `{ ok }` + cookie |
| POST | `/auth/recover` | `{ recovery_code, device_name }` → same as setup/start (registers a new passkey) |
| POST | `/auth/stepup/start` | `{ action }` → `{ challenge_id, options }` |
| POST | `/auth/stepup/finish` | `{ challenge_id, action, credential }` → `{ stepup_token, expires_at }` (single use, 120 s, bound to `action`) |
| POST | `/auth/pair` | (session) → `{ pairing_code, expires_at }` to add another device (Windows) |
| POST | `/auth/pair/start` | `{ pairing_code, device_name }` → `{ challenge_id, options }`; finish via `/auth/setup/finish` |
| GET | `/auth/devices` | → `[{ id, name, created_at, last_used_at }]` |
| DELETE | `/auth/devices/{id}` | header `X-Stepup-Token` → `{ ok }` |
| POST | `/auth/logout` | → `{ ok }` |

Step-up actions: `live_enable`, `risk_change`, `manual_sell`, `kill_release`, `credentials_change`, `device_remove`, `plan_change`.
Step-up token is sent as header `X-Stepup-Token`.

## Overview & dashboards
- `GET /overview` →
```json
{ "as_of": "...", "mode": "normal|reduce_only|flatten|freeze|recovery",
  "live": { "trading212": true, "bitvavo": false },
  "realized": { "equity_huf": "...", "net_deposits_huf": "...", "net_pnl_total_huf": "...",
                "net_pnl_today_huf": "...", "fees_total_huf": "...", "nav_unit": "1.0123",
                "drawdown_pct": "1.2" },
  "brokers": [{ "broker": "bitvavo", "connected": true, "equity_huf": "...", "cash_huf": "...",
                "currency": "EUR", "last_sync": "...", "health": "ok|degraded|down" }],
  "bots": [{ "id": "crypto|stocks|invest", "name": "Kripto bot", "status": "active|idle|paused|blocked|error",
             "today_pnl_huf": "...", "open_positions": 1, "last_decision_at": "...", "message": "..." }],
  "equity_curve": [{ "t": "...", "equity_huf": "...", "net_deposits_huf": "..." }] }
```
- `GET /agents` → `[{ "agent": "scanner|technical|regime|volatility|news|listings|ranker|portfolio|exit|risk|execution|reconciler|long_term_invest|revolut_watch", "status", "last_run_at", "last_duration_ms", "summary", "metrics": { ... } }]`
- `GET /positions` → `[{ instrument, symbol, name, broker, asset_class, quantity, avg_price, last_price, currency, value_huf, realized: null, unrealized_huf, unrealized_pct, stop_price, take_profit, trailing_stop, strategy, opened_at, long_term, decision_id }]`
- `GET /orders?state=open|all&limit=` → `[{ id, instrument, side, kind, quantity, filled_quantity, state, reason, broker_order_id, created_at, updated_at, avg_fill_price, fee }]`
- `GET /trades?limit=&before=` → closed round trips `[{ id, instrument, strategy, entry_at, exit_at, entry_price, exit_price, quantity, realized: { gross_huf, fees_huf, net_huf, return_pct }, forecast: { p_win, expected_net_bps }, exit_reason, decision_id }]`
- `GET /transactions?limit=&before=` → ledger `[{ id, ts, broker, kind: "deposit|withdrawal|fee|realized_pnl|dividend|interest|fx", amount, currency, amount_huf, ref }]`
- `GET /decisions?limit=&instrument=` → `[{ id, created_at, instrument, action: "enter|exit|hold|abstain", strategy, confidence, forecast: { p_win, expected_net_bps, cost_bps, uncertainty_bps, p05_bps, p95_bps }, rationale: string[], signals: [{ agent, direction, strength, confidence, expected_edge_bps, rationale }], risk: { verdict: "allow|allow_resized|veto", vetoes: [{ code, detail }], quantity } }]`
- `GET /decisions/{id}` → one decision (same shape).
- `GET /instruments/{id}/chart?interval=1h&limit=300` (`id` URL-encoded, e.g. `bitvavo%3ABTC-EUR`) → `{ candles: [{ t, o, h, l, c, v }], markers: [{ t, kind: "entry|exit|stop", price, text }], lines: [{ kind: "avg_price|stop|take_profit", price }] }`
- `GET /performance` → `{ realized: { total_return_pct, win_rate, profit_factor, max_drawdown_pct, trades, fees_huf, expectancy_huf }, by_strategy: [{ strategy, status, trades, win_rate, net_huf, fees_huf, max_drawdown_pct, backtest_oos: { sharpe, profit_factor, expectancy_r, trades } }], daily: [{ date, net_pnl_huf }] }`
- `GET /strategies` → `[{ name, version, status: "disabled|validated|live_half|live|demoted", asset_class, regimes, backtest: { folds, oos_trades, oos_sharpe, oos_profit_factor, oos_max_dd_pct, eligible }, live: { trades, expectancy_huf } }]`

## Plan, risk, controls
- `GET /plan` → `{ config: { start_month, initial_huf, monthly_until_apr_2027_huf, monthly_from_may_2027_huf, long_term_share_pct, long_term_start_month, reinvest_pct, withdraw_pct, etf_targets: [{ instrument, weight_pct }] }, schedule: [{ month, planned_trading_huf, planned_long_term_huf, actual_deposits_huf }], progress: { planned_total_huf, deposited_total_huf, trading_profit_total_huf, withdrawable_huf } }`
- `PUT /plan` (step-up `plan_change`) body = `config`.
- `GET /risk` → `{ config: {...RiskConfig}, hard_max: {...}, utilization: { exposure_pct, crypto_pct, equity_pct, day_pnl_pct, drawdown_pct, entries_today }, pending_change?: { config, effective_at } }`
- `PUT /risk` (step-up `risk_change`) → tightening applies now; loosening `{ effective_at }` in 24 h and revokes live authorization.
- `POST /control/kill` `{ mode: "reduce_only|flatten|freeze|normal" }` — stricter modes need only a session; `normal` needs step-up `kill_release`.
- `POST /control/live` `{ broker, enabled }` — `enabled: true` needs step-up `live_enable`; disabling needs only a session.
- `POST /positions/sell` `{ instrument, quantity: "all" | "<decimal>" }` (step-up `manual_sell`) → `{ command_id }`.

## Onboarding
- `GET /onboarding` → `{ steps: [{ id: "passkey|trading212|bitvavo|market_data|risk|plan|authorize", status: "todo|done|error|blocked", detail }], ready: boolean }`
- `POST /onboarding/credentials` `{ broker: "trading212|bitvavo|eodhd", api_key, api_secret?, operator_id? }` (step-up `credentials_change` once a credential exists) → `{ stored: true }`. Secrets are sealed immediately; never returned.
- `POST /onboarding/verify` `{ broker }` → `{ command_id }`; result appears in `GET /onboarding` and as a WS `onboarding` event: authenticated, permissions, withdraw check, balances.
- `POST /onboarding/attest` `{ broker, withdraw_disabled: true }` → records attestation (used only when the automatic withdrawal check is inconclusive).
- `POST /onboarding/risk` body = RiskConfig (validated against hard limits).
- `POST /onboarding/authorize` (step-up `live_enable`) `{ brokers: [...] }` → live authorization (expires after 30 days).

## Notifications
- `GET /push/vapid` → `{ public_key }`
- `POST /push/subscribe` `{ endpoint, keys: { p256dh, auth }, device_name }`; `DELETE /push/subscribe` `{ endpoint }`
- `GET /notifications/settings` / `PUT` `{ trades: bool, market_open: bool, errors: bool, critical_risk: bool, daily_summary: bool, daily_summary_time: "21:00" }`
- Daily summary text (Europe/Budapest): `💰 Mai eredmény: +1 000 Ft` / `📉 Mai eredmény: -1 000 Ft`, from realized+mark-to-market net P&L excluding deposits.

## WebSocket `GET /api/ws`
Cookie-authenticated, `Origin` checked, server → client only (client may send `{"type":"ping"}`).
Messages: `{ "type": "overview|position|order|decision|agent|alert|mode|onboarding", "data": {...}, "ts": "..." }`.
Clients refetch the matching REST resource on reconnect.

## Health
`GET /health` (no auth) → `{ status: "ok|degraded", db: bool, engine_heartbeat_age_s: number, version }`

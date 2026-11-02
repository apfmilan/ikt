# Proposal A: Modular Monolith (one Rust binary)

Status: proposal, Architect A, 2026-11-02. The workload is tiny: two brokers, with rate limits between 1 req/s and 50 req/min. Correctness is the main risk, not scale. One process plus one MySQL gives one transaction boundary and no distributed consistency problems.

## 1. Crate layout

| Crate | Responsibility |
|---|---|
| `tc-domain` | Decimal newtypes (`Money`, `Qty`, `Price`), OrderIntent state machine, events. No IO. |
| `tc-risk` | Deterministic veto `fn(snapshot, intent, limits, now) -> Verdict`. No IO. |
| `tc-strategy` | Signal agents: edge vs costs + uncertainty, abstain by default. |
| `tc-ledger` | Deposits vs trading P&L, fees, HUF conversion, profit split. No IO. |
| `tc-store` | SQLx repositories, additive `tc_` migrations, outbox, audit. |
| `tc-broker` | `Broker` trait with T212 and Bitvavo adapters. |
| `tc-marketdata` | Bitvavo candles, stock-data provider trait (vendor TBD), ECB FX. |
| `tc-engine` | Supervisor, tasks, executor, reconciler, kill switch. |
| `tc-api` | Axum REST and WS, WebAuthn, command intake. |
| `tc-notify` | Web Push, Hungarian daily summary. |
| `tc-server` | Binary: config, secrets, wiring. |

Dependency direction: `domain` <- `risk`/`strategy`/`ledger` <- `store`/`broker`/`marketdata` <- `engine` <- `server`. `tc-api` and `tc-notify` depend only on `store` and `domain`. UI commands go through a `tc_command` table, so `tc-api` can later become its own binary with no logic change. CI keeps tokio, sqlx and reqwest out of the pure crates. `#![deny(clippy::float_arithmetic)]` applies to domain, risk and ledger.

## 2. Runtime topology

A supervisor `JoinSet` restarts crashed tasks with backoff. If risk, executor or reconciler dies, the kill switch trips.

- `md-bitvavo` and `md-stocks`: batch-insert candles and publish closed ones on a bounded `broadcast`. A lagging receiver resyncs from the DB.
- `scanner`: filters the universe on each closed candle.
- `signal-agent` xN: `try_send` into `mpsc(64)` toward risk. Signals carry `valid_until`. A full channel drops the signal, because a stale signal is worthless.
- `risk`: a single task and the only intent writer, so exposure cannot race. An approval writes `order_intent` and `outbox` in one transaction. Vetoes are stored too.
- `executor` (one per broker): the DB is the queue, claimed with `SELECT ... FOR UPDATE SKIP LOCKED`. `Notify` is only a wake-up hint.
- `reconciler` (one per broker): periodic, plus on demand for UNKNOWN attempts.
- `scheduler`: FX, metadata, history sync, the 20:00 Europe/Budapest summary.
- `notifier`, `api`, `watchdog`.

Backpressure: all channels are bounded. Market data and signals may drop. Intents, fills and audit events travel only through MySQL.

## 3. Order lifecycle and exactly-once intent

States: `APPROVED -> SUBMITTING -> ACKED | REJECTED | UNKNOWN -> OPEN | PARTIAL -> FILLED | CANCELLED | NOT_PLACED`.

Invariants:
1. A `tc_order_attempt` row (SUBMITTING, request, `sent_at`) is committed before any HTTP send. No DB means no send.
2. An intent has at most one unresolved attempt. A new attempt needs proof that the previous one created no order.
3. A timeout, reset, T212 408, Bitvavo 109, or crash after send leads to UNKNOWN. UNKNOWN is never retried blindly.

**Bitvavo:** `clientOrderId` is the attempt's UUIDv7, persisted before send. `operatorId` is fixed. On UNKNOWN, query `GET /order?market&clientOrderId`, then trade history. The id is unique among open orders only, so it cannot stop a duplicate of a filled order. Lookup always comes first. A 5 s `Bitvavo-Access-Window` bounds late execution. If nothing is found after window plus margin, the attempt becomes NOT_PLACED. An integration test must confirm this bound before go-live.

**Trading 212 (no client id):** `tc_broker_inflight(account, ticker)` allows one unresolved attempt per ticker, so a fingerprint match is unambiguous. On UNKNOWN, wait 5 s, then search `GET /equity/orders` and `/equity/history/orders`. The fingerprint is `initiatedFrom=API`, ticker, signed quantity, type, prices, and `createdAt` within `[sent_at-2s, sent_at+60s]`. The position change seen in `/equity/positions` is a second witness.
- One match: adopt its broker id.
- No match after 3 minutes of checks: NOT_PLACED.
- Several matches: broker kill switch and alert.

The T212 API key is used by the bot only. NOT_PLACED never resubmits by itself: the intent returns to risk with fresh prices.

**Restart:** every boot enters RECOVERY with trading off. It loads the kill switch and reconciles all SUBMITTING and UNKNOWN attempts. Then it diffs broker positions, cash and open orders against the ledger. A diff beyond tolerance keeps the system halted. Recovery reuses the reconciler code.

**Before an order:** risk needs a broker snapshot younger than 60 s and no UNKNOWN attempt on the account. **After an order:** poll until the order is final, record fills and fees, then re-check positions.

## 4. Failure handling

- **Broker outage:** circuit breaker per broker. The other broker keeps trading. Push alert.
- **DB outage:** no sends (invariant 1). Push subscriptions are cached in memory, so a "DB down" alert still goes out. On reconnect, RECOVERY runs. MySQL runs with `innodb_flush_log_at_trx_commit=1` and `sync_binlog=1`. Backups are copied off the VPS.
- **Clock skew:** chrony, plus an hourly check against Bitvavo `GET /time` and the T212 `Date` header. Skew over 1 s halts trading. Timestamps are stored as UTC `DATETIME(6)`.
- **Rate limits:** `governor` buckets per documented limit, corrected from the rate-limit headers. 30% of Bitvavo weight is reserved for orders and reconciliation. On a 429, wait for the reset. Bitvavo error 105 pauses that broker for 60 s.
- **Kill switch:** the in-memory `watch<bool>` is set first, then the `tc_kill_switch` row is written (scope, actor, reason). Risk checks it, and the executor checks it right before each send. Auto triggers: loss limit, reconciliation mismatch, ambiguous match, task death, clock skew. If the row is unreadable at boot, trading stays halted. Release requires WebAuthn step-up. Positions are never sold automatically.
- **Restart:** Docker `restart: unless-stopped` plus a healthcheck. More than 3 boots in 10 minutes forces a HALTED boot.

## 5. MySQL schema outline

- **Orders:** `tc_order_intent` (UUIDv7 `BINARY(16)`), `tc_outbox`, `tc_order_attempt` (raw request and response), `tc_broker_inflight`, `tc_broker_order`, `tc_fill` (UNIQUE broker fill id).
- **Accounting:** `tc_ledger_entry` (DEPOSIT/WITHDRAWAL/FILL/FEE/DIVIDEND/FX, amount, ccy, amount_huf, UNIQUE external_ref), position and balance snapshots, `tc_fx_rate`.
- **Signals and control:** `tc_candle`, `tc_signal`, `tc_risk_decision`, `tc_kill_switch`, `tc_config_version`, `tc_command`, `tc_audit_event`.
- **Users:** `tc_user`, `tc_webauthn_credential`, `tc_session`, `tc_push_subscription`.

**Money:** `rust_decimal::Decimal` everywhere, never f64. MySQL uses `DECIMAL(38,18)` for quantities and prices, `DECIMAL(24,8)` for cash. Bitvavo strings fall back to `from_scientific` for values such as "6.95E+4". T212 JSON numbers parse through serde_json `arbitrary_precision`, so they never pass through f64. Quantities round down. Cross-currency math needs an explicit `FxRate`.

**Audit:** `tc_audit_event` is a SHA-256 hash chain. The app DB user has only INSERT and SELECT on the audit, fill and ledger tables. Triggers reject UPDATE and DELETE. Corrections are reversal entries. The daily chain head is also stored outside the VPS.

## 6. Dependencies

- Runtime and web: `tokio`, `axum`, `tower-http`.
- Data: `sqlx` (MySQL, decimal, compile-checked SQL, no ORM), `serde_json` (`arbitrary_precision`), `rust_decimal`, `uuid`.
- Broker IO: `reqwest`, `tokio-tungstenite`, `hmac`/`sha2` (signing, audit chain), `governor`, `quick-xml` (ECB).
- Time: `chrono-tz` (Budapest daylight saving).
- Security and push: `webauthn-rs`, `web-push`, `secrecy`.
- Ops: `tracing`, `thiserror`. Dev: `proptest`, `wiremock`.

Left out on purpose: actor frameworks, Redis, Kafka, ORMs.

## 7. Self-assessment

| Criterion | Score | Reason |
|---|---|---|
| Security | 6 | Broker keys share an address space with the public HTTP surface. |
| Correctness | 9 | One transaction boundary, one risk writer. |
| Performance | 9 | Broker limits, not the process, are the bottleneck. |
| Maintainability | 8 | One repo, compiler-checked refactors. Boundaries need CI rules. |
| UX enablement | 7 | Live events go straight to WS. Deploys pause trading for seconds. |
| Cost | 9 | ~100 MB RSS plus MySQL. Fits a 2 vCPU / 4 GB VPS. |

**Top 3 risks**
1. **Blast radius:** one deadlock or blocking call stops trading, the API and the in-app kill switch together. Mitigation: supervisor, no lock across `.await`, `spawn_blocking`, watchdog, and a runbook for revoking broker keys.
2. **Shared secrets:** remote code execution in the API exposes trading keys. Mitigation: no withdraw permission, T212 IP allowlist, a read-only container. The `tc_command` seam lets `tc-api` split into a keyless binary.
3. **T212 false NOT_PLACED:** history lag beyond 3 minutes lets a duplicate slip through. Mitigation: no auto-resubmit, risk checks broker positions and pending orders, exposure caps.

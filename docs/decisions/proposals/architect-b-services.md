# Proposal B: privilege-separated services

Status: proposal for panel review (2026-11-02). Author: Architect B.

## 1. Boundaries, protocol, what isolation buys

Boundaries follow privilege and blast radius, not domain, so market data and strategy stay together. Three Rust binaries from one workspace ship as **one image** with three entrypoints, plus Caddy and MySQL.

| Service | Owns | Secrets | Never |
|---|---|---|---|
| `gateway` | HTTPS/WS API, passkeys, Web Push, HUF reports, static Next.js export | session key, VAPID key, executor **public** key | decrypts broker keys, writes orders |
| `engine` | Bitvavo public data, ECB FX, stock data provider, signals, edge-vs-cost | data-provider key | holds broker credentials |
| `executor` | risk veto, order state machine, rate budgets, reconciliation, ledger, kill switch | executor private key (Docker secret file) | accepts inbound internet traffic |

**Protocol: MySQL is the bus.** No Redis or NATS. Tables are prefixed `tc2_` and migrations are additive.
- `tc2_proposal` (engine to executor) has a unique `proposal_key` = hash(strategy, instrument, bar_close_ts, direction). A restarted engine re-emits the same key, and `INSERT IGNORE` drops the duplicate.
- `tc2_command` (gateway to executor), `tc2_event` (outbox fanned out to WebSockets), `tc2_control` (one row: `halted`, `halt_epoch`).
- Consumers claim rows with `SELECT ... FOR UPDATE SKIP LOCKED`, polling every 200 ms. A best-effort internal `POST /nudge` gives instant wake-up. The DB stays the source of truth.

**What isolation buys:**
1. **Least-privilege secrets.** The gateway seals entered broker keys to the executor's X25519 public key and stores only ciphertext. A gateway RCE or a DB dump cannot read stored keys. Caveat: a compromised gateway can capture keys entered *after* the compromise.
2. **Signed risk-increasing commands.** Resume, limit raise, strategy enable and credential change carry a WebAuthn assertion over the command hash, and the executor verifies it against the stored passkey public key. A hijacked gateway can halt trading but cannot resume it or loosen limits on its own. Halt needs no signature. Registering a new passkey also needs a signature, from an existing passkey or a one-time executor CLI token. Limit: the user cannot see what a challenge covers. So this stops silent abuse, not a gateway that tricks the user into signing a different command.
3. **DB grants per service.** The gateway cannot `UPDATE` orders or ledger. The engine can insert only proposals and candles.
4. **Crash containment.** A UI bug or an OOM in indicator code cannot kill the process that manages open orders. Only the executor spends the per-account T212 and Bitvavo rate budgets.

## 2. Order lifecycle (exactly-once intent)

States: `APPROVED` or `VETOED`, then `SUBMITTING`. From `SUBMITTING`: `ACKED` (then `PARTIAL`, `FILLED`, `CANCELLED`), `REJECTED`, or `UNKNOWN`. `UNKNOWN` resolves to `ACKED` or `NOT_PLACED`.

Invariants for both brokers:
- **Write-ahead.** `SUBMITTING` (with `submit_started_at` and a request fingerprint) commits before the HTTP call. The commit is a compare-and-set: `UPDATE ... WHERE state='APPROVED'`, in a transaction that reads `tc2_control` `FOR SHARE`.
- **Single submitter.** `GET_LOCK('tc2_executor')` blocks double submitters during overlapping deploys.
- **Single-flight.** At most one intent per broker account is in `SUBMITTING` or `UNKNOWN`.
- **Pessimistic exposure.** Risk counts every `UNKNOWN` intent as fully filled.
- **No blind retry.** A timeout, reset, 5xx, T212 `408` or Bitvavo `109` moves the intent to `UNKNOWN`. Any other 4xx is `REJECTED`.
- **TTL.** `APPROVED` expires after 60 s, so a restart never fires a stale backlog.

**Bitvavo.** `clientOrderId` = intent UUID, with `Bitvavo-Access-Window` = 10 s. On `UNKNOWN`, call `GET /order?market&clientOrderId` and bind if the order is found. Requests that arrive after the window are refused (`304`). So an order still absent after window + 30 s is `NOT_PLACED`. `clientOrderId` is documented unique only "across open orders", so an instantly filled market order will *not* block a duplicate. Always GET before any resubmit. Error `220` means the order exists: bind it.

**Trading 212 (no client id):**
1. Before submit, snapshot the known order ids (`/equity/orders`, `/equity/history/orders?ticker=X`) and the position quantity.
2. On `UNKNOWN`, poll for up to 120 s within the rate limits: orders every 5 s, history every 3 s, positions every 1 s.
3. A match must have the same ticker, signed quantity and type, `initiatedFrom=API`, `createdAt >= submit_started_at - 5s`, and a new, unbound id. Single-flight makes the match unambiguous.
4. If nothing matches, mark `NOT_PLACED_PRESUMED` and lock the ticker for 10 min while a sweep continues. A late match binds back and raises an alert.
5. Never resubmit a T212 intent. Re-entry needs a new proposal and a fresh risk check, which sees any ghost fill through positions.

A dedicated API key makes `initiatedFrom=API` ours; manual app orders are imported as `EXTERNAL`.

**Restart.** After taking the lock, the executor turns every `SUBMITTING` intent into `UNKNOWN` and resolves it as above. It then diffs orders and positions against the ledger and upserts fills by unique `(broker, fill_id)`. Only then does it accept proposals.

## 3. Failures, emergency stop, recovery

| Failure | Behaviour |
|---|---|
| engine down | no new signals; orders still managed; late proposals expire by TTL |
| gateway down | trading continues; heartbeat stale > 5 min means "no new entries", because the user's halt path is gone |
| executor down | UI, reports and push still work; protective stops rest **broker-side** (T212 stop, Bitvavo `stopLoss`) |
| MySQL down | write-ahead is impossible, so all submits stop (fail closed) |
| 429 / Bitvavo `105` | broker `DEGRADED`, backoff from rate-limit headers |
| 3 consecutive `UNKNOWN` / reconcile mismatch | broker auto-halt; passkey-signed resume required |

**Emergency stop:**
1. The gateway commits `halted=1, halt_epoch+1` and acknowledges only after the commit.
2. The submit transaction reads that row `FOR SHARE`. So no order is sent after the halt commits, except at most one in-flight request per broker that committed before it. That request is reconciled and cancelled if it is resting.
3. Blocking new orders takes milliseconds and does not depend on the nudge.
4. Cancels of resting non-protective orders start within ~50 ms through the nudge, or within 250 ms through polling. Completion is bound by broker RTT (T212 `DELETE` allows 50/min).
5. Break-glass: `docker compose stop executor` fails closed and leaves the UI up.

**Recovery.** `restart: unless-stopped` with healthchecks. Transient halts (network, 429) auto-resume after a clean reconcile. Safety halts never auto-resume. Host NTP; skew checked against Bitvavo `GET /time`.

## 4. Operational cost

- **Containers:** 5 (caddy, mysql, gateway, engine, executor) vs 3 for a monolith. Next.js is a static export served by Caddy in both designs.
- **RAM:** Rust services use ~20–40 MB each. MySQL 8.4 with a 128 MB buffer pool and `performance_schema=OFF` uses ~350 MB. Total ~0.5 GB, about +60 MB vs a monolith. Fits a 2 GB VPS.
- **Build:** one `Cargo.lock`, one image tag, and all services deploy together, so no API versioning. A shared `tc-core` crate holds row types and SQLx queries, so contract drift fails at compile time. A one-shot `tc migrate` runs migrations.
- **Burden:** three log streams (correlated by `intent_id`), three healthchecks, DB grants, one nudge token. `tc all` runs every role in one process for development.

```yaml
x-app: &app {image: "tc:${TAG}", restart: unless-stopped, read_only: true, cap_drop: [ALL]}
executor: {<<: *app, command: [tc, executor], secrets: [exec_x25519], networks: [core]}
```

## 5. Self-score and risks

| Criterion | Score | Note |
|---|---|---|
| Security | 8 | real gateway-compromise containment; host root or the Docker socket still wins |
| Correctness | 8 | comes from the state machine, which a monolith can equal |
| Performance | 7 | ≤250 ms of polling, irrelevant for bar-based strategies |
| Maintainability | 6 | more moving parts for one person |
| UX enablement | 7 | UI survives executor crashes; extra passkey prompts on risky actions |
| Cost | 7 | +2 containers, +~60 MB |

**Top risks:**
1. **Complexity tax.** Grants, healthchecks or cross-service debugging get neglected. Mitigations: single image, `tc all`, a `tc doctor` check.
2. **Partial security theater.** `build.rs` supply-chain code runs for every binary, and host compromise beats process isolation. Host hardening is mandatory anyway.
3. **T212 ghost orders.** Fingerprint matching can misjudge. Mitigations: single-flight, ticker lock, `UNKNOWN` counted as filled, alerts. This risk applies to both designs.

## 6. Hybrid fallback if a monolith wins

Keep, in priority order:
1. **Executor as a second process** (one container, ~25 MB). The monolith covers gateway and engine. A signing-only sidecar would not work, because T212 Basic auth needs the raw secret.
2. **Passkey-signed risk-increasing commands, verified by the executor.**
3. **Even in a pure monolith:**
   - the write-ahead intent table with compare-and-set transitions;
   - `GET_LOCK`;
   - halt checked inside the submit transaction;
   - broker keys encrypted under a Docker-secret key file, never in env vars or the DB;
   - role flags (`--roles=...`), so a later split is a Compose change, not a rewrite;
   - the execution boundary kept as a DB queue, not an in-memory channel.

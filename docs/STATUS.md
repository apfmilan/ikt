# Status — 2026-11-02

Honest state of Trading Center V2 on branch `claude/trading-center-v2`. "Verified" means a test or
command ran and passed in the build environment; nothing below has traded real money.

## Verified working

| Area | Evidence |
|---|---|
| Rust workspace (14 crates): fmt, clippy `-D warnings` | `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`: clean |
| Unit + mock-integration tests | `cargo test --workspace`: **150 passed, 0 failed** (risk engine 37, strategy 25, connectors 47 on wiremock, vault, plan, backtest, API router, engine helpers) |
| MySQL 8.4 integration | `make db-test`: **8 passed** — intent state machine, atomic idempotent fill booking, hash-chained audit log + append-only triggers, idempotent ledger, late-deposit ledger cursor, stale-pending abort, sessions/tokens/kill switch, command queue, engine singleton lock |
| Passkeys end-to-end (real API + real built UI, Chromium virtual authenticator with user verification) | `scripts/e2e/passkey-e2e.mjs`: **28/28** — setup token single-use, 10 recovery codes, login, logout, step-up bound to one action and single-use, kill-switch rules, pairing/flatten need step-up, push-endpoint allowlist, all 11 screens render with real API data |
| Engine on live public market data | Ran `--role all` against Bitvavo public API + ECB: scanner picked liquid EUR markets (stablecoins excluded), regimes classified, candidates **abstained** (no validated edge), FX stored, agents reported status |
| Historical backtests (real data) | `tc backtest` on ~5.5 years of Bitvavo daily candles, 6 markets: no strategy passes the out-of-sample gates → all `disabled` (docs/backtests/2026-11-02.md) |
| Web PWA | `npm run typecheck`, `npm run lint`, `npm run build` pass; `npm audit --omit=dev`: 0 vulnerabilities |
| Tauri 2 desktop shell | `cargo test` + `cargo clippy` in `apps/desktop/src-tauri` (Linux) pass |
| Docker | `server` image builds (multi-stage, distroless, non-root). MySQL migration as non-root user creates all 9 append-only triggers. Compose config validated |
| Independent reviews | security-engineer, risk-analyst and code-reviewer reports; all CRITICAL/BLOCKER/HIGH findings fixed (see below) |

## Blockers (need you)

**BLOCKER 1 — Broker API keys.** WHY: live order placement, balances and reconciliation cannot be
verified without real Trading 212 / Bitvavo keys. FIX: onboarding (docs/setup.md §2). The Bitvavo
withdrawal-permission probe and the Trading 212 trade scope are confirmed only against the live API.
STATUS: connectors implemented to official docs and tested on mocks only. **No live integration is
claimed as working.**

**BLOCKER 2 — Stock/ETF market data (EODHD key).** WHY: Trading 212's API has no prices. FIX: create
an EODHD key (free tier is enough for the ETF plan + 10-stock daily scan) and enter it in onboarding.
STATUS: adapter implemented; stock trading and the long-term ETF plan stay blocked until the key exists.

**BLOCKER 3 — No validated strategy.** WHY: no strategy family passed out-of-sample validation on
real data, so the system will not open trades (by design). FIX: research more markets / the 4h variant
and re-run `tc backtest --promote` (quant-researcher workflow). STATUS: the engine runs, scans and
explains abstentions; it will start trading only after a strategy passes every gate.

**BLOCKER 4 — VPS + domain.** WHY: no VPS access in this session. FIX: docs/deployment.md
(DNS → bootstrap script → `generate-secrets.sh` → `docker compose up -d` → `tc setup-token`).
STATUS: deployment files prepared and validated locally; **not deployed**.

**BLOCKER 5 — iPhone home-screen widget (three bots).** WHY: needs a native WidgetKit app (macOS +
Xcode + Apple Developer account). FIX: build a small native companion on a Mac. STATUS: not built;
the PWA shows the three bots on the overview and sends push notifications instead.

**BLOCKER 6 — Windows installer + Windows Hello.** WHY: no Windows host here. FIX: run
`.github/workflows/desktop-windows.yml` (repository variable `TC_ORIGIN`) and test passkeys in
WebView2 on real hardware (fallback: install the PWA from Edge). STATUS: shell builds/tests on Linux.

**Optional — Claude code review on PRs** needs repository secret `ANTHROPIC_API_KEY`
(`.github/workflows/claude-review.yml`). The project `.mcp.json` GitHub server needs
`GITHUB_PERSONAL_ACCESS_TOKEN` in your environment.

## Review findings fixed (2026-11-02)
- FREEZE never cancels or replaces broker stops; exits/stop raises are risk-approved *before* the
  resting stop is cancelled; cancels are confirmed with the broker (fills during cancel are booked).
- Protective stop re-sized after partial fills; stale `pending` intents aborted (never sent);
  broker-scoped flatten only touches that broker; flatten requires a passkey.
- Fills booked atomically and idempotently (fill + intent + position + trade + ledger in one
  transaction); unpriced (T212 resting) fills and missing-FX cases are deferred, never zero-filled;
  entry fees attributed; realized P&L + fees no longer double-counted in the ledger.
- Snapshots only when every broker/currency is valued; deposits tracked by ledger id (late deposits
  can no longer appear as profit); drawdown halt and weekly loss latched via the kill switch.
- Quote freshness enforced (no timestamp clamping); exposure and cash reservations converted with the
  instrument's own currency; resting entry orders block new entries; regime hysteresis advances once
  per closed bar; DST-correct week/month boundaries; engine lock re-checked every 15 s.
- Protective exits are no longer vetoed by missing FX/instrument metadata.
- Security: pairing a device needs step-up and triggers a critical alert; recovery revokes all
  sessions and switches to REDUCE_ONLY; attestation reset when keys change and step-up protected;
  per-IP auth rate limits; push endpoints restricted to browser push services (no redirects);
  120 s WebAuthn ceremonies; Caddy logs drop step-up tokens and cookies; core dumps disabled; CSP
  build fails without script hashes; risk tightening overrides pending loosening; DB URL redacted.

## Known limitations / open items
- Paired and recovered devices get a full session (no enrolment-only session yet).
- Engine-side re-verification of step-up assertions is not implemented (API verifies; ADR-0001 dissent).
- Egress allowlist for the engine container is documented but not automated (nftables DOCKER-USER).
- Images/actions pinned by version, not digest/SHA; no cargo-deny job yet.
- `cargo audit`: RUSTSEC-2023-0071 (`rsa` via sqlx-mysql, no fix; MySQL stays on the internal
  network) and RUSTSEC-2026-0387 (`crypto-bigint` via web-push/p256, needs upstream bump).
- WebSocket event polling can skip an event if inserts commit out of id order (UI refetches on reconnect).
- Volume-participation and −15 % stress checks from the risk spec are not yet enforced; the engine-side
  stop safety net has no retry cap.
- News/sentiment agent: no news source configured (reported as idle; not used for decisions).
  Verified public-trader monitoring (optional) is not implemented.
- Revolut: signal-only by requirement; no Revolut API is used. Revolut X offers an official crypto
  trading API, but automated trading there was not requested.
- Trading 212 quantity precision/minimum and FX fee are configurable assumptions until verified live.
- Desktop tray shows Open/Quit only (bot status in tray would need IPC from the remote page).

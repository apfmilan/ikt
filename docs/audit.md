# Repository & Environment Audit — 2026-11-02

## 1. Existing project

| Item | Finding |
|---|---|
| Repository | `apfmilan/ikt`, branch `main`, single commit `29c66b3` (2023-10-25, "Initial commit") |
| Contents | `README.md` containing only `# ikt` |
| Existing Trading Center code | **Not present.** No source, configs, migrations, or history beyond the initial commit. |
| Other repositories reachable by this session | None (`list_repos` returns only `apfmilan/ikt`) |
| User computer folders (Remote Control) | None shared |
| Existing MySQL database | Not reachable from this environment; schema unknown |

**Consequence:** there is no existing code to preserve, so building the platform from a clean
foundation in this repository is the only option (the "never restart from scratch unless proven
necessary" condition is met: nothing exists to restart from). Because a production MySQL database
may exist on the VPS with an unknown schema, every migration in this repository is **additive and
non-destructive**: all tables are prefixed `tc_`, no `DROP`/`TRUNCATE`/destructive `ALTER`, and
SQLx's `_sqlx_migrations` ledger tracks what has been applied. If the user has an existing schema,
run `make db-inspect` (see `docs/setup.md`) and share the output before applying migrations.

## 2. Toolchain in the build environment

| Tool | Version | Status |
|---|---|---|
| Rust / Cargo | 1.97.0 (clippy, rustfmt) | OK |
| Node / npm / pnpm | 22.23 / 10.9 / present | OK |
| Docker / Compose | 29.9.2 / v5.6.1 | OK (daemon started manually in session) |
| MySQL | `mysql:8.4` image | OK via Docker (no host client) |
| Chromium + Playwright | pre-installed | OK (used for the UI preview PNG) |
| Xcode / macOS | absent | **Native iOS build impossible here** |
| Windows | absent | Tauri Windows bundle built via GitHub Actions `windows-latest` |
| `gh` CLI | present, but GitHub API access is via MCP tools only | OK |

## 3. Network reachability (verified with curl)

- `api.bitvavo.com` public endpoints: 200 (time, markets, candles, ticker)
- `docs.bitvavo.com`, `docs.trading212.com` (OpenAPI bundle downloaded): 200
- `live.trading212.com` private endpoint without credentials: 401 (expected)
- ECB euro reference rates XML (includes HUF): 200
- crates.io, npm registry, Docker Hub: OK

## 4. Verified broker capabilities (official docs, fetched 2026-11-02)

### Trading 212 Public API v0 (beta)
- HTTP Basic auth with API key + secret; optional IP allow-list on the key.
- Only **Invest** and **Stocks ISA** accounts; orders only in the primary account currency.
- Market, limit, stop, stop-limit orders; quantity-based only; sell = negative quantity.
- **Order endpoints are not idempotent** → duplicate prevention and timeout reconciliation are implemented on our side.
- Strict per-account rate limits (e.g. limit/stop orders 1 req / 2 s, positions 1 req / 1 s).
- **No market-data endpoints** (no quotes/candles) → stock scanning needs a separate data provider.

### Bitvavo API v2
- HMAC-SHA256 signed requests; key permissions Read-only / Trade / Withdraw.
- `POST /order` requires `operatorId`; supports `clientOrderId` (UUID) → idempotent intent mapping.
- Order types: market, limit, stopLoss(Limit), takeProfit(Limit); 1000 weight points/minute.
- Public candles/ticker/book endpoints usable without keys (used for scanning and backtests).
- Live responses can contain scientific-notation decimals (e.g. `"6.95E+4"`) → parser handles it.

### Revolut
- Signal-only by requirement. No automated Revolut trading is built.

## 5. Claude Code environment

- Skills available in the session: superpowers, caveman, code-review, security-review, etc.
- MCP: GitHub (official, via session), others unrelated to this project (Figma, Gmail, Linear, ...).
- Project-level `.claude/` config did not exist; created in this change (agents, skills, hooks, settings).

## 6. Blockers and deferred items

See `docs/STATUS.md` for the live list. Summary at audit time:

1. **Stock market-data provider** — Trading 212 provides none. A provider must be chosen and an API key supplied at onboarding (research in `docs/decisions/proposals/market-data-research.md`).
2. **Live broker credentials** — deferred to secure onboarding (allowed). No live integration is claimed as verified until onboarding verification passes.
3. **Native iOS companion / widget** — requires macOS + Xcode + Apple Developer account; not available here.
4. **VPS deployment** — no VPS access/credentials in this session; deployment files are prepared, not deployed.
5. **Windows installer** — cannot be built on Linux; GitHub Actions workflow provided.

# Trading Center V2 — guide for Claude Code

Autonomous, single-user trading platform. Real money. Read `docs/STATUS.md` first.

## Layout
- `crates/tc-core` domain types · `tc-risk` deterministic risk engine (absolute veto) · `tc-strategy`
  indicators/regime/strategies/fusion/sizing · `tc-backtest` walk-forward + promotion gates ·
  `tc-plan` financial plan + unitized NAV · `tc-vault` sealed credentials · `tc-store` MySQL (SQLx) ·
  `tc-broker` connector contract + live guard · `tc-broker-bitvavo`, `tc-broker-t212`, `tc-marketdata`
  · `tc-engine` autonomous runtime · `tc-server` (`--role api|engine|all`) · `tc-cli` (`tc` binary).
- `apps/web` Next.js static PWA · `apps/desktop` Tauri 2 shell · `deploy/`, `docker-compose.yml`.
- Decisions: `docs/decisions/` (ADRs) · API contract: `docs/api.md`.

## Commands
- Gate: `make check` (fmt, clippy -D warnings, tests, web typecheck/lint/build).
- MySQL integration tests: `make db-test` (throwaway mysql:8.4 container).
- Passkey E2E: see `scripts/e2e/passkey-e2e.mjs` header.
- Backtest: `.claude/skills/backtest/SKILL.md`.

## Non-negotiable rules
1. Never call broker order/withdrawal endpoints from a shell or test. Tests use wiremock only;
   `tc_broker::live::arm` can never succeed in test builds. Hooks in `.claude/hooks/` enforce this.
2. Money and quantities are `rust_decimal::Decimal`; `f64` only for statistics.
3. Order state changes only via `Store::transition_intent` (validated state machine). Never retry
   an order whose outcome is `unknown`; reconciliation resolves it.
4. The risk engine runs before every order, including manual sells and exits. Hard limits in
   `tc_risk::HardMax` must never be loosened.
5. Migrations are additive only, tables prefixed `tc_` (`.claude/skills/db-migration`).
6. Secrets: never log, never commit, `SecretString` only; API process never decrypts credentials.
7. No demo/paper mode, no mock data in the app bundle. Backtests are offline research only.
8. Do not claim a live integration works unless verified against the live broker during onboarding.
9. Broker behaviour must match official docs (`.claude/skills/broker-api-check`).

## Team
Subagents in `.claude/agents/` (orchestrator, architect, quant-researcher, trading-engineer,
risk-analyst, security-engineer, ui-ux-designer, qa-engineer, devops-engineer, code-reviewer).
Major features: competing designs → scored ADR (`.claude/skills/adr`) → implementation → independent
review by risk-analyst / security-engineer / code-reviewer → `make check`.

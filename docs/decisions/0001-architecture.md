# 0001. Hybrid modular monolith with two process roles

Status: Accepted · Date: 2026-11-02

## Context
Single user, small VPS, live money, one maintainer. Requirements favour minimal services and
dependencies, but broker trade keys must be protected from the internet-facing HTTP surface.
Verified constraints: Trading 212 order endpoints are not idempotent; Bitvavo `clientOrderId`
is unique only among *open* orders (docs.bitvavo.com create-order). Proposals:
`proposals/architect-a-monolith.md`, `proposals/architect-b-services.md`.

## Options
- **A. Single-process modular monolith** — one binary, crates per bounded context, Tokio tasks,
  MySQL as the only source of truth.
- **B. Multi-service** — gateway, engine, executor (+ Caddy, MySQL) as separate containers,
  MySQL tables as queues; only the executor can decrypt broker keys.
- **C. Hybrid (chosen)** — the A codebase compiled into one binary `tc-server` with
  `--role api|engine|all`. Production runs two containers from one image:
  `api` (public via Caddy, holds no broker secrets) and `engine` (agents, risk, execution,
  reconciliation; holds the decryption key; no inbound port). They communicate only through
  MySQL: `tc_commands` (api → engine) and `tc_events` (engine → api, fanned out over WebSockets).

## Scores (1-10)
| Option | Security | Correctness | Performance | Maintainability | UX | Cost | Total |
|---|---|---|---|---|---|---|---|
| A | 6 | 9 | 9 | 8 | 7 | 9 | 48 |
| B | 8 | 8 | 7 | 6 | 7 | 7 | 43 |
| C | 8 | 9 | 8 | 8 | 7 | 8 | 48 |

A and C tie on total; C wins on the highest-priority criterion (security) at ~25 MB extra RAM
and no extra codebase. B's extra split (engine vs executor) adds a queue hop and a third
deployable for little additional isolation (both would run on the same host; a root
compromise defeats either).

## Decision
Option C. Additional rules adopted from both proposals:
1. **Attempt-before-send**: the order attempt row is committed before any HTTP request; if the
   DB is unavailable nothing is sent. At most one unresolved attempt per instrument.
2. **Fail-closed boot**: the engine starts in `REDUCE_ONLY` (recovery) and returns to the prior
   mode only after a clean reconciliation and valid live authorization.
3. **Kill switch** is written to `tc_control` by the API and re-read by the executor
   immediately before every send (no cache), plus an in-memory flag for FREEZE.
4. **Credentials** are sealed by the API with the engine's X25519 public key
   (ephemeral ECDH + HKDF-SHA256 + AES-256-GCM, AAD = broker|key-version). The API can encrypt
   but never decrypt (ADR-0004).
5. Money is `rust_decimal`; the audit log is append-only and SHA-256 hash-chained.

## Dissent
Architect B: a hijacked API can still write a forged "resume"/"manual sell" command. Mitigation
now: step-up WebAuthn is verified by the API and every command row carries the verified
credential id and is audited; engine refuses loosening commands not followed by the 24 h delay
(ADR-0003). Full engine-side assertion verification is deferred (tracked in docs/STATUS.md).

## Consequences
- One image, two containers, same crates; `--role all` for local runs and integration tests.
- Engine-to-UI latency is bounded by the event poll interval (250 ms).
- A future split of the executor is a Compose change, not a rewrite.

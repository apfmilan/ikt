# 0002. Order lifecycle: attempt-before-send, UNKNOWN never retried

Status: Accepted · Date: 2026-11-02

## Context
Verified in official docs (2026-11-02): Trading 212 order endpoints are **not idempotent** and
have no client order id; Bitvavo supports `clientOrderId` (UUID) but it is unique only among
*open* orders, and signed requests expire after `Bitvavo-Access-Window` (max 60 s).
Proposals: `proposals/architect-a-monolith.md` §3, `proposals/architect-b-services.md` §2.

## Options
- **A. Retry with backoff on timeout** — simple, but can create duplicate live orders on T212.
- **B. Attempt-before-send + reconciliation-only resolution (chosen)** — the intent is committed
  as `submitting` before any request; an uncertain result becomes `unknown` and is resolved only by
  reading broker state.
- **C. Broker-side idempotency only** — impossible on T212.

## Scores
| Option | Security | Correctness | Performance | Maintainability | UX | Cost | Total |
|---|---|---|---|---|---|---|---|
| A | 5 | 3 | 8 | 8 | 7 | 9 | 40 |
| B | 8 | 9 | 7 | 7 | 7 | 9 | 47 |
| C | 6 | 4 | 8 | 8 | 7 | 9 | 42 |

## Decision
Option B, implemented in `tc-core::OrderState::transition` (single authority; `unknown → submitting`
is impossible) and enforced by `tc-store::transition_intent` inside a transaction:
- `pending` (risk-approved, persisted) → `submitting` (committed) → HTTP send exactly once.
- Timeout / network error / 5xx / T212 408 / Bitvavo errorCodes 101,107,108,109,111,400,419 → `unknown`.
- Bitvavo: lookup by `clientOrderId`; orders are signed with a 5 s access window, so if absent after
  window + 30 s the order cannot exist → `not_placed`.
- Trading 212: match pending orders + history by ticker, side, quantity, `initiatedFrom=API`, ±120 s,
  and an order id not already linked. Unresolved after 10 min → global `REDUCE_ONLY` + critical push.
- Any `pending/submitting/unknown` intent on an instrument blocks further orders on it (risk `IN_FLIGHT`).
- Kill switch is re-read from MySQL immediately before each send.

## Dissent
T212 history lag could leave a filled order unmatched. Mitigation: position reconciliation is a
second witness (tracked > broker quantity raises a mismatch) and entries stay blocked meanwhile.

## Consequences
Verified by `tc-core` state-machine tests, `tc-store` MySQL integration tests and connector mock
tests (timeouts → `Unknown`). Live behaviour against real brokers is untested until onboarding.

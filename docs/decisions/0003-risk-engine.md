# 0003. Deterministic risk engine with absolute veto

Status: Accepted · Date: 2026-11-02

## Context
Spec and test matrix: `proposals/risk-engine.md` (Risk Analyst). Requirement: deterministic risk
engine with absolute veto; no leverage; daily loss limits; drawdown protection; exposure caps;
duplicate prevention; never chase losses.

## Options
- **A. Rule list inside strategies** — scattered, bypassable.
- **B. Central pure function `evaluate(request, snapshot, config, now)` (chosen)**.
- **C. LLM/agent judgement with rule hints** — non-deterministic; rejected by requirement.

## Scores
| Option | Security | Correctness | Performance | Maintainability | UX | Cost | Total |
|---|---|---|---|---|---|---|---|
| A | 4 | 5 | 9 | 4 | 6 | 9 | 37 |
| B | 9 | 9 | 9 | 8 | 7 | 9 | 51 |
| C | 3 | 3 | 5 | 4 | 7 | 5 | 27 |

## Decision
Option B (`crates/tc-risk`), with these amendments to the proposal:
1. **Clamp, then veto**: cap checks (risk per trade, position, class, total, cluster, cash) shrink the
   quantity; if the result falls below the venue minimum the binding check's code vetoes.
2. **`LongTerm` class** for the ETF DCA core: whitelist + cash + market hours + health checks; no edge,
   stop or trading caps.
3. **Withdrawal permission is tri-state**: verified-absent (Bitvavo probe errorCode 312, T212 has no
   withdrawal API), proven-present (refused), or unknown → user attestation required (audited).
4. Hard limits are compiled constants (`HardMax`); `RiskConfig::validate` refuses anything looser;
   loosening applies after 24 h and revokes live authorization (`PUT /api/risk`).

## Dissent
Defaults sized for 25,000 HUF make many trades infeasible (venue minimum vs 1 % risk). Accepted:
abstaining is the correct outcome when risk cannot be bounded.

## Consequences
35 unit tests mirror the spec's test matrix. Exits are never blocked by entry-only checks.

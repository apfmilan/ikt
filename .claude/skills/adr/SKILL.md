---
name: adr
description: Record an architecture decision for Trading Center in docs/decisions after a competing-design review. Use when a design choice between options has been made.
---
# Architecture Decision Record

1. Find the next number: `ls docs/decisions/ | grep -E '^[0-9]{4}-' | sort | tail -1`.
2. Create `docs/decisions/NNNN-kebab-title.md` with exactly these sections:
   - `# NNNN. Title`
   - `Status: Accepted | Superseded by NNNN | Proposed` and `Date: YYYY-MM-DD`
   - `## Context` – constraints, verified facts (cite official docs / benchmarks).
   - `## Options` – each option with a one-paragraph description.
   - `## Scores` – table: option × {security, correctness, performance, maintainability, UX, cost} 1-10, plus total.
   - `## Decision` – chosen option and why the evidence supports it.
   - `## Dissent` – strongest objection raised and how it is mitigated.
   - `## Consequences` – what becomes easier/harder; follow-up tasks.
3. Link proposals in `docs/decisions/proposals/` that informed it.
4. Keep it under ~600 words. Never record a decision that has no evidence beyond agent agreement.

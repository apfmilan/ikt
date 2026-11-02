---
name: quality-gate
description: Run the full Trading Center quality gate (format, lint, tests, MySQL integration tests, web build, audits) and report verified results. Use before every commit/PR.
---
# Quality gate

Run from repo root, in order, and stop at the first failure:
1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. MySQL integration: `make db-test` (starts `mysql:8.4` in Docker, runs migrations and `cargo test -p tc-store -- --ignored`)
5. Web: `cd apps/web && npm ci && npm run typecheck && npm run lint && npm run build`
6. Audits: `cargo audit` (if installed) and `cd apps/web && npm audit --omit=dev`
7. Secret scan of the diff: `git diff --cached | grep -nE '(api[_-]?secret|BEGIN [A-Z ]*PRIVATE KEY|Bitvavo-Access-Key: [A-Za-z0-9])' && echo FOUND`

Report each step with pass/fail and the shortest decisive error line. Never mark the gate green if a step was skipped; say which step was skipped and why.

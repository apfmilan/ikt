---
name: db-migration
description: Add a safe, additive MySQL migration for Trading Center (SQLx). Use whenever the schema changes.
---
# Add a migration

1. `ls migrations/` and pick the next timestamp prefix: `YYYYMMDDHHMMSS_description.sql`.
2. Rules: additive only (CREATE TABLE IF NOT EXISTS, ADD COLUMN with DEFAULT, CREATE INDEX). No DROP, TRUNCATE, RENAME, or type narrowing. All tables prefixed `tc_`. Money columns `DECIMAL(28,10)`; timestamps `DATETIME(6)` UTC; InnoDB, utf8mb4.
3. Audit tables (`tc_audit_log`, `tc_ledger_entries`, `tc_decisions`) are append-only: never UPDATE/DELETE them in code.
4. Test: `make db-test`.
5. Never edit an already-applied migration (SQLx checksums will fail); add a new one.

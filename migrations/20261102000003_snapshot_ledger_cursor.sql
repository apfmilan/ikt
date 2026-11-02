-- Additive: equity snapshots remember the last ledger id whose external flow was applied to
-- the unit ledger, so late-arriving deposits are never mistaken for profit.
ALTER TABLE tc_equity_snapshots ADD COLUMN ledger_id_applied BIGINT NOT NULL DEFAULT 0;

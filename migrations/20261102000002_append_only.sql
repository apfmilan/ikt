-- Append-only enforcement for the audit trail. Single-statement trigger bodies (no BEGIN/END)
-- so the file runs through any client without DELIMITER handling.
CREATE TRIGGER tc_audit_log_no_update BEFORE UPDATE ON tc_audit_log FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'tc_audit_log is append-only';
CREATE TRIGGER tc_audit_log_no_delete BEFORE DELETE ON tc_audit_log FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'tc_audit_log is append-only';
CREATE TRIGGER tc_ledger_no_update BEFORE UPDATE ON tc_ledger FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'tc_ledger is append-only';
CREATE TRIGGER tc_ledger_no_delete BEFORE DELETE ON tc_ledger FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'tc_ledger is append-only';
CREATE TRIGGER tc_order_events_no_update BEFORE UPDATE ON tc_order_events FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'tc_order_events is append-only';
CREATE TRIGGER tc_order_events_no_delete BEFORE DELETE ON tc_order_events FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'tc_order_events is append-only';
CREATE TRIGGER tc_decisions_no_update BEFORE UPDATE ON tc_decisions FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'tc_decisions is append-only';
CREATE TRIGGER tc_decisions_no_delete BEFORE DELETE ON tc_decisions FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'tc_decisions is append-only';
CREATE TRIGGER tc_fills_no_delete BEFORE DELETE ON tc_fills FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'tc_fills is append-only';

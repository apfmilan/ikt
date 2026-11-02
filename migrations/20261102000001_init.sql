-- Trading Center V2 initial schema. Additive only: every table is new and prefixed tc_.
-- Money/quantities DECIMAL(36,18); timestamps DATETIME(6) in UTC. InnoDB / utf8mb4.

CREATE TABLE IF NOT EXISTS tc_passkeys (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  cred_id VARBINARY(1023) NOT NULL,
  cred_id_hash BINARY(32) NOT NULL,
  passkey_json JSON NOT NULL,
  device_name VARCHAR(100) NOT NULL,
  created_at DATETIME(6) NOT NULL,
  last_used_at DATETIME(6) NULL,
  revoked_at DATETIME(6) NULL,
  UNIQUE KEY uq_tc_passkeys_cred (cred_id_hash)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_user (
  id TINYINT PRIMARY KEY,
  user_uuid CHAR(36) NOT NULL,
  display_name VARCHAR(100) NOT NULL,
  created_at DATETIME(6) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_sessions (
  token_hash BINARY(32) PRIMARY KEY,
  passkey_id BIGINT NULL,
  created_at DATETIME(6) NOT NULL,
  last_seen_at DATETIME(6) NOT NULL,
  idle_expires_at DATETIME(6) NOT NULL,
  absolute_expires_at DATETIME(6) NOT NULL,
  user_agent VARCHAR(255) NULL,
  revoked_at DATETIME(6) NULL,
  KEY ix_tc_sessions_exp (absolute_expires_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_one_time_tokens (
  token_hash BINARY(32) PRIMARY KEY,
  kind VARCHAR(16) NOT NULL,            -- setup | pairing | recovery | stepup
  action VARCHAR(32) NULL,              -- step-up action binding
  passkey_id BIGINT NULL,
  created_at DATETIME(6) NOT NULL,
  expires_at DATETIME(6) NULL,
  used_at DATETIME(6) NULL,
  KEY ix_tc_ott_kind (kind, used_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_credentials (
  broker VARCHAR(32) NOT NULL,
  field VARCHAR(32) NOT NULL,
  key_version INT NOT NULL,
  envelope VARBINARY(2048) NOT NULL,
  created_at DATETIME(6) NOT NULL,
  PRIMARY KEY (broker, field)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_credential_status (
  broker VARCHAR(32) PRIMARY KEY,
  authenticated BOOLEAN NOT NULL DEFAULT FALSE,
  can_trade BOOLEAN NOT NULL DEFAULT FALSE,
  can_withdraw TINYINT NULL,            -- 0 verified no, 1 yes (refused), NULL unknown
  attested_no_withdraw BOOLEAN NOT NULL DEFAULT FALSE,
  scopes_ok BOOLEAN NOT NULL DEFAULT FALSE,
  account_currency CHAR(3) NULL,
  notes JSON NULL,
  checked_at DATETIME(6) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_control (
  scope VARCHAR(80) PRIMARY KEY,        -- global | broker:<b> | strategy:<s>
  mode VARCHAR(16) NOT NULL,            -- normal | reduce_only | flatten | freeze
  reason VARCHAR(255) NULL,
  updated_at DATETIME(6) NOT NULL,
  updated_by VARCHAR(64) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_live_authorizations (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  broker VARCHAR(32) NOT NULL,
  authorized_at DATETIME(6) NOT NULL,
  expires_at DATETIME(6) NOT NULL,
  config_hash CHAR(64) NOT NULL,
  passkey_id BIGINT NOT NULL,
  revoked_at DATETIME(6) NULL,
  revoke_reason VARCHAR(255) NULL,
  KEY ix_tc_live_broker (broker, revoked_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_config (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  kind VARCHAR(16) NOT NULL,            -- risk | plan | notifications | engine
  config_json JSON NOT NULL,
  effective_at DATETIME(6) NOT NULL,
  created_at DATETIME(6) NOT NULL,
  created_by VARCHAR(64) NOT NULL,
  KEY ix_tc_config_kind (kind, effective_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_instruments (
  id VARCHAR(96) PRIMARY KEY,
  broker VARCHAR(32) NOT NULL,
  symbol VARCHAR(64) NOT NULL,
  name VARCHAR(255) NOT NULL,
  asset_class VARCHAR(16) NOT NULL,
  quote_currency CHAR(3) NOT NULL,
  isin CHAR(12) NULL,
  min_notional DECIMAL(36,18) NOT NULL,
  min_quantity DECIMAL(36,18) NOT NULL,
  quantity_decimals INT NOT NULL,
  tick_size DECIMAL(36,18) NOT NULL,
  tradable BOOLEAN NOT NULL,
  first_seen DATETIME(6) NOT NULL,
  updated_at DATETIME(6) NOT NULL,
  meta JSON NULL,
  KEY ix_tc_instruments_broker (broker, tradable)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_candles (
  instrument_id VARCHAR(96) NOT NULL,
  bar_interval VARCHAR(4) NOT NULL,
  ts DATETIME(6) NOT NULL,
  o DECIMAL(36,18) NOT NULL,
  h DECIMAL(36,18) NOT NULL,
  l DECIMAL(36,18) NOT NULL,
  c DECIMAL(36,18) NOT NULL,
  v DECIMAL(36,18) NOT NULL,
  PRIMARY KEY (instrument_id, bar_interval, ts)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_order_intents (
  id CHAR(36) PRIMARY KEY,
  instrument_id VARCHAR(96) NOT NULL,
  side VARCHAR(4) NOT NULL,
  kind_json JSON NOT NULL,
  quantity DECIMAL(36,18) NOT NULL,
  time_in_force VARCHAR(8) NOT NULL,
  reason VARCHAR(32) NOT NULL,
  strategy VARCHAR(64) NOT NULL,
  decision_id CHAR(36) NULL,
  reference_price DECIMAL(36,18) NOT NULL,
  state VARCHAR(20) NOT NULL,
  broker_order_id VARCHAR(80) NULL,
  filled_quantity DECIMAL(36,18) NOT NULL DEFAULT 0,
  avg_fill_price DECIMAL(36,18) NULL,
  fee_paid DECIMAL(36,18) NOT NULL DEFAULT 0,
  fee_currency CHAR(3) NULL,
  risk_verdict JSON NULL,
  last_error VARCHAR(512) NULL,
  created_at DATETIME(6) NOT NULL,
  submitted_at DATETIME(6) NULL,
  updated_at DATETIME(6) NOT NULL,
  version INT NOT NULL DEFAULT 0,
  KEY ix_tc_intents_inst_state (instrument_id, state),
  KEY ix_tc_intents_state (state, updated_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_order_events (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  intent_id CHAR(36) NOT NULL,
  from_state VARCHAR(20) NOT NULL,
  to_state VARCHAR(20) NOT NULL,
  detail JSON NULL,
  ts DATETIME(6) NOT NULL,
  KEY ix_tc_order_events_intent (intent_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_fills (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  broker VARCHAR(32) NOT NULL,
  broker_fill_id VARCHAR(96) NOT NULL,
  intent_id CHAR(36) NULL,
  instrument_id VARCHAR(96) NOT NULL,
  side VARCHAR(4) NOT NULL,
  quantity DECIMAL(36,18) NOT NULL,
  price DECIMAL(36,18) NOT NULL,
  fee DECIMAL(36,18) NOT NULL,
  fee_currency CHAR(3) NULL,
  ts DATETIME(6) NOT NULL,
  UNIQUE KEY uq_tc_fills (broker, broker_fill_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_positions (
  instrument_id VARCHAR(96) PRIMARY KEY,
  quantity DECIMAL(36,18) NOT NULL,
  avg_price DECIMAL(36,18) NOT NULL,
  opened_at DATETIME(6) NOT NULL,
  stop_price DECIMAL(36,18) NULL,
  take_profit DECIMAL(36,18) NULL,
  initial_risk DECIMAL(36,18) NULL,
  high_watermark DECIMAL(36,18) NOT NULL,
  strategy VARCHAR(64) NOT NULL,
  long_term BOOLEAN NOT NULL DEFAULT FALSE,
  bars_held INT NOT NULL DEFAULT 0,
  partial_taken BOOLEAN NOT NULL DEFAULT FALSE,
  stop_intent_id CHAR(36) NULL,
  decision_id CHAR(36) NULL,
  updated_at DATETIME(6) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_trades (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  instrument_id VARCHAR(96) NOT NULL,
  strategy VARCHAR(64) NOT NULL,
  entry_at DATETIME(6) NOT NULL,
  exit_at DATETIME(6) NOT NULL,
  entry_price DECIMAL(36,18) NOT NULL,
  exit_price DECIMAL(36,18) NOT NULL,
  quantity DECIMAL(36,18) NOT NULL,
  gross_quote DECIMAL(36,18) NOT NULL,
  fees_quote DECIMAL(36,18) NOT NULL,
  net_quote DECIMAL(36,18) NOT NULL,
  net_huf DECIMAL(36,18) NOT NULL,
  fees_huf DECIMAL(36,18) NOT NULL,
  return_pct DECIMAL(20,10) NOT NULL,
  exit_reason VARCHAR(32) NOT NULL,
  stopped_out BOOLEAN NOT NULL,
  decision_id CHAR(36) NULL,
  forecast_json JSON NULL,
  KEY ix_tc_trades_exit (exit_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_ledger (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  ts DATETIME(6) NOT NULL,
  broker VARCHAR(32) NOT NULL,
  kind VARCHAR(16) NOT NULL,            -- deposit | withdrawal | fee | realized_pnl | dividend | interest | fx
  amount DECIMAL(36,18) NOT NULL,
  currency CHAR(3) NOT NULL,
  amount_huf DECIMAL(36,18) NOT NULL,
  fx_rate DECIMAL(36,18) NOT NULL,
  ref VARCHAR(128) NOT NULL,
  note VARCHAR(255) NULL,
  UNIQUE KEY uq_tc_ledger_ref (broker, kind, ref),
  KEY ix_tc_ledger_ts (ts)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_equity_snapshots (
  ts DATETIME(6) PRIMARY KEY,
  equity_huf DECIMAL(36,18) NOT NULL,
  cash_huf DECIMAL(36,18) NOT NULL,
  positions_huf DECIMAL(36,18) NOT NULL,
  net_deposits_huf DECIMAL(36,18) NOT NULL,
  units DECIMAL(36,18) NOT NULL,
  nav_unit DECIMAL(36,18) NOT NULL,
  nav_peak DECIMAL(36,18) NOT NULL,
  per_broker JSON NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_decisions (
  id CHAR(36) PRIMARY KEY,
  created_at DATETIME(6) NOT NULL,
  instrument_id VARCHAR(96) NOT NULL,
  action VARCHAR(16) NOT NULL,
  strategy VARCHAR(64) NOT NULL,
  confidence DOUBLE NOT NULL,
  forecast JSON NOT NULL,
  signals JSON NOT NULL,
  rationale JSON NOT NULL,
  risk JSON NULL,
  KEY ix_tc_decisions_created (created_at),
  KEY ix_tc_decisions_inst (instrument_id, created_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_agent_status (
  agent VARCHAR(32) PRIMARY KEY,
  status VARCHAR(16) NOT NULL,
  last_run_at DATETIME(6) NOT NULL,
  last_duration_ms BIGINT NOT NULL,
  summary VARCHAR(512) NOT NULL,
  metrics JSON NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_strategies (
  name VARCHAR(64) PRIMARY KEY,
  version INT NOT NULL,
  status VARCHAR(16) NOT NULL,          -- disabled | validated | live_half | live | demoted
  asset_class VARCHAR(16) NOT NULL,
  report JSON NULL,
  updated_at DATETIME(6) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_strategy_edges (
  strategy VARCHAR(64) NOT NULL,
  market_group VARCHAR(64) NOT NULL,
  regime VARCHAR(16) NOT NULL,
  trades INT NOT NULL,
  mean_gross_bps DOUBLE NOT NULL,
  se_bps DOUBLE NOT NULL,
  win_rate DOUBLE NOT NULL,
  updated_at DATETIME(6) NOT NULL,
  PRIMARY KEY (strategy, market_group, regime)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_commands (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  kind VARCHAR(32) NOT NULL,
  payload JSON NOT NULL,
  created_at DATETIME(6) NOT NULL,
  created_by VARCHAR(64) NOT NULL,
  stepup_action VARCHAR(32) NULL,
  status VARCHAR(16) NOT NULL,          -- pending | running | done | failed
  result JSON NULL,
  processed_at DATETIME(6) NULL,
  KEY ix_tc_commands_status (status, id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_events (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  ts DATETIME(6) NOT NULL,
  type VARCHAR(32) NOT NULL,
  data JSON NOT NULL,
  KEY ix_tc_events_ts (ts)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_push_subscriptions (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  endpoint VARCHAR(1024) NOT NULL,
  endpoint_hash BINARY(32) NOT NULL,
  p256dh VARCHAR(255) NOT NULL,
  auth VARCHAR(255) NOT NULL,
  device_name VARCHAR(100) NOT NULL,
  created_at DATETIME(6) NOT NULL,
  UNIQUE KEY uq_tc_push_endpoint (endpoint_hash)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_audit_log (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  ts DATETIME(6) NOT NULL,
  actor VARCHAR(64) NOT NULL,
  action VARCHAR(64) NOT NULL,
  detail JSON NOT NULL,
  prev_hash BINARY(32) NOT NULL,
  hash BINARY(32) NOT NULL,
  KEY ix_tc_audit_ts (ts)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_fx_rates (
  rate_date DATE NOT NULL,
  currency CHAR(3) NOT NULL,
  per_eur DECIMAL(36,18) NOT NULL,
  source VARCHAR(16) NOT NULL,
  fetched_at DATETIME(6) NOT NULL,
  PRIMARY KEY (rate_date, currency)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_heartbeat (
  component VARCHAR(16) PRIMARY KEY,
  ts DATETIME(6) NOT NULL,
  mode VARCHAR(16) NOT NULL,
  version VARCHAR(32) NOT NULL,
  detail JSON NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tc_watchlist (
  instrument_id VARCHAR(96) PRIMARY KEY,
  venue VARCHAR(32) NOT NULL,           -- revolut (signal-only) etc.
  note VARCHAR(255) NULL,
  created_at DATETIME(6) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

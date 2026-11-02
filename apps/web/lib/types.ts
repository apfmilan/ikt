// Types mirroring docs/api.md. Money values are decimal strings; never parse them into
// floats for arithmetic — only for display and charting.

export type Dec = string;
export type Iso = string;

export type BrokerId = "trading212" | "bitvavo" | "revolut";
export type Mode = "normal" | "reduce_only" | "flatten" | "freeze" | "recovery";
export type KillMode = Exclude<Mode, "recovery">;
export type Health = "ok" | "degraded" | "down";
export type BotId = "crypto" | "stocks" | "invest";
export type BotStatus = "active" | "idle" | "paused" | "blocked" | "error";

export type StepUpAction =
  | "live_enable"
  | "risk_change"
  | "manual_sell"
  | "kill_release"
  | "credentials_change"
  | "device_remove"
  | "plan_change"
  | "flatten"
  | "device_add";

export interface ApiErrorBody {
  error: { code: string; message: string };
}

// ---- Auth
export interface AuthStatus {
  setup_required: boolean;
  authenticated: boolean;
  user?: { display_name: string };
  devices?: number;
}

export interface ChallengeResponse {
  challenge_id: string;
  // WebAuthn options JSON (base64url fields). webauthn-rs wraps them in { publicKey }.
  options: unknown;
}

export interface SetupFinishResponse {
  ok: boolean;
  recovery_codes?: string[];
}

export interface StepUpToken {
  stepup_token: string;
  expires_at: Iso;
}

export interface Device {
  id: string;
  name: string;
  created_at: Iso;
  last_used_at: Iso | null;
}

export interface PairingCode {
  pairing_code: string;
  expires_at: Iso;
}

// ---- Overview
export interface BrokerCard {
  broker: BrokerId;
  connected: boolean;
  equity_huf: Dec;
  cash_huf: Dec;
  currency: string;
  last_sync: Iso | null;
  health: Health;
}

export interface BotCard {
  id: BotId;
  name: string;
  status: BotStatus;
  today_pnl_huf: Dec;
  open_positions: number;
  last_decision_at: Iso | null;
  message: string;
}

export interface Overview {
  as_of: Iso;
  mode: Mode;
  live: Partial<Record<BrokerId, boolean>>;
  realized: {
    equity_huf: Dec;
    net_deposits_huf: Dec;
    net_pnl_total_huf: Dec;
    net_pnl_today_huf: Dec;
    fees_total_huf: Dec;
    nav_unit: Dec;
    drawdown_pct: Dec;
  };
  brokers: BrokerCard[];
  bots: BotCard[];
  equity_curve: { t: Iso; equity_huf: Dec; net_deposits_huf: Dec }[];
}

export type AgentName =
  | "scanner"
  | "technical"
  | "regime"
  | "volatility"
  | "news"
  | "listings"
  | "ranker"
  | "portfolio"
  | "exit"
  | "risk"
  | "execution"
  | "reconciler"
  | "long_term_invest"
  | "revolut_watch";

export interface AgentStatus {
  agent: AgentName;
  status: string;
  last_run_at: Iso | null;
  last_duration_ms: number | null;
  summary: string;
  metrics: Record<string, string | number | boolean | null>;
}

export interface PositionRow {
  instrument: string;
  symbol: string;
  name: string;
  broker: BrokerId;
  asset_class: "crypto" | "stock" | "etf";
  quantity: Dec;
  avg_price: Dec;
  last_price: Dec | null;
  currency: string;
  value_huf: Dec;
  realized: null;
  unrealized_huf: Dec;
  unrealized_pct: Dec;
  stop_price: Dec | null;
  take_profit: Dec | null;
  trailing_stop: Dec | null;
  strategy: string;
  opened_at: Iso;
  long_term: boolean;
  decision_id: string | null;
}

export interface OrderRow {
  id: string;
  instrument: string;
  side: "buy" | "sell";
  kind: string;
  quantity: Dec;
  filled_quantity: Dec;
  state: string;
  reason: string;
  broker_order_id: string | null;
  created_at: Iso;
  updated_at: Iso;
  avg_fill_price: Dec | null;
  fee: Dec | null;
}

export interface TradeRow {
  id: string;
  instrument: string;
  strategy: string;
  entry_at: Iso;
  exit_at: Iso;
  entry_price: Dec;
  exit_price: Dec;
  quantity: Dec;
  realized: { gross_huf: Dec; fees_huf: Dec; net_huf: Dec; return_pct: Dec };
  forecast: { p_win: number; expected_net_bps: number } | null;
  exit_reason: string;
  decision_id: string | null;
}

export type LedgerKind = "deposit" | "withdrawal" | "fee" | "realized_pnl" | "dividend" | "interest" | "fx";

export interface TransactionRow {
  id: string;
  ts: Iso;
  broker: BrokerId;
  kind: LedgerKind;
  amount: Dec;
  currency: string;
  amount_huf: Dec;
  ref: string | null;
}

export interface ForecastBlock {
  p_win: number;
  expected_net_bps: number;
  cost_bps: number;
  uncertainty_bps: number;
  p05_bps: number;
  p95_bps: number;
}

export interface SignalRow {
  agent: AgentName;
  direction: "long" | "flat" | "exit";
  strength: number;
  confidence: number;
  expected_edge_bps: number;
  rationale: string;
}

export interface Veto {
  code: string;
  detail: string;
}

export interface DecisionRow {
  id: string;
  created_at: Iso;
  instrument: string;
  action: "enter" | "exit" | "hold" | "abstain";
  strategy: string;
  confidence: number;
  forecast: ForecastBlock;
  rationale: string[];
  signals: SignalRow[];
  risk: { verdict: "allow" | "allow_resized" | "veto"; vetoes: Veto[]; quantity: Dec | null } | null;
}

export interface ChartData {
  candles: { t: Iso; o: Dec; h: Dec; l: Dec; c: Dec; v: Dec }[];
  markers: { t: Iso; kind: "entry" | "exit" | "stop"; price: Dec; text: string }[];
  lines: { kind: "avg_price" | "stop" | "take_profit"; price: Dec }[];
}

export interface StrategyPerf {
  strategy: string;
  status: string;
  trades: number;
  win_rate: number;
  net_huf: Dec;
  fees_huf: Dec;
  max_drawdown_pct: Dec;
  backtest_oos: { sharpe: number; profit_factor: number; expectancy_r: number; trades: number } | null;
}

export interface Performance {
  realized: {
    total_return_pct: Dec;
    win_rate: number;
    profit_factor: number;
    max_drawdown_pct: Dec;
    trades: number;
    fees_huf: Dec;
    expectancy_huf: Dec;
  };
  by_strategy: StrategyPerf[];
  daily: { date: string; net_pnl_huf: Dec }[];
}

export type StrategyStatus = "disabled" | "validated" | "live_half" | "live" | "demoted";

export interface StrategyRow {
  name: string;
  version: string;
  status: StrategyStatus;
  asset_class: string;
  regimes: string[];
  backtest: {
    folds: number;
    oos_trades: number;
    oos_sharpe: number;
    oos_profit_factor: number;
    oos_max_dd_pct: number;
    eligible: boolean;
  } | null;
  live: { trades: number; expectancy_huf: Dec } | null;
}

export interface PlanConfig {
  start_month: string;
  initial_huf: Dec;
  monthly_until_apr_2027_huf: Dec;
  monthly_from_may_2027_huf: Dec;
  long_term_share_pct: Dec;
  long_term_start_month: string;
  reinvest_pct: Dec;
  withdraw_pct: Dec;
  etf_targets: { instrument: string; weight_pct: Dec }[];
}

export interface Plan {
  config: PlanConfig;
  schedule: { month: string; planned_trading_huf: Dec; planned_long_term_huf: Dec; actual_deposits_huf: Dec }[];
  progress: { planned_total_huf: Dec; deposited_total_huf: Dec; trading_profit_total_huf: Dec; withdrawable_huf: Dec };
}

export type RiskConfig = Record<string, Dec | number>;

export interface RiskView {
  config: RiskConfig;
  hard_max: Record<string, Dec | number>;
  utilization: {
    exposure_pct: Dec;
    crypto_pct: Dec;
    equity_pct: Dec;
    day_pnl_pct: Dec;
    drawdown_pct: Dec;
    entries_today: number;
  };
  pending_change?: { config: RiskConfig; effective_at: Iso };
}

export type OnboardingStepId = "passkey" | "trading212" | "bitvavo" | "market_data" | "risk" | "plan" | "authorize";

export interface Onboarding {
  steps: { id: OnboardingStepId; status: "todo" | "done" | "error" | "blocked"; detail: string }[];
  ready: boolean;
}

export interface NotificationSettings {
  trades: boolean;
  market_open: boolean;
  errors: boolean;
  critical_risk: boolean;
  daily_summary: boolean;
  daily_summary_time: string;
}

export interface HealthResponse {
  status: "ok" | "degraded";
  db: boolean;
  engine_heartbeat_age_s: number;
  version: string;
}

export type WsType = "overview" | "position" | "order" | "decision" | "agent" | "alert" | "mode" | "onboarding";

export interface WsMessage<T = unknown> {
  type: WsType;
  data: T;
  ts: Iso;
}

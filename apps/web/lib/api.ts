// Typed client for the Trading Center API (docs/api.md). Same-origin only: the session
// cookie is __Host- + SameSite=Strict, so no cross-origin base URL is supported.

import type {
  AgentStatus,
  AuthStatus,
  ChallengeResponse,
  ChartData,
  DecisionRow,
  Device,
  HealthResponse,
  KillMode,
  NotificationSettings,
  Onboarding,
  OrderRow,
  Overview,
  PairingCode,
  Performance,
  Plan,
  PlanConfig,
  PositionRow,
  RiskConfig,
  RiskView,
  SetupFinishResponse,
  StepUpAction,
  StepUpToken,
  StrategyRow,
  TradeRow,
  TransactionRow,
} from "./types";

export const API_BASE = "/api";

export class ApiError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

interface RequestOptions {
  method?: "GET" | "POST" | "PUT" | "DELETE";
  body?: unknown;
  stepupToken?: string;
  signal?: AbortSignal;
}

export async function request<T>(path: string, opts: RequestOptions = {}): Promise<T> {
  const headers: Record<string, string> = { Accept: "application/json" };
  if (opts.body !== undefined) headers["Content-Type"] = "application/json";
  if (opts.stepupToken) headers["X-Stepup-Token"] = opts.stepupToken;

  let res: Response;
  try {
    res = await fetch(`${API_BASE}${path}`, {
      method: opts.method ?? "GET",
      headers,
      body: opts.body === undefined ? undefined : JSON.stringify(opts.body),
      credentials: "same-origin",
      cache: "no-store",
      signal: opts.signal,
    });
  } catch (e) {
    if (e instanceof DOMException && e.name === "AbortError") throw e;
    throw new ApiError(0, "network", "A szerver nem érhető el.");
  }

  const text = await res.text();
  let json: unknown = undefined;
  if (text) {
    try {
      json = JSON.parse(text);
    } catch {
      throw new ApiError(res.status, "bad_response", `Érvénytelen válasz (${res.status}).`);
    }
  }
  if (!res.ok) {
    const err = (json as { error?: { code?: string; message?: string } } | undefined)?.error;
    throw new ApiError(res.status, err?.code ?? `http_${res.status}`, err?.message ?? `Hiba (${res.status}).`);
  }
  return json as T;
}

const enc = encodeURIComponent;

export const api = {
  // Auth
  authStatus: () => request<AuthStatus>("/auth/status"),
  setupStart: (setup_token: string, device_name: string) =>
    request<ChallengeResponse>("/auth/setup/start", { method: "POST", body: { setup_token, device_name } }),
  setupFinish: (challenge_id: string, credential: unknown) =>
    request<SetupFinishResponse>("/auth/setup/finish", { method: "POST", body: { challenge_id, credential } }),
  loginStart: () => request<ChallengeResponse>("/auth/login/start", { method: "POST", body: {} }),
  loginFinish: (challenge_id: string, credential: unknown) =>
    request<{ ok: boolean }>("/auth/login/finish", { method: "POST", body: { challenge_id, credential } }),
  recover: (recovery_code: string, device_name: string) =>
    request<ChallengeResponse>("/auth/recover", { method: "POST", body: { recovery_code, device_name } }),
  stepupStart: (action: StepUpAction) =>
    request<ChallengeResponse>("/auth/stepup/start", { method: "POST", body: { action } }),
  stepupFinish: (challenge_id: string, action: StepUpAction, credential: unknown) =>
    request<StepUpToken>("/auth/stepup/finish", { method: "POST", body: { challenge_id, action, credential } }),
  pair: (stepupToken: string) => request<PairingCode>("/auth/pair", { method: "POST", body: {}, stepupToken }),
  pairStart: (pairing_code: string, device_name: string) =>
    request<ChallengeResponse>("/auth/pair/start", { method: "POST", body: { pairing_code, device_name } }),
  devices: () => request<Device[]>("/auth/devices"),
  removeDevice: (id: string, stepupToken: string) =>
    request<{ ok: boolean }>(`/auth/devices/${enc(id)}`, { method: "DELETE", stepupToken }),
  logout: () => request<{ ok: boolean }>("/auth/logout", { method: "POST", body: {} }),

  // Data
  overview: () => request<Overview>("/overview"),
  agents: () => request<AgentStatus[]>("/agents"),
  positions: () => request<PositionRow[]>("/positions"),
  orders: (state: "open" | "all" = "open", limit = 100) => request<OrderRow[]>(`/orders?state=${state}&limit=${limit}`),
  trades: (limit = 100, before?: string) =>
    request<TradeRow[]>(`/trades?limit=${limit}${before ? `&before=${enc(before)}` : ""}`),
  transactions: (limit = 100, before?: string) =>
    request<TransactionRow[]>(`/transactions?limit=${limit}${before ? `&before=${enc(before)}` : ""}`),
  decisions: (limit = 50, instrument?: string) =>
    request<DecisionRow[]>(`/decisions?limit=${limit}${instrument ? `&instrument=${enc(instrument)}` : ""}`),
  decision: (id: string) => request<DecisionRow>(`/decisions/${enc(id)}`),
  chart: (instrument: string, interval = "1h", limit = 300) =>
    request<ChartData>(`/instruments/${enc(instrument)}/chart?interval=${interval}&limit=${limit}`),
  performance: () => request<Performance>("/performance"),
  strategies: () => request<StrategyRow[]>("/strategies"),

  // Plan / risk / controls
  plan: () => request<Plan>("/plan"),
  updatePlan: (config: PlanConfig, stepupToken: string) =>
    request<Plan>("/plan", { method: "PUT", body: config, stepupToken }),
  risk: () => request<RiskView>("/risk"),
  updateRisk: (config: RiskConfig, stepupToken: string) =>
    request<{ effective_at?: string }>("/risk", { method: "PUT", body: config, stepupToken }),
  kill: (mode: KillMode, stepupToken?: string) =>
    request<{ ok: boolean }>("/control/kill", { method: "POST", body: { mode }, stepupToken }),
  setLive: (broker: string, enabled: boolean, stepupToken?: string) =>
    request<{ ok: boolean }>("/control/live", { method: "POST", body: { broker, enabled }, stepupToken }),
  sell: (instrument: string, quantity: "all" | string, stepupToken: string) =>
    request<{ command_id: string }>("/positions/sell", { method: "POST", body: { instrument, quantity }, stepupToken }),

  // Onboarding
  onboarding: () => request<Onboarding>("/onboarding"),
  saveCredentials: (
    body: { broker: "trading212" | "bitvavo" | "eodhd"; api_key: string; api_secret?: string; operator_id?: string },
    stepupToken?: string,
  ) => request<{ stored: boolean }>("/onboarding/credentials", { method: "POST", body, stepupToken }),
  verify: (broker: string) => request<{ command_id: string }>("/onboarding/verify", { method: "POST", body: { broker } }),
  attest: (broker: string, stepupToken: string) =>
    request<{ ok: boolean }>("/onboarding/attest", { method: "POST", body: { broker, withdraw_disabled: true }, stepupToken }),
  onboardingRisk: (config: RiskConfig) => request<{ ok: boolean }>("/onboarding/risk", { method: "POST", body: config }),
  authorize: (brokers: string[], stepupToken: string) =>
    request<{ ok: boolean }>("/onboarding/authorize", { method: "POST", body: { brokers }, stepupToken }),

  // Notifications
  vapid: () => request<{ public_key: string }>("/push/vapid"),
  pushSubscribe: (sub: { endpoint: string; keys: { p256dh: string; auth: string }; device_name: string }) =>
    request<{ ok: boolean }>("/push/subscribe", { method: "POST", body: sub }),
  pushUnsubscribe: (endpoint: string) =>
    request<{ ok: boolean }>("/push/subscribe", { method: "DELETE", body: { endpoint } }),
  notificationSettings: () => request<NotificationSettings>("/notifications/settings"),
  updateNotificationSettings: (s: NotificationSettings) =>
    request<NotificationSettings>("/notifications/settings", { method: "PUT", body: s }),

  health: () => request<HealthResponse>("/health"),
};

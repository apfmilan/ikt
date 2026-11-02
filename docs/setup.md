# Setup and onboarding

Deployment of the VPS stack is in `docs/deployment.md`. This page covers what you do once,
after the stack is running, to let the system trade autonomously.

## 1. First passkey (iPhone, Face ID)
1. On the VPS: `docker compose run --rm migrate tc setup-token` (or `docker compose exec api tc setup-token`
   with `TC_RP_ORIGIN=https://<domain>`). It prints `https://<domain>/#token=…` (valid 30 min, single use).
2. Open the link in Safari on the iPhone → **Első beállítás** → Face ID creates the passkey.
3. Save the 10 recovery codes (shown once).
4. Safari → Share → **Add to Home Screen**. Open the app from the home screen (required for push).
5. Windows: open the desktop app (or the site in Edge) → **Új eszköz párosítása** with a pairing code
   created on the iPhone (Settings → Devices).

## 2. Broker API keys (never enable withdrawals)
**Trading 212** (Invest or Stocks ISA account; app → Settings → API):
- Grant only: account, portfolio, orders (read + execute), metadata, history. Restrict the key to the
  VPS IP address. The API cannot withdraw money.
**Bitvavo** (Settings → API):
- Enable **Read-only** and **Trade digital assets** only. **Do not enable "Withdraw digital assets"**
  (Bitvavo API withdrawals need no 2FA). Add the VPS IP to the whitelist.
- Choose an `operatorId` (any positive integer, e.g. 1001); it is sent with every order.
**EODHD** (stock/ETF prices — Trading 212 provides none): free key (20 calls/day) covers the ETF
plan and the 10-stock daily scan; paid plans add capacity.

Enter keys in **Onboarding**. They are sealed in your browser session's request by the API with the
engine's public key and can only be opened by the engine. The engine then verifies authentication,
permissions and balances and shows the result. If the withdrawal check is inconclusive you must
confirm explicitly that withdrawal permission is disabled.

## 3. Risk ceilings and plan
- Review the conservative defaults (1 % risk/trade, 3 % daily loss, 10 % drawdown halt, max 3
  positions …). You can tighten freely; loosening takes effect after 24 h and revokes live
  authorization. Hard limits cannot be exceeded.
- Financial plan defaults: Nov 2026 25 000 Ft; Dec 2026–Apr 2027 +50 000 Ft/month; from May 2027
  100 000 Ft/month split between trading and long-term investing (default 50/50). For the ETF core
  pick Trading 212 instruments and their EODHD symbols (e.g. an S&P 500 UCITS ETF). Profit split
  reinvest/withdraw is a report only — withdrawals are always manual.

## 4. Authorize live trading
- **Élesítés** → Face ID step-up → per-broker authorization (valid 30 days, bound to the current
  risk configuration).
- The engine container must also run with `TC_LIVE_TRADING=enabled` (set in `.env`). Without it the
  engine never sends orders.
- Strategies trade only after passing walk-forward validation (`tc backtest --promote`); until then
  the system abstains and records why.

## 5. Daily operation
Nothing is required. Push notifications: trades, errors, critical risk, optional market open, and
the daily summary `💰 Mai eredmény: +1 000 Ft` (actual net P&L, deposits excluded). Emergency stop is
always in the top bar; releasing it needs Face ID.

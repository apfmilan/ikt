# Trading Center UI — PWA (iPhone) and Windows desktop

## Stack

| Part | Choice | Version |
|---|---|---|
| Framework | Next.js App Router, **static export** (`output: "export"`) | 16.4.2 |
| UI runtime | React | 19.3.1 |
| Language | TypeScript, `strict` + `noUncheckedIndexedAccess` | 6.0.3 |
| Styling | Tailwind CSS v4 (CSS-first `@theme` tokens) | 4.3.3 |
| Components | shadcn/ui-style primitives vendored in `apps/web/components/ui` (cva + clsx + tailwind-merge) | — |
| Icons | lucide-react | 1.59.0 |
| Charts | lightweight-charts (lazy-loaded in effects) | 5.2.1 |
| Lint | ESLint 10 + `eslint-config-next` (core-web-vitals + typescript) | 10.13.0 / 16.4.2 |
| Desktop | Tauri 2 (`apps/desktop/src-tauri`, own Cargo workspace) | tauri 2.12.4 |

All versions were at least two weeks old when pinned (2026-11-02). WebAuthn uses the native
`navigator.credentials` API with ~60 lines of base64url helpers (`lib/webauthn.ts`) instead of
a dependency.

There is **no mock, sample or demo data in the app bundle**. Every screen renders loading,
empty and error states only, and shows nothing but API responses.

## Design system

- Pure black background (`#000`). Surfaces are `#0a0a0a`, `#111` and `#171717`, with hairline
  borders (`#1f1f1f`).
- Text is `#fafafa`, muted text `#8a8a8a`, subtle text `#5c5c5c`.
- Gains are `#22c55e` and losses `#ef4444`. The accent `#a3e635` is used sparingly.
  Forecast blue `#7dd3fc` marks only estimates.
- Numbers use tabular numerals (the `.num` class). HUF goes through `Intl` `hu-HU` with
  `useGrouping: "always"`, so values read `+1 000 Ft`. Hungarian CLDR would otherwise drop the
  separator for 4-digit amounts. The minus sign is `−`.
- **Realized vs forecast:** each figure carries a `Tényleges` (realized) or `Előrejelzés`
  (forecast) badge. Forecast blocks have their own blue frame and the note
  "Becslés, nem garantált eredmény." No single figure mixes the two.
- Mobile first. A bottom tab bar holds 4 tabs: Áttekintés, Pozíciók, Botok, Napló. A "Menü"
  sheet holds the remaining screens. From `lg` up there is a sidebar.
- Safe-area insets (`viewport-fit=cover`) are applied. Touch targets are at least 44 px.
- Motion is limited to 150–260 ms fades and sheets, and is disabled under
  `prefers-reduced-motion`.
- System font stack (SF Pro on iPhone, Segoe UI Variable on Windows), so no web-font download
  is needed.

## Screens (`apps/web/app`)

| Route | Content |
|---|---|
| `/login/` | Passkey login (Face ID / Windows Hello) and recovery-code flow that registers a new passkey |
| `/setup/` | First passkey via one-time setup token read from the URL **fragment** `#token=…` (never sent in a URL; posted once in a JSON body, fragment removed after use). Also device pairing (`#pair=…` or typed code). Shows recovery codes once. |
| `/` | Overview: total equity, today's and total net P&L (excluding deposits), net deposits, drawdown, realized equity curve with the deposits line, three bot cards (crypto / stocks / invest), broker cards with health and live state, latest decisions |
| `/positions/` | Trading and long-term positions with stop/target, open orders, one-tap **Sell** → confirm sheet (full or partial) → passkey step-up → `POST /positions/sell` |
| `/instrument/?id=…` | Candles + volume, entry/exit/stop markers, avg-price/stop/target lines, position, related decisions |
| `/bots/` | Three bot cards and every runtime agent's dashboard (status, summary, metrics, last run) grouped as analysis / decision / execution |
| `/timeline/` | Decision log with filters (all, actions, vetoes). Each entry shows the forecast (p_win, net bps, cost, uncertainty, 5/95 %), agent consensus bars, the risk verdict with veto codes, and a chart link. |
| `/trades/` | Closed trades (realized net, fees, entry-time forecast shown separately) and the full ledger (deposits, withdrawals, fees, dividends, FX) |
| `/performance/` | Return, win rate, profit factor, max drawdown, fees, expectancy, daily P&L bars, strategy comparison including backtest OOS |
| `/strategies/` | Registry: status (disabled / validated / live half-risk / live / demoted), OOS backtest metrics, live stats |
| `/plan/` | Financial plan progress, monthly schedule (planned vs deposited), editable plan + reinvest/withdraw split (step-up). Withdrawals are manual only. |
| `/risk/` | Utilization, ceilings with the hard maximum shown, validation against hard limits, pending loosening (24 h). Saving needs step-up. |
| `/onboarding/` | Wizard: passkey, Trading 212, Bitvavo (operator id, "never enable Withdraw", attestation fallback), EODHD, risk ceilings, plan, explicit live authorization (acknowledgement + Face ID) |
| `/settings/` | Web Push enable/disable (permission requested only on tap; iPhone install hint), notification toggles and daily summary time, passkey devices (pair new, remove with step-up), logout |

The emergency **Stop** button is in the top bar on every screen. Its modes are
`reduce_only`, `flatten` and `freeze`, and each needs only the session. Returning to automatic
trading (`normal`) needs a passkey step-up. A mode banner appears whenever trading is not fully
live.

Live data comes from `lib/live.tsx`, a cookie-authenticated WebSocket `/api/ws`. It reconnects
with exponential backoff and jitter, and pings every 25 s. Each reconnect bumps an epoch, which
makes every `useResource` refetch. Resources also reload, debounced, on matching event types.

## PWA

- `public/manifest.webmanifest`: `display: standalone`, black theme/background, PNG icons
  (192, 512, maskable 512), apple-touch-icon 180, and an SVG icon. The PNGs are generated from
  `public/icons/icon.svg` by `node scripts/preview/make-icons.mjs`.
- `public/sw.js`:
  - Caches only the app shell: `/_next/static/*`, icons, and navigations for an offline shell.
  - **Never caches `/api/*`.**
  - Handles `push`: shows the notification with title, body, tag and url; `critical` severity
    stays on screen.
  - Handles `notificationclick`: focuses or opens the app.
- **CSP:** `npm run build` runs `scripts/csp-hashes.mjs`. It writes `out/_csp/header.txt`, a
  complete `Content-Security-Policy` built from SHA-256 hashes of every inline script, with no
  `'unsafe-inline'` for scripts. The deploy step must send it as the `Content-Security-Policy` header for the static files,
  for example by templating the file contents into the Caddyfile. The hashes change with every
  build, so regenerate on each deploy.

### Install on iPhone
1. Open `https://<your-domain>/` in **Safari**.
2. Tap Share, then **Add to Home Screen**, then Add.
3. Open the app from the Home Screen icon. It runs standalone with its own cookie jar,
   separate from Safari, so you log in once inside the app.
4. To get notifications, go to Beállítások → Push értesítések → **Bekapcsolás**. Web Push needs
   iOS 16.4+ and works only from the Home Screen app.

## Windows desktop (`apps/desktop`)

- Tauri 2 shell. It loads the remote HTTPS origin fixed at build time (`TC_ORIGIN`, default
  `https://trading.example.com`). There is **no bundled UI**: passkeys are bound to the real
  origin, and a local `tauri.localhost` UI could not use them.
- **No IPC is exposed to the page.** No commands are registered. `capabilities/default.json`
  grants no permissions and lists no remote URLs, and `withGlobalTauri` is false. Navigation is
  pinned to the configured origin (`on_navigation`); other origins are blocked.
- Single instance: a second launch focuses the window. The tray icon has Megnyitás / Kilépés
  and opens the window on left click.
- `fallback/index.html` is the only local asset (shown if the window cannot load the origin).
- Build: `cd apps/desktop && npm ci && TC_ORIGIN=https://<domain> npx tauri build`. The output
  is NSIS + MSI and must be built on Windows; CI uses the `windows-latest` job. `cargo test` and
  `cargo clippy` pass on Linux in this repository's environment.
- Windows Hello inside WebView2 is expected to work for WebAuthn but **has not been verified on
  real Windows 11 hardware**. Fallback: install the PWA from Edge (⋯ → Apps → Install this site
  as an app).

## Three-bot status widget

- **iPhone:** not possible from a PWA. Home Screen widgets need a native app with a WidgetKit
  extension built in Xcode on macOS, plus an Apple Developer account. This environment has
  neither, so the widget is **blocked**, not implemented.
- **Windows:** not implemented. Showing live bot statuses in the tray would need either IPC from
  the remote page (deliberately disabled) or separate authenticated access from the Rust shell
  (a second credential path). The tray offers open/quit only. The three bot cards are on the
  Overview and Botok screens.

## Design preview

`docs/preview/ui-preview.png` is the single preview image: the real `apps/web` build at iPhone
15 Pro size (393×852 @3x) and desktop size (1440×900 @2x), composited. The `/api/*` responses
come from `scripts/preview/fixtures.json`, which holds **illustrative values only**. That file
is used only by the preview script and is never bundled; the image caption says so.
Regenerate with:

```bash
(cd apps/web && npm ci && npm run build) && node scripts/preview/screenshot.mjs
```

## Development

```bash
cd apps/web
npm ci
TC_API_URL=http://127.0.0.1:8080 npm run dev   # proxies /api to tc-server --role all
npm run typecheck && npm run lint && npm run build   # build → out/ (+ out/_csp/header.txt)
```

## Limitations

- Passkey ceremonies assume webauthn-rs JSON (`{ publicKey: … }`, base64url). They have not
  been tested end-to-end against the Rust API from this fork.
- Push delivery depends on the backend's VAPID implementation; on iPhone it works only from the
  Home Screen app.
- Home Screen web apps in the EU (Hungary) should be confirmed on a real device.

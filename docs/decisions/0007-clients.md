# 0007. Clients: installable PWA, Tauri 2 shell loading the remote origin

Status: Accepted · Date: 2026-11-02

## Context
iPhone (Face ID passkeys, push) and Windows desktop. No macOS/Xcode in the build environment.

## Options
- **A. PWA + Tauri shell loading the HTTPS origin (chosen)** — one UI, WebAuthn bound to the real RP ID.
- **B. Tauri with bundled UI** — passkeys would be bound to `tauri.localhost`, not the RP ID.
- **C. Native iOS app** — needs macOS + Xcode + Apple Developer account (not available).

## Scores
| Option | Security | Correctness | Performance | Maintainability | UX | Cost | Total |
|---|---|---|---|---|---|---|---|
| A | 8 | 8 | 8 | 9 | 8 | 9 | 50 |
| B | 6 | 5 | 8 | 7 | 7 | 9 | 42 |
| C | 8 | 8 | 9 | 4 | 9 | 3 | 41 |

## Decision
Option A. Next.js static export served by Caddy; service worker caches only the app shell; Web
Push from the engine (VAPID). Tauri 2 exposes no IPC commands to the remote page; navigation is
pinned to the configured origin.

## Consequences / blockers
- iPhone home-screen widget (three-bot status): **blocked** — requires a native WidgetKit app.
- Windows tray bot status: not implemented (would require IPC from the remote page).
- Windows installer is built by `.github/workflows/desktop-windows.yml`; WebView2 + Windows Hello
  passkeys must be verified on real hardware.

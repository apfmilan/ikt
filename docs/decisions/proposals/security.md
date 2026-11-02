# Security Proposal: Trading Center V2

Role: Security Engineer · 2026-11-02 · Status: proposal

**Premise.** VPS root can do anything the bot can. Only broker-side controls survive that: no withdrawal permission, and broker IP allowlists pinned to the VPS IP. Everything else reduces the odds or the damage.

## 1. Threat model

| Threat (STRIDE) | Controls | Residual |
|---|---|---|
| VPS compromise (E,I) | Key-only SSH, patched host, hardened containers, no-withdraw keys, broker IP allowlist | Bad trades. Keep balances proportionate. |
| Stolen phone (S) | Face ID passkey, timeouts, revoke from Windows, step-up | Thief knows passcode. Use Stolen Device Protection. |
| XSS (T,E) | React escaping, nonce CSP, no third-party JS, HttpOnly cookie | Can raise Face ID prompt. Each step-up action pushes a notice. |
| CSRF (T) | SameSite=Strict, Origin check, JSON-only | Low |
| Session theft (S) | `__Host-` cookie, hashed at rest, rotated at login | Client malware |
| Malicious dependency (E,I) | Lockfiles, cargo-deny, pnpm release delay, pinned digests | Backend crate reads memory |
| Log leakage (I) | Redaction types, field denylist, no tokens in URLs | Low |
| DB dump / backup leak (I) | Envelope encryption, hashed tokens, age-encrypted backups | Trade history |
| WebSocket hijack/replay (S,T) | Origin + session at upgrade, server-to-client only | Low |
| Bitvavo request replay (T) | `Bitvavo-Access-Window: 5000`, chrony | 5 s window, needs TLS break |
| SSRF (I,E) | Egress allowlist, Web Push endpoint validation | Low |
| Auth brute force (D) | Backoff, no hard lockout, alerts | Volumetric DoS |
| Repudiation (R) | Hash-chained audit log, INSERT-only grant | Root rewrite; daily push carries chain head |

## 2. Broker credentials

- **Envelope encryption.** KEK: 32 bytes in Docker secret `/run/secrets/ikt_kek_v{n}` (0400, app UID), never an env var. Startup fails if it is missing, the wrong length or loosely permissioned. Each credential gets a random DEK. XChaCha20-Poly1305 (random 24-byte nonce) seals the secret under the DEK and the DEK under the KEK. AAD `ikt:v1|{user_id}|{broker}|{credential_id}|{kek_version}`: a ciphertext moved to another row fails. Secret fields are write-only; the UI shows a 4-character fingerprint.
- **Memory.** `secrecy`/`zeroize` types with no `Debug`/`Serialize`/`Clone`. Core dumps off (`RLIMIT_CORE=0`, `PR_SET_DUMPABLE=0`). No unencrypted swap.
- **Never logged.** `Redacted<T>` prints `[REDACTED]`. A tracing filter drops `authorization|secret|signature|cookie|token|key` fields. Headers are never logged. A CI canary test fails if fake secrets reach logs, panics or responses.
- **Rotation.** Broker key, every 90 days: new key at the broker, submit with step-up, server validates and swaps atomically, user deletes the old key. KEK: mount v2, run `ikt-admin rewrap --to 2`, verify, remove v1. Keep KEKs in an offline password manager.
- **Permissions.** Bitvavo has no introspection endpoint. Grant only *View access* + *Trade digital assets*, whitelist the VPS IP, enable the withdrawal address whitelist. On submit and daily, probe `POST /v2/withdrawal` with `amount "0"` and an invalid address. Accept only errorCode 312 (no withdraw permission); otherwise reject the key, pause trading, alert. The probe fails closed and moves nothing. T212 v0 has no withdrawal endpoint, and its scopes are unreadable. The UI tells the user to grant only account, portfolio, orders, history and metadata (not pies) and to IP-restrict the key.

## 3. Authentication (webauthn-rs)

- **RP.** `rp_id` = exact host, origin `https://<host>`. Passkey registration with UV required, discoverable preferred, attestation `none`. Login uses `allowCredentials` (single user, no username). Ceremony state is server-side, single-use, 120 s TTL. Counter 0 (synced passkeys) is not a cloning signal.
- **First passkey.** Valid only while zero passkeys exist. Get the token from `docker compose exec api ikt-admin setup-token`; fallback is a one-time stdout print at first boot. 256-bit, stored as SHA-256, single-use, 15 min TTL, 5 tries/IP/hour. URL `https://<host>/setup#<token>`: the fragment never reaches logs, and the page POSTs it. Afterwards `/api/setup/*` returns 404 permanently and recovery codes are shown once.
- **Second device.** The phone (with step-up) issues a pairing code: 50-bit, 10 min, single-use, 3 tries. The desktop redeems it for an enrolment-only session and registers Windows Hello. Every new passkey pushes an alert.
- **Recovery.** 10 offline 80-bit codes, Argon2id-hashed, single-use. Use revokes all sessions, grants enrolment only, puts the risk engine in safe mode (no new entries) and alerts. Last resort: `ikt-admin reset-passkeys` on the VPS.
- **Sessions.** `__Host-ikt_sid; HttpOnly; Secure; SameSite=Strict; Path=/`. 256-bit CSPRNG token, stored as SHA-256 with device/IP. Idle 60 min (user requests only, not WS pings), absolute 12 h. Session list with revoke-all.
- **Step-up.** Required for: enabling live trading, risk-limit changes, manual sell, disabling the emergency stop, credential/passkey changes, profit-split changes. `POST /api/stepup/begin {action, params_hash}` returns a challenge bound to that action; the action request carries the assertion (UV, single-use, ≤120 s). No elevation window. *Enabling* the emergency stop needs only a session.

## 4. Web surface

- **CORS:** UI and `/api` share one origin via Caddy. No `Access-Control-*` headers.
- **CSRF:** SameSite=Strict, plus exact `Origin` match on every non-GET (missing = reject), plus required `application/json`. GET never mutates.
- **WebSocket:** `wss://<host>/api/ws`. Upgrade requires exact Origin (blocks cross-site hijacking) and a valid session. Re-checked every 60 s and on revocation; close code 4401. Server-to-client only, client frames dropped, so replay does nothing. 64 KiB frames, 5 sockets/session.
- **CSP** (per-request nonce, Next.js middleware): `default-src 'self'; script-src 'self' 'nonce-{N}' 'strict-dynamic'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' wss://<host>; object-src 'none'; base-uri 'none'; frame-ancestors 'none'`. Plus HSTS preload, `nosniff`, `Referrer-Policy: no-referrer`, `COOP: same-origin`. Self-hosted fonts.
- **Rate limits** (`tower_governor`, client IP from Caddy only): ceremony begin 10/min/IP. Per-IP backoff on failures, 1 s up to 15 min. Above 50 failures/hour globally: delay and alert. No hard lockout, so attackers cannot lock the owner out. Setup, recovery, pairing: 5/hour.

## 5. Clients

- **Tauri 2.** Load the remote origin (`WebviewUrl::External`). A bundled UI at `http://tauri.localhost` cannot use the RP ID. WebView2 hands WebAuthn to Windows Hello; verify on real Windows 11 hardware, fallback Edge "Install as app". No capability lists the remote URL, so zero IPC. `withGlobalTauri: false`, navigation locked, signed installer and updater.
- **iOS PWA.** Web Push needs a Home Screen install on iOS 16.4+ and a permission request from a user tap. Lock screen shows payloads: offer "hide amounts", never send tokens. Face ID passkeys work in standalone mode. The PWA's cookie jar is separate from Safari's, so sign in inside the PWA. iCloud Keychain sync makes the Apple ID a passkey: require 2FA on it.

## 6. Infrastructure

- Only Caddy publishes ports (80, 443 tcp/udp) with auto-HTTPS; `log_credentials` stays off.
- Firewall allows 80/443. SSH key-only, no root, source-restricted. Docker-published ports bypass ufw, so no other service has `ports:`. MySQL and the API sit on `internal: true` networks.
- MySQL 8.4: app user gets DML on `ikt_*` only and INSERT/SELECT on `ikt_audit_log`. Separate migration user. `local_infile=OFF`.
- Containers: non-root, `read_only: true`, tmpfs `/tmp`, `cap_drop: [ALL]`, `no-new-privileges`, resource limits. Distroless API image.
- Egress (DOCKER-USER nftables): broker, ECB/MNB, market-data and push hosts only. Push endpoints must be https on `*.push.apple.com`, `fcm.googleapis.com` or `*.notify.windows.com`, with public IPs.
- Backups: nightly dump, `age`-encrypted to an offline key, off-site. The VPS cannot decrypt them. Restore drill monthly.
- Supply chain: committed lockfiles, `--frozen-lockfile`. CI runs `cargo deny`, `cargo audit`, `pnpm audit --prod`. pnpm `minimumReleaseAge: 10080`, `onlyBuiltDependencies` allowlist. Images pinned `@sha256:`, Actions pinned by SHA.

## 7. Requirements checklist

P0 = before broker keys entered; P1 = before live trading; P2 = within 30 days.

| ID | P | Requirement | Verify |
|---|---|---|---|
| SEC-01 | P0 | Envelope encryption, AAD binding, KEK only from secret file | AAD-swap test; DB plaintext scan |
| SEC-02 | P0 | No secret, token or signature in logs, panics, responses | CI canary test |
| SEC-03 | P0 | Bitvavo key accepted only on probe 312; daily re-probe | Mock + live check |
| SEC-04 | P0 | Setup token: zero passkeys, hashed, 15 min, single-use; 404 after | Integration test |
| SEC-05 | P0 | Exact RP ID/origin, UV required, single-use 120 s challenges | Replay/expiry/origin tests |
| SEC-06 | P0 | `__Host-` HttpOnly Secure Strict cookie, hashed, 60 min/12 h | Header + clock test |
| SEC-07 | P0 | Action-bound single-use step-up on listed actions | Reused assertion returns 403 |
| SEC-08 | P0 | Non-GET rejects missing/foreign Origin and non-JSON | curl suite |
| SEC-09 | P0 | WebSocket Origin + session check; revocation closes within 60 s | Integration test |
| SEC-10 | P0 | CSP and headers per §4 | Header test |
| SEC-11 | P0 | Only Caddy exposed; MySQL unreachable externally | External nmap |
| SEC-12 | P1 | Containers non-root, read-only, no capabilities | `docker inspect` script |
| SEC-13 | P1 | Auth rate limits with backoff, no hard lockout | Load test |
| SEC-14 | P1 | Recovery codes hashed, single-use; use revokes sessions, enters safe mode | Integration test |
| SEC-15 | P1 | Hash-chained audit log; grants block UPDATE/DELETE | UPDATE as app user fails |
| SEC-16 | P1 | Egress allowlist; push endpoint validation | SSRF test (169.254.169.254) |
| SEC-17 | P1 | Tauri remote origin, zero remote IPC, signed | `window.__TAURI__` undefined |
| SEC-18 | P1 | Supply-chain CI gates; pinned digests and SHAs | CI review |
| SEC-19 | P1 | age-encrypted off-site backups, offline keys | Restore drill log |
| SEC-20 | P2 | Zeroizing secret types; core dumps off | Code review; `/proc/<pid>/limits` |

# 0004. Security model: sealed credentials, passkeys, step-up

Status: Accepted · Date: 2026-11-02

## Context
`proposals/security.md`. Single user, live broker keys, internet-facing PWA. No email, no Telegram,
no third-party auth.

## Options
- **A. Symmetric master key shared by API and engine** — API compromise reveals keys.
- **B. Sealed box: API holds only the engine's X25519 public key (chosen)**.
- **C. External KMS/HSM** — extra service/cost; not available on a small VPS.

## Scores
| Option | Security | Correctness | Performance | Maintainability | UX | Cost | Total |
|---|---|---|---|---|---|---|---|
| A | 5 | 8 | 9 | 8 | 8 | 9 | 47 |
| B | 8 | 8 | 9 | 8 | 8 | 9 | 50 |
| C | 9 | 8 | 7 | 5 | 8 | 4 | 41 |

## Decision
- `tc-vault`: ephemeral X25519 ECDH + HKDF-SHA256 + AES-256-GCM, AAD = `broker|field|kv`.
  Only the engine container mounts `engine_secret_key`.
- WebAuthn passkeys (webauthn-rs, user verification required). First passkey via single-use
  30-min token from `tc setup-token` delivered in the URL **fragment**; more devices via 10-min
  pairing code; 10 single-use recovery codes (~100 bits, SHA-256 at rest).
- Sessions: 256-bit random token, SHA-256 at rest, `__Host-` HttpOnly Secure SameSite=Strict,
  60 min idle / 12 h absolute.
- Step-up (fresh assertion → 120 s single-use token bound to one action) for live enable, risk
  change, manual sell, kill release, credential change, device removal, plan change.
- CSRF: Origin must equal the configured origin on every state-changing request and WS upgrade.
- Live order sending additionally requires `TC_LIVE_TRADING=enabled` on the engine container
  (`tc-broker::live` guard; test builds can never arm it).

## Dissent
A compromised API process can still enqueue commands after a valid step-up; full engine-side
assertion verification is deferred (docs/STATUS.md).

## Consequences
Verified: vault unit tests, MySQL token/session tests, and a browser E2E with a virtual
authenticator (setup, login, step-up binding, single use, kill-switch rules: 25/25 checks).

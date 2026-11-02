# Deployment (VPS, Docker Compose, HTTPS)

Architecture: ADR-0001. One image (`server`) runs two containers (`api`, `engine`), plus
`mysql`, a one-shot `migrate`, and `caddy` (static PWA + `/api` reverse proxy + automatic HTTPS).

```
internet ──443/80──> caddy ──edge(internal)──> api ──backend(internal)──> mysql
                                                engine ──backend──> mysql
                                                engine ──egress──> brokers / ECB / market data / push
```

Only `caddy` publishes ports. The `api` has no internet egress (both its networks are
`internal`). The `engine` has no inbound port; its health endpoint listens on
`127.0.0.1:9090` inside its own container.

## 1. Requirements

- VPS: 2 vCPU, 2 GB RAM (stack limits ≈ 1.6 GB), 20 GB SSD, Ubuntu 22.04/24.04 or Debian 12, x86_64.
- A domain with an `A` (and optionally `AAAA`) record pointing to the VPS. The domain is also
  the WebAuthn relying-party ID: **changing it later invalidates all registered passkeys.**
- Outbound HTTPS from the VPS to `live.trading212.com`, `api.bitvavo.com`, `www.ecb.europa.eu`,
  the market-data provider, and Web Push services (`*.push.apple.com`, `fcm.googleapis.com`,
  `*.notify.windows.com`).
- An offline computer to hold the backup decryption key (`age`).

## 2. First deployment

Run as root on the VPS unless noted.

1. Clone the repository (as a normal user, e.g. `/opt/trading-center`) and enter it.
2. Prepare the host: `sudo deploy/scripts/bootstrap-vps.sh`
   (Docker from Docker's official apt repo, ufw 22/80/443, unattended upgrades, `age`, backup timer).
3. Edit `.env` (created from `.env.example`): set `TC_DOMAIN` and `TC_ACME_EMAIL`.
   Leave `TC_LIVE_TRADING` **empty**.
4. Backup key — on your **own computer**, not the VPS:
   `age-keygen -o tc-backup.key` → store `tc-backup.key` offline (password manager / USB).
   Copy only the public `age1…` line to `/etc/trading-center/backup-recipient.txt` on the VPS.
5. Build images: `docker compose build`
6. Generate secrets: `sudo deploy/scripts/generate-secrets.sh`
   (MySQL passwords, database URLs, engine X25519 key pair and VAPID key pair via `tc keygen`;
   files in `deploy/secrets/`, mode 0400). Back up `deploy/secrets/engine_secret_key` offline
   next to the backup key — without it, stored broker credentials cannot be decrypted.
7. Start: `docker compose up -d` and wait for `docker compose ps` to show all services
   `healthy` (`migrate` shows `exited (0)`). Caddy obtains the TLS certificate automatically.
8. First passkey: `make setup-token` prints a one-time link `https://<domain>/#token=…`
   (single use, short expiry; the token is in the URL fragment so it never reaches server logs).
   Open it **on the iPhone**, add the passkey with Face ID, store the recovery codes offline.
   Then "Add to Home Screen" from Safari to install the PWA (needed for Web Push on iOS ≥ 16.4).
9. Onboarding in the app: enter Trading 212 and Bitvavo keys (trade + read permissions only,
   **never withdrawal**; restrict each key to the VPS IP in the broker's settings), market-data
   key, risk ceilings, financial plan. The engine verifies permissions, connections and balances.
10. Live trading needs both:
    - passkey-signed authorization in onboarding (expires after 30 days), and
    - `TC_LIVE_TRADING=enabled` in `.env`, then `docker compose up -d engine`.
    Without the env flag the engine never sends orders to live hosts, whatever the database says.

## 3. Upgrades

```bash
git pull
docker compose build            # or pull tagged images from your registry
make backup                     # always back up before migrations
docker compose up -d            # migrate runs first; api/engine start only if it succeeds
docker compose ps
```
Migrations are additive only (see `.claude/skills/db-migration/SKILL.md`). On restart the engine
boots in `REDUCE_ONLY` and resumes the prior mode only after a clean broker reconciliation.

### Rollback
`git checkout <previous-tag> && docker compose build && docker compose up -d`. Because migrations
are additive, the previous binary keeps working on the newer schema. If a migration itself is
faulty, restore the pre-upgrade backup (§4).

## 4. Backups and restore drill

- `tc-backup.timer` runs `deploy/scripts/backup.sh` nightly at 02:30 Europe/Budapest:
  `mysqldump --single-transaction` → gzip → `age` encrypted to the offline key → 14-day retention
  in `/var/backups/trading-center`. Optional off-site copy: set `TC_BACKUP_OFFSITE_CMD` in
  `/etc/trading-center/backup.env` to a script that receives the file path.
- Manual backup: `make backup`. Status: `systemctl list-timers tc-backup.timer`, `journalctl -u tc-backup`.
- **Monthly restore drill** (on a scratch VPS or locally):
  `deploy/scripts/restore.sh <file.sql.gz.age> tc-backup.key` → it stops `engine` and `api`,
  asks you to type `RESTORE`, replaces the database, and leaves the engine stopped for checks.
  Delete the key file from the machine afterwards.

## 5. Monitoring and automatic recovery

| What | How |
|---|---|
| Service state | `docker compose ps` (healthchecks: api `/api/health`, engine `127.0.0.1:9090/health`, mysql ping, caddy admin) |
| Logs | `docker compose logs -f api engine` (JSON lines, 10 MB × 5 rotation per service) |
| App health | `curl -s https://<domain>/api/health` → `status`, `db`, `engine_heartbeat_age_s` |
| Alerts | Push notifications from the engine (errors, critical risk events, reconciliation mismatches) |

Recovery behaviour:
- All long-running services use `restart: unless-stopped`; Docker restarts crashed containers
  and the stack comes back after a host reboot (Docker is enabled at boot).
- `api`/`engine` only start after `migrate` succeeded; `caddy` only after `api` is healthy.
- The engine starts fail-closed (`REDUCE_ONLY`) and reconciles positions, cash and open orders
  with the brokers before resuming entries. Orders whose outcome is unknown are never re-sent
  blindly (ADR-0001/0002).
- Caddy marks the API unhealthy within ~15 s and returns 503 instead of hanging requests.

## 6. Security notes

- **Firewall:** ports published by Docker bypass ufw. Only `caddy` has `ports:`; never add
  `ports:` to `mysql`, `api` or `engine`.
- Containers run non-root (uid 10001) with read-only root filesystems, `cap_drop: ALL`,
  `no-new-privileges`, memory/PID limits. MySQL keeps only the capabilities its entrypoint needs.
- MySQL users: `tc_app` (DML only, used by api/engine), `tc_migrate` (DDL, used by `migrate`).
  `local_infile` is off, strict SQL mode, UTC.
- Secrets are Docker secret files under `deploy/secrets/` (git-ignored, 0400), not environment
  variables. Broker keys live encrypted in MySQL, sealed to the engine's public key.
- Headers (Caddy): HSTS (2 years, preload), CSP, `X-Frame-Options: DENY`, `frame-ancestors 'none'`,
  `Referrer-Policy: no-referrer`, restrictive `Permissions-Policy` (passkeys allowed for self).
- **Known gap — CSP for scripts:** the image uses script hashes when the web build emits
  `apps/web/out/csp-hashes.txt` (one `'sha256-…'` source per line). If that file is absent the
  image falls back to `script-src 'self' 'unsafe-inline'`. Check with
  `curl -sI https://<domain>/ | grep -i content-security-policy`.
- **Known gap — image pinning:** base images are pinned by major tag (`rust:1-bookworm`,
  `gcr.io/distroless/cc-debian12`, `node:22-alpine`, `caddy:2-alpine`, `mysql:8.4`). For
  reproducible production builds, pin them by digest (`image@sha256:…`) and update via PRs.
- **Known gap — egress allow-list:** the engine network allows all outbound traffic. Restricting
  it to broker/data/push hosts needs host nftables rules in the `DOCKER-USER` chain (not automated).

## 7. CI/CD (GitHub Actions)

| Workflow | Purpose | Needs |
|---|---|---|
| `ci.yml` | rustfmt, clippy `-D warnings`, tests, MySQL integration tests, cargo audit, web build + npm audit, compose validation | nothing |
| `docker.yml` | builds `server` and `web` images; pushes only if configured | variable `TC_REGISTRY`, secrets `REGISTRY_USERNAME`, `REGISTRY_PASSWORD` |
| `desktop-windows.yml` | Tauri 2 Windows bundle (MSI/NSIS) as an artifact | variable `TC_ORIGIN` (`https://<domain>`) |
| `claude-review.yml` | automated PR review with the project's reviewer agents | secret `ANTHROPIC_API_KEY` |

CI never sets `TC_LIVE_TRADING`, so live order sending cannot be armed in tests.

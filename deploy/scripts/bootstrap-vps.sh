#!/usr/bin/env bash
# One-time VPS preparation (Ubuntu 22.04/24.04 or Debian 12). Run as root from the repo root:
#   sudo deploy/scripts/bootstrap-vps.sh
# Installs Docker Engine from Docker's official apt repository
# (https://docs.docker.com/engine/install/ubuntu/ and /debian/), enables ufw (22, 80, 443),
# unattended security upgrades, age for encrypted backups, and the backup timer.
set -euo pipefail
[[ $(id -u) == 0 ]] || { echo "run as root" >&2; exit 1; }
cd "$(dirname "$0")/../.."
REPO_DIR="$PWD"
. /etc/os-release
case "$ID" in ubuntu|debian) ;; *) echo "unsupported OS $ID" >&2; exit 1;; esac

apt-get update
apt-get install -y ca-certificates curl gnupg ufw unattended-upgrades age openssl

# Docker Engine (official repository).
if ! command -v docker >/dev/null; then
  install -m 0755 -d /etc/apt/keyrings
  curl -fsSL "https://download.docker.com/linux/$ID/gpg" -o /etc/apt/keyrings/docker.asc
  chmod a+r /etc/apt/keyrings/docker.asc
  echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/$ID ${VERSION_CODENAME} stable" \
    > /etc/apt/sources.list.d/docker.list
  apt-get update
  apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
fi
systemctl enable --now docker

# Unattended security upgrades.
dpkg-reconfigure -f noninteractive unattended-upgrades

# Firewall. NOTE: ports published by Docker bypass ufw; only the caddy service publishes ports.
ufw default deny incoming
ufw default allow outgoing
ufw allow 22/tcp
ufw allow 80/tcp
ufw allow 443/tcp
ufw allow 443/udp
ufw --force enable

# SSH hardening reminder (not automated to avoid locking you out).
echo "Reminder: use key-only SSH (PasswordAuthentication no, PermitRootLogin no) and restrict source IPs."

# Configuration and backups.
install -d -m 0750 /etc/trading-center /var/backups/trading-center
[[ -f .env ]] || { cp .env.example .env; echo "created .env from .env.example — edit TC_DOMAIN and TC_ACME_EMAIL"; }
if [[ ! -s /etc/trading-center/backup-recipient.txt ]]; then
  echo "Generate the backup key OFFLINE (on your own computer): age-keygen -o tc-backup.key"
  echo "Then put its public 'age1...' line into /etc/trading-center/backup-recipient.txt"
fi

# Backup timer (02:30 Europe/Budapest via systemd OnCalendar with timezone).
sed "s#@REPO_DIR@#$REPO_DIR#g" deploy/systemd/tc-backup.service > /etc/systemd/system/tc-backup.service
cp deploy/systemd/tc-backup.timer /etc/systemd/system/tc-backup.timer
systemctl daemon-reload
systemctl enable --now tc-backup.timer

echo "Next: edit .env, then: docker compose build && deploy/scripts/generate-secrets.sh && docker compose up -d"

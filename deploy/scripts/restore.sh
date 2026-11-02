#!/usr/bin/env bash
# Restore an encrypted backup. DESTRUCTIVE: replaces the trading_center database.
# Usage: deploy/scripts/restore.sh <backup.sql.gz.age> <age-identity-file>
# The identity file is the OFFLINE private key; copy it to the VPS only for the restore and
# delete it afterwards. Trading is stopped (engine + api) before restoring.
set -euo pipefail
cd "$(dirname "$0")/../.."
backup="${1:?backup file}"; identity="${2:?age identity file}"
[[ -s "$backup" && -s "$identity" ]] || { echo "missing file" >&2; exit 1; }

echo "This will STOP trading and REPLACE database trading_center with: $backup"
read -r -p "Type RESTORE to continue: " answer
[[ "$answer" == "RESTORE" ]] || { echo "aborted"; exit 1; }

docker compose stop engine api
age -d -i "$identity" "$backup" | gunzip | docker compose exec -T mysql sh -c \
  'MYSQL_PWD="$(cat /run/secrets/mysql_root_password)" exec mysql -uroot --default-character-set=utf8mb4'
echo "restore complete. Engine stays stopped. Verify data, then: docker compose up -d engine api"
echo "On start the engine boots in REDUCE_ONLY until reconciliation with the brokers is clean."

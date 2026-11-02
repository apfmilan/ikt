#!/usr/bin/env bash
# Nightly encrypted MySQL backup. The VPS holds only the age PUBLIC recipient, so it cannot
# decrypt its own backups; the identity (private key) is kept offline.
# Env: TC_BACKUP_DIR (default /var/backups/trading-center)
#      TC_BACKUP_AGE_RECIPIENT_FILE (default /etc/trading-center/backup-recipient.txt)
#      TC_BACKUP_RETENTION_DAYS (default 14)
#      TC_BACKUP_OFFSITE_CMD (optional, receives the file path as $1, e.g. an rclone wrapper)
set -euo pipefail
cd "$(dirname "$0")/../.."
BACKUP_DIR="${TC_BACKUP_DIR:-/var/backups/trading-center}"
RECIPIENT_FILE="${TC_BACKUP_AGE_RECIPIENT_FILE:-/etc/trading-center/backup-recipient.txt}"
RETENTION="${TC_BACKUP_RETENTION_DAYS:-14}"
command -v age >/dev/null || { echo "age not installed" >&2; exit 1; }
[[ -s "$RECIPIENT_FILE" ]] || { echo "missing age recipient $RECIPIENT_FILE" >&2; exit 1; }
umask 077
mkdir -p "$BACKUP_DIR"
ts="$(date -u +%Y%m%dT%H%M%SZ)"
out="$BACKUP_DIR/trading_center-$ts.sql.gz.age"
tmp="$out.partial"
trap 'rm -f "$tmp"' EXIT

# Password passed via MYSQL_PWD inside the container (not on any command line).
docker compose exec -T mysql sh -c \
  'MYSQL_PWD="$(cat /run/secrets/mysql_root_password)" exec mysqldump -uroot \
     --single-transaction --quick --routines --triggers --events \
     --set-gtid-purged=OFF --default-character-set=utf8mb4 --databases trading_center' \
  | gzip -9 | age -R "$RECIPIENT_FILE" > "$tmp"

[[ -s "$tmp" ]] || { echo "backup empty" >&2; exit 1; }
mv "$tmp" "$out"
trap - EXIT
echo "backup written: $out ($(du -h "$out" | cut -f1))"

if [[ -n "${TC_BACKUP_OFFSITE_CMD:-}" ]]; then
  "$TC_BACKUP_OFFSITE_CMD" "$out"
fi
find "$BACKUP_DIR" -name 'trading_center-*.sql.gz.age' -type f -mtime "+$RETENTION" -print -delete

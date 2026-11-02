#!/usr/bin/env bash
# Generate all Docker secret files in deploy/secrets/ (idempotent: never overwrites).
# Requires: openssl, docker, and the server image (built or pulled) for `tc keygen`.
# Secrets never leave the VPS; nothing is printed except file names.
set -euo pipefail
cd "$(dirname "$0")/../.."
SECRETS_DIR="deploy/secrets"
IMAGE="${TC_IMAGE:-trading-center/server}:${TC_IMAGE_TAG:-latest}"
APP_UID=10001
MYSQL_UID=999
umask 077
mkdir -p "$SECRETS_DIR"

gen_pw() { # name
  local f="$SECRETS_DIR/$1"
  if [[ -s "$f" ]]; then echo "keep   $f"; return; fi
  openssl rand -hex 24 | tr -d '\n' > "$f"
  echo "create $f"
}

gen_pw mysql_root_password
gen_pw mysql_app_password
gen_pw mysql_migrate_password

if [[ ! -s "$SECRETS_DIR/database_url" ]]; then
  printf 'mysql://tc_app:%s@mysql:3306/trading_center' "$(cat "$SECRETS_DIR/mysql_app_password")" > "$SECRETS_DIR/database_url"
  echo "create $SECRETS_DIR/database_url"
fi
if [[ ! -s "$SECRETS_DIR/database_url_migrate" ]]; then
  printf 'mysql://tc_migrate:%s@mysql:3306/trading_center' "$(cat "$SECRETS_DIR/mysql_migrate_password")" > "$SECRETS_DIR/database_url_migrate"
  echo "create $SECRETS_DIR/database_url_migrate"
fi

# Engine X25519 key pair (credential sealing, ADR-0001/0004) and VAPID key pair (Web Push).
# `tc keygen --out-dir DIR` writes engine_secret_key, engine_public_key,
# vapid_private_key, vapid_public_key and refuses to overwrite existing files.
need_keys=0
for f in engine_secret_key engine_public_key vapid_private_key vapid_public_key; do
  [[ -s "$SECRETS_DIR/$f" ]] || need_keys=1
done
if [[ $need_keys == 1 ]]; then
  docker run --rm --network none --user "$(id -u):$(id -g)" \
    -v "$PWD/$SECRETS_DIR:/out" "$IMAGE" /usr/local/bin/tc keygen --out-dir /out
  echo "create engine + vapid key pairs"
else
  echo "keep   engine + vapid key pairs"
fi

# Ownership: MySQL secrets readable by the mysql user (uid 999), app secrets by uid 10001.
if [[ $(id -u) == 0 ]]; then
  chown "$MYSQL_UID:$MYSQL_UID" "$SECRETS_DIR"/mysql_*_password
  chown "$APP_UID:$APP_UID" "$SECRETS_DIR"/database_url* "$SECRETS_DIR"/engine_* "$SECRETS_DIR"/vapid_*
fi
chmod 0400 "$SECRETS_DIR"/mysql_*_password "$SECRETS_DIR"/database_url* "$SECRETS_DIR"/engine_* "$SECRETS_DIR"/vapid_*
echo "secrets ready in $SECRETS_DIR (mode 0400)"

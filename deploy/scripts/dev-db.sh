#!/usr/bin/env bash
# Start (or reuse) a local development MySQL on 127.0.0.1:3307 and write its URL to
# target/dev/database_url for `tc-server --role all` (TC_DATABASE_URL_FILE).
set -euo pipefail
cd "$(dirname "$0")/../.."
name="tc-dev-mysql"
mkdir -p target/dev
if [[ ! -s target/dev/mysql_password ]]; then
  (umask 077; openssl rand -hex 16 | tr -d '\n' > target/dev/mysql_password)
fi
pw="$(cat target/dev/mysql_password)"
if ! docker ps -a --format '{{.Names}}' | grep -qx "$name"; then
  docker run -d --name "$name" -p 127.0.0.1:3307:3306 \
    -e MYSQL_ROOT_PASSWORD="$pw" -e MYSQL_DATABASE=trading_center \
    -v tc-dev-mysql:/var/lib/mysql \
    --health-cmd 'mysqladmin ping -h 127.0.0.1 --silent' --health-interval 2s --health-retries 60 \
    mysql:8.4 --character-set-server=utf8mb4 --default-time-zone=+00:00 >/dev/null
else
  docker start "$name" >/dev/null
fi
for _ in $(seq 1 90); do
  [[ "$(docker inspect -f '{{.State.Health.Status}}' "$name")" == healthy ]] && break
  sleep 2
done
(umask 077; printf 'mysql://root:%s@127.0.0.1:3307/trading_center' "$pw" > target/dev/database_url)
echo "dev mysql ready; TC_DATABASE_URL_FILE=$PWD/target/dev/database_url"

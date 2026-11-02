#!/usr/bin/env bash
# Run MySQL integration tests against a throwaway mysql:8.4 container on a random port.
set -euo pipefail
cd "$(dirname "$0")/../.."
name="tc-db-test-$$"
pw="test-$(openssl rand -hex 8)"
cleanup() { docker rm -f "$name" >/dev/null 2>&1 || true; }
trap cleanup EXIT
docker run -d --name "$name" -P \
  -e MYSQL_ROOT_PASSWORD="$pw" -e MYSQL_DATABASE=tc_test \
  --health-cmd 'mysqladmin ping -h 127.0.0.1 --silent' --health-interval 2s --health-retries 60 \
  mysql:8.4 --character-set-server=utf8mb4 --default-time-zone=+00:00 \
  --sql-mode=STRICT_ALL_TABLES,NO_ZERO_IN_DATE,NO_ZERO_DATE,ERROR_FOR_DIVISION_BY_ZERO,NO_ENGINE_SUBSTITUTION >/dev/null
for _ in $(seq 1 90); do
  status="$(docker inspect -f '{{.State.Health.Status}}' "$name")"
  [[ "$status" == healthy ]] && break
  sleep 2
done
[[ "$status" == healthy ]] || { docker logs "$name" | tail -20; echo "mysql not healthy" >&2; exit 1; }
port="$(docker port "$name" 3306/tcp | head -1 | sed 's/.*://')"
export TC_TEST_DATABASE_URL="mysql://root:${pw}@127.0.0.1:${port}/tc_test"
echo "mysql ready on port $port"
cargo test -p tc-store -- --include-ignored "$@"

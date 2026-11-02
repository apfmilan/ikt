#!/usr/bin/env bash
# PreToolUse(Write|Edit) guard: refuse writing secrets into the repository.
set -euo pipefail
input=$(cat)
path=$(jq -r '.tool_input.file_path // ""' <<<"$input")
content=$(jq -r '(.tool_input.content // "") + (.tool_input.new_string // "")' <<<"$input")
block() { echo "BLOCKED by guard-secrets: $1" >&2; exit 2; }
case "$path" in
  */deploy/secrets/*|*.pem|*.key|*/.env|*/.env.local|*/.env.production) block "secret file path $path";;
esac
if grep -qE -- '-----BEGIN [A-Z ]*PRIVATE KEY-----' <<<"$content"; then block "private key material"; fi
if grep -qiE '(t212|trading212|bitvavo)[_-]?(api)?[_-]?(key|secret)\s*[:=]\s*"[A-Za-z0-9+/=_-]{16,}"' <<<"$content"; then block "hard-coded broker credential"; fi
exit 0

#!/usr/bin/env bash
# PreToolUse(Bash) guard: blocks shell commands that could place live orders,
# leak secrets, or rewrite protected history. Exit 2 = block (stderr shown to Claude).
set -euo pipefail
cmd=$(jq -r '.tool_input.command // ""')
block() { echo "BLOCKED by guard-live-trading: $1" >&2; exit 2; }

# Live order placement / cancellation against real brokers from the dev shell.
if grep -qiE 'live\.trading212\.com[^ ]*/equity/orders' <<<"$cmd"; then block "Trading 212 live order endpoint"; fi
if grep -qiE 'api\.bitvavo\.com/v2/(order|orders|withdrawal|cancelOrdersAfter)' <<<"$cmd"; then block "Bitvavo private trading endpoint"; fi
if grep -qiE 'TC_ALLOW_LIVE=1' <<<"$cmd"; then block "enabling live trading from the shell is not allowed"; fi
# History protection.
if grep -qE 'git push .*(--force|-f)( |$).*\b(main|master)\b' <<<"$cmd"; then block "force-push to main"; fi
# Printing secret files.
if grep -qE '(cat|less|head|tail|more) [^|;]*deploy/secrets/' <<<"$cmd"; then block "reading deploy/secrets"; fi
exit 0

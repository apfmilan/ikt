#!/usr/bin/env bash
# PostToolUse(Write|Edit): format edited Rust files. Never fails the tool call.
path=$(jq -r '.tool_input.file_path // ""')
if [[ "$path" == *.rs && -f "$path" ]] && command -v rustfmt >/dev/null; then
  rustfmt --edition 2021 "$path" >/dev/null 2>&1 || true
fi
exit 0

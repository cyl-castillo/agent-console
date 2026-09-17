#!/usr/bin/env bash
# Fail CI when a Tauri command is synchronous without being on the allowlist.
# Sync commands block the webview main thread (see scripts/ipc-sync-allowlist.txt).
set -euo pipefail
cd "$(dirname "$0")/.."
allow="scripts/ipc-sync-allowlist.txt"
status=0
# Pair every `#[tauri::command]` attribute (no `(async)`) with the fn it decorates.
while IFS= read -r line; do
  file="${line%%:*}"
  rest="${line#*:}"
  lineno="${rest%%:*}"
  name=$(sed -n "$((lineno + 1)),$((lineno + 3))p" "$file" | grep -oE 'fn [a-z_0-9]+' | head -1 | cut -d' ' -f2)
  [ -n "$name" ] || continue
  # `async fn` runs on the async runtime regardless of the attribute form.
  if sed -n "$((lineno + 1)),$((lineno + 3))p" "$file" | grep -qE '^\s*pub async fn'; then continue; fi
  if ! grep -qxF "$name" "$allow"; then
    echo "::error file=$file,line=$lineno::sync tauri command '$name' is not allowlisted — make it #[tauri::command(async)] or add it to $allow with a reason"
    status=1
  fi
done < <(grep -rn '^#\[tauri::command\]$' src-tauri/src/commands/)
# The reverse: an allowlisted name that no longer exists as a sync command is stale.
while IFS= read -r name; do
  [[ -z "$name" || "$name" == \#* ]] && continue
  if ! grep -rqE "^\s*pub fn $name\b" src-tauri/src/commands/; then
    echo "::warning::allowlisted command '$name' not found as a sync command — remove it from $allow"
  fi
done < "$allow"
exit $status

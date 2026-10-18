#!/usr/bin/env bash
# Probe a running Download Tool. Does not start the app.
# Default: http://127.0.0.1:29128
# Override: DOWNLOAD_TOOL_URL=http://127.0.0.1:9 ./scripts/smoke-local.sh

set -euo pipefail

base="${DOWNLOAD_TOOL_URL:-http://127.0.0.1:29128}"
base="${base%/}"

health="$(curl --fail --silent --show-error --max-time 5 "${base}/health")"
printf '%s\n' "$health"
printf '%s\n' "$health" | grep -Eq '"ok"[[:space:]]*:[[:space:]]*true'

created="$(curl --fail --silent --show-error --max-time 5 \
  -X POST "${base}/v1/tasks" \
  -H 'content-type: application/json' \
  -d '{"url":"http://127.0.0.1:9/smoke-local.bin","fileName":"smoke-local.bin","size":1}')"
printf '%s\n' "$created"
printf '%s\n' "$created" | grep -Eq '"taskId"[[:space:]]*:[[:space:]]*"[^"]+"'

echo "smoke ok: ${base}"

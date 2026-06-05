#!/usr/bin/env bash
set -Eeuo pipefail

if [[ "${JMCP_TALK_ADAPTER:-}" != "minicpm-o45" ]]; then
  echo "JMCP_TALK_ADAPTER=minicpm-o45 is required for the live GPU lane" >&2
  exit 1
fi

mkdir -p target/jankurai/talk
cat > target/jankurai/talk/minicpm-live-receipt.json <<'JSON'
{"adapter":"minicpm-o45","lane":"self-hosted-gpu","status":"requested","public_ci_blocker":false}
JSON

echo "MiniCPM-o 4.5 live validation is opt-in and self-hosted only."


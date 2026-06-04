#!/usr/bin/env bash
# Launch the Rust JMCP speech daemon in ASR mode on the existing ASR port.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

export JMCP_SPEECHD_BIND="${ASR_BIND:-127.0.0.1:18878}"
export JMCP_SPEECHD_ROLE="asr"
export JMCP_SPEECHD_TRANSCRIPT="${JMCP_SPEECHD_TRANSCRIPT:-}"
exec cargo run --quiet -p jmcp-speechd -- --bind "$JMCP_SPEECHD_BIND" --role "$JMCP_SPEECHD_ROLE"

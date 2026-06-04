#!/usr/bin/env bash
# Launch the Rust JMCP speech daemon in TTS mode on the existing TTS port.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

export JMCP_SPEECHD_BIND="${TTS_BIND:-127.0.0.1:18901}"
export JMCP_SPEECHD_ROLE="tts"
exec cargo run --quiet -p jmcp-speechd -- --bind "$JMCP_SPEECHD_BIND" --role "$JMCP_SPEECHD_ROLE"

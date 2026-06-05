#!/usr/bin/env bash
# Launch the real JMCP TTS sidecar (Kokoro-82M) on the local GPU/CPU.
set -Eeuo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENV="$HERE/.venv-tts"

if [[ ! -x "$VENV/bin/python" ]]; then
  echo "[tts] creating venv (--system-site-packages) + installing deps (one-time)..." >&2
  python3 -m venv --system-site-packages "$VENV"
  "$VENV/bin/pip" install --quiet --upgrade pip
  "$VENV/bin/pip" install --quiet -r "$HERE/requirements-tts.txt"
fi

export TTS_VOICE="${TTS_VOICE:-af_heart}"
export TTS_LANG="${TTS_LANG:-a}"
export TTS_DEVICE="${TTS_DEVICE:-auto}"
export TTS_BIND="${TTS_BIND:-127.0.0.1:18901}"
exec "$VENV/bin/python" "$HERE/tts_sidecar.py"

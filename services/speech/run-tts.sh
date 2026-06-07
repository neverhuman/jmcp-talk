#!/usr/bin/env bash
# Launch the JMCP TTS sidecar on the local GPU/CPU.
#
# The venv is created with --system-site-packages so it reuses the system torch
# (no multi-GB duplicate). First run downloads TTS weights into the HF cache.
# Binds 127.0.0.1:18901 by default (a JMCP-safe port, never a Jeryu-protected one).
set -Eeuo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENV="$HERE/.venv-tts"

if [[ ! -x "$VENV/bin/python" ]]; then
  echo "[tts] creating venv (--system-site-packages) + installing deps (one-time)…" >&2
  python3 -m venv --system-site-packages "$VENV"
  "$VENV/bin/pip" install --quiet --upgrade pip
  "$VENV/bin/pip" install --quiet -r "$HERE/requirements-tts.txt"
fi

# VoxCPM2 currently resolves CUDA 13 runtime libs from the venv's bundled
# NVIDIA wheel paths, not the host CUDA 12 install.
CUDA13_LIB_DIR="$(find "$VENV/lib" -path '*/site-packages/nvidia/cu13/lib' -type d 2>/dev/null | head -n 1 || true)"
if [[ -n "$CUDA13_LIB_DIR" ]]; then
  export LD_LIBRARY_PATH="$CUDA13_LIB_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

export TTS_ENGINE="${TTS_ENGINE:-voxcpm2}"
export TTS_FALLBACK_ENGINE="${TTS_FALLBACK_ENGINE:-kokoro}"
export TTS_VOICE="${TTS_VOICE:-jmcp_male_v1}"
export TTS_LANG="${TTS_LANG:-a}"
export TTS_DEVICE="${TTS_DEVICE:-auto}"
export TTS_BIND="${TTS_BIND:-127.0.0.1:18901}"
# Keep the primary VoxCPM2 path on the narrow side of the GPU budget so it can
# coexist with the realtime LLM profile on a 24 GB card.
export TTS_VOXCPM_CFG_VALUE="${TTS_VOXCPM_CFG_VALUE:-1.6}"
export TTS_VOXCPM_INFERENCE_TIMESTEPS="${TTS_VOXCPM_INFERENCE_TIMESTEPS:-4}"
exec "$VENV/bin/python" "$HERE/tts_sidecar.py"

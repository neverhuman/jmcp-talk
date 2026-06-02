#!/usr/bin/env bash
# Launch the JMCP ASR sidecar on the local GPU.
#
# First run creates the venv + installs faster-whisper and downloads the model
# (~3GB for large-v3) into the shared HF cache. Subsequent runs are instant.
# Binds 127.0.0.1:18878 by default (a JMCP-safe port, never a Jeryu-protected one).
set -Eeuo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENV="$HERE/.venv"

if [[ ! -x "$VENV/bin/python" ]]; then
  echo "[asr] creating venv + installing deps (one-time)…" >&2
  python3 -m venv "$VENV"
  "$VENV/bin/pip" install --quiet --upgrade pip
  "$VENV/bin/pip" install --quiet -r "$HERE/requirements-asr.txt"
fi

# Auto-pick the device: CUDA when an NVIDIA GPU is present (Linux), else CPU.
# CTranslate2 (faster-whisper) has no Metal/MPS backend, so on Apple Silicon
# the right default is CPU with int8 — which still runs near real-time for the
# 82M..1.5B class and fine for large-v3 on M-series. Override with ASR_DEVICE.
if [[ -z "${ASR_DEVICE:-}" ]]; then
  if command -v nvidia-smi >/dev/null 2>&1; then
    ASR_DEVICE=cuda
    ASR_COMPUTE="${ASR_COMPUTE:-float16}"
  else
    ASR_DEVICE=cpu
    ASR_COMPUTE="${ASR_COMPUTE:-int8}"
  fi
fi
export ASR_MODEL="${ASR_MODEL:-large-v3}"
export ASR_DEVICE
export ASR_COMPUTE="${ASR_COMPUTE:-float16}"
export ASR_BIND="${ASR_BIND:-127.0.0.1:18878}"
exec "$VENV/bin/python" "$HERE/asr_sidecar.py"

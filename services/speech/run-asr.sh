#!/usr/bin/env bash
# Launch the JMCP ASR sidecar on the local GPU/CPU.
#
# First run creates the venv + installs faster-whisper and downloads the model
# into the shared HF cache. The realtime default is distil-small.en; set
# ASR_MODEL=large-v3 explicitly for higher-accuracy offline runs.
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
# the right default is CPU with int8. Override with ASR_DEVICE.
if [[ -z "${ASR_DEVICE:-}" ]]; then
  if command -v nvidia-smi >/dev/null 2>&1; then
    ASR_DEVICE=cuda
  else
    ASR_DEVICE=cpu
  fi
fi
if [[ -z "${ASR_COMPUTE:-}" ]]; then
  if [[ "$ASR_DEVICE" == "cpu" ]]; then
    ASR_COMPUTE=int8
  else
    ASR_COMPUTE=float16
  fi
fi
export ASR_MODEL="${ASR_MODEL:-distil-small.en}"
export ASR_DEVICE
export ASR_COMPUTE
export ASR_BEAM_SIZE="${ASR_BEAM_SIZE:-1}"
export ASR_BIND="${ASR_BIND:-127.0.0.1:18878}"
exec "$VENV/bin/python" "$HERE/asr_sidecar.py"

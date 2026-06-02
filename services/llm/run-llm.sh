#!/usr/bin/env bash
# Launch the JMCP local reasoning-model sidecar: vLLM serving an OpenAI-compatible
# /v1 API on the GPU. This is the "brain" the voice loop and jnoccio route to.
#
# First run creates the venv + installs vLLM and downloads the model (~17GB for the
# 30B-A3B AWQ) into the shared HF cache. Binds 127.0.0.1:18902 by default (a
# JMCP-safe port, never a Jeryu-protected one). Weights + venv are git-ignored.
#
# Defaults to the GPU-dedicated 30B (move speech to CPU first via dedicate-gpu.sh).
# For the co-located 14B fallback that runs ALONGSIDE the GPU speech sidecars:
#   LLM_MODEL=Qwen/Qwen2.5-Coder-14B-Instruct-AWQ LLM_SERVED_NAME=local/qwen2.5-coder-14b \
#   LLM_GPU_UTIL=0.55 LLM_MAX_LEN=16384 ./run-llm.sh
set -Eeuo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENV="$HERE/.venv-llm"

if [[ ! -x "$VENV/bin/vllm" ]]; then
  echo "[llm] creating venv + installing vLLM (one-time, large)…" >&2
  python3 -m venv "$VENV"
  "$VENV/bin/pip" install --quiet --upgrade pip
  # cu126 wheel index so torch lands as a CUDA-12 build (matches the driver).
  "$VENV/bin/pip" install -r "$HERE/requirements-llm.txt" \
    --extra-index-url https://download.pytorch.org/whl/cu126
fi

# Put the venv's bin on PATH so vLLM/FlashInfer find `ninja` for JIT kernel builds.
export PATH="$VENV/bin:$PATH"

export LLM_MODEL="${LLM_MODEL:-cpatonn/Qwen3-30B-A3B-Instruct-2507-AWQ-4bit}"
export LLM_SERVED_NAME="${LLM_SERVED_NAME:-local/qwen3-30b-a3b}"
export LLM_PORT="${LLM_PORT:-18902}"
export LLM_GPU_UTIL="${LLM_GPU_UTIL:-0.92}"
export LLM_MAX_LEN="${LLM_MAX_LEN:-32768}"

# Optional explicit quantization kernel (vLLM usually auto-detects AWQ from the
# model config; set LLM_QUANT=awq_marlin to force).
quant_arg=()
[[ -n "${LLM_QUANT:-}" ]] && quant_arg=(--quantization "$LLM_QUANT")

exec "$VENV/bin/vllm" serve "$LLM_MODEL" \
  --host 127.0.0.1 --port "$LLM_PORT" \
  --served-model-name "$LLM_SERVED_NAME" \
  --gpu-memory-utilization "$LLM_GPU_UTIL" \
  --max-model-len "$LLM_MAX_LEN" \
  --enable-auto-tool-choice --tool-call-parser hermes \
  "${quant_arg[@]}"

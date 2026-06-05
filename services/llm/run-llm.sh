#!/usr/bin/env bash
# Launch the JMCP local reasoning-model sidecar: vLLM serving an
# OpenAI-compatible /v1 API on the GPU.
set -Eeuo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENV="$HERE/.venv-llm"

if [[ ! -x "$VENV/bin/vllm" ]]; then
  echo "[llm] creating venv + installing vLLM (one-time, large)..." >&2
  python3 -m venv "$VENV"
  "$VENV/bin/pip" install --quiet --upgrade pip
  "$VENV/bin/pip" install -r "$HERE/requirements-llm.txt" \
    --extra-index-url https://download.pytorch.org/whl/cu126
fi

export PATH="$VENV/bin:$PATH"

export LLM_MODEL="${LLM_MODEL:-cpatonn/Qwen3-30B-A3B-Instruct-2507-AWQ-4bit}"
export LLM_SERVED_NAME="${LLM_SERVED_NAME:-local/qwen3-30b-a3b}"
export LLM_PORT="${LLM_PORT:-18902}"
export LLM_GPU_UTIL="${LLM_GPU_UTIL:-0.92}"
export LLM_MAX_LEN="${LLM_MAX_LEN:-32768}"

quant_arg=()
[[ -n "${LLM_QUANT:-}" ]] && quant_arg=(--quantization "$LLM_QUANT")

exec "$VENV/bin/vllm" serve "$LLM_MODEL" \
  --host 127.0.0.1 --port "$LLM_PORT" \
  --served-model-name "$LLM_SERVED_NAME" \
  --gpu-memory-utilization "$LLM_GPU_UTIL" \
  --max-model-len "$LLM_MAX_LEN" \
  --enable-auto-tool-choice --tool-call-parser hermes \
  "${quant_arg[@]}"

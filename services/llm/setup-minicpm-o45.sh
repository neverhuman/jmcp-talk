#!/usr/bin/env bash
# Build/install the MiniCPM-o 4.5 GGUF live stack under the split-owned live dir.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SPLIT_ROOT="${JMCP_SPLIT_ROOT:-/home/ubuntu/jmcp-split}"
LIVE_ROOT="${JMCP_TALK_MINICPM_LIVE_ROOT:-$SPLIT_ROOT/.live/minicpm-o45}"
LLAMA_ROOT="${JMCP_TALK_MINICPM_LLAMA_CPP_ROOT:-$LIVE_ROOT/llama.cpp-omni}"
DEMO_ROOT="${JMCP_TALK_MINICPM_DEMO_ROOT:-$LIVE_ROOT/MiniCPM-o-Demo}"
MODEL_DIR="${JMCP_TALK_MINICPM_MODEL_DIR:-$LIVE_ROOT/models/MiniCPM-o-4_5-gguf}"
QUANT="${JMCP_TALK_MINICPM_QUANT:-Q4_K_M}"
DOWNLOAD_MODEL="${JMCP_TALK_MINICPM_DOWNLOAD:-0}"
PYTHON_BIN="${PYTHON:-python3.10}"

step() {
  printf '\n[minicpm-setup] %s\n' "$*"
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$1" >&2
    exit 1
  }
}

need_cmd git
need_cmd cmake
need_cmd python3
need_cmd "$PYTHON_BIN"
mkdir -p "$LIVE_ROOT" "$MODEL_DIR"

if [[ ! -d "$LLAMA_ROOT/.git" ]]; then
  step "cloning llama.cpp-omni"
  git clone --branch feat/web-demo https://github.com/tc-mb/llama.cpp-omni.git "$LLAMA_ROOT"
fi

step "building llama.cpp-omni llama-server"
(
  cd "$LLAMA_ROOT"
  cmake -B build -DGGML_CUDA=ON -DLLAMA_CURL=OFF -DCMAKE_BUILD_TYPE=Release
  cmake --build build --target llama-server -j"$(nproc 2>/dev/null || echo 4)"
)

if [[ ! -d "$DEMO_ROOT/.git" ]]; then
  step "cloning OpenBMB MiniCPM-o-Demo Comni branch"
  git clone --branch Comni https://github.com/OpenBMB/MiniCPM-o-Demo.git "$DEMO_ROOT"
fi

step "installing Comni Python environment"
(
  cd "$DEMO_ROOT"
  PYTHON="$PYTHON_BIN" bash install.sh
)

if [[ "$DOWNLOAD_MODEL" == "1" ]]; then
  step "downloading openbmb/MiniCPM-o-4_5-gguf to $MODEL_DIR"
  "$DEMO_ROOT/.venv/base/bin/python" -m pip install -q -U "huggingface_hub>=0.36,<1"
  model_files=(
    "MiniCPM-o-4_5-${QUANT}.gguf"
    "audio/MiniCPM-o-4_5-audio-F16.gguf"
    "tts/MiniCPM-o-4_5-projector-F16.gguf"
    "tts/MiniCPM-o-4_5-tts-F16.gguf"
    "vision/MiniCPM-o-4_5-vision-F16.gguf"
    "token2wav-gguf/encoder.gguf"
    "token2wav-gguf/flow_extra.gguf"
    "token2wav-gguf/flow_matching.gguf"
    "token2wav-gguf/hifigan2.gguf"
    "token2wav-gguf/prompt_cache.gguf"
  )
  for model_file in "${model_files[@]}"; do
    "$DEMO_ROOT/.venv/base/bin/hf" download \
      openbmb/MiniCPM-o-4_5-gguf \
      "$model_file" \
      --local-dir "$MODEL_DIR"
  done
else
  step "model download skipped"
  printf 'Set JMCP_TALK_MINICPM_DOWNLOAD=1 to download openbmb/MiniCPM-o-4_5-gguf.\n'
  printf 'Expected model file: %s/MiniCPM-o-4_5-%s.gguf\n' "$MODEL_DIR" "$QUANT"
fi

step "setup complete"
printf 'LLAMA_ROOT=%s\nDEMO_ROOT=%s\nMODEL_DIR=%s\n' "$LLAMA_ROOT" "$DEMO_ROOT" "$MODEL_DIR"

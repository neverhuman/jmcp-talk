#!/usr/bin/env bash
# Run the JMCP MiniCPM-o 4.5 debug voice gateway on 127.0.0.1:8041.
set -Eeuo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SPLIT_ROOT="${JMCP_SPLIT_ROOT:-/home/ubuntu/jmcp-split}"
LIVE_ROOT="${JMCP_TALK_MINICPM_LIVE_ROOT:-$SPLIT_ROOT/.live/minicpm-o45}"
LOG_DIR="${JMCP_TALK_MINICPM_LOG_DIR:-$SPLIT_ROOT/.live/logs}"
AUDIO_DIR="${JMCP_TALK_AUDIO_DIR:-$SPLIT_ROOT/.live/audio}"
LLAMA_ROOT="${JMCP_TALK_MINICPM_LLAMA_CPP_ROOT:-$LIVE_ROOT/llama.cpp-omni}"
DEMO_ROOT="${JMCP_TALK_MINICPM_DEMO_ROOT:-$LIVE_ROOT/MiniCPM-o-Demo}"
MODEL_DIR="${JMCP_TALK_MINICPM_MODEL_DIR:-$LIVE_ROOT/models/MiniCPM-o-4_5-gguf}"
CONFIG_PATH="${JMCP_TALK_MINICPM_CONFIG:-$LIVE_ROOT/config.json}"
GATEWAY_BIND="${JMCP_TALK_MINICPM_BIND:-127.0.0.1:8041}"
VOICE_PROFILE="${JMCP_TALK_MINICPM_VOICE_PROFILE:-jmcp_friendly_male}"
REF_AUDIO="${JMCP_TALK_MINICPM_REF_AUDIO:-$HERE/assets/ref_audio/jmcp_friendly_male_16k.wav}"
COMNI_PORT="${JMCP_TALK_MINICPM_COMNI_PORT:-18040}"
WORKER_BASE_PORT="${JMCP_TALK_MINICPM_WORKER_BASE_PORT:-22440}"
CPP_SERVER_PORT="${JMCP_TALK_MINICPM_CPP_SERVER_PORT:-19080}"
CTX_SIZE="${JMCP_TALK_MINICPM_CTX_SIZE:-8192}"
N_GPU_LAYERS="${JMCP_TALK_MINICPM_N_GPU_LAYERS:-99}"
QUANT="${JMCP_TALK_MINICPM_QUANT:-Q4_K_M}"
GPU="${CUDA_VISIBLE_DEVICES:-0}"
AUTO_SETUP="${JMCP_TALK_MINICPM_AUTO_SETUP:-1}"
START_COMNI="${JMCP_TALK_MINICPM_START_COMNI:-1}"
GATEWAY_VENV="${JMCP_TALK_MINICPM_GATEWAY_VENV:-$LIVE_ROOT/.venv-gateway}"
EVENT_LOG="${JMCP_TALK_MINICPM_EVENT_LOG:-$LOG_DIR/voice-events.jsonl}"
MODEL_FILE="$MODEL_DIR/MiniCPM-o-4_5-$QUANT.gguf"
REQUIRED_MODEL_FILES=(
  "$MODEL_FILE"
  "$MODEL_DIR/audio/MiniCPM-o-4_5-audio-F16.gguf"
  "$MODEL_DIR/tts/MiniCPM-o-4_5-projector-F16.gguf"
  "$MODEL_DIR/tts/MiniCPM-o-4_5-tts-F16.gguf"
  "$MODEL_DIR/vision/MiniCPM-o-4_5-vision-F16.gguf"
  "$MODEL_DIR/token2wav-gguf/encoder.gguf"
  "$MODEL_DIR/token2wav-gguf/flow_extra.gguf"
  "$MODEL_DIR/token2wav-gguf/flow_matching.gguf"
  "$MODEL_DIR/token2wav-gguf/hifigan2.gguf"
  "$MODEL_DIR/token2wav-gguf/prompt_cache.gguf"
)

models_ready() {
  local file
  for file in "${REQUIRED_MODEL_FILES[@]}"; do
    [[ -f "$file" ]] || return 1
  done
}

mkdir -p "$LIVE_ROOT" "$LOG_DIR" "$AUDIO_DIR" "$(dirname "$CONFIG_PATH")"

if [[ ! -f "$REF_AUDIO" ]]; then
  printf 'MiniCPM reference audio not found: %s\n' "$REF_AUDIO" >&2
  exit 1
fi

cleanup() {
  set +e
  if [[ -d "$DEMO_ROOT/tmp" ]]; then
    while IFS= read -r pid_file; do
      pid="$(cat "$pid_file" 2>/dev/null || true)"
      [[ "$pid" =~ ^[0-9]+$ ]] || continue
      kill -TERM "$pid" >/dev/null 2>&1 || true
    done < <(find "$DEMO_ROOT/tmp" -maxdepth 1 -name '*.pid' -type f 2>/dev/null)
  fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

setup_needed=0
if [[ ! -x "$LLAMA_ROOT/build/bin/llama-server" || ! -x "$DEMO_ROOT/.venv/base/bin/python" ]]; then
  setup_needed=1
elif ! models_ready; then
  setup_needed=1
fi

if [[ "$AUTO_SETUP" == "1" && "$setup_needed" == "1" ]]; then
  JMCP_TALK_MINICPM_LIVE_ROOT="$LIVE_ROOT" \
    JMCP_TALK_MINICPM_LLAMA_CPP_ROOT="$LLAMA_ROOT" \
    JMCP_TALK_MINICPM_DEMO_ROOT="$DEMO_ROOT" \
    JMCP_TALK_MINICPM_MODEL_DIR="$MODEL_DIR" \
    JMCP_TALK_MINICPM_QUANT="$QUANT" \
    bash "$HERE/setup-minicpm-o45.sh" 2>&1 | tee "$LOG_DIR/minicpm-setup.log"
fi

if [[ ! -x "$GATEWAY_VENV/bin/python" ]]; then
  python3 -m venv "$GATEWAY_VENV"
  "$GATEWAY_VENV/bin/pip" install -q -U pip wheel
  "$GATEWAY_VENV/bin/pip" install -q -r "$HERE/requirements-minicpm-gateway.txt"
fi

python3 - "$CONFIG_PATH" "$LLAMA_ROOT" "$MODEL_DIR" "$QUANT" "$COMNI_PORT" "$WORKER_BASE_PORT" "$CPP_SERVER_PORT" "$CTX_SIZE" "$N_GPU_LAYERS" "$REF_AUDIO" <<'PY'
import json
import sys
from pathlib import Path

config_path, llama_root, model_dir, quant, gateway_port, worker_base, cpp_port, ctx, ngl, ref_audio = sys.argv[1:]
llm_model = f"MiniCPM-o-4_5-{quant}.gguf"
data = {
    "backend": "cpp",
    "model": {
        "model_path": "openbmb/MiniCPM-o-4_5",
        "pt_path": None,
        "attn_implementation": "auto",
    },
    "audio": {
        "ref_audio_path": ref_audio,
        "playback_delay_ms": 120,
        "chat_vocoder": "token2wav",
    },
    "service": {
        "gateway_port": int(gateway_port),
        "worker_base_port": int(worker_base),
        "num_workers": 1,
        "max_queue_size": 1000,
        "request_timeout": 300.0,
        "compile": False,
        "data_dir": "data",
        "eta_chat_s": 15.0,
        "eta_streaming_s": 20.0,
        "eta_half_duplex_s": 180.0,
        "eta_audio_duplex_s": 120.0,
        "eta_omni_duplex_s": 90.0,
        "eta_duplex_s": 90.0,
        "eta_ema_alpha": 0.3,
        "eta_ema_min_samples": 3,
    },
    "duplex": {"pause_timeout": 60.0},
    "cpp_backend": {
        "llamacpp_root": llama_root,
        "model_dir": model_dir,
        "llm_model": llm_model,
        "cpp_server_port": int(cpp_port),
        "ctx_size": int(ctx),
        "n_gpu_layers": int(ngl),
    },
}
Path(config_path).write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
print(config_path)
PY

if command -v nvidia-smi >/dev/null 2>&1; then
  nvidia-smi --query-gpu=memory.used,memory.total --format=csv,noheader,nounits \
    > "$LOG_DIR/minicpm-gpu-at-launch.csv" || true
fi

if [[ "$START_COMNI" == "1" ]]; then
  [[ -x "$DEMO_ROOT/start_all.sh" ]] || {
    printf 'Comni demo missing. Run services/llm/setup-minicpm-o45.sh first.\n' >&2
    exit 1
  }
  cp "$CONFIG_PATH" "$DEMO_ROOT/config.json"
  (
    cd "$DEMO_ROOT"
    CUDA_VISIBLE_DEVICES="$GPU" bash start_all.sh --http
  ) >"$LOG_DIR/comni-start.log" 2>&1 &

  comni_ready=0
  deadline=$((SECONDS + 300))
  while (( SECONDS < deadline )); do
    if curl -fsS --max-time 2 "http://127.0.0.1:$COMNI_PORT/health" >/dev/null 2>&1; then
      comni_ready=1
      break
    fi
    sleep 2
  done
  if [[ "$comni_ready" != "1" ]]; then
    printf 'Comni did not become healthy on 127.0.0.1:%s within 300s.\n' "$COMNI_PORT" >&2
    tail -n 120 "$LOG_DIR/comni-start.log" >&2 || true
    exit 1
  fi
fi

exec "$GATEWAY_VENV/bin/python" "$HERE/minicpm_live_gateway.py" \
  --bind "$GATEWAY_BIND" \
  --upstream "http://127.0.0.1:$COMNI_PORT" \
  --voice-profile "$VOICE_PROFILE" \
  --ref-audio "$REF_AUDIO" \
  --audio-dir "$AUDIO_DIR" \
  --event-log "$EVENT_LOG"

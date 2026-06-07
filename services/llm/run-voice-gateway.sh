#!/usr/bin/env bash
# Run the JMCP local voice gateway on 127.0.0.1:8040.
set -Eeuo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TALK_ROOT="$(cd "$HERE/../.." && pwd)"
SPLIT_ROOT="${JMCP_SPLIT_ROOT:-/home/ubuntu/jmcp-split}"
LIVE_ROOT="${JMCP_TALK_VOICE_LIVE_ROOT:-$SPLIT_ROOT/.live/voice-gateway}"
LOG_DIR="${JMCP_TALK_VOICE_LOG_DIR:-$SPLIT_ROOT/.live/logs}"
AUDIO_DIR="${JMCP_TALK_AUDIO_DIR:-$SPLIT_ROOT/.live/audio}"
VENV="${JMCP_TALK_VOICE_GATEWAY_VENV:-$LIVE_ROOT/.venv-gateway}"
IMPL="${JMCP_TALK_VOICE_GATEWAY_IMPL:-rust}"

mkdir -p "$LIVE_ROOT" "$LOG_DIR" "$AUDIO_DIR"

export JMCP_TALK_VOICE_BIND="${JMCP_TALK_VOICE_BIND:-127.0.0.1:8040}"
export JMCP_TALK_LLM_UPSTREAM="${JMCP_TALK_LLM_UPSTREAM:-http://127.0.0.1:18902/v1}"
export JMCP_TALK_LLM_MODEL="${JMCP_TALK_LLM_MODEL:-local/qwen2.5-7b-instruct-awq}"
export JMCP_TALK_ASR_UPSTREAM="${JMCP_TALK_ASR_UPSTREAM:-http://127.0.0.1:18878}"
export JMCP_TALK_TTS_UPSTREAM="${JMCP_TALK_TTS_UPSTREAM:-http://127.0.0.1:18901}"
export JMCP_TALK_VOICE_PROFILE="${JMCP_TALK_VOICE_PROFILE:-jmcp_male_v1}"
export JMCP_TALK_AUDIO_DIR="$AUDIO_DIR"
export JMCP_TALK_VOICE_EVENT_LOG="${JMCP_TALK_VOICE_EVENT_LOG:-$LOG_DIR/voice-events.jsonl}"

if [[ "$IMPL" == "python" || "$IMPL" == "python-minicpm" ]]; then
  if [[ ! -x "$VENV/bin/python" ]]; then
    python3 -m venv "$VENV"
    "$VENV/bin/pip" install -q -U pip wheel
    "$VENV/bin/pip" install -q -r "$HERE/requirements-minicpm-gateway.txt"
  fi
  exec "$VENV/bin/python" "$HERE/voice_gateway.py" \
    --bind "$JMCP_TALK_VOICE_BIND" \
    --llm-upstream "$JMCP_TALK_LLM_UPSTREAM" \
    --llm-model "$JMCP_TALK_LLM_MODEL" \
    --asr-upstream "$JMCP_TALK_ASR_UPSTREAM" \
    --tts-upstream "$JMCP_TALK_TTS_UPSTREAM" \
    --voice-profile "$JMCP_TALK_VOICE_PROFILE" \
    --audio-dir "$AUDIO_DIR" \
    --event-log "$JMCP_TALK_VOICE_EVENT_LOG"
fi

cd "$TALK_ROOT"
exec cargo run -p jmcp-voiced

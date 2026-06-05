#!/usr/bin/env bash
# Bring up the realtime voice stack: local LLM + ASR + TTS on one box.
set -Eeuo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SPEECH="$HERE/../speech"

echo "[realtime] stopping existing model/speech sidecars..." >&2
pkill -f 'vllm serve' 2>/dev/null || true
pkill -f asr_sidecar.py 2>/dev/null || true
pkill -f tts_sidecar.py 2>/dev/null || true
pkill -f 'jmcp-speechd' 2>/dev/null || true
sleep 2

export ASR_MODEL="${ASR_MODEL:-distil-small.en}"
export ASR_DEVICE="${ASR_DEVICE:-cuda}"
export ASR_COMPUTE="${ASR_COMPUTE:-float16}"
export ASR_BEAM_SIZE="${ASR_BEAM_SIZE:-1}"
export TTS_DEVICE="${TTS_DEVICE:-cuda}"
export LLM_GPU_UTIL="${LLM_GPU_UTIL:-0.80}"
export LLM_MAX_LEN="${LLM_MAX_LEN:-8192}"

echo "[realtime] ASR $ASR_MODEL on $ASR_DEVICE/$ASR_COMPUTE (beam=$ASR_BEAM_SIZE, :18878)..." >&2
setsid nohup "$SPEECH/run-asr.sh" >/tmp/asr.log 2>&1 < /dev/null &

echo "[realtime] Kokoro TTS on $TTS_DEVICE (:18901)..." >&2
setsid nohup "$SPEECH/run-tts.sh" >/tmp/tts.log 2>&1 < /dev/null &

echo "[realtime] local LLM (:18902, ctx $LLM_MAX_LEN, util $LLM_GPU_UTIL)..." >&2
setsid nohup "$HERE/run-llm.sh" >/tmp/llm.log 2>&1 < /dev/null &

echo "[realtime] starting; the model can take 1-2 min to load. Watch: tail -f /tmp/llm.log" >&2
echo "[realtime] health: curl :18878/health :18901/health :18902/v1/models" >&2

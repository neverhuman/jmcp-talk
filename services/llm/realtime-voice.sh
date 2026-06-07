#!/usr/bin/env bash
# Bring up the REALTIME voice stack: a smaller text LLM + ASR + TTS all
# co-resident on the 3090, tuned so a full voice turn stays fast.
#
# Why co-located beats "GPU-dedicated + speech on CPU": the realtime ASR path is
# a distilled model on CUDA with beam 1. The 30B itself is fast enough at ctx
# 8192, so the win is putting ASR+TTS back on the GPU next to it.
#
# VRAM budget (24GB): keep the smaller AWQ LLM below the ceiling so VoxCPM2 TTS
# can stay resident with ASR on the same card.
set -Eeuo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SPEECH="$HERE/../speech"

echo "[realtime] stopping any existing model/speech processes…" >&2
pkill -f 'vllm serve' 2>/dev/null || true
pkill -f asr_sidecar.py 2>/dev/null || true
pkill -f tts_sidecar.py 2>/dev/null || true
pkill -f voice_gateway.py 2>/dev/null || true
pkill -f jmcp-voiced 2>/dev/null || true
sleep 2

export ASR_MODEL="${ASR_MODEL:-distil-small.en}"
export ASR_DEVICE="${ASR_DEVICE:-cuda}"
export ASR_COMPUTE="${ASR_COMPUTE:-float16}"
export ASR_BEAM_SIZE="${ASR_BEAM_SIZE:-1}"
export TTS_ENGINE="${TTS_ENGINE:-voxcpm2}"
export TTS_FALLBACK_ENGINE="${TTS_FALLBACK_ENGINE:-kokoro}"
export TTS_VOICE="${TTS_VOICE:-jmcp_male_v1}"
export TTS_DEVICE="${TTS_DEVICE:-cuda}"
export LLM_GPU_UTIL="${LLM_GPU_UTIL:-0.72}"
export LLM_MAX_LEN="${LLM_MAX_LEN:-8192}"

if [[ "${JMCP_TALK_REALTIME_FOREGROUND:-0}" == "1" ]]; then
  PIDS=()
  cleanup() {
    set +e
    for pid in "${PIDS[@]:-}"; do
      pkill -TERM -P "$pid" >/dev/null 2>&1 || true
      kill -TERM "$pid" >/dev/null 2>&1 || true
    done
  }
  trap cleanup EXIT
  trap 'exit 130' INT TERM

  echo "[realtime] ASR $ASR_MODEL on $ASR_DEVICE/$ASR_COMPUTE (beam=$ASR_BEAM_SIZE, :18878)…" >&2
  "$SPEECH/run-asr.sh" >/tmp/asr.log 2>&1 &
  PIDS+=("$!")

  echo "[realtime] VoxCPM2 TTS on $TTS_DEVICE (:18901, Kokoro degraded mode)…" >&2
  "$SPEECH/run-tts.sh" >/tmp/tts.log 2>&1 &
  PIDS+=("$!")

  echo "[realtime] Qwen2.5-7B-Instruct-AWQ on GPU (:18902, ctx $LLM_MAX_LEN, util $LLM_GPU_UTIL)…" >&2
  "$HERE/run-llm.sh" >/tmp/llm.log 2>&1 &
  PIDS+=("$!")

  echo "[realtime] Rust local voice gateway (:8040; /voice and /voice-ws cockpit proxy)…" >&2
  "$HERE/run-voice-gateway.sh" >/tmp/voice-gateway.log 2>&1 &
  gateway_pid="$!"
  PIDS+=("$gateway_pid")

  echo "[realtime] starting; the 30B takes ~1-2 min to load. Watch: tail -f /tmp/llm.log" >&2
  echo "[realtime] health: curl :8040/health (voice) :18878/health (ASR) :18901/health (TTS) :18902/health (LLM)" >&2
  wait "$gateway_pid"
  exit "$?"
fi

echo "[realtime] ASR $ASR_MODEL on $ASR_DEVICE/$ASR_COMPUTE (beam=$ASR_BEAM_SIZE, :18878)…" >&2
setsid nohup "$SPEECH/run-asr.sh" >/tmp/asr.log 2>&1 < /dev/null &

echo "[realtime] VoxCPM2 TTS on $TTS_DEVICE (:18901, Kokoro degraded mode)…" >&2
setsid nohup "$SPEECH/run-tts.sh" >/tmp/tts.log 2>&1 < /dev/null &

echo "[realtime] Qwen2.5-7B-Instruct-AWQ on GPU (:18902, ctx $LLM_MAX_LEN, util $LLM_GPU_UTIL)…" >&2
setsid nohup "$HERE/run-llm.sh" >/tmp/llm.log 2>&1 < /dev/null &

echo "[realtime] Rust local voice gateway (:8040; /voice and /voice-ws cockpit proxy)…" >&2
setsid nohup "$HERE/run-voice-gateway.sh" >/tmp/voice-gateway.log 2>&1 < /dev/null &

echo "[realtime] starting; the 30B takes ~1-2 min to load. Watch: tail -f /tmp/llm.log" >&2
echo "[realtime] health: curl :8040/health (voice) :18878/health (ASR) :18901/health (TTS) :18902/health (LLM)" >&2

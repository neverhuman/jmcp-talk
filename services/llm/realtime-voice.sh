#!/usr/bin/env bash
# Bring up the REALTIME voice stack: the 30B + ASR + TTS all co-resident on the
# 3090, tuned so a full voice turn is ~1-2s (vs ~40s with ASR on CPU).
#
# Why co-located beats "GPU-dedicated + speech on CPU": the realtime ASR path is
# a distilled model on CUDA with beam 1. The 30B itself is fast enough at ctx
# 8192, so the win is putting ASR+TTS back on the GPU next to it.
#
# VRAM budget (24GB): the 30B-A3B AWQ process is ~21GB @ gpu-util 0.80. That leaves
# ~3GB for speech, so ASR must be small: distil-small.en (~0.5GB resident, low
# transient) + Kokoro TTS (~1GB) keeps ~1.3GB free for inference peaks. distil-large-v3
# (1.9GB) leaves only ~130MB and OOMs on the first transcribe — use distil-small.en.
set -Eeuo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SPEECH="$HERE/../speech"

echo "[realtime] stopping any existing model/speech processes…" >&2
pkill -f 'vllm serve' 2>/dev/null || true
pkill -f asr_sidecar.py 2>/dev/null || true
pkill -f tts_sidecar.py 2>/dev/null || true
sleep 2

export ASR_MODEL="${ASR_MODEL:-distil-small.en}"
export ASR_DEVICE="${ASR_DEVICE:-cuda}"
export ASR_COMPUTE="${ASR_COMPUTE:-float16}"
export ASR_BEAM_SIZE="${ASR_BEAM_SIZE:-1}"
export TTS_DEVICE="${TTS_DEVICE:-cuda}"
export LLM_GPU_UTIL="${LLM_GPU_UTIL:-0.80}"
export LLM_MAX_LEN="${LLM_MAX_LEN:-8192}"

echo "[realtime] ASR $ASR_MODEL on $ASR_DEVICE/$ASR_COMPUTE (beam=$ASR_BEAM_SIZE, :18878)…" >&2
setsid nohup "$SPEECH/run-asr.sh" >/tmp/asr.log 2>&1 < /dev/null &

echo "[realtime] Kokoro TTS on $TTS_DEVICE (:18901)…" >&2
setsid nohup "$SPEECH/run-tts.sh" >/tmp/tts.log 2>&1 < /dev/null &

echo "[realtime] Qwen3-30B-A3B on GPU (:18902, ctx $LLM_MAX_LEN, util $LLM_GPU_UTIL)…" >&2
setsid nohup "$HERE/run-llm.sh" >/tmp/llm.log 2>&1 < /dev/null &

echo "[realtime] starting; the 30B takes ~1-2 min to load. Watch: tail -f /tmp/llm.log" >&2
echo "[realtime] health: curl :18878/health (ASR) :18901/health (TTS) :18902/health (LLM)" >&2

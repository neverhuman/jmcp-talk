#!/usr/bin/env bash
# Bring up the REALTIME voice stack: the 30B + ASR + TTS all co-resident on the
# 3090, tuned so a full voice turn is ~1-2s (vs ~40s with ASR on CPU).
#
# Why co-located beats "GPU-dedicated + speech on CPU": faster-whisper large-v3
# on CPU runs at ~25x SLOWER than realtime (85s for a 3s clip). On the GPU a
# distilled model runs at ~0.15x realtime (0.5s). The 30B itself is ~110 tok/s
# either way, so the win is putting ASR+TTS back on the GPU next to a slightly
# smaller-context 30B.
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

echo "[realtime] ASR distil-small.en on GPU (:18878)…" >&2
ASR_MODEL="${ASR_MODEL:-distil-small.en}" ASR_DEVICE=cuda ASR_COMPUTE=float16 \
  setsid nohup "$SPEECH/.venv/bin/python" "$SPEECH/asr_sidecar.py" >/tmp/asr.log 2>&1 < /dev/null &

echo "[realtime] Kokoro TTS on GPU (:18901)…" >&2
TTS_DEVICE=cuda setsid nohup "$SPEECH/.venv-tts/bin/python" "$SPEECH/tts_sidecar.py" >/tmp/tts.log 2>&1 < /dev/null &

echo "[realtime] Qwen3-30B-A3B on GPU (:18902, ctx 8192, util 0.80)…" >&2
LLM_GPU_UTIL=0.80 LLM_MAX_LEN=8192 setsid nohup "$HERE/run-llm.sh" >/tmp/llm.log 2>&1 < /dev/null &

echo "[realtime] starting; the 30B takes ~1-2 min to load. Watch: tail -f /tmp/llm.log" >&2
echo "[realtime] health: curl :18878/health (ASR) :18901/health (TTS) :18902/health (LLM)" >&2

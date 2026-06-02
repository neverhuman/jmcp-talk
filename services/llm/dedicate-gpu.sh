#!/usr/bin/env bash
# Toggle the 3090 between "GPU-dedicated to the reasoning model" and "co-located".
#
#   dedicate-gpu.sh dedicate   # move the speech sidecars to CPU -> GPU is 100% the LLM's
#   dedicate-gpu.sh colocate   # move the speech sidecars back onto the GPU
#
# This is a pure restart with different ASR_DEVICE/TTS_DEVICE env — no code edits.
# faster-whisper int8 + Kokoro on CPU stay faster-than-realtime for short voice turns.
set -Eeuo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SPEECH="$HERE/../speech"
MODE="${1:-dedicate}"

pkill -f asr_sidecar.py 2>/dev/null || true
pkill -f tts_sidecar.py 2>/dev/null || true
sleep 1

if [[ "$MODE" == "dedicate" ]]; then
  echo "[gpu] speech -> CPU; GPU dedicated to the reasoning model"
  ASR_DEVICE=cpu ASR_COMPUTE=int8 setsid nohup "$SPEECH/run-asr.sh" >/tmp/asr.log 2>&1 < /dev/null &
  TTS_DEVICE=cpu setsid nohup "$SPEECH/run-tts.sh" >/tmp/tts.log 2>&1 < /dev/null &
elif [[ "$MODE" == "colocate" ]]; then
  echo "[gpu] speech -> GPU (co-located with a small ~14B model)"
  setsid nohup "$SPEECH/run-asr.sh" >/tmp/asr.log 2>&1 < /dev/null &
  setsid nohup "$SPEECH/run-tts.sh" >/tmp/tts.log 2>&1 < /dev/null &
else
  echo "usage: $0 {dedicate|colocate}" >&2; exit 2
fi
echo "[gpu] speech restarting in '$MODE' mode (ASR :18878, TTS :18901)"

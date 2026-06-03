#!/usr/bin/env bash
# Round-trip self-test: TTS synthesizes a known phrase, ASR transcribes it back,
# and we assert the text round-trips (case/punct-insensitive). Requires both
# sidecars running (run-asr.sh + run-tts.sh). Exits non-zero on mismatch.
set -Eeuo pipefail
ASR="${ASR_URL:-http://127.0.0.1:18878}"
TTS="${TTS_URL:-http://127.0.0.1:18901}"
PHRASE="${1:-Master control plane online. The autonomous dispatcher is running.}"

curl -sf "$ASR/health" | grep -q '"loaded": true' || { echo "ASR not loaded at $ASR" >&2; exit 1; }
curl -sf "$TTS/health" | grep -q '"loaded": true' || { echo "TTS not loaded at $TTS" >&2; exit 1; }

wav="$(mktemp --suffix=.wav)"
trap 'rm -f "$wav"' EXIT
curl -sf -X POST "$TTS/synthesize" -H 'content-type: application/json' \
  -d "$(python3 -c 'import json,sys;print(json.dumps({"text":sys.argv[1]}))' "$PHRASE")" -o "$wav"
heard="$(curl -sf -X POST --data-binary @"$wav" "$ASR/transcribe?language=en&beam_size=${ASR_BEAM_SIZE:-1}" \
  -H 'content-type: audio/wav' | python3 -c 'import sys,json;print(json.load(sys.stdin)["text"])')"

norm() { printf '%s' "$1" | tr 'A-Z' 'a-z' | tr -cd 'a-z0-9 ' | tr -s ' '; }
if [[ "$(norm "$heard")" == "$(norm "$PHRASE")" ]]; then
  echo "[selftest] OK round-trip: \"$heard\""
else
  echo "[selftest] MISMATCH" >&2
  echo "  sent:  $PHRASE" >&2
  echo "  heard: $heard" >&2
  exit 1
fi

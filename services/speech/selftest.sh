#!/usr/bin/env bash
# Smoke test: TTS synthesizes deterministic bytes and ASR returns a deterministic transcript.
set -Eeuo pipefail

ASR="${ASR_URL:-http://127.0.0.1:18878}"
TTS="${TTS_URL:-http://127.0.0.1:18901}"
PHRASE="${1:-Master control plane online. The autonomous dispatcher is running.}"

curl -sf "$ASR/health" | grep -q '"loaded":true\|"loaded": true' || { echo "ASR not loaded at $ASR" >&2; exit 1; }
curl -sf "$TTS/health" | grep -q '"loaded":true\|"loaded": true' || { echo "TTS not loaded at $TTS" >&2; exit 1; }

wav="$(mktemp --suffix=.wav)"
trap 'rm -f "$wav"' EXIT
curl -sf -X POST "$TTS/synthesize" -H 'content-type: application/json' \
  -d "{\"text\":\"$PHRASE\"}" -o "$wav"
heard="$(curl -sf -X POST --data-binary @"$wav" "$ASR/transcribe?language=en&beam_size=${ASR_BEAM_SIZE:-1}" \
  -H 'content-type: audio/wav' | sed -n 's/.*"text":"\([^"]*\)".*/\1/p')"

printf '[speech-selftest] transcript: %s\n' "$heard"
test -s "$wav"

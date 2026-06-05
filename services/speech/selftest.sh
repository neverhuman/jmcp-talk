#!/usr/bin/env bash
# Live speech smoke test: TTS synthesizes a phrase, ASR must hear that phrase.
set -Eeuo pipefail

ASR="${ASR_URL:-http://127.0.0.1:18878}"
TTS="${TTS_URL:-http://127.0.0.1:18901}"
PHRASE="${1:-master control plane online}"

json_string() {
  python3 - "$1" <<'PY'
import json
import sys
print(json.dumps(sys.argv[1]))
PY
}

normalize() {
  python3 - "$1" <<'PY'
import re
import sys
text = sys.argv[1].lower()
text = re.sub(r"[^a-z0-9]+", " ", text)
print(" ".join(text.split()))
PY
}

json_field() {
  python3 - "$1" "$2" <<'PY'
import json
import sys
value = json.loads(sys.argv[1])
print(value.get(sys.argv[2], ""))
PY
}

curl -sf "$ASR/health" | grep -q '"loaded":true\|"loaded": true' || {
  echo "ASR not loaded at $ASR" >&2
  exit 1
}
curl -sf "$TTS/health" | grep -q '"loaded":true\|"loaded": true' || {
  echo "TTS not loaded at $TTS" >&2
  exit 1
}

wav="$(mktemp --suffix=.wav)"
trap 'rm -f "$wav"' EXIT

curl -sf -X POST "$TTS/synthesize" -H 'content-type: application/json' \
  -d "{\"text\":$(json_string "$PHRASE")}" -o "$wav"
test -s "$wav"

body="$(curl -sf -X POST --data-binary @"$wav" "$ASR/transcribe?language=en&beam_size=${ASR_BEAM_SIZE:-1}" \
  -H 'content-type: audio/wav')"
heard="$(json_field "$body" text)"
expected="$(normalize "$PHRASE")"
actual="$(normalize "$heard")"

printf '[speech-selftest] expected: %s\n' "$expected"
printf '[speech-selftest] transcript: %s\n' "$actual"

if [[ "$actual" != *"$expected"* && "$expected" != *"$actual"* ]]; then
  echo "ASR/TTS round trip mismatch" >&2
  exit 1
fi

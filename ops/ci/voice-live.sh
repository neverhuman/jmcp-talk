#!/usr/bin/env bash
# Live voice proof lane. Requires a GPU runner with the real ASR/TTS/vLLM stack.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

TMP_DIR="$(mktemp -d)"
PIDS=()

cleanup() {
  local status=$?
  for pid in "${PIDS[@]:-}"; do
    if kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
    fi
  done
  for pid in "${PIDS[@]:-}"; do
    wait "$pid" 2>/dev/null || true
  done
  rm -rf "$TMP_DIR"
  exit "$status"
}
trap cleanup EXIT INT TERM

log() {
  printf '[voice-live] %s\n' "$*" >&2
}

json_field() {
  python3 - "$1" "$2" <<'PY'
import json
import sys
value = json.loads(sys.argv[1])
for part in sys.argv[2].split("."):
    if isinstance(value, dict):
        value = value.get(part, "")
    else:
        value = ""
print(value)
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

json_string() {
  python3 - "$1" <<'PY'
import json
import sys
print(json.dumps(sys.argv[1]))
PY
}

speech_roundtrip_probe() {
  local phrase="$1"
  python3 - "$ASR_URL" "$TTS_URL" "$phrase" <<'PY'
import json
import re
import sys
import time
import urllib.parse
import urllib.request

asr_url, tts_url, phrase = sys.argv[1], sys.argv[2], sys.argv[3]

def normalize(text):
    text = text.lower()
    text = re.sub(r"[^a-z0-9]+", " ", text)
    return " ".join(text.split())

payload = json.dumps({"text": phrase}).encode("utf-8")
tts_request = urllib.request.Request(
    tts_url.rstrip("/") + "/synthesize?format=wav",
    data=payload,
    headers={"content-type": "application/json"},
    method="POST",
)
started = time.perf_counter()
tts_started = time.perf_counter()
with urllib.request.urlopen(tts_request, timeout=60) as response:
    audio = response.read()
tts_ended = time.perf_counter()

params = urllib.parse.urlencode({"language": "en", "beam_size": "1"})
asr_request = urllib.request.Request(
    asr_url.rstrip("/") + "/transcribe?" + params,
    data=audio,
    headers={"content-type": "audio/wav"},
    method="POST",
)
asr_started = time.perf_counter()
with urllib.request.urlopen(asr_request, timeout=60) as response:
    body = json.loads(response.read().decode("utf-8"))
asr_ended = time.perf_counter()
elapsed_ms = (asr_ended - started) * 1000
transcript = str(body.get("text", ""))
expected = normalize(phrase)
actual = normalize(transcript)
matched = actual in expected or expected in actual
passed = matched and elapsed_ms <= 2500
print(json.dumps({
    "phrase": phrase,
    "expected": expected,
    "transcript": transcript,
    "actual": actual,
    "matched": matched,
    "ttsMs": round((tts_ended - tts_started) * 1000, 1),
    "asrMs": round((asr_ended - asr_started) * 1000, 1),
    "elapsedMs": round(elapsed_ms, 1),
    "thresholdMs": 2500,
    "passed": passed,
}))
if not passed:
    if not matched:
        raise SystemExit("ASR/TTS round trip mismatch")
    raise SystemExit("ASR/TTS round trip exceeded 2500ms")
PY
}

write_metrics_base() {
  local metrics_file="$1"
  local llm_token="$2"
  local roundtrip_json="$3"
  local stream_url="$4"
  python3 - "$metrics_file" "$JMCP_URL" "$ASR_URL" "$TTS_URL" "$LLM_URL" "$COCKPIT_URL" "$llm_token" "$roundtrip_json" "$stream_url" <<'PY'
import json
import os
import sys
from datetime import datetime, timezone

metrics_file, jmcp_url, asr_url, tts_url, llm_url, cockpit_url, llm_token, roundtrip_json, stream_url = sys.argv[1:]
now = datetime.now(timezone.utc).isoformat()
os.makedirs(os.path.dirname(metrics_file), exist_ok=True)
with open(metrics_file, "w", encoding="utf-8") as handle:
    json.dump({
        "schemaVersion": 1,
        "generatedAt": now,
        "updatedAt": now,
        "serviceChecks": {
            "jmcpHealth": {"url": jmcp_url + "/health", "loaded": True},
            "asrHealth": {"url": asr_url + "/health", "loaded": True},
            "ttsHealth": {"url": tts_url + "/health", "loaded": True},
            "vllmModels": {"url": llm_url + "/v1/models", "reachable": True},
            "firstStreamedLlmToken": llm_token,
            "speechRoundTrip": json.loads(roundtrip_json),
            "jituxFirstSseFrame": {"streamUrl": stream_url, "received": True},
            "cockpit": {"url": cockpit_url},
        },
        "runs": [],
        "summary": {
            "passed": False,
            "totalRuns": 0,
            "failedRuns": 0,
            "byMode": {},
            "failures": [],
        },
    }, handle, indent=2)
    handle.write("\n")
PY
}

make_fake_mic() {
  local phrase="$1"
  local raw_path="$2"
  local final_path="$3"
  curl -fsS -X POST "$TTS_URL/synthesize?format=wav" -H 'content-type: application/json' \
    -d "{\"text\":$(json_string "$phrase")}" -o "$raw_path"
  append_wav_silence "$raw_path" "$final_path"
}

run_voice_spec() {
  local mode="$1"
  local wav_path="$2"
  local iteration="$3"
  local run_id="${mode}-${iteration}"
  log "running voice spec mode=$mode iteration=$iteration"
  if [[ "$mode" == "typed" ]]; then
    env JMCP_VOICE_LIVE_BASE_URL="$COCKPIT_URL" \
      JMCP_VOICE_LIVE_MODE="$mode" \
      JMCP_VOICE_LIVE_ITERATION="$iteration" \
      JMCP_VOICE_LIVE_RUN_ID="$run_id" \
      JMCP_VOICE_LIVE_METRICS="$VOICE_METRICS" \
      npm --workspace @jmcp/cockpit run test:e2e -- e2e/voice-live.spec.ts --project=chromium
  else
    env JMCP_VOICE_LIVE_BASE_URL="$COCKPIT_URL" \
      JMCP_VOICE_LIVE_MODE="$mode" \
      JMCP_VOICE_LIVE_ITERATION="$iteration" \
      JMCP_VOICE_LIVE_RUN_ID="$run_id" \
      JMCP_VOICE_LIVE_WAV="$wav_path" \
      JMCP_VOICE_LIVE_METRICS="$VOICE_METRICS" \
      npm --workspace @jmcp/cockpit run test:e2e -- e2e/voice-live.spec.ts --project=chromium
  fi
}

validate_voice_metrics() {
  local metrics_file="$1"
  python3 - "$metrics_file" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as handle:
    receipt = json.load(handle)
summary = receipt.get("summary", {})
by_mode = summary.get("byMode", {})
failures = []
if summary.get("passed") is not True:
    failures.append("metrics summary is not passed")
expected = {"local": 3, "model": 3, "typed": 1}
for mode, count in expected.items():
    actual = by_mode.get(mode, {}).get("runs", 0)
    if actual != count:
        failures.append(f"expected {count} {mode} run(s), got {actual}")
service = receipt.get("serviceChecks", {})
roundtrip = service.get("speechRoundTrip", {})
if roundtrip.get("passed") is not True:
    failures.append("ASR/TTS round trip check did not pass")
if failures:
    for failure in failures:
        print(failure, file=sys.stderr)
    raise SystemExit(1)
print(json.dumps({
    "runs": summary.get("totalRuns"),
    "failedRuns": summary.get("failedRuns"),
    "speechRoundTripMs": roundtrip.get("elapsedMs"),
    "metrics": path,
}))
PY
}

wait_for_url() {
  local name="$1"
  local url="$2"
  local timeout="${3:-180}"
  local start
  start="$(date +%s)"
  until curl -fsS "$url" >/dev/null 2>&1; do
    if (( "$(date +%s)" - start > timeout )); then
      log "$name did not become reachable at $url"
      return 1
    fi
    sleep 2
  done
}

wait_for_loaded() {
  local name="$1"
  local url="$2"
  local timeout="${3:-300}"
  local start body loaded ok
  start="$(date +%s)"
  while true; do
    body="$(curl -fsS "$url" 2>/dev/null || true)"
    if [[ -n "$body" ]]; then
      loaded="$(json_field "$body" loaded 2>/dev/null || true)"
      ok="$(json_field "$body" ok 2>/dev/null || true)"
      if [[ "$loaded" == "True" || "$loaded" == "true" ]] && [[ "$ok" != "False" && "$ok" != "false" ]]; then
        return 0
      fi
    fi
    if (( "$(date +%s)" - start > timeout )); then
      log "$name did not report loaded=true at $url"
      [[ -n "${body:-}" ]] && log "$name last health: $body"
      return 1
    fi
    sleep 2
  done
}

start_bg() {
  local name="$1"
  local log_file="$2"
  shift 2
  log "starting $name; log: $log_file"
  "$@" >"$log_file" 2>&1 &
  PIDS+=("$!")
}

first_streamed_token() {
  local llm_url="$1"
  local model="$2"
  local payload
  payload="$(python3 - "$model" <<'PY'
import json
import sys
print(json.dumps({
    "model": sys.argv[1],
    "messages": [{"role": "user", "content": "Reply with exactly one word: ready"}],
    "temperature": 0,
    "max_tokens": 8,
    "stream": True,
}))
PY
)"
  python3 - "$llm_url" "$payload" <<'PY'
import json
import sys
import urllib.request

url = sys.argv[1].rstrip("/") + "/v1/chat/completions"
payload = sys.argv[2].encode()
request = urllib.request.Request(
    url,
    data=payload,
    headers={"content-type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(request, timeout=60) as response:
    for raw in response:
        line = raw.decode("utf-8", errors="replace").strip()
        if not line.startswith("data:"):
            continue
        data = line[5:].strip()
        if data == "[DONE]" or not data:
            continue
        chunk = json.loads(data)
        delta = chunk.get("choices", [{}])[0].get("delta", {}).get("content", "")
        if delta.strip():
            print(delta.strip())
            sys.exit(0)
raise SystemExit("no streamed token received")
PY
}

append_wav_silence() {
  local src="$1"
  local dst="$2"
  python3 - "$src" "$dst" <<'PY'
import wave
import sys

src, dst = sys.argv[1], sys.argv[2]
with wave.open(src, "rb") as inp:
    params = inp.getparams()
    frames = inp.readframes(inp.getnframes())
with wave.open(dst, "wb") as out:
    out.setparams(params)
    out.writeframes(frames)
    silence = b"\0" * params.nchannels * params.sampwidth * params.framerate
    out.writeframes(silence)
PY
}

first_sse_frame() {
  local url="$1"
  python3 - "$url" <<'PY'
import sys
import urllib.request

with urllib.request.urlopen(sys.argv[1], timeout=20) as response:
    for raw in response:
        line = raw.decode("utf-8", errors="replace").strip()
        if line.startswith("data:"):
            print(line)
            sys.exit(0)
raise SystemExit("no SSE data frame received")
PY
}

JMCP_HOST="${JMCP_HOST:-127.0.0.1}"
JMCP_PORT="${JMCP_PORT:-18977}"
ASR_PORT="${ASR_PORT:-18978}"
TTS_PORT="${TTS_PORT:-18979}"
LLM_PORT="${LLM_PORT:-18980}"
COCKPIT_PORT="${COCKPIT_PORT:-15978}"
LLM_SERVED_NAME="${LLM_SERVED_NAME:-local/qwen3-30b-a3b}"
VOICE_TIMEOUT="${JMCP_VOICE_LIVE_TIMEOUT_SECS:-900}"
VOICE_RUNS="${JMCP_VOICE_LIVE_RUNS:-3}"
VOICE_METRICS="${JMCP_VOICE_LIVE_METRICS:-$ROOT/target/jankurai/voice-live/voice-live-metrics.json}"
case "$VOICE_METRICS" in
  /*) ;;
  *) VOICE_METRICS="$ROOT/$VOICE_METRICS" ;;
esac

JMCP_URL="http://${JMCP_HOST}:${JMCP_PORT}"
ASR_URL="http://${JMCP_HOST}:${ASR_PORT}"
TTS_URL="http://${JMCP_HOST}:${TTS_PORT}"
LLM_URL="http://${JMCP_HOST}:${LLM_PORT}"
COCKPIT_URL="http://${JMCP_HOST}:${COCKPIT_PORT}"
mkdir -p "$(dirname "$VOICE_METRICS")"
rm -f "$VOICE_METRICS"

start_bg "jmcpd" "$TMP_DIR/jmcpd.log" \
  env JMCP_API_BIND="${JMCP_HOST}:${JMCP_PORT}" \
  cargo run --quiet -p jmcpd -- --database "$TMP_DIR/jmcp.db"
wait_for_url "jmcpd" "$JMCP_URL/health" 120

start_bg "ASR" "$TMP_DIR/asr.log" \
  env ASR_BIND="${JMCP_HOST}:${ASR_PORT}" ASR_MODEL="${ASR_MODEL:-distil-small.en}" \
    ASR_DEVICE="${ASR_DEVICE:-cuda}" ASR_COMPUTE="${ASR_COMPUTE:-float16}" \
    ASR_BEAM_SIZE="${ASR_BEAM_SIZE:-1}" \
    services/speech/run-asr.sh

start_bg "TTS" "$TMP_DIR/tts.log" \
  env TTS_BIND="${JMCP_HOST}:${TTS_PORT}" TTS_DEVICE="${TTS_DEVICE:-cuda}" \
    services/speech/run-tts.sh

start_bg "vLLM" "$TMP_DIR/llm.log" \
  env LLM_PORT="$LLM_PORT" LLM_SERVED_NAME="$LLM_SERVED_NAME" \
    LLM_GPU_UTIL="${LLM_GPU_UTIL:-0.80}" LLM_MAX_LEN="${LLM_MAX_LEN:-8192}" \
    services/llm/run-llm.sh

wait_for_loaded "ASR" "$ASR_URL/health" "$VOICE_TIMEOUT"
wait_for_loaded "TTS" "$TTS_URL/health" "$VOICE_TIMEOUT"
wait_for_url "vLLM models" "$LLM_URL/v1/models" "$VOICE_TIMEOUT"

token="$(first_streamed_token "$LLM_URL" "$LLM_SERVED_NAME")"
log "first streamed LLM token: $token"

roundtrip_json="$(speech_roundtrip_probe "master control plane online")"
log "ASR/TTS round trip: $roundtrip_json"

session_body="$(curl -fsS -X POST "$JMCP_URL/jitux/sessions" \
  -H 'content-type: application/json' \
  -d '{"prompt":"what is blocking the queue?","source":"voice-live"}')"
stream_url="$(json_field "$session_body" streamUrl)"
if [[ -z "$stream_url" ]]; then
  log "JITUX session did not return streamUrl: $session_body"
  exit 1
fi
first_sse_frame "$JMCP_URL$stream_url" >/dev/null
log "JITUX first SSE frame received"

write_metrics_base "$VOICE_METRICS" "$token" "$roundtrip_json" "$stream_url"

local_fake_raw="$TMP_DIR/fake-mic-local.raw.wav"
local_fake_wav="$TMP_DIR/fake-mic-local.wav"
model_fake_raw="$TMP_DIR/fake-mic-model.raw.wav"
model_fake_wav="$TMP_DIR/fake-mic-model.wav"
make_fake_mic "how is JMCP doing" "$local_fake_raw" "$local_fake_wav"
make_fake_mic "explain the current mission in one short sentence" "$model_fake_raw" "$model_fake_wav"

start_bg "cockpit" "$TMP_DIR/cockpit.log" \
  env JMCP_COCKPIT_HOST="$JMCP_HOST" JMCP_COCKPIT_PORT="$COCKPIT_PORT" \
    VITE_ASR_TARGET="$ASR_URL" VITE_TTS_TARGET="$TTS_URL" VITE_LLM_TARGET="$LLM_URL" \
    VITE_LLM_MODEL="$LLM_SERVED_NAME" VITE_JMCP_TARGET="$JMCP_URL" VITE_JMCP_API_URL="$JMCP_URL" VITE_JMCP_BASE="/jmcp" \
    npm --workspace @jmcp/cockpit run dev -- --host "$JMCP_HOST" --port "$COCKPIT_PORT" --strictPort
wait_for_url "cockpit" "$COCKPIT_URL" 120

for iteration in $(seq 1 "$VOICE_RUNS"); do
  run_voice_spec "local" "$local_fake_wav" "$iteration"
done

for iteration in $(seq 1 "$VOICE_RUNS"); do
  run_voice_spec "model" "$model_fake_wav" "$iteration"
done

run_voice_spec "typed" "" "1"

validate_voice_metrics "$VOICE_METRICS"

log "voice live lane passed; metrics: $VOICE_METRICS"

# JMCP local reasoning and live voice sidecars

## Local Voice Gateway

Live cockpit voice is routed through the Rust `jmcp-voiced` daemon in
`jmcp-talk` on a loopback port. The stable local contract is:

| Component | Bind | Notes |
|---|---:|---|
| JMCP voice gateway | `127.0.0.1:8040` | cockpit proxy target for `/voice` and `/voice-ws` |
| ASR sidecar | `127.0.0.1:18878` | local speech-to-text |
| TTS sidecar | `127.0.0.1:18901` | VoxCPM2 `jmcp_male_v1`, Kokoro degraded mode |
| LLM sidecar | `127.0.0.1:18902` | OpenAI-compatible `/v1` text reasoning |

```bash
./services/llm/realtime-voice.sh
curl http://127.0.0.1:8040/health
```

The default speaking profile is `jmcp_male_v1`, a VoxCPM2 voice-design profile
stored under `services/speech/voice_profiles`. It is not derived from MiniCPM
demo reference audio. `/health` reports `voice_engine`, `voice_profile`,
`voice_profile_hash`, `sample_rate`, `streaming_audio`, `tts_rtf_p50`, and
degraded status; it never returns audio bytes. Full transcripts are redacted in
logs; raw audio snippets are kept locally while `JMCP_TALK_CAPTURE_RAW_AUDIO=1`.

During live audio debugging, raw capture is enabled by default:

```bash
export JMCP_TALK_AUDIO_DIR=/home/ubuntu/jmcp-split/.live/audio
export JMCP_TALK_CAPTURE_RAW_AUDIO=1
export JMCP_TALK_AUDIO_RETENTION_DAYS=7
export JMCP_TALK_AUDIO_MAX_MB=2048
```

The gateway writes per-turn WAV snippets and `events.jsonl` to that folder so
choppy playback can be inspected after the run. Set
`JMCP_TALK_CAPTURE_RAW_AUDIO=0` to disable local raw snippet capture.

## MiniCPM-o 4.5 Debug Lane

MiniCPM/Comni remains available for explicit degraded/debug work. Keep it off
the cockpit gateway port unless you are intentionally replacing the local voice
path.

```bash
JMCP_TALK_MINICPM_DOWNLOAD=1 ./services/llm/setup-minicpm-o45.sh
JMCP_TALK_MINICPM_BIND=127.0.0.1:8041 ./services/llm/run-minicpm-o45.sh
```

The default GGUF artifact is `openbmb/MiniCPM-o-4_5-gguf` with
`MiniCPM-o-4_5-Q4_K_M.gguf`, `ctx_size=8192`, and `n_gpu_layers=99`.

## Legacy vLLM reasoning sidecar

The "brain": a strong local reasoning LLM served on the RTX 3090 via **vLLM** with an
**OpenAI-compatible `/v1` API** — the exact shape jnoccio's providers and
`jmcp-adapter-jekko` already speak, so wiring it in needs **no Rust changes**.

| | Model | Port | VRAM |
|---|---|---|---|
| **Realtime voice primary** | `Qwen/Qwen2.5-7B-Instruct-AWQ` (Apache-2.0) | `127.0.0.1:18902` | `LLM_GPU_UTIL=0.62`, ctx 8192, co-resident with ASR/TTS |
| **Standalone reasoning** | same 7B | `127.0.0.1:18902` | default `run-llm.sh` profile, ctx 32768 |

Port 18902 is JMCP-safe (never a Jeryu-protected port). Weights download to the HF
cache (`~/.cache/huggingface`, outside the repo); the venv + any local weights are git-ignored.

## Run

```bash
# Realtime Cockpit voice stack: ASR distil-small.en + VoxCPM2/Kokoro degraded mode + 7B + gateway.
./services/llm/realtime-voice.sh

# Standalone 7B reasoning only (first run installs vLLM + downloads the model):
./services/llm/run-llm.sh
```

Config via env (see `run-llm.sh`): `LLM_MODEL`, `LLM_SERVED_NAME`, `LLM_PORT`,
`LLM_GPU_UTIL`, `LLM_MAX_LEN`, `LLM_QUANT`. `realtime-voice.sh` sets
`LLM_GPU_UTIL=0.62` and `LLM_MAX_LEN=8192` unless explicitly overridden.

## Verify

```bash
curl -s http://127.0.0.1:18902/health
curl -s http://127.0.0.1:18902/v1/chat/completions -H 'content-type: application/json' \
  -d '{"model":"local/qwen3-30b-a3b","messages":[{"role":"user","content":"In one sentence, what is JMCP?"}]}'
./services/llm/smoke-real-tts.py --runs 5 --strict
./services/llm/smoke-real-voice-turn.py --mode both --runs 3 --strict
nvidia-smi   # realtime voice should leave headroom for ASR/TTS
```

The smoke scripts are live-only checks. They write local WAV artifacts,
stripped timing JSONL, `summary.json`, and a listening packet under
`/home/ubuntu/jmcp-split/.live/audio`; they are not part of default CI.

## Wire into JMCP / jnoccio

Add to `~/jnoccio/config/router.toml` (same pattern as `[providers.inception]`):

```toml
[providers.local_vllm]
enabled  = true
api_base = "http://127.0.0.1:18902/v1"
models   = ["local/qwen3-30b-a3b"]    # must equal --served-model-name
```

Then a JMCP `reason` work order with `JEKKO_MODEL=local/qwen3-30b-a3b` routes to the
local model (`jmcp-adapter-jekko` POSTs to `{jnoccio}/v1/chat/completions`).

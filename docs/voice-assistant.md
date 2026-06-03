# JMCP Local Voice Assistant + Reasoning Model

A fully on-box, private voice assistant. The browser captures the microphone, a
local ASR sidecar transcribes, a local reasoning LLM answers, and a local TTS
sidecar speaks the reply back. No audio or text ever leaves the machine.

The fast path is the co-resident GPU profile launched by
`services/llm/realtime-voice.sh`: Qwen3-30B-A3B at context 8192, ASR
`distil-small.en` on CUDA float16 with beam 1, and Kokoro TTS on CUDA.

## 1. Architecture

```
browser mic
   |  (continuous capture)
   v
energy VAD  ──segments each utterance──>  local ASR  (distil-small.en, :18878)
   |                                            |
   |                                       transcript text
   v                                            v
spoken command  ───────────────────>  local vLLM reasoning  (:18902)
                                            |
                                       one/two-sentence reply
                                            v
                                     local TTS  (Kokoro, :18901)
                                            |
                                         playback
```

- The mic runs continuously in the browser. A lightweight energy VAD (RMS
  threshold) decides where each spoken utterance starts and ends.
- Once the widget is active, each spoken turn is treated as a command. The
  wake-word parser remains tested for compatibility, but the cockpit fast path
  avoids an extra activation turn.
- The command is sent to the local vLLM `/v1/chat/completions` endpoint with a
  short system prompt asking for one or two spoken sentences.
- The reply is synthesized by the local TTS sidecar and played back.
- Barge-in: talking over the assistant cancels playback immediately and starts
  capturing your next utterance.

The widget is a floating mic control mounted only on the standalone cockpit. Its
states are: voice off, listening, transcribing, thinking, speaking, and error.

## 2. Bring-up

```bash
# 1. Start the realtime GPU voice stack: ASR + TTS + Qwen3-30B-A3B.
#    First run creates venvs and downloads weights into the shared HF cache.
./services/llm/realtime-voice.sh

# 2. Start the cockpit web UI (defaults to 127.0.0.1:15873):
npm --workspace @jmcp/cockpit run dev
```

Then open the cockpit in a browser, click the floating mic widget to start, and speak.

The widget shows what it heard and the spoken reply as text alongside the audio.

The cockpit reaches the three local services through the Vite dev proxy
(`/asr`, `/tts`, `/llm`), so the browser stays same-origin with no CORS and audio
stays on the machine.

## 3. Ports

| Service | Bind | Notes |
|---|---|---|
| ASR (faster-whisper) | `127.0.0.1:18878` | `GET /health`, `POST /transcribe` |
| TTS (Kokoro) | `127.0.0.1:18901` | `GET /health`, `POST /synthesize` |
| Reasoning LLM (vLLM) | `127.0.0.1:18902` | OpenAI-compatible `/v1` API |
| Cockpit (Vite dev) | `127.0.0.1:15873` | proxies `/asr`, `/tts`, `/llm` |

All four are JMCP-safe ports. None of them is ever a Jeryu-protected port
(`2224`, `8787`, `8799`, `8929`, `18787`, `18788`, `19800`); the cockpit refuses
to start on any of those.

## 4. GPU / VRAM

This box is a single RTX 3090 (24 GB). The realtime voice launcher keeps the
30B-A3B AWQ model at `LLM_GPU_UTIL=0.80` and `LLM_MAX_LEN=8192`, leaving enough
headroom for ASR `distil-small.en` and Kokoro TTS to stay on CUDA.

For an explicit accuracy or isolation run, you can still move speech off the GPU
or choose a larger ASR model manually:

```bash
./services/llm/dedicate-gpu.sh dedicate
ASR_MODEL=large-v3 ASR_BEAM_SIZE=5 ./services/speech/run-asr.sh
```

## 5. Configuration knobs

LLM sidecar (`services/llm/run-llm.sh`):

| Env | Default | Meaning |
|---|---|---|
| `LLM_MODEL` | `cpatonn/Qwen3-30B-A3B-Instruct-2507-AWQ-4bit` | model repo vLLM serves |
| `LLM_SERVED_NAME` | `local/qwen3-30b-a3b` | name clients send as `model` |
| `LLM_PORT` | `18902` | bind port |
| `LLM_GPU_UTIL` | `0.92` (`0.80` in realtime launcher) | GPU memory fraction for vLLM |
| `LLM_MAX_LEN` | `32768` (`8192` in realtime launcher) | max context length |
| `LLM_QUANT` | _unset_ | force a quantization kernel (e.g. `awq_marlin`) |

Speech device placement (`dedicate-gpu.sh` / `run-asr.sh` / `run-tts.sh`):

| Env | Meaning |
|---|---|
| `ASR_MODEL` | default `distil-small.en`; set `large-v3` only for accuracy overrides |
| `ASR_DEVICE` | `cuda` in realtime launcher or `cpu` for dedicated/isolation mode |
| `ASR_COMPUTE` | `float16` on CUDA, `int8` on CPU |
| `ASR_BEAM_SIZE` | default `1`; set higher only for accuracy overrides |
| `TTS_DEVICE` | `cuda` in realtime launcher, `auto` in standalone runner, or `cpu` for dedicated/isolation mode |

Cockpit proxy targets (`apps/cockpit/vite.config.ts`) and voice model:

| Env | Default | Meaning |
|---|---|---|
| `VITE_ASR_TARGET` | `http://127.0.0.1:18878` | where `/asr` is proxied |
| `VITE_TTS_TARGET` | `http://127.0.0.1:18901` | where `/tts` is proxied |
| `VITE_LLM_TARGET` | `http://127.0.0.1:18902` | where `/llm` is proxied |
| `VITE_LLM_MODEL` | `local/qwen3-30b-a3b` | `model` the voice client sends |

`VITE_LLM_MODEL` must match `LLM_SERVED_NAME` so vLLM accepts the request. The
cockpit host and port come from `JMCP_COCKPIT_HOST` (`127.0.0.1`) and
`JMCP_COCKPIT_PORT` (`15873`).

## 6. Verify

```bash
# LLM up?
curl -s http://127.0.0.1:18902/health

# One reasoning turn (served name must match LLM_SERVED_NAME / VITE_LLM_MODEL):
curl -s http://127.0.0.1:18902/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"local/qwen3-30b-a3b","messages":[{"role":"user","content":"In one sentence, what is JMCP?"}]}'

# Speech sidecars up?
curl -s http://127.0.0.1:18878/health
curl -s http://127.0.0.1:18901/health

# Round-trip check:
./services/speech/selftest.sh

# GPU occupancy — realtime mode should retain at least about 1 GB headroom:
nvidia-smi
```

## 7. Routing JMCP `reason` work orders to the local model

The same local vLLM endpoint can serve JMCP's own `reason` work orders, with no
Rust changes. Add a provider block to `~/jnoccio/config/router.toml`:

```toml
[providers.local_vllm]
enabled  = true
api_base = "http://127.0.0.1:18902/v1"
models   = ["local/qwen3-30b-a3b"]   # must equal --served-model-name
```

Then a JMCP `reason` work order with `JEKKO_MODEL=local/qwen3-30b-a3b` routes to
the local model: `jmcp-adapter-jekko` POSTs to
`{jnoccio}/v1/chat/completions`.

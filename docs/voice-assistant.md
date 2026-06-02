# JMCP Local Voice Assistant + Reasoning Model

A fully on-box, private voice assistant. The browser captures the microphone, a
local ASR sidecar transcribes, a local reasoning LLM answers, and a local TTS
sidecar speaks the reply back. No audio or text ever leaves the machine.

The "brain" is a strong reasoning LLM (vLLM serving an OpenAI-compatible `/v1`
API). The default is the GPU-dedicated 30B MoE; a smaller 14B alternative
co-locates on the GPU alongside the speech sidecars when you want everything on
one card at once.

## 1. Architecture

```
browser mic
   |  (continuous capture)
   v
energy VAD  ──segments each utterance──>  local ASR  (faster-whisper, :18878)
   |                                            |
   |                                       transcript text
   v                                            v
wake word "hey JMCP"  ── command ──>  local vLLM reasoning  (:18902)
                                            |
                                       one/two-sentence reply
                                            v
                                     local TTS  (Kokoro, :18901)
                                            |
                                         playback
```

- The mic runs continuously in the browser. A lightweight energy VAD (RMS
  threshold) decides where each spoken utterance starts and ends.
- While idle the assistant only acts when the transcript contains the wake word
  (`hey jmcp`, with `hey jim cp`, `jmcp`, and `computer` also accepted). The
  words after the wake word become the command; if you say only the wake word,
  the next utterance is treated as the command.
- The command is sent to the local vLLM `/v1/chat/completions` endpoint with a
  short system prompt asking for one or two spoken sentences.
- The reply is synthesized by the local TTS sidecar and played back.
- Barge-in: talking over the assistant cancels playback immediately and starts
  capturing your next utterance.

The widget is a floating mic control mounted only on the standalone cockpit. Its
states are: voice off, listening (waiting for the wake word), armed (heard the
wake word, capturing the command), transcribing, thinking, speaking.

## 2. Bring-up

```bash
# 1. Dedicate the GPU to the reasoning model (moves speech sidecars to CPU):
./services/llm/dedicate-gpu.sh dedicate

# 2. Serve the 30B reasoning model. The first run installs vLLM and downloads
#    ~17GB of weights into the shared HF cache (~/.cache/huggingface, git-ignored):
./services/llm/run-llm.sh

# 3. Start the cockpit web UI (defaults to 127.0.0.1:15873):
npm --workspace @jmcp/cockpit run dev
```

Then open the cockpit in a browser, click the floating mic widget to start, and
say:

> "hey JMCP, &lt;your question&gt;"

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

This box is a single RTX 3090 (24 GB). The 30B-A3B AWQ weights are ~17GB
(~20–21 GB once the KV cache is included at 32K context), so the reasoning model
wants the GPU to itself. `dedicate-gpu.sh dedicate` restarts the speech sidecars
on CPU (faster-whisper int8 + Kokoro stay faster-than-real-time for short voice
turns), freeing the whole card for vLLM.

When you would rather keep the speech sidecars on the GPU, run the co-located 14B
alternative instead:

```bash
./services/llm/dedicate-gpu.sh colocate   # speech back on the GPU
LLM_MODEL=Qwen/Qwen2.5-Coder-14B-Instruct-AWQ \
LLM_SERVED_NAME=local/qwen2.5-coder-14b \
LLM_GPU_UTIL=0.55 LLM_MAX_LEN=16384 \
  ./services/llm/run-llm.sh
```

`dedicate-gpu.sh colocate` moves ASR and TTS back onto the GPU; the lower
`LLM_GPU_UTIL=0.55` leaves room for both speech sidecars to share the card with
the 14B model.

## 5. Configuration knobs

LLM sidecar (`services/llm/run-llm.sh`):

| Env | Default | Meaning |
|---|---|---|
| `LLM_MODEL` | `cpatonn/Qwen3-30B-A3B-Instruct-2507-AWQ-4bit` | model repo vLLM serves |
| `LLM_SERVED_NAME` | `local/qwen3-30b-a3b` | name clients send as `model` |
| `LLM_PORT` | `18902` | bind port |
| `LLM_GPU_UTIL` | `0.92` | GPU memory fraction for vLLM |
| `LLM_MAX_LEN` | `32768` | max context length |
| `LLM_QUANT` | _unset_ | force a quantization kernel (e.g. `awq_marlin`) |

Speech device placement (`dedicate-gpu.sh` / `run-asr.sh` / `run-tts.sh`):

| Env | Meaning |
|---|---|
| `ASR_DEVICE` | `cuda` (co-located) or `cpu` (dedicated mode) |
| `TTS_DEVICE` | `auto`/`cuda` (co-located) or `cpu` (dedicated mode) |

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

# GPU occupancy — in dedicated mode vLLM owns the card and speech is on CPU:
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

# JMCP local reasoning-model sidecar (vLLM)

The "brain": a strong local reasoning LLM served on the RTX 3090 via **vLLM** with an
**OpenAI-compatible `/v1` API** — the exact shape jnoccio's providers and
`jmcp-adapter-jekko` already speak, so wiring it in needs **no Rust changes**.

| | Model | Port | VRAM |
|---|---|---|---|
| **Realtime voice primary** | `cpatonn/Qwen3-30B-A3B-Instruct-2507-AWQ-4bit` (MoE 30B/3B-active, Apache-2.0) | `127.0.0.1:18902` | `LLM_GPU_UTIL=0.80`, ctx 8192, co-resident with ASR/TTS |
| **Standalone reasoning** | same 30B | `127.0.0.1:18902` | default `run-llm.sh` profile, ctx 32768 |

Port 18902 is JMCP-safe (never a Jeryu-protected port). Weights download to the HF
cache (`~/.cache/huggingface`, outside the repo); the venv + any local weights are git-ignored.

## Run

```bash
# Realtime Cockpit voice stack: ASR distil-small.en + Kokoro + 30B.
./services/llm/realtime-voice.sh

# Standalone 30B reasoning only (first run installs vLLM + downloads ~17GB):
./services/llm/run-llm.sh
```

Config via env (see `run-llm.sh`): `LLM_MODEL`, `LLM_SERVED_NAME`, `LLM_PORT`,
`LLM_GPU_UTIL`, `LLM_MAX_LEN`, `LLM_QUANT`. `realtime-voice.sh` sets
`LLM_GPU_UTIL=0.80` and `LLM_MAX_LEN=8192` unless explicitly overridden.

## Verify

```bash
curl -s http://127.0.0.1:18902/health
curl -s http://127.0.0.1:18902/v1/chat/completions -H 'content-type: application/json' \
  -d '{"model":"local/qwen3-30b-a3b","messages":[{"role":"user","content":"In one sentence, what is JMCP?"}]}'
nvidia-smi   # realtime voice should leave headroom for ASR/TTS
```

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

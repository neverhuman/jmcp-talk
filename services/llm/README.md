# JMCP local reasoning-model sidecar (vLLM)

The "brain": a strong local reasoning LLM served on the RTX 3090 via **vLLM** with an
**OpenAI-compatible `/v1` API** — the exact shape jnoccio's providers and
`jmcp-adapter-jekko` already speak, so wiring it in needs **no Rust changes**.

| | Model | Port | VRAM |
|---|---|---|---|
| **Primary (max reasoning, GPU-dedicated)** | `cpatonn/Qwen3-30B-A3B-Instruct-2507-AWQ-4bit` (MoE 30B/3B-active, Apache-2.0) | `127.0.0.1:18902` | ~20–21 GB @ 32K ctx |
| **Fallback (co-located with speech)** | `Qwen/Qwen2.5-Coder-14B-Instruct-AWQ` | `127.0.0.1:18902` | ~12–13 GB (`LLM_GPU_UTIL=0.55`) |

Port 18902 is JMCP-safe (never a Jeryu-protected port). Weights download to the HF
cache (`~/.cache/huggingface`, outside the repo); the venv + any local weights are git-ignored.

## Run

```bash
# 1. Dedicate the GPU to the model (move the speech sidecars to CPU):
./services/llm/dedicate-gpu.sh dedicate

# 2. Serve the 30B (first run installs vLLM + downloads ~17GB):
./services/llm/run-llm.sh
#    fallback 14B co-located with GPU speech:
#    ./services/llm/dedicate-gpu.sh colocate
#    LLM_MODEL=Qwen/Qwen2.5-Coder-14B-Instruct-AWQ LLM_SERVED_NAME=local/qwen2.5-coder-14b \
#      LLM_GPU_UTIL=0.55 LLM_MAX_LEN=16384 ./services/llm/run-llm.sh
```

Config via env (see `run-llm.sh`): `LLM_MODEL`, `LLM_SERVED_NAME`, `LLM_PORT`,
`LLM_GPU_UTIL`, `LLM_MAX_LEN`, `LLM_QUANT`.

## Verify

```bash
curl -s http://127.0.0.1:18902/health
curl -s http://127.0.0.1:18902/v1/chat/completions -H 'content-type: application/json' \
  -d '{"model":"local/qwen3-30b-a3b","messages":[{"role":"user","content":"In one sentence, what is JMCP?"}]}'
nvidia-smi   # vLLM owns the card; speech sidecars are on CPU
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

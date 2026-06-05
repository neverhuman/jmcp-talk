# JMCP Local Reasoning Model Sidecar

The realtime voice stack uses a local OpenAI-compatible vLLM endpoint on
`127.0.0.1:18902`. Cockpit reaches it through the `/llm` Vite proxy, so voice
reasoning stays same-origin and on-box.

| Profile | Model | Port | Notes |
| --- | --- | ---: | --- |
| Realtime voice | `cpatonn/Qwen3-30B-A3B-Instruct-2507-AWQ-4bit` | `18902` | `LLM_GPU_UTIL=0.80`, context 8192, co-resident with ASR/TTS |
| Standalone reasoning | same | `18902` | default `run-llm.sh` profile, context 32768 |

Weights download to the Hugging Face cache outside the repo. The local venv and
model artifacts are ignored.

## Run

```bash
./services/llm/realtime-voice.sh
./services/llm/run-llm.sh
```

Config via env: `LLM_MODEL`, `LLM_SERVED_NAME`, `LLM_PORT`, `LLM_GPU_UTIL`,
`LLM_MAX_LEN`, and optional `LLM_QUANT`.

# JMCP Local Voice Assistant

The cockpit voice assistant stays local: the browser captures microphone audio,
`jmcp-speechd` serves deterministic ASR/TTS endpoints, and an optional local
OpenAI-compatible reasoning endpoint can answer spoken commands. Audio and text
do not need to leave the machine for the default offline path.

## Architecture

```text
browser mic
  -> energy VAD
  -> local ASR endpoint (:18878 /transcribe)
  -> optional local reasoning endpoint (:18902 /v1/chat/completions)
  -> local TTS endpoint (:18901 /synthesize)
  -> browser playback
```

- The mic runs continuously only while the widget is active. A lightweight
  energy VAD decides where each utterance starts and ends.
- The cockpit reaches local services through the Vite dev proxy (`/asr`, `/tts`,
  `/llm`) so browser requests remain same-origin.
- `jmcp-speechd` is deterministic by default. It is suitable for local contract
  tests and fail-closed wiring. Live model-backed speech providers must be added
  behind the same HTTP contract and separately governed.
- The reasoning endpoint is not shipped by this repository. Point
  `VITE_LLM_TARGET` at a local OpenAI-compatible service when interactive
  reasoning is desired.

## Bring-Up

```bash
./services/speech/run-asr.sh
./services/speech/run-tts.sh
npm --workspace @jmcp/cockpit run dev
```

Open the cockpit, click the floating mic widget, and speak. The widget shows the
recognized text and any spoken reply as text beside the audio state.

## Ports

| Service | Bind | Notes |
| --- | ---: | --- |
| ASR (`jmcp-speechd`) | `127.0.0.1:18878` | `GET /health`, `POST /transcribe` |
| TTS (`jmcp-speechd`) | `127.0.0.1:18901` | `GET /health`, `POST /synthesize?format=wav\|ogg` |
| Reasoning LLM | `127.0.0.1:18902` | Optional OpenAI-compatible `/v1` API |
| Cockpit (Vite dev) | `127.0.0.1:15873` | proxies `/asr`, `/tts`, `/llm` |

All four are JMCP-safe ports. None of them is a Jeryu-protected port (`2224`,
`8787`, `8799`, `8929`, `18787`, `18788`, `19800`); the cockpit refuses to start
on those ports.

## Configuration

Speech daemon:

| Env | Meaning |
| --- | --- |
| `ASR_BIND` | ASR launcher bind address, default `127.0.0.1:18878` |
| `TTS_BIND` | TTS launcher bind address, default `127.0.0.1:18901` |
| `JMCP_SPEECHD_TRANSCRIPT` | deterministic text returned by `/transcribe` |
| `JMCP_SPEECHD_FAIL_CLOSED` | set `true` to make speech endpoints fail closed |

Cockpit proxy targets:

| Env | Default | Meaning |
| --- | --- | --- |
| `VITE_ASR_TARGET` | `http://127.0.0.1:18878` | where `/asr` is proxied |
| `VITE_TTS_TARGET` | `http://127.0.0.1:18901` | where `/tts` is proxied |
| `VITE_LLM_TARGET` | `http://127.0.0.1:18902` | where `/llm` is proxied |
| `VITE_LLM_MODEL` | `local/qwen3-30b-a3b` | model name sent to the local reasoning endpoint |

The cockpit host and port come from `JMCP_COCKPIT_HOST` (`127.0.0.1`) and
`JMCP_COCKPIT_PORT` (`15873`).

## Verify

```bash
curl -s http://127.0.0.1:18878/health
curl -s http://127.0.0.1:18901/health
./services/speech/selftest.sh
```

For reasoning, run any local OpenAI-compatible service on `127.0.0.1:18902` and
check one turn:

```bash
curl -s http://127.0.0.1:18902/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"local/qwen3-30b-a3b","messages":[{"role":"user","content":"In one sentence, what is JMCP?"}]}'
```

## Routing JMCP `reason` Work Orders

The same local reasoning endpoint can serve JMCP `reason` work orders. Add a
provider block to `~/jnoccio/config/router.toml`:

```toml
[providers.local_openai]
enabled  = true
api_base = "http://127.0.0.1:18902/v1"
models   = ["local/qwen3-30b-a3b"]
```

Then a JMCP `reason` work order with `JEKKO_MODEL=local/qwen3-30b-a3b` routes to
the local model through `jmcp-adapter-jekko`.

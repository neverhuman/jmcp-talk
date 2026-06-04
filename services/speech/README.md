# JMCP Speech Daemon

JMCP voice approvals and the cockpit voice assistant use a local Rust HTTP
daemon, `jmcp-speechd`. It preserves the existing speech endpoints while keeping
the default provider deterministic, offline, and fail-closed instead of binding
the repository to a live model stack.

| Role | Default port | API |
| --- | ---: | --- |
| ASR | `127.0.0.1:18878` | `GET /health`, `POST /transcribe` (raw audio bytes -> JSON) |
| TTS | `127.0.0.1:18901` | `GET /health`, `POST /synthesize?format=wav\|ogg` (JSON -> audio bytes) |

## Start

```bash
./services/speech/run-asr.sh
./services/speech/run-tts.sh
```

Config via env:

- `ASR_BIND` / `TTS_BIND`: bind addresses for the two launcher scripts.
- `JMCP_SPEECHD_TRANSCRIPT`: deterministic ASR text returned by `/transcribe`.
- `JMCP_SPEECHD_FAIL_CLOSED=true`: make health and speech requests fail closed.

## Deterministic Behavior

The default daemon does not perform live speech recognition or live speech
synthesis. `/transcribe` returns `JMCP_SPEECHD_TRANSCRIPT` and timing metadata.
`/synthesize` returns small deterministic WAV or OGG-like bytes suitable for
client contract tests and local fail-closed wiring.

## Integration

- Cockpit dev proxy:
  - `/asr` -> `VITE_ASR_TARGET` (default `http://127.0.0.1:18878`)
  - `/tts` -> `VITE_TTS_TARGET` (default `http://127.0.0.1:18901`)
- `jmcpd --telegram-poll --telegram-voice` uses `JMCP_ASR_URL` /
  `JMCP_TTS_URL` through `crates/jmcp-adapter-speech`.
- `jmcpctl telegram voice-demo` uses the same Rust speech clients.

The plaintext challenge token never goes into audio metadata; approval decisions
still pass through the normal token flow.

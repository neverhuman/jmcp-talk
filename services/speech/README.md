# JMCP Speech Sidecars

Interactive voice uses real local speech services by default:

- ASR: `asr_sidecar.py` with faster-whisper/CTranslate2.
- TTS: `tts_sidecar.py` with Kokoro-82M.

The deterministic Rust `jmcp-speechd` remains available for offline
unit/contract tests through explicit `*-deterministic.sh` launchers. It should
not silently replace the interactive speech stack.

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
- `ASR_MODEL`: default `distil-small.en`.
- `ASR_DEVICE` / `ASR_COMPUTE` / `ASR_BEAM_SIZE`.
- `TTS_VOICE` / `TTS_LANG` / `TTS_DEVICE`.

## Deterministic Test Launchers

```bash
./services/speech/run-asr-deterministic.sh
./services/speech/run-tts-deterministic.sh
```

These run `cargo run -p jmcp-speechd` on the same endpoint shape. Configure
with `JMCP_SPEECHD_TRANSCRIPT` and `JMCP_SPEECHD_FAIL_CLOSED=true`.

## Verify

```bash
./services/speech/selftest.sh
```

The selftest synthesizes a WAV with TTS, transcribes it with ASR, normalizes both
strings, and fails unless ASR hears the generated phrase.

## Integration

- Cockpit dev proxy:
  - `/asr` -> `VITE_ASR_TARGET` (default `http://127.0.0.1:18878`)
  - `/tts` -> `VITE_TTS_TARGET` (default `http://127.0.0.1:18901`)
- `jmcpd --telegram-poll --telegram-voice` uses `JMCP_ASR_URL` /
  `JMCP_TTS_URL` through `crates/jmcp-adapter-speech`.
- `jmcpctl telegram voice-demo` uses the same Rust speech clients.

The plaintext challenge token never goes into audio metadata; approval decisions
still pass through the normal token flow.

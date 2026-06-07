# JMCP Local Voice Assistant

JMCP live voice is owned by `jmcp-talk`. The browser captures microphone audio
and plays returned audio, but ASR/TTS/model protocol details stay behind the
`jmcp-talk` gateway. Core authority remains in `jmcp-core` through the cockpit
`/jmcp` proxy.

## Architecture

```
browser mic / typed input
   |
   | same-origin /voice and /voice-ws through cockpit :8080
   v
jmcp-talk Rust live voice gateway (:8040)
   |
   | private loopback HTTP/WebSocket
   v
ASR (:18878) -> text LLM (:18902/v1) -> VoxCPM2 TTS (:18901)
```

- Live voice uses the Rust `jmcp-voiced` gateway, local ASR, text reasoning,
  and streaming VoxCPM2 TTS.
- The default voice profile is `jmcp_male_v1`, backed by a VoxCPM2 voice-design
  manifest at `services/speech/voice_profiles/jmcp_male_v1.json`.
- The `jmcp_male_v1` profile is not derived from MiniCPM demo reference audio.
- The cockpit does not call `/asr`, `/tts`, or `/llm` directly in live mode.
- Deterministic `jmcp-speechd` remains for CI and split smoke fixtures.
- For current live debugging, `JMCP_TALK_CAPTURE_RAW_AUDIO=1` writes local WAV
  snippets, playback underruns, TTS timings, and JSONL logs under `JMCP_TALK_AUDIO_DIR`
  (`/home/ubuntu/jmcp-split/.live/audio` by default).

## Bring-Up

```bash
# Run the realtime stack: ASR, VoxCPM2 TTS, LLM, and gateway.
./services/llm/realtime-voice.sh

# Or run only the gateway after ASR/TTS/LLM are already up.
./services/llm/run-voice-gateway.sh

# Or start the split live stack through jmcp-deploy.
cd /home/ubuntu/jmcp-split/jmcp-deploy
ops/split/launch.sh --run
```

Cockpit listens on `127.0.0.1:8080` and proxies:

| Browser path | Target | Notes |
|---|---|---|
| `/jmcp` | `127.0.0.1:18877` | core control-plane API |
| `/voice` | `127.0.0.1:8040` | voice health, events, metrics |
| `/voice-ws` | `127.0.0.1:8040/ws` | local voice sessions |

## Ports

| Service | Bind | Owner |
|---|---:|---|
| Cockpit | `127.0.0.1:8080` | `jmcp-web` |
| Core API | `127.0.0.1:18877` | `jmcp-core` |
| JMCP voice gateway | `127.0.0.1:8040` | `jmcp-talk` |
| ASR sidecar | `127.0.0.1:18878` | private `jmcp-talk` upstream |
| TTS sidecar | `127.0.0.1:18901` | private `jmcp-talk` upstream |
| LLM sidecar | `127.0.0.1:18902` | private `jmcp-talk` upstream |
| Comni gateway | `127.0.0.1:18040` | MiniCPM debug lane |

## Verify

```bash
curl http://127.0.0.1:8080/jmcp/health
curl http://127.0.0.1:8080/voice/health  # includes voice_engine/profile/hash/sample_rate
curl http://127.0.0.1:8040/metrics
./services/llm/smoke-real-tts.py --runs 5 --strict
./services/llm/smoke-real-voice-turn.py --mode both --runs 3 --strict
nvidia-smi
```

The smoke scripts are explicit live checks for the Rust gateway and VoxCPM2
lane. They keep raw WAV artifacts local under `.live/audio` and only write
stripped frame/timing summaries and listening packets.

The split launcher writes live logs under
`/home/ubuntu/jmcp-split/.live/logs/`, including `voice-events.jsonl` and the
GPU memory snapshot taken at launch.
Raw audio snippets for post-analysis are written under
`/home/ubuntu/jmcp-split/.live/audio/<turn_id>/`:

- `input_*.wav`: browser mic snippets, 16 kHz mono.
- `output_chunk_*.wav`: streamed assistant chunks, usually 48 kHz mono.
- `events.jsonl`: redacted turn, playback, VAD, profile, and timing events.

## Legacy Fixtures

MiniCPM/Comni is now the debug lane, not the default cockpit speaking path. Run
`services/llm/run-minicpm-o45.sh` on a non-8040 bind when testing it alongside
the local voice gateway.

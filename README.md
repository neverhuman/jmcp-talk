# jmcp-talk

Speech experience split for JMCP.

This repository owns speech adapters, deterministic speech fixtures, local
ASR/TTS sidecars, the local voice gateway, MiniCPM-o 4.5 spike scaffolding, and user-experience
troubleshooting receipts. `jmcp-core` remains authoritative for voice ledgers,
approvals, tool policy, and durable turn state.

## Workspaces

- `crates/jmcp-adapter-speech`: speech clients, runtime adapter selection, and
  non-secret trace receipts.
- `apps/jmcp-voiced`: Rust live voice orchestrator for `/health`, `/metrics`,
  `/events`, and `/ws/chat` on the cockpit voice port.
- `apps/jmcp-speechd`: deterministic local speech fixture daemon.
- `services/speech`: local ASR/TTS sidecar scripts, including VoxCPM2 primary
  TTS and Kokoro fallback.
- `services/llm`: local voice gateway, Comni debug launcher, and local LLM
  support scripts.

## Runtime Adapters

Use scoped talk configuration first; legacy aliases remain compatibility-only.

```dotenv
JMCP_TALK_ADAPTER=deterministic
JMCP_TALK_ASR_URL=http://127.0.0.1:18878
JMCP_TALK_TTS_URL=http://127.0.0.1:18901
JMCP_TALK_VOICE_BIND=127.0.0.1:8040
JMCP_TALK_LLM_UPSTREAM=http://127.0.0.1:18902/v1
JMCP_TALK_LLM_MODEL=local/qwen2.5-7b-instruct-awq
JMCP_TALK_ASR_UPSTREAM=http://127.0.0.1:18878
JMCP_TALK_TTS_UPSTREAM=http://127.0.0.1:18901
JMCP_TALK_VOICE_PROFILE=jmcp_male_v1
TTS_ENGINE=voxcpm2
TTS_FALLBACK_ENGINE=kokoro
TTS_VOICE=jmcp_male_v1
JMCP_TALK_AUDIO_DIR=/home/ubuntu/jmcp-split/.live/audio
JMCP_TALK_CAPTURE_RAW_AUDIO=1

# MiniCPM/Comni debug lane.
JMCP_TALK_MINICPM_BIND=127.0.0.1:8041
JMCP_TALK_MINICPM_COMNI_PORT=18040
JMCP_TALK_MINICPM_QUANT=Q4_K_M
JMCP_TALK_MINICPM_CTX_SIZE=8192
JMCP_TALK_MINICPM_N_GPU_LAYERS=99
JMCP_TALK_MINICPM_VOICE_PROFILE=jmcp_friendly_male
JMCP_TALK_MINICPM_REF_AUDIO=/home/ubuntu/jmcp-split/jmcp-talk/services/llm/assets/ref_audio/jmcp_friendly_male_16k.wav
```

Supported adapters are `deterministic`, `legacy-cascade`, and `minicpm-o45`.
The cockpit speaking path is the Rust local voice gateway: browser mic, local
ASR, text reasoning, VoxCPM2 streaming TTS, then browser PCM playback.
MiniCPM-o 4.5 live inference is an opt-in spike/debug lane.

The live browser contract is owned here: cockpit uses same-origin `/voice` and
`/voice-ws`, which proxy to `jmcp-talk` on `127.0.0.1:8040`. ASR, TTS, LLM,
PCM frame details, event redaction, and model health stay inside this repo.

## Local Voice Gateway

```bash
# Realtime cockpit voice stack: ASR + VoxCPM2 TTS + 30B LLM + gateway.
bash services/llm/realtime-voice.sh

# Gateway only, if ASR/TTS/LLM are already running.
bash services/llm/run-voice-gateway.sh
curl http://127.0.0.1:8040/health
```

`/health` reports `voice_engine`, `voice_profile`, `voice_profile_hash`,
`sample_rate`, `streaming_audio`, `tts_rtf_p50`, and fallback status. Live audio
frames are float32 PCM with sequence and timing metadata; local WAV snippets are
written only under `JMCP_TALK_AUDIO_DIR` while raw capture is enabled.

## MiniCPM-o 4.5 Debug Lane

```bash
# One-time build/install under /home/ubuntu/jmcp-split/.live/minicpm-o45.
JMCP_TALK_MINICPM_DOWNLOAD=1 bash services/llm/setup-minicpm-o45.sh

# Run the legacy MiniCPM gateway on its debug bind.
bash services/llm/run-minicpm-o45.sh
```

Defaults use `openbmb/MiniCPM-o-4_5-gguf`, `MiniCPM-o-4_5-Q4_K_M.gguf`,
`ctx_size=8192`, `n_gpu_layers=99`, and the bundled
`jmcp_friendly_male` 16 kHz mono reference WAV for the 24 GB RTX 3090 path.
The Comni gateway runs privately on `127.0.0.1:18040`; bind the MiniCPM gateway
away from `8040` unless intentionally replacing the local voice gateway.

When raw capture is enabled, each live turn writes local WAV snippets and JSONL
logs under `JMCP_TALK_AUDIO_DIR`: user input at 16 kHz, assistant output chunks,
playback underruns, TTS timing, profile hash, and per-turn JSONL events.

## Local Proof

```bash
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
bash services/speech/selftest.sh
```

Live voice quality and responsiveness checks are opt-in because they require
the local GPU/model stack:

```bash
./services/llm/smoke-real-tts.py --runs 5 --strict
./services/llm/smoke-real-voice-turn.py --mode both --runs 3 --strict
```

These write WAV artifacts, stripped timing JSONL, `summary.json`, and a human
listening packet under `/home/ubuntu/jmcp-split/.live/audio`.

## UX Troubleshooting

Trace receipts should preserve adapter, model, device, quantization, status, and
non-secret error class. Live debugging now keeps local raw WAV snippets under
`JMCP_TALK_AUDIO_DIR`; disable with `JMCP_TALK_CAPTURE_RAW_AUDIO=0` when that
post-analysis data is no longer needed.

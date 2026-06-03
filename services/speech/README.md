# JMCP speech sidecars — realtime ASR + great TTS on the local GPU

Two out-of-process HTTP sidecars own the heavy CUDA/ML stack so the Rust runtime
never has to. They mirror how `jmcp-adapter-jekko` shells out to a separate
engine: thin Rust clients (`crates/jmcp-adapter-speech`) call them over HTTP.

| Sidecar | Model | Port | License | Endpoints |
|---------|-------|------|---------|-----------|
| ASR | faster-whisper **distil-small.en** (CTranslate2) | `127.0.0.1:18878` | MIT | `GET /health`, `POST /transcribe` (raw audio bytes → JSON) |
| TTS | **Kokoro-82M** | `127.0.0.1:18901` | Apache-2.0 | `GET /health`, `POST /synthesize` (JSON → WAV/OGG) |

Both ports are JMCP-safe (never Jeryu-protected `2224/8787/8799/8929/18787/18788/19800`).

## Run

```bash
./services/speech/run-asr.sh   # first run: venv + faster-whisper + distil-small.en
./services/speech/run-tts.sh   # first run: venv(--system-site-packages) + Kokoro (~330MB)
```

Config via env (see each `run-*.sh`): `ASR_MODEL` (default `distil-small.en`),
`ASR_DEVICE`/`ASR_COMPUTE` (`cuda`/`float16` on NVIDIA), `ASR_BEAM_SIZE`
(default `1`), `TTS_VOICE` (default `af_heart`), `TTS_DEVICE` (`auto`). The
venvs and downloaded models are git-ignored.

## Verified on this box (1× RTX 3090, 24 GB)

- **ASR**: realtime profile is `distil-small.en` on CUDA float16 with beam 1,
  selected to keep first-audio latency low while sharing a 24 GB card with vLLM.
- **TTS**: Kokoro on cuda, 24 kHz output.
- **Round-trip** (the regression check, `selftest.sh`): TTS synthesizes
  *"Master control plane online. The autonomous dispatcher is running."* → ASR
  transcribes it back. Both sidecars are intended to co-reside with the 30B
  realtime voice profile.

## Two-way Telegram voice approvals

The sidecars back a full **voice approval loop** in `jmcpd` (Rust, via
`crates/jmcp-adapter-speech` + `jmcp-approval-telegram`):

- **Inbound**: a Telegram voice note → `getFile`/download → ASR `/transcribe` →
  the risk-scored [`evaluate_voice_approval`] decision → applied through the same
  `decide_approval_by_token` path as a typed `/approve`. Low-risk intents accept a
  spoken "approve"/"reject"; **high-risk** intents (deploy, delete, rotate, …)
  require the **spoken confirmation token**. Recognizer confidence must clear 0.75.
- **Outbound**: every reply is sent as text and **spoken back** as a voice note
  (`sendVoice`, TTS `?format=ogg`).

Enable it: `jmcpd --telegram-poll --telegram-voice` (sidecars must be running;
`JMCP_ASR_URL` / `JMCP_TTS_URL` override the defaults). The plaintext challenge
token is held **in memory only** (never persisted), keyed by approver.

The supported standalone demo is the Rust CLI:

```bash
rtk cargo run -p jmcpctl -- telegram voice-demo discover
rtk cargo run -p jmcpctl -- telegram voice-demo send <chat_id> "your message"
rtk cargo run -p jmcpctl -- telegram voice-demo listen --reply-voice --seconds 60
```

It reads the Telegram env file from `JMCP_TELEGRAM_ENV` (default
`telegram.env`) and uses `JMCP_ASR_URL` / `JMCP_TTS_URL` for the local speech
sidecars.

## Accuracy overrides

The sidecars are model-agnostic. For an explicit accuracy run, start ASR with
`ASR_MODEL=large-v3 ASR_BEAM_SIZE=5 ./services/speech/run-asr.sh`; do not use
that as the realtime default on the shared 24 GB GPU. Provisioning a different
ASR or TTS model is governed by the `local-speech.inventory-asr-tts` microtask and
must be benchmarked before it becomes a default. License discipline: ship only
MIT/Apache/CC-BY; avoid XTTS-v2 (non-commercial CPML).

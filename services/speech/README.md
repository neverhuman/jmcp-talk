# JMCP speech sidecars — master ASR + great TTS on the local GPU

Two out-of-process HTTP sidecars own the heavy CUDA/ML stack so the Rust runtime
never has to. They mirror how `jmcp-adapter-jekko` shells out to a separate
engine: thin Rust clients (`crates/jmcp-adapter-speech`) call them over HTTP.

| Sidecar | Model | Port | License | Endpoints |
|---------|-------|------|---------|-----------|
| ASR | faster-whisper **large-v3** (CTranslate2) | `127.0.0.1:18878` | MIT | `GET /health`, `POST /transcribe` (raw audio bytes → JSON) |
| TTS | **Kokoro-82M** | `127.0.0.1:18901` | Apache-2.0 | `GET /health`, `POST /synthesize` (JSON → WAV) |

Both ports are JMCP-safe (never Jeryu-protected `2224/8787/8799/8929/18787/18788/19800`).

## Run

```bash
./services/speech/run-asr.sh   # first run: venv + faster-whisper + large-v3 (~3GB)
./services/speech/run-tts.sh   # first run: venv(--system-site-packages) + Kokoro (~330MB)
```

Config via env (see each `run-*.sh`): `ASR_MODEL` (default `large-v3`),
`ASR_DEVICE`/`ASR_COMPUTE` (`cuda`/`float16`), `TTS_VOICE` (default `af_heart`),
`TTS_DEVICE` (`auto`). The venvs and downloaded models are git-ignored.

## Verified on this box (1× RTX 3090, 24 GB)

- **ASR**: jfk.wav → *"…ask not what your country can do for you, ask what you can do
  for your country."* — exact, **RTF ≈ 0.07** (~14× real-time), ~3.4 GB VRAM.
- **TTS**: Kokoro on cuda, 24 kHz output.
- **Round-trip** (the regression check, `selftest.sh`): TTS synthesizes
  *"Master control plane online. The autonomous dispatcher is running."* → ASR
  transcribes it back **exactly**. Both sidecars co-resident in ~4.4 GB VRAM
  (≈19.7 GB free for a 20–30B reasoning model).

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

## Upgrading the models

The sidecars are model-agnostic. The `local-speech.inventory-asr-tts` microtask
inventories readiness; provisioning a higher-accuracy ASR (e.g. `nvidia/canary-qwen-2.5b`,
#1 on the Open ASR leaderboard) or a streaming TTS (CosyVoice2-0.5B) is a governed,
benchmarked, approval-gated upgrade — set `ASR_MODEL`/`TTS_*` once a swap is proven.
License discipline: ship only MIT/Apache/CC-BY; avoid XTTS-v2 (non-commercial CPML).

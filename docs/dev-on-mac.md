# Developing JMCP on a Mac (Apple Silicon)

JMCP is portable Rust + Vite/TypeScript; only the optional speech sidecars are
hardware-specific. This is the bootstrap when moving active development from the
Linux/CUDA box to a Mac.

## 1. Get the code

```bash
git clone git@github.com:neverhuman/JMCP.git jmcp   # or: git pull origin main
cd jmcp
```

`main` is the source of truth and is kept green (CI: `fast`, `ci`, `security`,
`jankurai`). The following are **git-ignored** and recreated locally, not pulled:

- `services/speech/.venv*` and downloaded models — recreated on first sidecar run.
- `telegram.env` — create your own (a bare bot token, plus
  `TELEGRAM_ALLOWED_USER_IDS=…` / `TELEGRAM_ALLOWED_CHAT_IDS=…`).
- `target/`, `node_modules/`, `*.db`.

## 2. Toolchain

- Rust (stable) + `cargo`; `just` for the CI recipes; Node 22 + `npm`.
- `cargo build --workspace` then `just fast` and `just ci` to confirm green.

## 3. Local CI

```bash
just fast    # fmt + cargo check --locked + json/shell/actionlint
just ci      # rust tests + cockpit + conformance
just security
```

Cargo parallelizes across all cores automatically. The jankurai ratchet is part
of CI; note that the **local `jankurai` on PATH may differ from the CI-pinned
rev** and report a false regression — see `ops/ci/jankurai-ratchet.sh` and pass
`JANKURAI_BIN=<pinned>` to match CI exactly (the pinned rev is in
`.github/workflows/jankurai.yml`).

## 4. Speech sidecars on the Mac

The launchers auto-detect the device, so they "just work":

```bash
./services/speech/run-asr.sh   # no NVIDIA GPU -> CPU + int8 automatically
./services/speech/run-tts.sh   # Kokoro-82M -> CPU (fast on M-series)
```

- **ASR** (faster-whisper / CTranslate2) has **no Metal/MPS backend**, so the Mac
  default is `ASR_DEVICE=cpu ASR_COMPUTE=int8` (set automatically when `nvidia-smi`
  is absent). The model default remains `distil-small.en` for parity with the
  realtime CUDA profile. For explicit accuracy checks, override with
  `ASR_MODEL=large-v3 ASR_BEAM_SIZE=5`.
- **TTS** (Kokoro-82M, PyTorch) defaults to CPU. To try Metal: `TTS_DEVICE=mps
  ./services/speech/run-tts.sh` (most ops are MPS-supported; fall back to CPU if a
  kernel is missing).
- Ports are unchanged and JMCP-safe: ASR `127.0.0.1:18878`, TTS `127.0.0.1:18901`.
- Round-trip check once both are up: `./services/speech/selftest.sh`.

## 5. Voice approvals (optional)

`jmcpd --telegram-poll --telegram-voice` enables two-way Telegram voice approvals
against the sidecars (`JMCP_ASR_URL` / `JMCP_TTS_URL` override the defaults). The
standalone demo is `jmcpctl telegram voice-demo {discover|send|listen}`.

# Developing JMCP on a Mac (Apple Silicon)

JMCP is portable Rust + Vite/TypeScript. This is the bootstrap when moving
active development from the Linux box to a Mac.

## 1. Get the code

```bash
git clone git@github.com:neverhuman/JMCP.git jmcp   # or: git pull origin main
cd jmcp
```

`main` is the source of truth and is kept green (CI: `fast`, `ci`, `security`,
`jankurai`). The following are **git-ignored** and recreated locally, not pulled:

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

## 4. Speech Daemon on the Mac

The launchers run the Rust `jmcp-speechd` daemon on the existing ASR/TTS ports:

```bash
./services/speech/run-asr.sh
./services/speech/run-tts.sh
```

- Ports are unchanged and JMCP-safe: ASR `127.0.0.1:18878`, TTS `127.0.0.1:18901`.
- Set `JMCP_SPEECHD_TRANSCRIPT` to control deterministic ASR text.
- Smoke check once both are up: `./services/speech/selftest.sh`.

## 5. Voice approvals (optional)

`jmcpd --telegram-poll --telegram-voice` enables two-way Telegram voice approvals
against the speech daemon (`JMCP_ASR_URL` / `JMCP_TTS_URL` override the defaults). The
standalone demo is `jmcpctl telegram voice-demo {discover|send|listen}`.

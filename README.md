# jmcp-talk

Speech experience split for JMCP.

This repository owns speech adapters, deterministic speech fixtures, local
ASR/TTS sidecars, MiniCPM-o 4.5 spike scaffolding, and user-experience
troubleshooting receipts. `jmcp-core` remains authoritative for voice ledgers,
approvals, tool policy, and durable turn state.

## Workspaces

- `crates/jmcp-adapter-speech`: speech clients, runtime adapter selection, and
  non-secret trace receipts.
- `apps/jmcp-speechd`: deterministic local speech fixture daemon.
- `services/speech`: ASR/TTS sidecar scripts.
- `services/llm`: local LLM voice-support scripts.

## Runtime Adapters

Use scoped talk configuration first; legacy aliases remain compatibility-only.

```dotenv
JMCP_TALK_ADAPTER=deterministic
JMCP_TALK_ASR_URL=http://127.0.0.1:18878
JMCP_TALK_TTS_URL=http://127.0.0.1:18901
JMCP_TALK_MINICPM_O45_QUANTIZATION=int4
JMCP_TALK_MINICPM_O45_DEVICE=cuda:0
```

Supported adapters are `deterministic`, `legacy-cascade`, and `minicpm-o45`.
MiniCPM-o 4.5 live inference is an opt-in spike lane; deterministic and legacy
fallbacks stay available until latency, barge-in, and stability gates pass.

## Local Proof

```bash
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
bash services/speech/selftest.sh
```

## UX Troubleshooting

Trace receipts should preserve adapter, model, device, quantization, status, and
non-secret error class. Raw audio capture is not a default path; analysis and
fine-tuning data must be explicitly enabled and redacted before export.

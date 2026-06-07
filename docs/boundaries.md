# JMCP Talk Boundaries

Talk-owned Rust surfaces:
- `apps/jmcp-speechd/`
- `apps/jmcp-voiced/`
- `crates/jmcp-adapter-speech/`

Bounded Python sidecars live under `services/llm/` and `services/speech/`.
They may perform local speech inference and smoke validation, but they do not
own durable product truth, policy, approvals, ledgers, or databases.

Default proof lanes are deterministic and local. GPU live lanes are opt-in.

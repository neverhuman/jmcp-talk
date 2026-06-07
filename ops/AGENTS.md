# JMCP Talk Ops Agent Instructions

This directory owns deterministic talk CI helpers, GPU-dispatch wrappers, and
local proof scripts.

- Keep default CI local and deterministic; MiniCPM-o live GPU validation is opt-in.
- Do not put durable approval, policy, ledger, or turn-state authority in talk ops.
- Workflow files should delegate to `ops/ci/*.sh` scripts.
- Security and audit receipts belong under `.artifacts/security/` or `target/jankurai/`.

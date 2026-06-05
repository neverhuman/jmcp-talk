# JMCP Talk Agent Instructions

This repository follows the runtime toolchain instructions at:

@/home/ubuntu/.codex/RTK.md

## Scope

`jmcp-talk` owns speech runtime adapters, MiniCPM-o 4.5 spike work, legacy
voice fallback, deterministic speech fixtures, and local speech sidecars. Core
authority remains in `jmcp-core`; talk code must route mutations, approvals,
ledger records, and tool policy through core APIs.

## Agent Rules

- Use the `rtk` prefix for shell commands.
- Treat `AGENT_CHAT.md` as append-only.
- Keep work scoped to speech services, speech adapter crates, speech daemon
  apps, and voice runtime docs.
- Default tests must be deterministic and local. MiniCPM-o live GPU runs are an
  opt-in spike lane, not the default proof path.
- Preserve other agents' edits. If a file has changed unexpectedly, inspect and merge rather than overwrite.

## Jankurai

<!-- jankurai generated adapter -->
<!-- jankurai agent request v1 sha256:REPLACE_WITH_HASH -->

Read `agent/JANKURAI_STANDARD.md` before Jankurai-scoped work. For explicit phase or MASTER_PLAN work only, read `agent/MASTER_PLAN.md` before `tips/phases/00-phase-index.md`; otherwise, user-provided implementation or handoff plans are controlling. Keep generated artifacts under their declared source commands.

# JMCP Agent Instructions

This repository follows the runtime toolchain instructions at:

@/home/ubuntu/.codex/RTK.md

## Scope

JMCP is the system. JCP/1.0.0 is the protocol. JPCM is the backbone and transport profile. V1 targets a local production-shaped core with embedded SQLite, an in-process replayable event bus by default, a Rust backend, a React dashboard, a Rust TUI, Telegram text intake and approvals, local Jankurai/Jeryu/Jekko adapters, CI-local parity, and strong tests.

## Agent Rules

- Use the `rtk` prefix for shell commands.
- Treat `AGENT_CHAT.md` as append-only.
- Keep work scoped to the paths explicitly owned for the task.
- Do not edit Rust crates, apps, package files, scripts, CI, schemas, or `tips/` unless a later instruction grants ownership.
- Preserve other agents' edits. If a file has changed unexpectedly, inspect and merge rather than overwrite.

## Jankurai

<!-- jankurai generated adapter -->
<!-- jankurai agent request v1 sha256:REPLACE_WITH_HASH -->

Read `agent/JANKURAI_STANDARD.md` before Jankurai-scoped work. For explicit phase or MASTER_PLAN work only, read `agent/MASTER_PLAN.md` before `tips/phases/00-phase-index.md`; otherwise, user-provided implementation or handoff plans are controlling. Keep generated artifacts under their declared source commands.

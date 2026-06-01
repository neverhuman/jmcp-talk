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


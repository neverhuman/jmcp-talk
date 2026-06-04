# GOD MODE CODEX Plan: JMCP Agent Operating System

## Summary

Build JMCP into the governed authority kernel between the user and every local or remote intelligence surface: Jeryu for code/repo/CI truth, Jankurai for standards/proof/risk, Jekko/ZYAL for heavy reasoning and durable memory, Jailgun/VEOX for bounded execution, and nested MCP/CLI/API tools through a typed broker.

## Key Architecture

- L0 Authority Kernel: JMCP remains the only authority for leases, approvals, audit, replay, redaction, cost budgets, and stop conditions. No adapter, MCP, CLI, ZYAL, Jailgun, or VEOX path bypasses JMCP policy.
- L1 Capability Graph: Persist a registry for every tool/API/CLI/MCP/workflow/data source with schema, owner, transport, risk tier, cost, latency, health, auth, side effects, examples, and escalation requirements.
- L2 Tool Broker: Replace one-off adapter execution with a governed broker supporting discovery, dry-run, invoke, stream, cancel, health, evidence capture, replay safety, and quarantine.
- L3 Ask/Plan Kernel: Add `/ask`, `/ask/code`, `/plan`, plan execution, streaming progress, cancellation, and evidence-backed answers. Code questions default to Jeryu, standards questions to Jankurai, heavy synthesis/memory to Jekko/ZYAL.
- L4 Jeryu Code Oracle: Wire Jeryu REST/MCP/codegraph as the first-class answer path for code questions.
- L5 Jankurai Proof Oracle: Expose structured proof, risk, standards, reuse, drift, CI parity, false-positive, and repair-plan capabilities.
- L6 Jekko/ZYAL Brain: Route `zyal.run` for real, use Jekko/ZYAL supervisor state for durable heavy reasoning, and connect JMCP memory promotion/quarantine to Jekko memory kinds/status.
- L7 Mission Deck: Extend JITUX into a live mission deck for plans, tool calls, subprocess/PTY status, stuck-process detection, VEOX job awareness, and publication workflows.

## Escalation Model

- R0: Read-only local/JMCP/Jeryu/Jankurai/memory/codegraph calls. Auto allowed.
- R1: Evidence-only remote reads or bounded model calls. Auto allowed only within declared budget.
- R2: Local scratch/temp work, dry-runs, generated plans. Lease required; no durable repo mutation.
- R3: Repo branch/worktree/patch/PR preparation. Approval or scoped workcell lease required, with Jankurai proof.
- R4: External durable mutation: repo creation, PR open, CI trigger, Jailgun/VEOX run, publication pipeline. Explicit approval required.
- R5: Secrets, deploys, security policy changes, global memory promotion, new privileged tools, destructive actions. Explicit human approval, rollback path, proof, and audit required.

## Implementation Notes

This repository implementation should land as narrow, proof-backed slices: domain/store/API/CLI broker primitives first, real `zyal.run` dispatch, then deeper adapter discovery and VEOX/JITUX mission-deck integration after inspecting their local APIs.

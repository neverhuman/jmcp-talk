# GOD MODE FINAL: Claimable Work Plan

This file is the coordination surface for the remaining JMCP operating-system work. It is not an architecture essay. It is a claim queue ordered by dependency, with the current verified state called out first so we do not rebuild what already exists.

## 1. Current verified state

These items already exist in the repository and must not be rebuilt from scratch:

- `toolplane.rs` already carries the core governance shapes: `CapabilityCard`, `RiskTier`, `CapabilityTransport`, `CapabilityAuth`, `SideEffectClass`, `ToolCallMode`, `PlanDAG`, `EscalationPacket`, `MemoryProposal`, `MemoryRecord`, `PromotionDecision`, `ProcessObservation`, and the `Ask*` family.
- `toolplane_records` already exists as the 11th table and already participates in record/replay.
- The app layer already exposes `broker_tool_call`, `ask`, `create_plan`, and `execute_plan` as simulated surfaces.
- The API already routes `/capabilities`, `/tools/:id/{dry-run,call}`, `/tool-calls`, `/ask`, `/ask/code`, `/plans`, `/zyal/status`, and `/process-observations`.
- `dispatch_loop.rs` already routes `zyal.run` to `Some("zyal")`.
- `is_auto_dispatchable` is already the real auto-gate: `microtask && evidence_oriented && !live`.
- `builtin_capabilities()`, `jmcp-now`, and JITUX Mission Deck status already exist and must not be recreated.
- Jeryu, Jekko, and Jankurai already exist as local truth, reasoning, and proof surfaces.

Truth reconciliation matters here:

- The broker is still canned for read-only calls: `broker_tool_call` returns stub JSON rather than a real nested tool call.
- Tool discovery is still static in the broker path.
- An inbound MCP server is still missing.
- The current `zyal.run` work is not "add routing"; routing already exists. The pending work is to replace the blocking polling model with a durable async handle.

## 2. Pending work queue

Work is ordered by dependency. Each slice should be claimed explicitly before edits.

### Slice 0: Foundation cleanup

Goal: make the existing scaffold safer and more explicit without changing runtime behavior.

- Additive domain and risk fields only, with `serde(default)` on new fields so old stored cards keep deserializing.
- Land the shared foundation items together if they still need cleanup: `CapabilityHealthSample`, `ZyalRunHandle`, `BrainSession`, `ToolPoolStat`, and `AnswerContract`.
- Hoist the protected-port guard into a shared helper that every new bind path uses.
- Centralize the risk-tier policy into one table so capability tiers are not duplicated across layers.
- Make the `jmcp-domain` boundary rule real instead of aspirational.
- Keep the replay test green while confirming old persisted `CapabilityCard` JSON still loads.

This slice is the last shared-file foundation pass. After it lands, later work should be single-owner and narrower.

### Slice 1: First implementation claim, the JARVIS moment

Goal: make one real governed nested call through the broker.

- Add the `jmcp-broker` / MCP runtime substrate.
- Reuse the JSON-RPC `tools/list` / `tools/call` envelope already proven in the outbound Jekko worker-run path.
- Wire exactly one R0 capability: `jeryu-mcp get_system_snapshot`.
- Keep the current escalation behavior unchanged for anything above R1.
- Replace the canned broker response for that single card with a real nested call returning live `structuredContent`, while keeping the canned JSON as the explicit no-runtime-wired fallback until the runtime is live.

This is the first claimable implementation slice because it proves the broker is real without widening scope into the rest of L4/L5/L6/VEOX.

### Slice 2: Code oracle follow-on

Goal: turn Jeryu into a first-class answer path for code questions.

- Add the Jeryu code oracle path.
- Shape the answer payload as an `AnswerContract`.
- Expand from one nested snapshot call into richer codegraph-backed fan-out.
- Keep the code-answer path evidence-backed rather than narrative-only.

### Slice 3: Proof oracle follow-on

Goal: make Jankurai a structured proof and standards surface instead of a subprocess adapter.

- Add structured proof, risk, standards, reuse, drift, CI parity, false-positive, and repair-plan projections.
- Add the Jankurai tool-pool projection only when the proof slice is ready.
- Keep the existing digest-only CLI adapter as the fallback until the structured path is verified.

### Slice 4: Async ZYAL and hybrid memory

Goal: turn ZYAL from a blocking call into a durable re-attachable run.

- Add `ZyalRunHandle` as a persisted, event-sourced run handle.
- Replace synchronous polling with a JMCP-owned sweep loop.
- Keep submission on the current CLI path until a real start route exists on the Jekko side.
- Before swapping the runner, fetch Jekko's live `/openapi.json`, capture a real daemon-status golden, and confirm the cogcore read path exists for the memory refinery.
- Add the hybrid memory refinery only after the durable run path exists.

### Slice 5: Mission Deck and process awareness

Goal: make Mission Deck reflect live process state and work-order liveness.

- Add the watchdog over existing process observations.
- Extend Mission Deck from one-shot to continuous.
- Add stuck-process awareness and process status surfacing.
- Keep PTY and raw terminal plumbing behind a later slice.

### Slice 6: VEOX read-only, then governed mutation

Goal: keep VEOX read-only first, then unlock governed submission only after proof and policy exist.

- Add the read-only adapter and snapshot surface first.
- Add governed submit/import/publication only after the read path is proven.
- Keep publication authored JMCP-side through ZYAL and Jankurai gates.

### Slice 7: Inbound MCP

Goal: expose `/mcp` safely as the front-line JARVIS-facing contract.

- Add bearer auth and a per-caller allowlist.
- Default external callers to read-only R0/R1 visibility.
- Never expose R4+ mutating tools in `tools/list` to untrusted callers.

## 3. Deferred heavy work

These are intentionally deferred until the earlier slices prove out:

- Async ZYAL handles and durable re-attachment.
- Jankurai tool-pool projection.
- VEOX integration beyond the read-only adapter.
- `Spine` trait work with `InProcessSqlite` first, and only then a real NATS spine.
- PTY-backed process plumbing with redaction-before-hash.
- `secret-in-cast` coverage and the matching cast-redaction gate.
- The front-line `/mcp` consumer wiring that couples the allowlist to caller identity.
- NATS spine and PTY-backed process plumbing.
- Inbound MCP with auth and allowlists.

If a task depends on any of those, it should not be pulled forward into the foundation or first broker slice.

## 4. First implementation claim

Canonical claim: Slice 1 is the first real implementation slice, and the broker JARVIS moment lives there.

- Scope: `jmcp-broker` / MCP runtime substrate, plus the single `jeryu-mcp get_system_snapshot` capability.
- Outcome: a real governed nested call through the broker using the existing JSON-RPC envelope.
- Constraint: keep escalation for >R1 unchanged.
- Constraint: the canned JSON remains the explicit no-runtime-wired fallback until Slice 1 is live.
- Constraint: do not widen into other runtimes until this one call path is verified.

This is the smallest slice that proves the operating system is doing governed nesting rather than canned simulation.

## 5. Proof lanes

Use the existing proof lanes for the slice you are claiming:

- `just score` for routing and doc surfaces.
- `just fast` for local consistency and lightweight gates.
- `just security` for secret and approval boundaries.
- `just ci` or the equivalent workspace gate when runtime behavior changes.
- Focused cargo tests when a slice touches broker, API, app, or adapter runtime paths.
- Use `just score` before and after any documentation-only routing update.

Suggested verification by slice:

- Documentation and routing slice:
  - `cargo run -q -p jmcp-ci-tools -- validate-json agent/owner-map.json agent/test-map.json`
  - `just score`
- First broker implementation slice:
  - focused cargo tests for broker/API/app crates
  - `cargo check --workspace --all-targets --locked`
  - `just fast`
  - `just security`
  - an integration test proving R4 `zyal.run` escalates and does not invoke runtime

## 6. Coordination rules

- Append a claim to `AGENT_CHAT.md` before editing shared paths.
- Append progress at each major boundary.
- Append proof and handoff after verification.
- Keep claims narrow and explicit about touched paths.
- Do not expand scope into uncited files while another agent owns the active slice.
- Preserve other agents' edits; inspect and merge rather than overwrite if a file changed unexpectedly.
- After Phase 0, keep strict single ownership: L2 owns the broker rewrite, L3 owns `ask()`, and later layers only plug runtimes into the broker.
- Async handles and spine fan-in must not double-execute side effects on replay.
- The front-line `/mcp` consumer and the allowlist must be designed together, not as separate loose follow-ons.
- Any new Jankurai cap or issue discovered while claiming a slice must be logged explicitly before work continues.

## 7. Working summary

Claude can own the deep runtime-adapter follow-on work in disjoint slices once the broker foundation is stable.

Codex should use this plan as the dependency map and claim only one slice at a time.

The immediate next claimable work is Slice 0 if the shared governance scaffolding still needs cleanup, otherwise Slice 1 for the first real broker call.

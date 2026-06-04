# GOD MODE — JMCP as the J.A.R.V.I.S. Agent Operating System (Claude plan)

> Sibling to `GOD_MODE_CODEX.md`. Same **L0–L7 spine**; this doc goes deeper, corrects three stale premises
> against live code, disambiguates the two "layer" concepts, fills the gaps Codex left open (NATS spine, hybrid
> memory, veox MCP + publication, front-line UX, budgets), and ends with a turnkey first slice.

## 1. Context — why this, and where we actually start

**The goal.** Make JMCP feel like J.A.R.V.I.S.: the governed *driver* between the operator and every local
intelligence surface. It must (a) hold **nested access to ALL tools** (MCP/CLI/API), (b) answer hundreds of
questions about jeryu/jankurai/jekko/veox **instantly** with drill-down, (c) **execute autonomously, monitor,
and escalate**, (d) anticipate "what can you tell me about BLANK in the code?" → Jeryu, (e) **offload heavy
reasoning + memory** to a live Jekko/ZYAL brain, (f) hunt **false-positives / human-entropy** via Jankurai and
pool shared tools with usage tracking, (g) drive **veox** builds and author publications, and (h) consume
**all PTTY/process telemetry** into a durable spine so it knows what is running and what is stuck.

**The pleasant surprise: this is not greenfield.** The current tree is far ahead of the `tips/` tarball. Verified
by reading the files:

- A **`toolplane` capability scaffold already exists**: `crates/jmcp-domain/src/toolplane.rs` defines
  `CapabilityCard`, `CapabilityTransport {Internal,Rest,Mcp,Cli,Pty,Workflow,Filesystem}`, `CapabilityAuth`,
  `SideEffectClass`, **`RiskTier {R0..R5}`** with `requires_lease()/requires_approval_gate()/requires_explicit_human_approval()/auto_allowed()`,
  `EscalationRequirement::for_risk`, `ToolCallMode{DryRun,Invoke}`, `ToolCallPlan`, `ToolObservation`,
  `AskKind/AskRequest/AskRoute/AskResponse`, `PlanDAG/PlanNode/PlanEdge`, `EscalationPacket`, `MemoryProposal`,
  `ProcessObservation`. The store has an **11th table `toolplane_records`** with `record_capability_card / …_tool_call_plan /
  …_tool_observation / …_plan_dag / …_ask_response / …_escalation_packet / …_memory_proposal / …_process_observation`,
  all replayed by `replay.rs::toolplane_kind()`. The app layer has `broker_tool_call / ask / create_plan /
  execute_plan` and a `builtin_capabilities()` list of ~21 cards. The API exposes `/capabilities`,
  `/tools/:id/{dry-run,call}`, `/tool-calls`, `/ask`, `/ask/code`, `/plans`, `/zyal/status`, `/process-observations`.
- **The brains are running services** (verified ports/surfaces in the Appendix): Jeryu forge + **`jeryu-codegraph`**
  (SQLite symbol/crate-dep graph) + **jeryu-mcp (21 tools @ :9778)**; **jnoccio-router (13-tool MCP @ :8765)** incl.
  `worker_run`/`fanout_consensus` + on-box `local_vllm @ :18902`; Jekko **cogcore** deterministic memory +
  **ZYAL** durable daemon language; **Jankurai** (49 HLT rules, 47 commands incl. `copy-code`, `registry`,
  ratchet baselines); **veox** Warp/Enclave evolution API + telemetry + (503-disabled) paper-build.

**The catch: the scaffold is cosplay, not a fabric.** Three real gaps make it look done but behave inert:
1. `broker_tool_call` returns **canned JSON** (`"mode":"brokered-read-only"`) — it never makes a nested call.
2. **Discovery is a static `Vec`** — nothing calls MCP `tools/list`, reads a CLI catalog, or parses OpenAPI.
3. **No inbound MCP server** — `tools/list`/`tools/call` exist only *outbound* (private in `jmcp-adapter-jekko/src/worker_run.rs`).

### 1.1 Three corrections to the record (verified against live code)

The `tips/` were written against an old `jmcp-smallest.tar.gz` and could not compile; several claims are now **false**:

| Stale claim (tips / first recon) | Verified reality | Consequence for the plan |
|---|---|---|
| "`zyal.run` is deliberately **unrouted**; a test asserts `holder_for=None`." | `apps/jmcpd/src/dispatch_loop.rs:66` → `"zyal.run" => Some("zyal")`; test at `:139` asserts `Some("zyal")`; `ZyalAdapter` is wired at `:46/:55/:80`. | The work is **not** to add routing. It is to swap the blocking CLI poll for an async durable run handle. The real safety gate is `dispatch.rs::is_auto_dispatchable` (microtask && evidence_oriented && !live). |
| "No `CapabilityCard`/registry/planner/broker." | All present in `toolplane.rs`/`toolplane_records`/`broker_tool_call`. | **Evolve, don't rewrite.** Every change is additive; the build stays green slice-by-slice. |
| "`boundaries.toml` forbids `jmcp-domain` from importing `tracing::`." | The glob targets an **empty `crates/domain/` placeholder**, not `jmcp-domain`. It is clean by convention/Jankurai only. | Keep the discipline (PTY/transport/OTel live in non-domain crates) but **make the rule real** (extend `domain_paths` to include `jmcp-domain`) rather than citing a rule that doesn't fire. |

A fourth, found during exec-spec: **jekko's Rust daemon has no HTTP start/ARM route** (only `preview/list/get/pause/resume/abort`).
ZYAL *submission* therefore stays CLI (`jekko port-run --super`); only **run-control** is HTTP. The plan defines an
`HttpZyalRunner` seam for when jekko adds a start route, but does not assume it exists.

### 1.2 Owner decisions (baked in)

- **Log/telemetry spine = NATS JetStream** (canonical, aligned to JPCM + the Kafka-deprecation in `boundaries.toml`),
  with bridges to the existing **veox Kafka :9092** and **Jansu :4222** so nothing is siloed; the in-process SQLite
  event log stays the always-on default + authoritative replay anchor; all selectable behind one `Spine` trait.
- **Memory/brain = Hybrid**: JMCP store is **authoritative for *promoted* memory** (`MemoryRecord` provenance /
  poisoning / freshness / expiry + `PromotionDecision`); Jekko **cogcore** owns working/episodic memory + candidate
  generation. JMCP is strictly **read-only** against cogcore (preserves its determinism).
- **Autonomy = read/evidence only**: self-found work auto-runs at **R0/R1**; **any** mutation (files, branches, PRs,
  builds, deploys, external) requires approval — exactly today's `is_auto_dispatchable` gate.

## 2. The mental model — two orthogonal axes

The word "layer" means two different things in the two sibling docs. We keep **both**, on perpendicular axes.

**Axis A — architectural layers (L0–L7), the Codex spine = *what the components are*:**

| Layer | Component | Lives in | State today |
|---|---|---|---|
| **L0** | Authority Kernel (leases, approvals, audit, replay, redaction, budgets, stop-conditions) | `jcp-core`, `jmcp-store`, `jmcp-app`, `routes_approvals` | real, reused unchanged |
| **L1** | Capability Graph (registry of every tool/API/CLI/MCP/workflow + health) | `jmcp-domain/toolplane.rs`, `toolplane_records` | types exist; discovery is static |
| **L2** | Tool Broker (discover/dry-run/invoke/stream/cancel/health/quarantine, governed nesting) | `jmcp-app/toolplane.rs::broker_tool_call` | **simulated** |
| **L3** | Ask/Plan Kernel (`/ask`, `/ask/code`, `/plan`, evidence-backed answers) | `jmcp-app::ask/create_plan`, `routes_toolplane` | single-route stub |
| **L4** | Jeryu Code Oracle (codegraph over MCP+REST) | jeryu `jeryu-codegraph`/`jeryu-mcp`/`jeryu-api`, `jmcp-adapter-jeryu` | codegraph is **CLI-only** |
| **L5** | Jankurai Proof/Standards/Tool-Pool Oracle | jankurai CLI, `jmcp-adapter-jankurai` | CLI subprocess, digest-only |
| **L6** | Jekko/ZYAL Brain (heavy reasoning + hybrid memory) | jekko `cogcore`/ZYAL daemon, `jmcp-adapter-jekko` | routed, but blocking + CLI |
| **L7** | Mission Deck + raw-PTY/NATS spine + process awareness | `jmcp-now`, `jmcp-api/jitux`, **new** `jpty`/`jmcp-spine`/`jmcp-ingest` | deck one-shot; spine greenfield |

**Axis B — information tiers (L1/L2/L3), the operator's original ask = *how deep the data is*:**

| Tier | Meaning | Latency | Backed by |
|---|---|---|---|
| **L1 — Fingertips** | hot in-memory `FabricSnapshot`: all capability cards + live health + ecosystem graph + open escalations | sub-ms, zero I/O | last discovery sweep |
| **L2 — Drill-down** | fast SQLite/FTS over the projection tables ("every R≥4 tool with ExternalMutation", "last 10 invocations of X") | single-digit ms | `toolplane_records` + `capability_fts` |
| **L3 — Live** | an actual governed nested tool call (jeryu-mcp:9778, jnoccio:8765, jankurai CLI, REST, ZYAL) | network/process-bound | the runtimes |

**The rule that creates the JARVIS feel:** always answer from **information-L1** first, fall to **L2** on a miss,
and spend an **L3** call only to execute or when the answer is stale. Every architectural component (A) exposes all
three information tiers (B). The two axes compose; e.g. the *Capability Graph* (arch-L1) is read at fingertips
(info-L1), drilled via FTS (info-L2), and refreshed by live discovery (info-L3).

**North-star behaviors this must deliver:**
- *"What can you tell me about `broker_tool_call`?"* → classify → parallel read-only fan-out (codegraph + ecosystem +
  jmcp events/evidence + promoted memory) → evidence-cited answer (what/where/callers/callees/tests/health/risks/
  next-actions/confidence), escalating to Jekko synthesis only when confidence is low.
- *"Build veox on dataset X, then write the paper."* → one `PlanDAG`: `veox.job.submit` (R3, approval) → poll
  `veox.job.snapshot` (R0) → `veox.publication-plan` (R2 scratch) → ZYAL publication run (R6, approval) → Jankurai
  quality gate → human approval before any external publish.
- *"Anything stuck?"* → fingertip read of the process spine; drill into a PTY cast; attach/kill only under approval.

## 3. The seven layers — design (evolve the scaffold)

### L0 — Authority Kernel (reuse, don't reinvent)
Everything routes through JMCP's existing primitives: signed JCP envelopes (`jcp-core`), event-sourced SQLite +
replay (`jmcp-store`), leases (`claim_work_order`/`execute_with_lease`), approvals (`approval_challenges` +
`routes_approvals` + Telegram), attention/incidents, effect ledger. **No new authority mechanism is invented** by
any later layer — they all call back into L0. Invariant: the store lock is **never held across an `.await`**
(broker captures plan data, drops the lock, awaits the runtime, re-locks to persist) — the pattern `dispatch.rs`
already follows.

### L1 — Capability Graph (make discovery real)
- **Extend** `CapabilityCard` additively (all `#[serde(default)]`): `runtime` (`Internal/Mcp/Cli/Http/Workflow`),
  `endpoint`, `upstream_tool`, `depends_on`, `discovery_source`, `fingerprint` (blake3 of schema+endpoint+upstream,
  for drift). Add `CapabilityHealthSample` as its own replayable record.
- **Projections without new base tables**: lifecycle (`tool.call.started/completed/failed/escalated`) and
  `capability.health.recorded` ride the existing `toolplane_records` aggregate (last-write-wins by id → replay-safe).
  Add SQL **views** (`capability_cards`, `tool_invocations`, `tool_observations`, `capability_health`) + a
  `capability_fts` FTS5 virtual table for info-L2 drill-down.
- **Discovery + health sweep loop** in `jmcpd` (mirrors `dispatcher_loop`/`telegram_poll_loop`): MCP `tools/list`
  → jeryu:9778 & jnoccio:8765, CLI catalog (seeded from jankurai `agent/tool-adoption.toml` until a real
  `jankurai tools list --json` ships), OpenAPI from jekko:8080, ecosystem graph from jeryu:8799. Diff by
  `fingerprint`, persist (event-sourced), probe health, **atomically swap the info-L1 `FabricSnapshot`**
  (`Arc<ArcSwap<…>>` on `AppState`). Degradation is explicit (unreachable backend → `Degraded` card with reason,
  never dropped) — same discipline as `EcosystemSnapshot::degraded`. **Encrypted backends** (jeryu-fusion :4317,
  git-crypt) become a first-class `Degraded, schema-unknown` capability class.

### L2 — Tool Broker (turn cosplay into a fabric) — *new crate `jmcp-broker`*
- `trait ToolRuntime { kind; discover; health; dry_run; invoke }` with `RuntimeOutcome{output, evidence, redacted,
  diagnostic_class}`. Implementations:
  - **`McpRuntime`** — generalized JSON-RPC 2.0 `tools/list`+`tools/call`, **lifted from the proven
    `worker_run.rs` envelope**; targets jeryu-mcp:9778 (proto `2025-11-25`) and jnoccio-router:8765 (`2025-06-18`).
  - **`CliRuntime`** — bounded `tokio::process` + SHA256-of-output evidence (the jankurai pattern, 30s timeout).
  - **`HttpRuntime`** — REST discover (`/health`, OpenAPI) + invoke; jeryu:8799, jailgun:8787, veox:8000/8888, jekko:8080.
  - **`InternalRuntime`** — in-process `jmcp.*` ops; hosts the migrated canned behavior as an *explicit degradation
    fallback only* (never silent fakery).
  - **`WorkflowRuntime`** — long-running ZYAL/veox jobs (async handles, §L6/veox).
- `broker_tool_call` is **rewritten to delegate** to `ToolBroker::call`, applying the escalation gate (below) then
  dispatching to the runtime; it emits `tool.call.started/completed/failed/escalated`. The `Adapter` trait gains a
  **defaulted `capabilities() -> Vec<CapabilityCard>`** (default empty) so all five existing adapters compile
  untouched; jeryu/jankurai/jekko override it. A **`MAX_NEST_DEPTH`** budget + a `tool_call_edges (parent→child, depth)`
  view make the live nested-call tree queryable for cost, stuck-detection, and provenance.
- **Registry-driven dispatch**: replace `holder_for`'s hardcoded `match` with a registry-seeded map (keep
  `base_routes()` so the existing routing test passes), so `jailgun.*`/`veox.*`/discovered cards become routable —
  routing ≠ auto-run; `is_auto_dispatchable` stays the gate.

### L3 — Ask/Plan Kernel (the "What can you tell me about BLANK?" pipeline) — *new `jmcp-app/src/ask_plan.rs`*
- **Deterministic `classify_blank`** → `BlankKind {Symbol,File,Crate,Route,Tool,Repo,Incident,Memory,Cli,Port,Error,Unknown}`
  (pure, table-tested).
- **Typed read-only DAG** → budgeted **parallel** fan-out (info-L1/L2): jeryu code-oracle + `/api/v1/ecosystem` +
  jmcp events/evidence/work-orders + promoted `MemoryRecord` search, via `spawn_blocking` per leg (never hold the
  store lock across the timeout). First-evidence budget ~250–500 ms; escalate to **`jekko.reason` (R1)** only when
  confidence is low; **never** auto-escalate to ZYAL.
- **`AnswerContract`** (`what/where/callers/callees/tests/health/risks/suggested_next_actions/confidence/evidence`),
  added as `Option<AnswerContract>` on `AskResponse` (`serde(default)` → back-compatible). Partial answers stream over
  a dedicated **`POST /ask/stream`** SSE; the plan + each tool call render on the Mission Deck. A slow leg lands as
  `status:Failed, diagnostic_class:"budget_exceeded"` with `Confidence::Low` — **never fabricated data**.

### L4 — Jeryu Code Oracle (expose the graph that already exists)
- **Do not add a graph DB.** The data is small and crate-scoped; every store in the stack is deterministic-SQLite-first;
  recursive traversal fits SQLite CTEs. The leverage is **edge enrichment**, not the engine. (Revisit only when true
  cross-repo symbol resolution is needed.)
- In **jeryu**: capture real declaration `line` during `index_workspace`; add a `codegraph_symbol_refs` table +
  `ensure_indexed(store, root, head_digest)` (commit-digest-cached on-demand build, **no daemon** — :8799 is protected);
  add **6 MCP tools** (`code.symbol/references/definition/impact/owners/tests`, all `readOnlyHint=true`) via the single
  `CATALOG` seam (auto-appear in `/api/v1/ecosystem`) and matching **REST** `/api/v1/code/*`. Honesty: references are
  grep-derived (word-boundary, crate-scoped, common-ident stoplist) and **labelled** as `resolution: name-match` — they
  inform `callers/callees` but never drive impact/blast-radius numbers (impact stays at verified crate-dep granularity).
- In **jmcp**: `JeryuCodeOracle` trait on `HttpJeryuClient` (`search_symbols/references/impact/answer`) feeding the
  broker; `jeryu.code.*` work-order kinds are **R0 read+evidence, auto**. Evidence unit = `CodeEvidenceSpan{crate,file,
  line_start,line_end,symbol,kind,snippet}`. (Implement against canonical `/home/ubuntu/jeryu`; reconcile
  `jeryu-coverage` separately.)

### L5 — Jankurai Proof / Standards / Tool-Pool Oracle (project, don't reinvent)
- Replace the digest-only CLI adapter with a `trait JankuraiClient` (modeled on `HttpJeryuClient`: trait-abstracted,
  fail-closed, deterministic stubs); ship a **CLI-JSON** contract first, optional loopback **`jankurai serve --mcp`**
  later to kill 30s spawn latency (bind through the shared port guard; never a protected port).
- **Shared tool POOL with usage tracking = a `ToolPoolStat` projection over the `ToolObservation` stream** the broker
  already writes — usage tracking comes for free (which repos use which tools, run counts, finding rates).
- **Human-entropy / CI-variance** = cross-repo extension of the in-repo `copy-code` engine, scoped to ops surfaces
  (CI/scripts/config), emitting both a **reuse score** ("pool these") and a **variance/drift score** ("human entropy");
  high-variance surfaces become `PoolingProposal`s backed by `tool-adoption.toml`.
- **False-positive learning** unifies onto Jankurai's two existing suppressors — ratchet-baseline fingerprints
  (`allowed_drop=0`) and timeboxed `exceptions` docs — fed by a `FalsePositiveRecord` ledger; rule-confidence tuning
  only adjusts `confidence_policy`/advisory gating (**never silently disables a rule**; demotion High→Medium→advisory,
  owner-approved via `AttentionPacket`). The **L1 universe score** becomes Jankurai-audit-sourced.

### L6 — Jekko/ZYAL Brain (hybrid memory + async durable runs)
- **`ZyalRunHandle`** (durable, event-sourced on `toolplane_records`; re-attachable after restart via replay):
  `begin_zyal_run` submits non-blocking and persists `{Running, percent, phase}`; a JMCP-owned **`sweep_zyal_runs`**
  poller (cadenced) replaces the synchronous **30-min in-lease poll** in `zyal.rs` and the too-short **3×/1.5s**
  `worker_run` budget (the latter now applies only to jnoccio job_result). `pause/resume/abort` map to the real jekko
  daemon routes; **submission stays CLI** (`port-run --super`) until jekko exposes a start route (`HttpZyalRunner` seam ready).
- **`BrainSession`** = a persisted handle to a long-lived ZYAL run that owns heavy synthesis + candidate generation
  (re-attachable, not a held socket). This is the "offloaded brain."
- **Hybrid memory refinery**: `import_zyal_memory_candidates` reads only a **redacted `ZyalCapsuleView`**
  (`claim_text` + `content_hash`; never `payload_json`/embedding) → `MemoryProposal`; `decide_memory_promotion`
  flips `MemoryRecord` state + appends `PromotionDecision` (**JMCP-only authority**); `expire_stale_memory` decays
  non-promoted past-expiry records (promoted never auto-expire). ZYAL **maintenance** kinds
  (`zyal.memory.compact/contradiction-scan/freshness-refresh`) are **R1 evidence-only** and write **no** `MemoryRecord`.
  JMCP mints all its own timestamps/UUIDs → cogcore determinism untouched.

### L7 — Mission Deck + raw-PTY → NATS spine + process awareness
- **`Spine` trait** with `InProcessSqlite` (today's bus, always-on default + replay anchor) shipped first;
  `NatsJetStream` (canonical cross-host) + `Kafka`/`Jansu` bridges deferred behind the trait until a broker is
  operationally proven (per `boundaries.toml`'s replacement-proof requirement). Selectable via `JMCP_SPINE`.
- **`jpty`** (new non-domain crate): a thin `portable-pty`/openpty/ConPTY shim every long-running/live work-order
  launches under (opt-in via the existing `execution_boundary.pty`, default-on for live kinds). It writes
  asciinema-cast-compatible content-addressed logs and emits signed JPCM heartbeat/exit envelopes.
  **Redaction runs *before* hashing** (reuse `PreparedAction::validate_no_secret_material`) so a secret echoed to a
  terminal never becomes immutable store content; a `secret-in-cast` test gates the spine; JPCM `retention_days` applies.
- **Two-tier liveness**: `jpty` local idle/hard-timeout watchdog + a central **`jmcp-watchdog`** loop running
  `Running→Idle→Stuck→Incident` over the **existing `ProcessObservation` table** (no PTY required for phase A),
  reusing/extending `DefaultAttentionPolicy` + `AttentionPacket`/`IncidentRecord`. Raw runs become governed evidence
  (`pty.cast` + `process.exec` effect-ledger entries); OTel spans/metrics → evidence via the JPCM observability block.
- **Mission Deck goes continuous**: turn the one-shot JITUX backlog stream into a live SSE/broadcast fed by
  watchdog+ingest; add `PaneKind::Process`; float stuck jobs to the top via existing `jmcp-now` ranking.
- **L3 attach/kill is mutating** → R3+ → routed through approval + lease + effect-ledger. Autonomous (no-human-present)
  hard-kill authority is an explicit policy decision (see blind spots).

### veox (spans L2 + L7) — MCP adapter + warp awareness + autonomous ZYAL publication — *new `jmcp-adapter-veox`*
- New veox domain types carry **only already-redacted metrics/hashes** (`ChampionMetric` exposes `weights_hash`,
  never weights) — the encrypted-enclave boundary enforced at the **type level**. The **aegis** key/DEK/`veox_rpc`
  token are `EnvironmentSecret`-classed: read from env at call time, never stored on the struct's `Debug`, never in
  `Evidence`/SQLite/tracing (a no-secret-in-Evidence test gates it, mirroring jeryu/jekko convention).
- Capabilities: `veox.job.snapshot` (R0) / `veox.telemetry.subscribe` (R1) are auto/evidence-only; `veox.job.submit` /
  `veox.build` / `veox.dataset.upload` (R3) and **external publish** (R4) are approval-gated. WS telemetry is decimated
  (~1 summarized row/5s into the L7 spine; live 2Hz on demand) and drives **plateau-based stuck detection** (score/gen
  flat + non-empty queue + pinned compute), not work-order-timestamp staleness.
- **Publication** is authored **JMCP-side via ZYAL** (keep veox `build_paper` 503 so the enclave stays a pure
  metrics/hash provider): pull results → assemble an evidence pack → ZYAL incubator run → **Jankurai quality gate** →
  **human approval** before any external publish. Reuse the existing jekko (ZYAL) + jankurai crates; do not mint parallel ones.

## 4. Cross-cutting governance & invariants

**Unified escalation ladder (R0–R7).** Append `R6,R7` to the existing `RiskTier` (append-only → stored cards keep
deserializing); keep the existing helper thresholds, add `requires_dual_control() = R7`:

| Tier | Meaning | Gate (reuses L0) |
|---|---|---|
| R0 | read-only local/JMCP/jeryu/jankurai/memory/codegraph | auto |
| R1 | bounded model read / evidence-only remote | auto within budget |
| R2 | local scratch / dry-run / generated plan | lease |
| R3 | repo branch/worktree/patch/PR prep · veox build/submit/dataset | approval (+ Jankurai proof) |
| R4 | external durable mutation: repo create, PR open, CI trigger, jailgun/veox run, publish | explicit human approval |
| R5 | secrets / deploy / global memory promotion / new privileged tool / destructive | human approval + rollback proof |
| R6 | multi-system orchestration (ZYAL `RUN_FOREVER`, veox build→publish across >1 system) | approval + budget + rollback |
| R7 | mutate JMCP's **own** authority (registry trust, lease policy, the ladder itself) | **dual-control** (N-of-M distinct approvers) |

One **central risk-tier table** owns every capability's tier (resolves the P1↔P6 veox divergence). **R7 is not shipped
in v1** unless a real multi-party (distinct-identity) approval primitive lands — two sequential single-approver Telegram
challenges is **not** dual-control and is explicitly disallowed for an authority-mutation tier.

**Coordination & safety invariants:**
- **Phase 0 foundation PR (single owner, no behavior change)** lands *all* `toolplane.rs` domain additions together —
  because **5 layers touch that file and 3 rewrite `broker_tool_call`/`ask()`**. `RiskTier` append-only; new fields
  `serde(default)`; a replay test proves old stored cards still deserialize. After Phase 0, **strict single-ownership**:
  L2 owns the broker rewrite; L3 owns `ask()`; L4/L5/L6/veox plug *runtimes* into the broker, they don't re-edit it.
- **Hoist the protected-port guard** (`2224,8787,8799,8929,18787,18788,19800`) from `main.rs` (API-bind only) into a
  shared helper that **every** new bind path calls (jpty, OTLP, NATS, `jankurai serve --mcp`, veox-mcp). Verify any
  proposed new port is actually free before committing.
- **Two-jnoccios naming rule** (foundation PR): `jnoccio-router` (:8765, `worker_run`/`review_patch`) and
  `jnoccio-fusion` (:4317, chat completions) are **distinct providers**; never collapse to `jnoccio`; route by
  `CapabilityCard.endpoint`, not name.
- **Inbound `/mcp` requires bearer auth + a per-caller allowlist**: front-line Claude gets the full governed list;
  external (`.claude.json`) agents default to **read-only R0/R1** and never see R4+ mutating tools in `tools/list`.
  Ship the allowlist *with* the server.
- **Verify-before-build gates**: fetch jekko's live `/openapi.json` and capture a real `/api/v1/daemon` status sample
  into a new golden *before* swapping the ZYAL runner (keep the CLI golden); confirm a deterministic cogcore read path
  exists *before* building the memory refinery's candidate factory.
- **Replay / no-reissue**: async task handles and spine fan-in must not double-execute side effects on replay —
  add an explicit conflict test.

## 5. Blind spots to close (covered by no pillar)

- **Front-line agent UX** — how Claude itself consumes the info-L1 `FabricSnapshot`: wire `.claude.json` → JMCP
  `POST /mcp`; inject an L1 context pack at session init (beyond Mission Deck frames). *This is the actual JARVIS-facing
  contract and must be designed alongside the inbound MCP server.*
- **Voice** — `VoiceSession` + ASR:16000/TTS:17000 exist but no layer uses them; route voice through the same `/ask`
  and approval paths (no separate authority).
- **Telegram as a command/question channel** — today it is only an approval *sink*; add ask-over-Telegram + status push.
- **Cost / token / GPU budgets** — R6 has a `budget_required` flag but nothing computes/enforces a budget across ZYAL
  superworkflows, jnoccio LiteLLM spend, local_vllm/RTX-3090 contention, or veox GPU jobs. Build a budget accountant.
- **Multi-host scale** — named, deferred behind the NATS spine; document lease/approval/event-sourcing semantics when
  the spine is genuinely distributed.
- **Long-run crash recovery** — add a test: arm a `BrainSession`, kill `jmcpd`, restart, prove re-attach to the daemon
  `run_id` from event replay and resumed progress sweeps. The durability promise must be *tested*, not asserted.
- **jnoccio `worker_run` 3×500ms poll fix** — give it a named owner (belongs with L6's async-handle work).
- **Encrypted-backend discovery class** — commit to `Degraded, schema-unknown` for opaque backends (jeryu-fusion :4317).

## 6. Phased roadmap

- **Phase 0 — Truth reconciliation + foundation (ONE PR, single owner, blocking, no behavior change):** correct the
  `zyal.run`-is-routed reality; land all `toolplane.rs` additions (R6/R7 appended, new `CapabilityCard` fields
  `serde(default)`, `CapabilityHealthSample`, `ZyalRunHandle`/`BrainSession`/`ToolPoolStat`/`AnswerContract`) with a
  replay test; hoist the protected-port guard; publish the central risk-tier table; set the jnoccio naming rule; make
  the `jmcp-domain` tracing rule real.
- **Phase 1 — Real execution substrate (L2):** `jmcp-broker` crate + `McpRuntime` (reuse the `worker_run` envelope) +
  rewrite `broker_tool_call` to dispatch to runtimes + the escalation gate reusing leases/approvals. *Substrate L4/L5/L6/veox plug into.*
- **Phase 2 — Code oracle (L4):** codegraph query API + indexes + `/api/v1/code/*` + 6 MCP tools; make
  `jeryu.code.*` real; real `ask()` fan-out + `AnswerContract` (L3).
- **Phase 3 — Verify-then-build external contracts (L6):** `DaemonZyalRunner`/run-control gated on the fetched jekko
  openapi + new daemon golden; confirm the cogcore read path before the memory refinery.
- **Phase 4 — Awareness on existing data (L7-A):** `jmcp-watchdog` over the existing `ProcessObservation` table +
  continuous Mission Deck. *No PTY yet.*
- **Phase 5 — Structured standards contract (L5):** `jankurai tools list --json`, `JankuraiClient`, `ToolPoolStat`
  usage projection.
- **Phase 6 — veox read-only:** `jmcp-adapter-veox` health/snapshot/attestation + secret-redaction test + R0 cards.
- **Phase 7+ — heavy tails (each behind its own gate):** `jpty` + NATS spine (redaction-before-hash mandatory; InProcess
  first), ZYAL async task-handles + memory refinery + crash-recovery test, veox telemetry + governed launch + ZYAL
  publication, inbound `/mcp` **with auth+allowlist**, Jankurai entropy + confidence tuning.
- **In parallel once Phase 1 lands:** front-line `/mcp` consumer + `.claude.json` wiring, cost/budget enforcement for
  R6, and the multi-party R7 approval primitive (or shelve R7).

## 7. The first PR — the first JARVIS moment

Build `jmcp-broker`'s **`McpRuntime`** (reusing the exact JSON-RPC envelope already proven outbound in
`crates/jmcp-adapter-jekko/src/worker_run.rs`) and rewrite `broker_tool_call` so that **exactly one R0 read-only
capability — `jeryu-mcp get_system_snapshot`** — makes a *real* nested `tools/call` to `127.0.0.1:9778` and returns
live `structuredContent`, replacing the canned `"brokered-read-only"` stub for that one card. Keep the escalation gate
so anything > R1 still hits the existing leases/approvals path unchanged, and keep the canned output as the explicit
no-runtime-wired fallback. This converts the broker from cosplay to a real fabric, proves the envelope reuse, gives
L4/L5/L6/veox a working substrate, and is the literal first JARVIS moment: **JMCP making a real governed nested tool
call and reading another agent's live state** — without touching shared domain types beyond Phase 0.

## 8. Verification

- **Per slice:** `cargo fmt --all -- --check`, `cargo check/test --workspace --all-targets --locked`, cockpit
  typecheck/test/build, `just fast` / `just ci` / `just security` / `just conformance`, and `just jankurai-local`.
- **Phase 0:** replay test — old stored `CapabilityCard` JSON still deserializes after the field/tier additions;
  `holder_routing_*` still green.
- **Phase 1 (the JARVIS moment, end-to-end):** start jeryu (`:9778`) + jmcpd; `POST /tools/jeryu.system.snapshot/call`
  returns live `structuredContent`; an R4 card (`zyal.run`) returns `GateDecision::Escalate` with one `EscalationPacket`
  and **never** reaches `runtime.invoke` (spy-runtime assert). Conformance test in `jmcp-conformance`.
- **Phase 2:** `POST /ask "what can you tell me about broker_tool_call?"` returns an `AnswerContract` with ≥2
  observations + file/line `CodeEvidenceSpan`s; a slow leg degrades honestly (no fabricated symbols).
- **Phase 3:** daemon golden passes; `begin_zyal_run` returns <1s and persists `{Running}`; `sweep_zyal_runs` drives a
  stub Running→Completed; user-submitted `zyal.run` stays non-auto-dispatchable.
- **Phase 4/7:** kill-`jmcpd`-mid-run crash-recovery test re-attaches; `secret-in-cast` redaction test gates the spine;
  inbound `/mcp` allowlist test proves external agents can't see/call R4+ tools.

## 9. Appendix — verified ground-truth reference

**Ports** — jmcp API **18877** (cockpit 15873); jeryu REST **8799** (README says 8787; jmcp adapter uses 8799),
jeryu-mcp **9778** (stdio + loopback HTTP, proto `2025-11-25`, 21 tools); jnoccio-router MCP **8765** (proto
`2025-06-18`, 13 tools); jekko jnoccio-fusion **4317** (encrypted, `/v1/chat/completions`), jekko serve **8080**
(`/api/v1/daemon`, `/openapi.json`); jailgun **8787**; veox warp **8000** / enclave **8090** / gateway **8888** /
local_vllm **18902**; speech ASR **16000** / TTS **17000**; veox bus Kafka **9092** / Jansu **4222**.
**Protected (never bind):** 2224, 8787, 8799, 8929, 18787, 18788, 19800.

**Already present in jmcp (do not recreate):** `toolplane.rs` (CapabilityCard, RiskTier R0–R5, PlanDAG, EscalationPacket,
MemoryProposal, ProcessObservation, Ask*); `toolplane_records` (11th table) + record/replay arms;
`broker_tool_call`/`ask`/`create_plan`/`execute_plan` (simulated); routes `/capabilities`, `/tools/:id/{dry-run,call}`,
`/tool-calls`, `/ask`, `/ask/code`, `/plans`, `/zyal/status`, `/process-observations`; `dispatch_loop.rs` routes
`zyal.run→Some("zyal")`; `is_auto_dispatchable` (microtask && evidence_oriented && !live) is the real auto-gate;
`MemoryRecord`+`PromotionDecision` state machine; leases/approvals/attention/incidents/effect-ledger; `jmcp-now`
9-factor ranking; JITUX Mission Deck (one-shot today). Domain `MemoryRecord` provenance/freshness/poisoning/expiry exist.

**Already present in the brains:** jeryu `jeryu-codegraph` (SQLite symbol+crate-dep graph at `~/.jeryu/codegraph.sqlite`,
CLI `index/impact/slice-check`, **CLI-only**) + jeryu-mcp 21 tools + `/api/v1/ecosystem`; jnoccio-router 13 tools +
local_vllm; jekko `cogcore` (WAL+Hebbian+FSRS+BM25, deterministic, `Budget::ZERO`) + ZYAL daemon (durable, 9 tables,
`.jekko/daemon/<run>/`, arm via CLI `port-run --super`, run-control via HTTP, **no HTTP start route**); jankurai 49 HLT
rules / 47 commands (`copy-code`, `registry`, ratchet baselines `allowed_drop=0`, `tool-adoption.toml`); veox Warp/Enclave
evolution API + WS telemetry + aegis enclave auth + 503-disabled paper-build + Kafka/Jansu bus.

**New crates this plan introduces:** `jmcp-broker` (L2 runtimes), `jmcp-api/routes_mcp.rs` (inbound MCP server),
`jmcp-app/{ask_plan,dispatch_zyal,brain_session,memory_refinery}.rs`, `jpty` + `jmcp-spine` + `jmcp-ingest` (L7),
`jmcp-adapter-veox`. Everything else is **additive edits** to existing files.

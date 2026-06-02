# JMCP V1 Security Model

## Security Posture

JMCP V1 is local by default, but it must not treat locality as a substitute for security. The core security goals are explicit approval, bounded adapters, secret-safe auditability, replay safety, and reproducible verification.

## Security Lane

The explicit security lane is `ops/ci/security.sh`, which is also what the GitHub security workflow executes. It runs secret scanning, dependency scanning, and workflow linting, then writes its artifacts under `.artifacts/security/`.

## Trust Boundaries

- **Operator interfaces:** React dashboard, Rust TUI, and Telegram approval flows may request or display decisions but do not bypass backend policy.
- **Adapters:** Jankurai, Jeryu, Jailgun, and Jekko are capability providers. They are untrusted with respect to policy enforcement.
- **Persistence:** SQLite is authoritative local state and must not store raw secrets in audit-oriented tables.
- **Event bus:** The in-process bus carries runtime facts and decisions. Events are inspectable and replayable, so payloads must be classified before logging or export.

## Approval Requirements

Any operation that mutates durable state, calls external services, changes adapter configuration, or releases user-visible output requires an approval gate unless it is explicitly classified as read-only. Approval records must include actor, operation, decision, timestamp, and correlation id.

## Secret Handling

Secret-like fields are redacted from logs and audit exports. Raw inbound payloads are not logged by default. Diagnostic records should preserve classes and identifiers, not credentials, tokens, cookies, or full authorization headers.

## Jailgun Boundary

JMCP starts Jailgun agent runs through `POST {JMCP_JAILGUN_URL}/api/runs` and
sends `JMCP_JAILGUN_TOKEN` only as `x-jailgun-token`. The URL must match a
configured local submission policy entry exactly after normalization; wildcard
hosts, broad CIDR-style matching, and response-provided cross-origin URLs are
rejected.

If the Jailgun URL is reached through SSH port forwarding, the tunnel setup must
use an exact user/host/port target and pinned host keys in `known_hosts`. Do not
use wildcard host-key trust, agent-forwarded broad bastion targets, or a shared
local port that can be rebound by another service.

Jailgun run requests are inline JSON by default. Compatibility `request_path`
input is read and submitted as the same JSON body. Jailgun run requests,
review-packet requests, summaries, and review packets must carry `version: 1`;
unsupported or missing versions fail before JMCP accepts the evidence.

## Autonomous Action Boundary

`GET /autonomous-actions` exposes the committed full-auto ZYAL catalog, and
`POST /autonomous-actions/:id/submit` self-submits a signed local JCP envelope
through the same `submit_envelope` path used by external clients. Initial
actions default to `live=false`, bounded stage/time limits, evidence-oriented
metadata, and `submitted_by: "jmcp.full_auto"`. Live execution or broader
mutation requires a separate approval policy instead of an override.

## Replay Safety

Replay must not duplicate committed side effects. Side effects need idempotency keys, approval references, or replay guards. During replay, JMCP should reconstruct state from stored facts and mark side-effect execution as skipped, simulated, or already committed.

## Auditability

Audit records are security evidence. They must be structured, minimally sufficient, and exportable as local JSONL. See `agent/audit-policy.toml` for event classes and redaction rules.

## Release Evidence

Security release evidence should be traceable to `docs/release.md`, the security workflow, and `target/jankurai/security/evidence.json`.

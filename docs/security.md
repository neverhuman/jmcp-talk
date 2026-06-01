# JMCP V1 Security Model

## Security Posture

JMCP V1 is local by default, but it must not treat locality as a substitute for security. The core security goals are explicit approval, bounded adapters, secret-safe auditability, replay safety, and reproducible verification.

## Security Lane

The explicit security lane is `ops/ci/security.sh`, which is also what the GitHub security workflow executes. It runs secret scanning, dependency scanning, and workflow linting, then writes its artifacts under `.artifacts/security/`.

## Trust Boundaries

- **Operator interfaces:** React dashboard, Rust TUI, and Telegram approval flows may request or display decisions but do not bypass backend policy.
- **Adapters:** Jankurai, Jeryu, and Jekko are local capability providers. They are untrusted with respect to policy enforcement.
- **Persistence:** SQLite is authoritative local state and must not store raw secrets in audit-oriented tables.
- **Event bus:** The in-process bus carries runtime facts and decisions. Events are inspectable and replayable, so payloads must be classified before logging or export.

## Approval Requirements

Any operation that mutates durable state, calls external services, changes adapter configuration, or releases user-visible output requires an approval gate unless it is explicitly classified as read-only. Approval records must include actor, operation, decision, timestamp, and correlation id.

## Secret Handling

Secret-like fields are redacted from logs and audit exports. Raw inbound payloads are not logged by default. Diagnostic records should preserve classes and identifiers, not credentials, tokens, cookies, or full authorization headers.

## Replay Safety

Replay must not duplicate committed side effects. Side effects need idempotency keys, approval references, or replay guards. During replay, JMCP should reconstruct state from stored facts and mark side-effect execution as skipped, simulated, or already committed.

## Auditability

Audit records are security evidence. They must be structured, minimally sufficient, and exportable as local JSONL. See `agent/audit-policy.toml` for event classes and redaction rules.

## Release Evidence

Security release evidence should be traceable to `docs/release.md`, the security workflow, and `target/jankurai/security/evidence.json`.

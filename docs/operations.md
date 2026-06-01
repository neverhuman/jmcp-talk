# JMCP V1 Operations

## Local Operating Model

JMCP V1 runs as a local production-shaped system. The operator should be able to start the backend, inspect state in the React dashboard or Rust TUI, process Telegram text intake/approvals where configured, and replay events using embedded SQLite and the in-process event bus.

## Release Gate

Operations and release use the same local proofs. A candidate is ready only when the fast, conformance, security, rendered UX, and workspace build lanes all pass and the advisory score baseline is non-regressing. See `docs/release.md`.

## Startup Expectations

Startup must:

- locate or initialize the SQLite database;
- apply pending migrations;
- initialize the in-process event bus;
- register local adapters;
- expose dashboard/TUI/API surfaces;
- report readiness with component-level status.

## Shutdown Expectations

Shutdown must drain in-flight approval and audit writes, stop adapter intake, checkpoint replay state, and close SQLite cleanly. Forced shutdown may leave incomplete operations, but recovery should classify them explicitly.

## Backup and Recovery

The SQLite database is the primary local artifact to back up. Audit exports should be generated as JSONL for review or long-term retention. Recovery should prefer replaying durable events and approval records over reconstructing state from logs.

## Degraded Operation

If Telegram is unavailable, local dashboard and TUI approvals remain authoritative. If an adapter fails, the core runtime continues and records the adapter failure class. If the dashboard fails, the TUI remains the recovery surface.

## Operator Evidence

Operations should produce evidence suitable for incident review: component readiness, migration status, adapter registration, approval decisions, policy denials, replay runs, and audit export metadata.

## Repair Evidence

When a lane fails, keep the repair evidence local and reviewable. The expected evidence trail is the score report, the security artifact bundle, the UX proof bundle, and any replay or migration receipt written under `target/jankurai/`.

# JMCP Talk Data Boundary

`jmcp-talk` is a speech runtime and sidecar repository. It does not own a
production database, durable approval records, tool policy, ledgers, or voice
turn state.

Durable truth lives in `jmcp-core`. Talk may emit local voice receipts and
runtime evidence under configured artifact directories, but those records are
diagnostic evidence rather than authoritative product state.

## Migration Policy

No database migration should be added here unless ownership is moved through a
reviewed boundary change. If a future talk feature needs durable storage, add the
schema to `jmcp-core` first and expose it through generated contracts.

Any approved future migration must include rollback safety, reviewed backfill
steps, and explicit proof that data access remains compartmentalized behind the
owning Rust/API boundary. Talk runtime code must not import database clients
directly.

## Constraint Policy

Constraints for talk-owned artifacts are file/artifact constraints only:
redaction, retention, local-only binding, and explicit receipt paths. Database
constraints belong to `jmcp-core`.

If ownership is ever moved, database constraints must include foreign key
integrity, row level security or an equivalent isolation control, and generated
contract drift proof before any runtime path reads or writes the data.

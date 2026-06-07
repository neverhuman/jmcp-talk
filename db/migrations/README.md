# Migration Boundary

This directory is intentionally empty. `jmcp-talk` does not own database
migrations; durable schema changes belong to `jmcp-core`.

If a reviewed ownership change ever adds migrations here, every migration must
ship with rollback safety notes, backfill proof, and generated contract drift
proof.

# JMCP Talk Contracts Agent Instructions

`contracts/` owns speech adapter contract documents for talk-owned runtime
surfaces.

- Keep speech contracts generated-zone declared in `agent/generated-zones.toml`.
- Verify contract changes with `just contract-drift`.
- Do not hand-edit contract routes without updating speech daemon and adapter tests.

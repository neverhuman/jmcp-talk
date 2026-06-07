# JMCP Talk Release

A release candidate is ready only after the deterministic local proof set
passes:

- `CHANGELOG.md` records the release delta and rollback notes
- `just fast`
- `just test`
- `just contract-drift`
- `just security`
- `just cost-budget`
- `just release-readiness`
- `just score`

Live GPU receipts can support promotion but do not replace deterministic local
proof. The rollback path is to revert the reviewed release commit and rerun the
same proof set.

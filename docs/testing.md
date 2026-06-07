# JMCP Talk Testing

Default local proof:
- `just fast`
- `just test`
- `just contract-drift`
- `just security`
- `just cost-budget`
- `just release-readiness`
- `just score`

Python sidecar tests must use deterministic fixtures unless a command is
explicitly documented as a GPU/live lane. Live MiniCPM-o validation is routed
through `just gpu-live` and `.github/workflows/gpu-live.yml`.

Receipts belong under `target/jankurai/`.

## Launch-Gate Evidence

Release readiness requires artifact-backed evidence, not just passing commands:
security evidence at `target/jankurai/security/evidence.json`, cost evidence at
`target/jankurai/cost-budget.json`, and launch evidence at
`target/jankurai/release-readiness.json`. The launch gate covers security,
backup and recovery, monitoring, rollback, abuse controls, and zero-spend stop
conditions from `agent/cost-budget.toml` and `agent/security-policy.toml`.

Cost budget proof is mandatory for any live or external lane. The default quota
caps are `external_api_usd = 0`, `model_api_usd = 0`, and `gpu_cloud_usd = 0`;
`JMCP_COST_KILL_SWITCH=1` is the kill switch. The stop-condition policy is to
stop when a receipt is missing, an unknown paid tool appears, a quota is
exceeded, or the kill switch is set.

## Agent-Friendly Exception Pattern

- purpose: make failed speech and voice gates repairable without hidden context.
- reason: each failure should name the lane, artifact, owner route, sidecar profile, and rerun command.
- common fixes: rerun `just score`, rerun `just security`, rerun Python unit tests, repair `agent/test-map.json`, or switch to deterministic sidecar fixtures.
- docs_url: `docs/testing.md`
- repair_hint: use the failing finding path to choose the smallest `agent/test-map.json` command.

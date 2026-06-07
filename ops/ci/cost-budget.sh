#!/usr/bin/env bash
set -Eeuo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cd "$ROOT_DIR"
mkdir -p target/jankurai

log "cost-budget: validating local zero-spend manifest"
python3 - <<'PY'
import json
from pathlib import Path
import tomllib

manifest_path = Path("agent/cost-budget.toml")
manifest = tomllib.loads(manifest_path.read_text())
for key in ("default_external_spend_usd", "default_network_spend_usd"):
    if manifest.get(key) != 0:
        raise SystemExit(f"{key} must be zero by default")
quota_caps = manifest.get("quota_caps", {})
for key in ("external_api_usd", "model_api_usd", "gpu_cloud_usd"):
    if quota_caps.get(key) != 0:
        raise SystemExit(f"quota cap {key} must be zero by default")
receipt = {
    "ok": True,
    "manifest": str(manifest_path),
    "quota_caps": quota_caps,
    "kill_switch_env": manifest["kill_switch_env"],
}
Path("target/jankurai/cost-budget.json").write_text(json.dumps(receipt, indent=2) + "\n")
PY

log "cost-budget: complete"

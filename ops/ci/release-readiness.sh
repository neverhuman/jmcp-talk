#!/usr/bin/env bash
set -Eeuo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cd "$ROOT_DIR"
mkdir -p target/jankurai

log "release-readiness: validating release evidence surface"
python3 - <<'PY'
import json
from pathlib import Path

required_files = [
    "CHANGELOG.md",
    "docs/release.md",
    "docs/testing.md",
    "docs/operations.md",
    "agent/cost-budget.toml",
    "agent/security-policy.toml",
]
missing = [path for path in required_files if not Path(path).exists()]
text = "\n".join(Path(path).read_text() for path in required_files if Path(path).exists())
terms = [
    "just fast",
    "just security",
    "just contract-drift",
    "just cost-budget",
    "target/jankurai",
    "target/jankurai/security/evidence.json",
    "target/jankurai/cost-budget.json",
    "target/jankurai/release-readiness.json",
    "security",
    "backup",
    "monitoring",
    "rollback",
    "abuse",
]
missing_terms = [term for term in terms if term not in text]
receipt = {
    "ok": not missing and not missing_terms,
    "required_files": required_files,
    "missing_files": missing,
    "missing_terms": missing_terms,
}
Path("target/jankurai/release-readiness.json").write_text(json.dumps(receipt, indent=2) + "\n")
if not receipt["ok"]:
    raise SystemExit(f"release readiness missing evidence: files={missing} terms={missing_terms}")
PY

log "release-readiness: complete"

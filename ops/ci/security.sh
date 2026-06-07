#!/usr/bin/env bash
set -Eeuo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cd "$ROOT_DIR"
mkdir -p .artifacts/security

if has gitleaks; then
  log "security: running gitleaks"
  gitleaks detect --source . --config gitleaks.toml --no-banner --redact --no-git
else
  missing_tool gitleaks "secret scanning"
fi

if [[ -e Cargo.lock ]]; then
  log "security: running cargo-audit"
  run_if_has cargo-audit "RustSec advisory scanning" cargo audit --ignore RUSTSEC-2024-0436 --ignore RUSTSEC-2026-0002
fi

if has zizmor; then
  log "security: running zizmor"
  zizmor .github/workflows
else
  missing_tool zizmor "GitHub Actions security linting"
fi

if has syft; then
  log "security: generating SBOM"
  syft dir:. -o spdx-json=.artifacts/security/jmcp-talk.spdx.json
else
  missing_tool syft "SBOM generation"
fi

log "security: complete"

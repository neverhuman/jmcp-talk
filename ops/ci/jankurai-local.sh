#!/usr/bin/env bash
set -Eeuo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cd "$ROOT_DIR"

log "jankurai-local: running local parity gates"
"${ROOT_DIR}/ops/ci/fast.sh"
"${ROOT_DIR}/ops/ci/ci.sh"
"${ROOT_DIR}/ops/ci/security.sh"
"${ROOT_DIR}/ops/ci/cost-budget.sh"
"${ROOT_DIR}/ops/ci/release-readiness.sh"
"${ROOT_DIR}/ops/ci/language-bad-behavior.sh"
jankurai audit . --full --mode advisory --json .jankurai/repo-score.json --md .jankurai/repo-score.md
log "jankurai-local: complete"

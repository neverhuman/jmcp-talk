#!/usr/bin/env bash
set -Eeuo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cd "$ROOT_DIR"

"${ROOT_DIR}/ops/ci/fast.sh"

if ! has cargo; then
  missing_tool cargo "Rust tests"
elif cargo_workspace_ready; then
  log "ci: running Rust tests"
  cargo test --workspace --all-targets --locked
fi

log "ci: contract drift"
python3 ops/ci/contract_drift.py

if [[ -d services/llm/tests || -d services/speech/tests ]]; then
  log "ci: Python unit tests"
  for test_dir in services/llm/tests services/speech/tests; do
    if [[ -d "$test_dir" ]]; then
      python3 -m unittest discover -s "$test_dir" -p 'test_*.py'
    fi
  done
fi

log "ci: complete"

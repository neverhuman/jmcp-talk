#!/usr/bin/env bash
set -Eeuo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cd "$ROOT_DIR"

log "fast: checking shell syntax"
while IFS= read -r script; do
  bash -n "$script"
done < <(find scripts services ops/ci \( -path '*/.venv' -o -path '*/.venv-*' -o -path '*/__pycache__' -o -path '*/models' \) -prune -o -type f -name '*.sh' -print | sort)

log "fast: checking Python syntax"
while IFS= read -r file; do
  python3 -m py_compile "$file"
done < <(find services ops/ci \( -path '*/.venv' -o -path '*/.venv-*' -o -path '*/__pycache__' -o -path '*/models' \) -prune -o -type f -name '*.py' -print | sort)

if ! has cargo; then
  missing_tool cargo "Rust formatting and checks"
elif cargo_workspace_ready; then
  log "fast: checking Rust workspace"
  cargo fmt --all -- --check
  cargo check --workspace --all-targets --locked
else
  warn "skipping Rust checks: Cargo workspace metadata is not ready"
fi

if has actionlint; then
  log "fast: linting GitHub Actions"
  actionlint
else
  missing_tool actionlint "GitHub Actions linting"
fi

log "fast: complete"

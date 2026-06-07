#!/usr/bin/env bash
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMMAND="${1:-ci}"

case "$COMMAND" in
  fast)
    exec bash "${ROOT_DIR}/ops/ci/fast.sh"
    ;;
  ci)
    exec bash "${ROOT_DIR}/ops/ci/ci.sh"
    ;;
  security)
    exec bash "${ROOT_DIR}/ops/ci/security.sh"
    ;;
  contract-drift)
    exec python3 "${ROOT_DIR}/ops/ci/contract_drift.py"
    ;;
  jankurai-local)
    exec bash "${ROOT_DIR}/ops/ci/jankurai-local.sh"
    ;;
  doctor)
    exec bash "${ROOT_DIR}/scripts/ci-doctor.sh"
    ;;
  *)
    printf 'usage: %s [fast|ci|security|contract-drift|jankurai-local|doctor]\n' "$0" >&2
    exit 64
    ;;
esac

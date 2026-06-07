#!/usr/bin/env bash
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

printf '[jmcp-talk-ci] doctor: repository %s\n' "$ROOT_DIR"

for tool in bash python3 cargo just jankurai gitleaks cargo-audit zizmor syft actionlint; do
  if command -v "$tool" >/dev/null 2>&1; then
    version="$("$tool" --version 2>/dev/null | head -n 1 || true)"
    printf '[jmcp-talk-ci] tool %-12s %s\n' "$tool" "${version:-present}"
  else
    printf '[jmcp-talk-ci][warn] tool %-12s missing\n' "$tool"
  fi
done

printf '[jmcp-talk-ci] manifests:'
for file in Cargo.toml Cargo.lock gitleaks.toml agent/cost-budget.toml; do
  if [[ -e "$file" ]]; then
    printf ' %s' "$file"
  fi
done
printf '\n'

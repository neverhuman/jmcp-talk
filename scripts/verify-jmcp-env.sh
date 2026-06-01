#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

api_bind="${JMCP_API_BIND:-127.0.0.1:18877}"
api_url="${JMCP_API_URL:-http://127.0.0.1:18877}"
cockpit_host="${JMCP_COCKPIT_HOST:-127.0.0.1}"
cockpit_port="${JMCP_COCKPIT_PORT:-15873}"
protected_ports=(2224 8787 8799 8929 18787 18788 19800)
failed=0

port_from_bind() {
  local value="$1"
  printf '%s\n' "${value##*:}"
}

is_protected_port() {
  local needle="$1"
  local port
  for port in "${protected_ports[@]}"; do
    [[ "$needle" == "$port" ]] && return 0
  done
  return 1
}

owner_for_port() {
  local port="$1"
  ss -ltnp 2>/dev/null | awk -v port=":${port}" '$0 ~ port"[[:space:]]" {print; found=1} END {exit found ? 0 : 1}'
}

check_bind_port() {
  local label="$1"
  local port="$2"

  if ! [[ "$port" =~ ^[0-9]+$ ]]; then
    printf 'error: %s port is not numeric: %s\n' "$label" "$port" >&2
    failed=1
    return
  fi

  if is_protected_port "$port"; then
    printf 'error: %s uses Jeryu protected port %s\n' "$label" "$port" >&2
    failed=1
  fi

  if owner="$(owner_for_port "$port")"; then
    printf '%s port %s is already occupied: %s\n' "$label" "$port" "$owner"
  fi
}

api_port="$(port_from_bind "$api_bind")"

printf 'JMCP repo: %s\n' "$repo_root"
printf 'JMCP_API_BIND=%s\n' "$api_bind"
printf 'JMCP_API_URL=%s\n' "$api_url"
printf 'JMCP_COCKPIT_HOST=%s\n' "$cockpit_host"
printf 'JMCP_COCKPIT_PORT=%s\n' "$cockpit_port"

check_bind_port "JMCP_API_BIND" "$api_port"
check_bind_port "JMCP_COCKPIT_PORT" "$cockpit_port"

for protected_port in "${protected_ports[@]}"; do
  if owner="$(owner_for_port "$protected_port")"; then
    printf 'Jeryu protected port %s is occupied by: %s\n' "$protected_port" "$owner"
  fi
done

if ! owner_for_port 8799 >/dev/null && ! owner_for_port 8787 >/dev/null; then
  printf 'warning: Jeryu was not detected on 127.0.0.1:8799 or 127.0.0.1:8787\n' >&2
fi

if [[ "$failed" -ne 0 ]]; then
  printf 'JMCP environment is not safe for Jeryu coexistence\n' >&2
  exit 1
fi

printf 'JMCP environment is safe for Jeryu coexistence\n'

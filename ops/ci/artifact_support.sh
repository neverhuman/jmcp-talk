#!/usr/bin/env bash
set -Eeuo pipefail

out_dir="${1:-target/jankurai/artifacts}"
mkdir -p "$out_dir"
printf '{"repo":"jmcp-talk","kind":"artifact-support","ok":true}\n' > "$out_dir/artifact-support.json"


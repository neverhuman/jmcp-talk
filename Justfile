set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

fast:
    bash ops/ci/fast.sh

fast-shell:
    while IFS= read -r script; do bash -n "$script"; done < <(find scripts services ops/ci -type f -name '*.sh' | sort)

fast-rust:
    cargo fmt --all -- --check
    cargo check --workspace --all-targets --locked

fast-voiced:
    cargo check -p jmcp-voiced --all-targets --locked

test-speech-fast:
    if command -v cargo-nextest >/dev/null 2>&1; then cargo nextest run -p jmcp-adapter-speech --locked; else cargo test -p jmcp-adapter-speech --locked; fi

build-timings:
    cargo build --workspace --locked --timings

fast-python:
    while IFS= read -r file; do python3 -m py_compile "$file"; done < <(find services ops/ci \( -path '*/.venv' -o -path '*/.venv-*' -o -path '*/__pycache__' -o -path '*/models' \) -prune -o -type f -name '*.py' -print | sort)

fast-actions:
    if command -v actionlint >/dev/null 2>&1; then actionlint; else echo "[jmcp-talk][warn] skipping actionlint: not installed" >&2; fi

ci:
    bash ops/ci/ci.sh

security: security-evidence

conformance:
    bash ops/ci/conformance.sh

jankurai-local:
    ./ops/ci/jankurai-local.sh

build: build-rust

build-rust:
    cargo build --workspace --locked

test: test-rust

test-rust:
    cargo test --workspace --all-targets --locked

contract-drift:
    python3 ops/ci/contract_drift.py

score: score-advisory

score-advisory:
    jankurai audit . --full --mode advisory --json .jankurai/repo-score.json --md .jankurai/repo-score.md --score-history .jankurai/score-history.jsonl --score-history-csv .jankurai/score-history.csv

proof-routing:
    jankurai proof . --changed-from "${JANKURAI_BASE_REF:-origin/main}" --out target/jankurai/proof-routing.json --md target/jankurai/proof-routing.md

proofbind:
    jankurai proofbind verify . --changed-from "${JANKURAI_BASE_REF:-origin/main}" --out target/jankurai/proofbind/surface-witness.json --obligations-out target/jankurai/proofbind/obligations.json --md target/jankurai/proofbind/proofbind.md

proofmark-rust:
    jankurai proofmark rust . --obligations target/jankurai/proofbind/obligations.json --out target/jankurai/proofmark/proofmark-receipt.json --proof-receipt target/jankurai/proofmark/proof-receipt.json --md target/jankurai/proofmark/proofmark.md

copy-code:
    jankurai copy-code . --json target/jankurai/copy-code.json --md target/jankurai/copy-code.md

security-evidence:
    jankurai security run --script ops/ci/security.sh --out target/jankurai/security/evidence.json

language-bad-behavior:
    bash ops/ci/language-bad-behavior.sh

rust-map:
    jankurai rust map .

rust-witness:
    jankurai rust witness build . --out target/jankurai/rust/witness-graph.json

rust-diagnose:
    jankurai rust diagnose .

gpu-live:
    bash ops/ci/minicpm-live.sh

cost-budget:
    bash ops/ci/cost-budget.sh

release-readiness:
    bash ops/ci/release-readiness.sh

authz-matrix:
    jankurai audit . --full --mode advisory --json .jankurai/repo-score.json --md .jankurai/repo-score.md

input-boundary:
    jankurai audit . --full --mode advisory --json .jankurai/repo-score.json --md .jankurai/repo-score.md

agent-tool-supply:
    jankurai audit . --full --mode advisory --json .jankurai/repo-score.json --md .jankurai/repo-score.md

check: fast build test contract-drift security cost-budget release-readiness score

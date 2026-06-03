set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

fast: fast-shell fast-json fast-rust fast-npm fast-actions

fast-shell:
    source ops/ci/common.sh; log "fast-shell: checking shell syntax"; while IFS= read -r script; do bash -n "$script"; done < <(find scripts ops/ci -type f -name '*.sh' | sort)

fast-json:
    source ops/ci/common.sh; log "fast-json: validating JSON"; while IFS= read -r file; do python3 -m json.tool "$file" >/dev/null; done < <(find schemas contracts/events -type f -name '*.json' | sort); python3 -m json.tool package.json >/dev/null; python3 -m json.tool package-lock.json >/dev/null

fast-rust:
    cargo fmt --all -- --check
    cargo check --workspace --all-targets --locked

fast-npm:
    npm ci --ignore-scripts --no-audit --no-fund
    npm --workspace @jankurai/ux-qa run build
    npm --workspace @jankurai/ux-qa run test

fast-actions:
    source ops/ci/common.sh; if has actionlint; then actionlint; else missing_tool actionlint "GitHub Actions linting"; fi

ci: fast test-rust test-cockpit test-web conformance contract-drift

security: security-evidence

conformance:
    bash ops/ci/conformance.sh

jankurai-local:
    ./ops/ci/jankurai-local.sh

build: build-rust build-cockpit build-web

build-rust:
    cargo build --workspace --locked

build-cockpit:
    npm --workspace @jmcp/cockpit run build

build-web:
    npm --prefix apps/web run build

test: test-rust test-cockpit test-web

test-rust:
    cargo test --workspace --all-targets --locked

test-cockpit:
    npm --workspace @jmcp/cockpit run test

test-web:
    npm --prefix apps/web run test:ux

ux-qa: ux-qa-playwright ux-qa-audit

ux-qa-playwright:
    npm --prefix apps/web run test:ux

ux-qa-audit:
    jankurai ux audit --config agent/ux-qa.toml --out target/jankurai/ux-qa.json

ux-qa-package-build:
    npm --workspace @jankurai/ux-qa run build

ux-qa-package-test:
    npm --workspace @jankurai/ux-qa run test

score: score-advisory

score-advisory:
    jankurai audit . --mode advisory --json .jankurai/repo-score.json --md .jankurai/repo-score.md --score-history .jankurai/score-history.jsonl --score-history-csv .jankurai/score-history.csv

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

contract-drift:
    bash ops/ci/contract-drift.sh

cost-budget:
    bash ops/ci/cost-budget.sh

release-readiness:
    bash ops/ci/release-readiness.sh

publish-main:
    bash ops/ci/publish-main.sh

authz-matrix:
    jankurai audit . --mode advisory --json .jankurai/repo-score.json --md .jankurai/repo-score.md

input-boundary:
    jankurai audit . --mode advisory --json .jankurai/repo-score.json --md .jankurai/repo-score.md

agent-tool-supply:
    jankurai audit . --mode advisory --json .jankurai/repo-score.json --md .jankurai/repo-score.md

check: fast build test security conformance contract-drift ux-qa cost-budget release-readiness score rust-map rust-witness rust-diagnose

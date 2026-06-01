set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

fast:
    ./ops/ci/fast.sh

ci:
    ./ops/ci/ci.sh

security:
    ./ops/ci/security.sh

conformance:
    ./ops/ci/conformance.sh

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

score:
    jankurai audit . --mode advisory --json .jankurai/repo-score.json --md .jankurai/repo-score.md --score-history .jankurai/score-history.jsonl --score-history-csv .jankurai/score-history.csv

rust-map:
    jankurai rust map .

rust-witness:
    jankurai rust witness build .

rust-diagnose:
    jankurai rust diagnose .

check: fast build test security conformance score rust-map rust-witness rust-diagnose

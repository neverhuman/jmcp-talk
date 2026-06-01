set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

doctor:
    ./scripts/ci-local.sh doctor

fast:
    ./scripts/ci-local.sh fast

ci:
    ./scripts/ci-local.sh ci

security:
    ./scripts/ci-local.sh security

conformance:
    ./scripts/ci-local.sh conformance

jankurai-local:
    ./scripts/ci-local.sh jankurai-local

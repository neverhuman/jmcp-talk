#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts/speech/speech.openapi.yaml"
DAEMON = ROOT / "apps/jmcp-speechd/src/main.rs"
GPU_WORKFLOW = ROOT / ".github/workflows/gpu-live.yml"


def require(text: str, needle: str, label: str) -> None:
    if needle not in text:
        raise SystemExit(f"{label} missing {needle}")


def main() -> int:
    contract = CONTRACT.read_text()
    daemon = DAEMON.read_text()
    gpu = GPU_WORKFLOW.read_text()

    require(
        contract,
        "Contract zone: verified by just contract-drift",
        "speech contract",
    )
    for route in ["/health", "/metrics", "/transcribe", "/synthesize"]:
        require(contract, f"  {route}:", "speech contract")
        require(daemon, f'"/{route.lstrip("/")}"', "speech daemon")

    for field in [
        "turn_id",
        "session_id",
        "event",
        "status",
        "adapter",
        "model",
        "device",
        "quantization",
        "latency_ms",
        "error_class",
        "redacted_transcript",
        "audio_hash",
    ]:
        require(contract, field, "speech trace schema")
        require(daemon, field, "speech trace implementation")

    require(gpu, "workflow_dispatch", "GPU workflow")
    require(gpu, "self-hosted", "GPU workflow")
    require(gpu, "rtx3090", "GPU workflow")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())


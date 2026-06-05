#!/usr/bin/env python3
"""Export redacted speech trace analysis from JSONL receipts."""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--trace",
        default="target/jankurai/talk/speech-trace.jsonl",
        help="Speech trace JSONL path.",
    )
    parser.add_argument(
        "--out",
        default="target/jankurai/talk/redacted-analysis.json",
        help="Redacted analysis output path.",
    )
    args = parser.parse_args()

    events = []
    for line in Path(args.trace).read_text().splitlines():
        if not line.strip():
            continue
        event = json.loads(line)
        events.append(
            {
                "event": event.get("event"),
                "status": event.get("status"),
                "adapter": event.get("adapter"),
                "model": event.get("model"),
                "device": event.get("device"),
                "quantization": event.get("quantization"),
                "latency_ms": event.get("latency_ms"),
                "error_class": event.get("error_class"),
                "redacted_transcript": event.get("redacted_transcript"),
                "audio_hash": event.get("audio_hash"),
            }
        )

    summary = {
        "event_count": len(events),
        "by_event": Counter(event["event"] for event in events),
        "by_status": Counter(event["status"] for event in events),
        "events": events,
    }
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())


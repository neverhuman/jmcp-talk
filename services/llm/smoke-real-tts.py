#!/usr/bin/env python3
"""Opt-in live smoke analyzer for the real VoxCPM2 TTS lane."""

from __future__ import annotations

import argparse
import json
import os
import urllib.error
from pathlib import Path
from typing import Any

from voice_smoke_common import (
    TimedFrame,
    collect_audio_samples,
    post_ndjson,
    read_json,
    stream_frame_metrics,
    summarize_values,
    utc_timestamp,
    validate_real_tts_health,
    write_timing_jsonl,
    write_wav,
)

DEFAULT_TTS_URL = os.environ.get("JMCP_TALK_TTS_UPSTREAM", "http://127.0.0.1:18901").rstrip("/")
DEFAULT_AUDIO_DIR = Path(
    os.environ.get("JMCP_TALK_AUDIO_DIR", "/home/ubuntu/jmcp-split/.live/audio")
)
EXPECTED_PROFILE = os.environ.get("JMCP_TALK_VOICE_PROFILE", "jmcp_male_v1")
EXPECTED_SAMPLE_RATE = 48_000

PROMPT_CASES = [
    {
        "id": "short_ack",
        "label": "Short acknowledgement",
        "text": "Ready.",
        "pace_gate": False,
        "allow_punctuation_gap": False,
    },
    {
        "id": "status_update",
        "label": "Status update",
        "text": "Local voice is ready. ASR, reasoning, and VoxCPM2 speech are connected.",
        "pace_gate": True,
        "allow_punctuation_gap": False,
    },
    {
        "id": "technical_route",
        "label": "Technical route phrase",
        "text": "The route is browser microphone to ASR, then local L L M, then VoxCPM2 T T S on port eighteen nine oh one.",
        "pace_gate": True,
        "allow_punctuation_gap": False,
    },
    {
        "id": "confirmation",
        "label": "Confirmation phrase",
        "text": "Confirmed. I will wait for the approval token before making durable changes.",
        "pace_gate": True,
        "allow_punctuation_gap": False,
    },
    {
        "id": "long_paragraph",
        "label": "Longer paragraph",
        "text": (
            "The local JMCP voice path should sound calm and natural during a longer update. "
            "It needs clear pacing, warm tone, and stable pronunciation when it explains technical status without rushing."
        ),
        "pace_gate": True,
        "allow_punctuation_gap": True,
    },
]


def main() -> int:
    args = parse_args()
    output_dir = Path(args.output_dir) if args.output_dir else DEFAULT_AUDIO_DIR / "smoke-real-tts" / utc_timestamp()
    output_dir.mkdir(parents=True, exist_ok=True)
    health_response = read_json(f"{args.tts_url}/health", timeout=args.health_timeout)
    health = health_response.payload
    failures = validate_real_tts_health(
        health,
        health_response.status_code,
        expected_profile=args.voice_profile,
        expected_sample_rate=args.sample_rate,
    )

    summary: dict[str, Any] = {
        "ok": False,
        "strict": args.strict,
        "tts_url": args.tts_url,
        "artifact_dir": str(output_dir),
        "health": health,
        "preflight_failures": failures,
        "cases": [],
        "aggregate": {},
        "failures": list(failures),
    }
    if failures:
        write_summary(output_dir, summary)
        print(json.dumps(summary, indent=2, sort_keys=True))
        return 2

    for case in PROMPT_CASES:
        case_dir = output_dir / case["id"]
        case_runs: list[dict[str, Any]] = []
        for run_index in range(args.runs + 1):
            measured = run_index > 0
            result = run_tts_case(
                args=args,
                case=case,
                case_dir=case_dir,
                run_index=run_index,
                measured=measured,
            )
            case_runs.append(result)
        summary["cases"].append(
            {
                "id": case["id"],
                "label": case["label"],
                "text": case["text"],
                "pace_gate": case["pace_gate"],
                "allow_punctuation_gap": case["allow_punctuation_gap"],
                "runs": case_runs,
            }
        )

    summary["aggregate"] = aggregate_summary(summary["cases"])
    summary["failures"].extend(evaluate_gates(summary["cases"], summary["aggregate"]))
    summary["ok"] = not summary["failures"]
    listening_packet = write_listening_packet(output_dir, summary)
    summary["listening_packet"] = str(listening_packet)
    write_summary(output_dir, summary)
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 1 if args.strict and summary["failures"] else 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tts-url", default=DEFAULT_TTS_URL)
    parser.add_argument("--runs", type=int, default=5, help="Measured runs per prompt after one warmup")
    parser.add_argument("--strict", action="store_true", help="Return non-zero when smoke gates fail")
    parser.add_argument("--output-dir", default="")
    parser.add_argument("--voice-profile", default=EXPECTED_PROFILE)
    parser.add_argument("--sample-rate", type=int, default=EXPECTED_SAMPLE_RATE)
    parser.add_argument("--health-timeout", type=float, default=10.0)
    parser.add_argument("--stream-timeout", type=float, default=120.0)
    return parser.parse_args()


def run_tts_case(
    args: argparse.Namespace,
    case: dict[str, Any],
    case_dir: Path,
    run_index: int,
    measured: bool,
) -> dict[str, Any]:
    run_name = f"run-{run_index:02d}" if measured else "warmup"
    wav_path = case_dir / f"{run_name}.wav"
    timing_path = case_dir / f"{run_name}.timing.jsonl"
    try:
        timed_frames, elapsed_ms = post_ndjson(
            f"{args.tts_url}/stream",
            {
                "text": case["text"],
                "voice": args.voice_profile,
                "sequence_start": 0,
                "queue_depth_ms": 0.0,
            },
            timeout=args.stream_timeout,
        )
    except (urllib.error.URLError, TimeoutError, OSError) as exc:
        return {
            "ok": False,
            "measured": measured,
            "run_index": run_index,
            "error": f"{exc.__class__.__name__}: {exc}",
            "wav": None,
            "timing": None,
        }

    write_timing_jsonl(timing_path, timed_frames)
    samples, sample_rate = collect_audio_samples(timed_frames)
    if samples and sample_rate:
        write_wav(wav_path, samples, sample_rate)
    metrics = stream_frame_metrics(timed_frames, samples, sample_rate, elapsed_ms, case["text"])
    return {
        "ok": metrics["audio_frame_count"] > 0 and metrics["stream_error_count"] == 0,
        "measured": measured,
        "run_index": run_index,
        "wav": str(wav_path) if samples and sample_rate else None,
        "timing": str(timing_path),
        "metrics": metrics,
        "frames": stripped_frames(timed_frames),
    }


def stripped_frames(timed_frames: list[TimedFrame]) -> list[dict[str, Any]]:
    return [
        {
            "received_ms": timed.received_ms,
            **{key: value for key, value in timed.frame.items() if key not in {"audio_data", "data"}},
        }
        for timed in timed_frames
    ]


def aggregate_summary(cases: list[dict[str, Any]]) -> dict[str, Any]:
    measured = [
        run
        for case in cases
        for run in case["runs"]
        if run.get("measured") and isinstance(run.get("metrics"), dict)
    ]
    first_audio = [
        float(run["metrics"]["first_frame_latency_ms"])
        for run in measured
        if isinstance(run["metrics"].get("first_frame_latency_ms"), (int, float))
    ]
    rtfs = [
        float(run["metrics"]["rtf"])
        for run in measured
        if isinstance(run["metrics"].get("rtf"), (int, float))
    ]
    elapsed = [float(run["metrics"]["total_elapsed_ms"]) for run in measured]
    durations = [float(run["metrics"]["audio_duration_ms"]) for run in measured]
    return {
        "measured_runs": len(measured),
        "first_frame_latency_ms": summarize_values(first_audio),
        "rtf": summarize_values(rtfs),
        "total_elapsed_ms": summarize_values(elapsed),
        "audio_duration_ms": summarize_values(durations),
    }


def evaluate_gates(cases: list[dict[str, Any]], aggregate: dict[str, Any]) -> list[str]:
    failures: list[str] = []
    first = aggregate["first_frame_latency_ms"]
    if first["p50"] is None or first["p50"] > 450:
        failures.append(f"direct TTS first audio p50 exceeded 450 ms: {first['p50']}")
    if first["p95"] is None or first["p95"] > 900:
        failures.append(f"direct TTS first audio p95 exceeded 900 ms: {first['p95']}")
    rtf = aggregate["rtf"]
    if rtf["p50"] is None or rtf["p50"] > 0.75:
        failures.append(f"direct TTS RTF p50 exceeded 0.75: {rtf['p50']}")
    if rtf["p95"] is None or rtf["p95"] > 1.10:
        failures.append(f"direct TTS RTF p95 exceeded 1.10: {rtf['p95']}")

    for case in cases:
        for run in case["runs"]:
            if not run.get("measured"):
                continue
            metrics = run.get("metrics")
            label = f"{case['id']} run {run.get('run_index')}"
            if not isinstance(metrics, dict):
                failures.append(f"{label}: missing metrics")
                continue
            if metrics["audio_frame_count"] <= 0:
                failures.append(f"{label}: no audio frames")
            if metrics["stream_error_count"] > 0:
                failures.append(f"{label}: stream returned {metrics['stream_error_count']} error frame(s)")
            if metrics["missing_audio_frames"] > 0:
                failures.append(f"{label}: missing {metrics['missing_audio_frames']} audio frame(s)")
            if not metrics["sequence_monotonic"]:
                failures.append(f"{label}: non-monotonic audio sequence")
            if metrics["degraded_active"]:
                failures.append(f"{label}: degraded TTS was active")
            if metrics["clipping_ratio"] > 0.001:
                failures.append(f"{label}: clipping ratio exceeded 0.1%: {metrics['clipping_ratio']}")
            if metrics["leading_silence_ms"] > 200:
                failures.append(f"{label}: leading silence exceeded 200 ms: {metrics['leading_silence_ms']}")
            if metrics["trailing_silence_ms"] > 350:
                failures.append(f"{label}: trailing silence exceeded 350 ms: {metrics['trailing_silence_ms']}")
            if metrics["max_internal_silence_gap_ms"] > 500 and not case["allow_punctuation_gap"]:
                failures.append(
                    f"{label}: max internal silence exceeded 500 ms: {metrics['max_internal_silence_gap_ms']}"
                )
            if case["pace_gate"]:
                wpm = float(metrics["words_per_minute"])
                if wpm < 125 or wpm > 190:
                    failures.append(f"{label}: spoken pace outside 125-190 WPM: {wpm}")
    return failures


def write_listening_packet(output_dir: Path, summary: dict[str, Any]) -> Path:
    path = output_dir / "listening_packet.md"
    lines = [
        "# JMCP Real TTS Listening Packet",
        "",
        "Score each required case from 1 to 5 for clarity, naturalness, warmth, pacing, technical-term pronunciation, fatigue, and artifacts.",
        "Pass requires clarity, naturalness, and pacing >= 4 for every required case, average score >= 4, and no severe artifact note.",
        "",
    ]
    for case in summary["cases"]:
        lines.extend([f"## {case['label']}", "", case["text"], ""])
        for run in case["runs"]:
            if run.get("measured") and run.get("wav"):
                lines.append(f"- run {run['run_index']}: `{run['wav']}`")
        lines.append("")
    path.write_text("\n".join(lines), encoding="utf-8")
    return path


def write_summary(output_dir: Path, summary: dict[str, Any]) -> None:
    (output_dir / "summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True),
        encoding="utf-8",
    )


if __name__ == "__main__":
    raise SystemExit(main())

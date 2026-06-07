#!/usr/bin/env python3
"""Opt-in full-turn smoke for the Rust JMCP realtime voice gateway."""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import re
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

from voice_smoke_common import (
    TimedFrame,
    collect_audio_samples,
    endpoint_ok,
    has_raw_audio_keys,
    parse_json_frame,
    parse_prometheus_metrics,
    post_ndjson,
    read_json,
    resample_linear,
    samples_to_f32le_base64,
    stream_frame_metrics,
    strip_audio_data,
    summarize_values,
    utc_timestamp,
    validate_real_tts_health,
    write_timing_jsonl,
    write_wav,
)

DEFAULT_GATEWAY_URL = os.environ.get("JMCP_TALK_VOICE_GATEWAY", "http://127.0.0.1:8040").rstrip("/")
DEFAULT_TTS_URL = os.environ.get("JMCP_TALK_TTS_UPSTREAM", "http://127.0.0.1:18901").rstrip("/")
DEFAULT_ASR_URL = os.environ.get("JMCP_TALK_ASR_UPSTREAM", "http://127.0.0.1:18878").rstrip("/")
DEFAULT_LLM_URL = os.environ.get("JMCP_TALK_LLM_UPSTREAM", "http://127.0.0.1:18902/v1").rstrip("/")
DEFAULT_AUDIO_DIR = Path(
    os.environ.get("JMCP_TALK_AUDIO_DIR", "/home/ubuntu/jmcp-split/.live/audio")
)
EXPECTED_PROFILE = os.environ.get("JMCP_TALK_VOICE_PROFILE", "jmcp_male_v1")
EXPECTED_SAMPLE_RATE = 48_000
INPUT_SAMPLE_RATE = 16_000
TYPED_PROMPT = "Reply in one short sentence that the local voice path is ready."
VOICE_INPUT_PROMPT = "Reply in one short sentence that the local voice path is ready."
FINAL_STATES = {"completed", "failed", "interrupted"}
REQUIRED_CORE_STATES = ["started", "reasoning_started", "audio_started"]


def main() -> int:
    args = parse_args()
    output_dir = Path(args.output_dir) if args.output_dir else DEFAULT_AUDIO_DIR / "smoke-real-voice-turn" / utc_timestamp()
    output_dir.mkdir(parents=True, exist_ok=True)

    pre_snapshot = take_snapshot(args)
    (output_dir / "preflight_snapshot.json").write_text(
        json.dumps(pre_snapshot, indent=2, sort_keys=True),
        encoding="utf-8",
    )
    preflight_failures = validate_preflight(pre_snapshot, args)
    summary: dict[str, Any] = {
        "ok": False,
        "strict": args.strict,
        "mode": args.mode,
        "runs": args.runs,
        "artifact_dir": str(output_dir),
        "gateway_url": args.gateway_url,
        "ws_url": args.ws_url,
        "preflight_failures": preflight_failures,
        "typed": [],
        "audio": [],
        "aggregate": {},
        "metrics_comparison": {},
        "core_verification": {"checked": False, "skipped": "JMCP_API_URL not set"},
        "failures": list(preflight_failures),
    }
    if preflight_failures:
        write_summary(output_dir, summary)
        print(json.dumps(summary, indent=2, sort_keys=True))
        return 2

    try:
        import websockets  # noqa: F401
    except ImportError:
        summary["failures"].append("Python package 'websockets' is required for full gateway smoke")
        write_summary(output_dir, summary)
        print(json.dumps(summary, indent=2, sort_keys=True))
        return 2

    audio_prompt: dict[str, Any] | None = None
    if args.mode in {"audio", "both"}:
        audio_prompt = prepare_audio_prompt(args, output_dir)
        if not audio_prompt.get("ok"):
            summary["failures"].append(f"audio input prompt generation failed: {audio_prompt.get('error')}")

    turn_lanes: dict[str, str] = {}
    metrics_before = read_metrics(args.gateway_url)
    if not summary["failures"]:
        if args.mode in {"typed", "both"}:
            for index in range(args.runs):
                turn_id = f"smoke-typed-{utc_timestamp()}-{index + 1:02d}"
                result = asyncio.run(run_gateway_turn(args, output_dir, "typed", turn_id, index + 1))
                summary["typed"].append(result)
                turn_lanes[turn_id] = "typed"
        if args.mode in {"audio", "both"}:
            assert audio_prompt is not None
            for index in range(args.runs):
                turn_id = f"smoke-audio-{utc_timestamp()}-{index + 1:02d}"
                result = asyncio.run(
                    run_gateway_turn(
                        args,
                        output_dir,
                        "audio",
                        turn_id,
                        index + 1,
                        audio_data=str(audio_prompt["audio_data"]),
                    )
                )
                summary["audio"].append(result)
                turn_lanes[turn_id] = "audio"

    metrics_after = read_metrics(args.gateway_url)
    post_snapshot = take_snapshot(args)
    (output_dir / "post_run_snapshot.json").write_text(
        json.dumps(post_snapshot, indent=2, sort_keys=True),
        encoding="utf-8",
    )
    summary["aggregate"] = aggregate_summary(summary)
    summary["metrics_comparison"] = compare_gateway_metrics(summary, metrics_before, metrics_after)
    if audio_prompt is not None:
        summary["audio_input_prompt"] = audio_prompt_without_audio(audio_prompt)
    core_url = args.core_url or os.environ.get("JMCP_API_URL", "").strip()
    if core_url:
        summary["core_verification"] = verify_core_turns(core_url.rstrip("/"), turn_lanes, args.core_poll_seconds)
    summary["failures"].extend(evaluate_gates(summary, metrics_before, metrics_after, post_snapshot))
    summary["ok"] = not summary["failures"]
    listening_packet = write_listening_packet(output_dir, summary)
    summary["listening_packet"] = str(listening_packet)
    write_summary(output_dir, summary)
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 1 if args.strict and summary["failures"] else 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gateway-url", default=DEFAULT_GATEWAY_URL)
    parser.add_argument("--ws-url", default="")
    parser.add_argument("--tts-url", default=DEFAULT_TTS_URL)
    parser.add_argument("--asr-url", default=DEFAULT_ASR_URL)
    parser.add_argument("--llm-url", default=DEFAULT_LLM_URL)
    parser.add_argument("--core-url", default="")
    parser.add_argument("--mode", choices=["typed", "audio", "both"], default="both")
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--strict", action="store_true")
    parser.add_argument("--output-dir", default="")
    parser.add_argument("--voice-profile", default=EXPECTED_PROFILE)
    parser.add_argument("--sample-rate", type=int, default=EXPECTED_SAMPLE_RATE)
    parser.add_argument("--connect-timeout", type=float, default=15.0)
    parser.add_argument("--frame-timeout", type=float, default=60.0)
    parser.add_argument("--stream-timeout", type=float, default=120.0)
    parser.add_argument("--core-poll-seconds", type=float, default=5.0)
    args = parser.parse_args()
    if not args.ws_url:
        args.ws_url = args.gateway_url.replace("http://", "ws://").replace("https://", "wss://") + "/ws/chat"
    return args


def take_snapshot(args: argparse.Namespace) -> dict[str, Any]:
    health = {
        "gateway": response_record(read_json(f"{args.gateway_url}/health", timeout=5.0)),
        "tts": response_record(read_json(f"{args.tts_url}/health", timeout=5.0)),
        "asr": response_record(read_json(f"{args.asr_url}/health", timeout=5.0)),
        "llm": response_record(read_json(f"{llm_health_base(args.llm_url)}/health", timeout=5.0)),
    }
    return {
        "captured_at": utc_timestamp(),
        "health": health,
        "metrics": read_metrics(args.gateway_url),
        "nvidia_smi": command_output(["nvidia-smi"], timeout=5.0),
        "ports": port_ownership(),
        "logs": {
            "/tmp/asr.log": tail_file(Path("/tmp/asr.log")),
            "/tmp/tts.log": tail_file(Path("/tmp/tts.log")),
            "/tmp/llm.log": tail_file(Path("/tmp/llm.log")),
            "/tmp/voice-gateway.log": tail_file(Path("/tmp/voice-gateway.log")),
        },
    }


def response_record(response: Any) -> dict[str, Any]:
    return {
        "status_code": response.status_code,
        "error": response.error,
        "payload": response.payload,
    }


def validate_preflight(snapshot: dict[str, Any], args: argparse.Namespace) -> list[str]:
    failures: list[str] = []
    health = snapshot["health"]
    gateway_payload = health["gateway"]["payload"]
    gateway_status = int(health["gateway"]["status_code"])
    if not endpoint_ok(gateway_payload, gateway_status):
        failures.append(f"gateway health is not OK: status={gateway_status} error={gateway_payload.get('error')}")
    adapter = str(gateway_payload.get("adapter") or "").lower()
    engine = str(gateway_payload.get("engine") or "").lower()
    if adapter != "jmcp-rust-voice":
        failures.append(f"gateway adapter must be jmcp-rust-voice, got {adapter or '<missing>'}")
    if "minicpm" in adapter or "comni" in adapter or "minicpm" in engine or "comni" in engine:
        failures.append("gateway appears to be MiniCPM/Comni instead of Rust realtime voice")

    for name in ["asr", "llm"]:
        payload = health[name]["payload"]
        status = int(health[name]["status_code"])
        if not endpoint_ok(payload, status):
            failures.append(f"{name.upper()} health is not OK: status={status} error={payload.get('error')}")
    failures.extend(
        validate_real_tts_health(
            health["tts"]["payload"],
            int(health["tts"]["status_code"]),
            expected_profile=args.voice_profile,
            expected_sample_rate=args.sample_rate,
        )
    )
    if bool(gateway_payload.get("degraded_active")):
        failures.append("gateway health reports degraded_active=true")
    return failures


def prepare_audio_prompt(args: argparse.Namespace, output_dir: Path) -> dict[str, Any]:
    prompt_dir = output_dir / "audio-input-prompt"
    timing_path = prompt_dir / "source-tts.timing.jsonl"
    source_wav = prompt_dir / "source-48k.wav"
    input_wav = prompt_dir / "input-16k.wav"
    try:
        timed_frames, elapsed_ms = post_ndjson(
            f"{args.tts_url}/stream",
            {
                "text": VOICE_INPUT_PROMPT,
                "voice": args.voice_profile,
                "sequence_start": 0,
                "queue_depth_ms": 0.0,
            },
            timeout=args.stream_timeout,
        )
    except (urllib.error.URLError, TimeoutError, OSError) as exc:
        return {"ok": False, "error": f"{exc.__class__.__name__}: {exc}"}
    write_timing_jsonl(timing_path, timed_frames)
    samples, sample_rate = collect_audio_samples(timed_frames)
    if not samples or sample_rate <= 0:
        return {"ok": False, "error": "TTS prompt generation returned no audio"}
    write_wav(source_wav, samples, sample_rate)
    input_samples = resample_linear(samples, sample_rate, INPUT_SAMPLE_RATE)
    write_wav(input_wav, input_samples, INPUT_SAMPLE_RATE)
    metrics = stream_frame_metrics(timed_frames, samples, sample_rate, elapsed_ms, VOICE_INPUT_PROMPT)
    return {
        "ok": True,
        "text": VOICE_INPUT_PROMPT,
        "source_wav": str(source_wav),
        "input_wav": str(input_wav),
        "timing": str(timing_path),
        "source_sample_rate": sample_rate,
        "input_sample_rate": INPUT_SAMPLE_RATE,
        "metrics": metrics,
        "audio_data": samples_to_f32le_base64(input_samples),
    }


def audio_prompt_without_audio(prompt: dict[str, Any]) -> dict[str, Any]:
    return {key: value for key, value in prompt.items() if key != "audio_data"}


async def run_gateway_turn(
    args: argparse.Namespace,
    output_dir: Path,
    lane: str,
    turn_id: str,
    run_index: int,
    audio_data: str = "",
) -> dict[str, Any]:
    import websockets

    run_dir = output_dir / lane / f"run-{run_index:02d}"
    run_dir.mkdir(parents=True, exist_ok=True)
    frames: list[TimedFrame] = []
    started = time.perf_counter()
    error: str | None = None
    try:
        async with websockets.connect(
            args.ws_url,
            max_size=128 * 1024 * 1024,
            open_timeout=args.connect_timeout,
        ) as ws:
            if lane == "typed":
                await ws.send(json.dumps({"turn_id": turn_id, "prompt": TYPED_PROMPT}))
            else:
                await ws.send(json.dumps({"type": "turn.start", "turn_id": turn_id}))
                await ws.send(
                    json.dumps(
                        {
                            "type": "input.audio",
                            "turn_id": turn_id,
                            "audio_format": "f32le",
                            "sample_rate": INPUT_SAMPLE_RATE,
                            "audio_data": audio_data,
                        }
                    )
                )
                await ws.send(json.dumps({"type": "turn.end", "turn_id": turn_id}))
            while True:
                try:
                    raw = await asyncio.wait_for(ws.recv(), timeout=args.frame_timeout)
                except Exception as exc:  # noqa: BLE001
                    error = f"{exc.__class__.__name__}: {exc}"
                    break
                frame = parse_json_frame(raw)
                if frame is None:
                    continue
                received_ms = round((time.perf_counter() - started) * 1000, 1)
                frames.append(TimedFrame(received_ms, frame))
                if frame.get("type") in {"done", "error"}:
                    if frame.get("type") == "error":
                        error = str(frame.get("error") or "gateway_error_frame")
                    break
    except Exception as exc:  # noqa: BLE001
        error = f"{exc.__class__.__name__}: {exc}"

    elapsed_ms = round((time.perf_counter() - started) * 1000, 1)
    timing_path = run_dir / "frames.timing.jsonl"
    write_timing_jsonl(timing_path, frames)
    samples, sample_rate = collect_audio_samples(frames)
    wav_path = run_dir / "assistant.wav"
    if samples and sample_rate:
        write_wav(wav_path, samples, sample_rate)
    done_text = done_reply(frames)
    metrics = stream_frame_metrics(frames, samples, sample_rate, elapsed_ms, done_text)
    first_token_ms = first_frame_ms(frames, "chunk")
    result = {
        "ok": error is None and metrics["audio_frame_count"] > 0,
        "turn_id": turn_id,
        "lane": lane,
        "run_index": run_index,
        "error": error,
        "prompt": TYPED_PROMPT if lane == "typed" else VOICE_INPUT_PROMPT,
        "reply": done_text,
        "wav": str(wav_path) if samples and sample_rate else None,
        "timing": str(timing_path),
        "first_token_latency_ms": first_token_ms,
        "first_audio_latency_ms": metrics["first_frame_latency_ms"],
        "metrics": metrics,
        "frames": [{"received_ms": timed.received_ms, **strip_audio_data(timed.frame)} for timed in frames],
    }
    return result


def done_reply(frames: list[TimedFrame]) -> str:
    for timed in reversed(frames):
        if timed.frame.get("type") == "done":
            return str(timed.frame.get("text") or "")
    chunks = [
        str(timed.frame.get("text_delta") or timed.frame.get("text") or "")
        for timed in frames
        if timed.frame.get("type") == "chunk"
    ]
    return "".join(chunks).strip()


def first_frame_ms(frames: list[TimedFrame], frame_type: str) -> float | None:
    for timed in frames:
        if timed.frame.get("type") == frame_type:
            return timed.received_ms
    return None


def aggregate_summary(summary: dict[str, Any]) -> dict[str, Any]:
    typed = summary["typed"]
    audio = summary["audio"]
    typed_first_token = [
        float(run["first_token_latency_ms"])
        for run in typed
        if isinstance(run.get("first_token_latency_ms"), (int, float))
    ]
    typed_first_audio = [
        float(run["first_audio_latency_ms"])
        for run in typed
        if isinstance(run.get("first_audio_latency_ms"), (int, float))
    ]
    audio_first_audio = [
        float(run["first_audio_latency_ms"])
        for run in audio
        if isinstance(run.get("first_audio_latency_ms"), (int, float))
    ]
    return {
        "typed_first_token_latency_ms": summarize_values(typed_first_token),
        "typed_first_audio_latency_ms": summarize_values(typed_first_audio),
        "audio_first_audio_latency_ms": summarize_values(audio_first_audio),
        "typed_runs": len(typed),
        "audio_runs": len(audio),
    }


def compare_gateway_metrics(
    summary: dict[str, Any],
    before: dict[str, float],
    after: dict[str, float],
) -> dict[str, Any]:
    aggregate = summary["aggregate"]
    return {
        "before": before,
        "after": after,
        "observed": {
            "typed_first_token_p50_ms": aggregate["typed_first_token_latency_ms"]["p50"],
            "typed_first_audio_p50_ms": aggregate["typed_first_audio_latency_ms"]["p50"],
            "audio_first_audio_p50_ms": aggregate["audio_first_audio_latency_ms"]["p50"],
        },
        "gateway_reported": {
            "first_token_p50_ms": after.get("jmcp_voice_first_token_latency_ms_p50"),
            "first_audio_p50_ms": after.get("jmcp_voice_first_audio_latency_ms_p50"),
            "tts_rtf_p50": after.get("jmcp_voice_tts_rtf_p50"),
        },
    }


def evaluate_gates(
    summary: dict[str, Any],
    metrics_before: dict[str, float],
    metrics_after: dict[str, float],
    post_snapshot: dict[str, Any],
) -> list[str]:
    failures: list[str] = []
    aggregate = summary["aggregate"]
    if summary["typed"]:
        typed_token = aggregate["typed_first_token_latency_ms"]
        typed_audio = aggregate["typed_first_audio_latency_ms"]
        if typed_token["p50"] is None or typed_token["p50"] > 800:
            failures.append(f"typed first token p50 exceeded 800 ms: {typed_token['p50']}")
        if typed_audio["p50"] is None or typed_audio["p50"] > 1500:
            failures.append(f"typed first audio p50 exceeded 1500 ms: {typed_audio['p50']}")
        if typed_audio["p95"] is None or typed_audio["p95"] > 2500:
            failures.append(f"typed first audio p95 exceeded 2500 ms: {typed_audio['p95']}")
    if summary["audio"]:
        audio = aggregate["audio_first_audio_latency_ms"]
        if audio["p50"] is None or audio["p50"] > 2500:
            failures.append(f"audio-input first audio p50 exceeded 2500 ms: {audio['p50']}")
        if audio["p95"] is None or audio["p95"] > 4000:
            failures.append(f"audio-input first audio p95 exceeded 4000 ms: {audio['p95']}")

    for lane in ["typed", "audio"]:
        for run in summary[lane]:
            label = f"{lane} run {run.get('run_index')} ({run.get('turn_id')})"
            metrics = run.get("metrics", {})
            if not run.get("ok"):
                failures.append(f"{label}: failed: {run.get('error')}")
            if metrics.get("audio_frame_count", 0) <= 0:
                failures.append(f"{label}: no assistant audio frames")
            if metrics.get("stream_error_count", 0) > 0:
                failures.append(f"{label}: gateway returned {metrics['stream_error_count']} error frame(s)")
            if metrics.get("missing_audio_frames", 0) > 0:
                failures.append(f"{label}: missing {metrics['missing_audio_frames']} audio frame(s)")
            if metrics.get("sequence_monotonic") is False:
                failures.append(f"{label}: non-monotonic assistant audio sequence")
            error_text = str(run.get("error") or "").lower()
            if "tts idle timeout" in error_text:
                failures.append(f"{label}: TTS idle timeout")

    before_dropped = metrics_before.get("jmcp_voice_dropped_frames_total", 0.0)
    after_dropped = metrics_after.get("jmcp_voice_dropped_frames_total", 0.0)
    if after_dropped > before_dropped:
        failures.append(f"gateway dropped frames increased: {before_dropped} -> {after_dropped}")
    before_failures = sum_gateway_failures(metrics_before)
    after_failures = sum_gateway_failures(metrics_after)
    if after_failures > before_failures:
        failures.append(f"gateway failure counters increased: {before_failures} -> {after_failures}")

    tts_health = post_snapshot["health"]["tts"]["payload"]
    gateway_health = post_snapshot["health"]["gateway"]["payload"]
    if bool(tts_health.get("degraded_active")) or bool(gateway_health.get("degraded_active")):
        failures.append("post-run health reports degraded degraded TTS")

    core = summary.get("core_verification", {})
    if core.get("checked") and not core.get("ok"):
        failures.extend(str(item) for item in core.get("failures", []))
    return failures


def verify_core_turns(core_url: str, turn_lanes: dict[str, str], poll_seconds: float) -> dict[str, Any]:
    deadline = time.time() + poll_seconds
    turns: list[dict[str, Any]] = []
    failures: list[str] = []
    while time.time() < deadline:
        response = read_json(f"{core_url}/voice/turns", timeout=5.0)
        if endpoint_ok(response.payload, response.status_code):
            turns = response.payload if isinstance(response.payload, list) else response.payload.get("value", [])
            if isinstance(turns, list) and all(latest_turn(turns, turn_id) for turn_id in turn_lanes):
                break
        time.sleep(0.25)

    if not isinstance(turns, list):
        turns = []
    event_states = core_event_states(core_url, set(turn_lanes))
    checked_turns: dict[str, Any] = {}
    for turn_id, lane in turn_lanes.items():
        record = latest_turn(turns, turn_id)
        if record is None:
            failures.append(f"core /voice/turns missing turn_id {turn_id}")
            continue
        state = str(record.get("state") or "")
        if state not in FINAL_STATES:
            failures.append(f"core turn {turn_id} final state is not valid: {state}")
        if state == "completed" and not str(record.get("reply") or "").strip():
            failures.append(f"core turn {turn_id} completed without reply text")
        if lane == "audio" and not str(record.get("transcript") or "").strip():
            failures.append(f"core audio turn {turn_id} missing transcript text")
        audio = record.get("audio") if isinstance(record.get("audio"), dict) else {}
        metrics = record.get("metrics") if isinstance(record.get("metrics"), dict) else {}
        if state == "completed" and not (audio.get("outputHash") or audio.get("outputRef")):
            failures.append(f"core turn {turn_id} missing output audio hash/ref")
        if not audio.get("voiceProfileHash"):
            failures.append(f"core turn {turn_id} missing voice profile hash")
        if not metrics:
            failures.append(f"core turn {turn_id} missing metrics object")
        raw_keys = has_raw_audio_keys(record)
        if raw_keys:
            failures.append(f"core turn {turn_id} contains raw audio fields: {sorted(set(raw_keys))}")
        states = event_states.get(turn_id) or [state]
        state_failure = required_state_failure(turn_id, states)
        if state_failure:
            failures.append(state_failure)
        checked_turns[turn_id] = {
            "lane": lane,
            "state": state,
            "states": states,
            "has_reply": bool(str(record.get("reply") or "").strip()),
            "has_transcript": bool(str(record.get("transcript") or "").strip()),
            "audio_keys": sorted(audio.keys()),
            "metric_keys": sorted(metrics.keys()),
        }
    return {
        "checked": True,
        "ok": not failures,
        "core_url": core_url,
        "turns_checked": checked_turns,
        "failures": failures,
    }


def latest_turn(turns: list[Any], turn_id: str) -> dict[str, Any] | None:
    for item in turns:
        if isinstance(item, dict) and (item.get("turnId") == turn_id or item.get("turn_id") == turn_id):
            return item
    return None


def core_event_states(core_url: str, turn_ids: set[str]) -> dict[str, list[str]]:
    states: dict[str, list[str]] = {turn_id: [] for turn_id in turn_ids}
    try:
        with urllib.request.urlopen(f"{core_url}/events?after=0", timeout=4.0) as response:
            for raw in response:
                line = raw.decode("utf-8").strip()
                if not line.startswith("data:"):
                    continue
                events = json.loads(line[5:].strip())
                if not isinstance(events, list):
                    return states
                for event in events:
                    if not isinstance(event, dict) or event.get("event_type") != "voice.turn.recorded":
                        continue
                    data = event.get("data")
                    if not isinstance(data, dict):
                        continue
                    turn_id = str(data.get("turnId") or data.get("turn_id") or "")
                    state = str(data.get("state") or "")
                    if turn_id in states and state:
                        states[turn_id].append(state)
                return states
    except Exception:  # noqa: BLE001
        return states
    return states


def required_state_failure(turn_id: str, states: list[str]) -> str | None:
    cursor = 0
    for required in REQUIRED_CORE_STATES:
        try:
            index = states.index(required, cursor)
        except ValueError:
            return f"core turn {turn_id} missing durable state {required}; observed {states}"
        cursor = index + 1
    if not states or states[-1] not in FINAL_STATES:
        return f"core turn {turn_id} missing valid final state; observed {states}"
    return None


def read_metrics(gateway_url: str) -> dict[str, float]:
    try:
        with urllib.request.urlopen(f"{gateway_url}/metrics", timeout=5.0) as response:
            text = response.read().decode("utf-8")
            return parse_prometheus_metrics(text)
    except Exception:  # noqa: BLE001
        return {}


def sum_gateway_failures(metrics: dict[str, float]) -> float:
    return sum(value for key, value in metrics.items() if key.startswith("jmcp_voice_failures_total{"))


def llm_health_base(llm_url: str) -> str:
    return llm_url[:-3] if llm_url.endswith("/v1") else llm_url


def command_output(argv: list[str], timeout: float = 5.0) -> dict[str, Any]:
    try:
        result = subprocess.run(argv, check=False, capture_output=True, text=True, timeout=timeout)
        return {
            "ok": result.returncode == 0,
            "returncode": result.returncode,
            "stdout": result.stdout[-8000:],
            "stderr": result.stderr[-4000:],
        }
    except Exception as exc:  # noqa: BLE001
        return {"ok": False, "error": f"{exc.__class__.__name__}: {exc}"}


def port_ownership() -> dict[str, Any]:
    result = command_output(["ss", "-ltnp"], timeout=5.0)
    stdout = str(result.get("stdout") or "")
    ports = [":8040", ":18878", ":18901", ":18902"]
    result["filtered"] = [line for line in stdout.splitlines() if any(port in line for port in ports)]
    return result


def tail_file(path: Path, lines: int = 80) -> dict[str, Any]:
    if not path.exists():
        return {"exists": False, "tail": ""}
    try:
        content = path.read_text(encoding="utf-8", errors="replace").splitlines()
        return {"exists": True, "tail": redact_raw_audio_text("\n".join(content[-lines:]))}
    except Exception as exc:  # noqa: BLE001
        return {"exists": True, "error": f"{exc.__class__.__name__}: {exc}"}


def redact_raw_audio_text(text: str) -> str:
    for key in ["audio_data", "audioData", "raw_audio", "rawAudio", "ref_audio_data"]:
        text = re.sub(rf'("{key}"\s*:\s*")[^"]*(")', rf'\1<redacted {key}>\2', text)
        text = re.sub(rf"({key}=)[^\s,]+", rf"\1<redacted {key}>", text)
    return text


def write_listening_packet(output_dir: Path, summary: dict[str, Any]) -> Path:
    path = output_dir / "listening_packet.md"
    lines = [
        "# JMCP Real Voice Turn Listening Packet",
        "",
        "Score assistant WAVs from 1 to 5 for clarity, naturalness, warmth, pacing, technical-term pronunciation, fatigue, and artifacts.",
        "Pass requires clarity, naturalness, and pacing >= 4 for every required case, average score >= 4, and no severe artifact note.",
        "",
    ]
    for lane in ["typed", "audio"]:
        if not summary[lane]:
            continue
        lines.extend([f"## {lane.title()} Lane", ""])
        for run in summary[lane]:
            lines.append(f"- {run['turn_id']}: `{run.get('wav')}`")
            lines.append(f"  Prompt: {run.get('prompt')}")
            lines.append(f"  Reply: {run.get('reply')}")
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

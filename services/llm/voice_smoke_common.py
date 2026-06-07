#!/usr/bin/env python3
"""Shared helpers for opt-in live JMCP voice smoke tests."""

from __future__ import annotations

import base64
import json
import math
import re
import struct
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable

SILENCE_THRESHOLD = 0.001


@dataclass(frozen=True)
class HttpJson:
    status_code: int
    payload: dict[str, Any]
    error: str | None = None


@dataclass(frozen=True)
class TimedFrame:
    received_ms: float
    frame: dict[str, Any]


def utc_timestamp() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def read_json(url: str, timeout: float = 10.0) -> HttpJson:
    try:
        with urllib.request.urlopen(url, timeout=timeout) as response:
            body = response.read().decode("utf-8")
            if not body.strip():
                return HttpJson(
                    status_code=int(getattr(response, "status", 200)),
                    payload={"ok": True},
                )
            payload = json.loads(body)
            return HttpJson(
                status_code=int(getattr(response, "status", 200)),
                payload=payload if isinstance(payload, dict) else {"value": payload},
            )
    except urllib.error.HTTPError as exc:
        try:
            payload = json.loads(exc.read().decode("utf-8"))
        except Exception:  # noqa: BLE001
            payload = {"error": exc.reason}
        return HttpJson(
            status_code=exc.code,
            payload=payload if isinstance(payload, dict) else {"value": payload},
            error=f"http_{exc.code}",
        )
    except Exception as exc:  # noqa: BLE001
        return HttpJson(status_code=0, payload={"error": str(exc)}, error=exc.__class__.__name__)


def post_ndjson(
    url: str,
    payload: dict[str, Any],
    timeout: float = 120.0,
) -> tuple[list[TimedFrame], float]:
    req = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={"content-type": "application/json"},
        method="POST",
    )
    started = time.perf_counter()
    frames: list[TimedFrame] = []
    with urllib.request.urlopen(req, timeout=timeout) as response:
        for raw in response:
            line = raw.decode("utf-8").strip()
            if not line:
                continue
            frame = parse_json_frame(line)
            if frame is not None:
                frames.append(TimedFrame(round((time.perf_counter() - started) * 1000, 1), frame))
    return frames, round((time.perf_counter() - started) * 1000, 1)


def parse_json_frame(raw: str | bytes) -> dict[str, Any] | None:
    if isinstance(raw, bytes):
        raw = raw.decode("utf-8")
    try:
        frame = json.loads(raw)
    except json.JSONDecodeError:
        return None
    return frame if isinstance(frame, dict) else None


def strip_audio_data(frame: dict[str, Any]) -> dict[str, Any]:
    return {key: value for key, value in frame.items() if key not in {"audio_data", "data"}}


def write_timing_jsonl(path: Path, timed_frames: Iterable[TimedFrame]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as file:
        for timed in timed_frames:
            payload = {"received_ms": timed.received_ms, **strip_audio_data(timed.frame)}
            file.write(json.dumps(payload, sort_keys=True) + "\n")


def float32_base64_to_samples(data: str) -> list[float]:
    raw = base64.b64decode(data)
    return samples_from_f32le(raw)


def samples_from_f32le(raw: bytes) -> list[float]:
    usable = len(raw) - (len(raw) % 4)
    if usable <= 0:
        return []
    return [sample[0] for sample in struct.iter_unpack("<f", raw[:usable])]


def samples_to_f32le_base64(samples: Iterable[float]) -> str:
    raw = bytearray()
    for sample in samples:
        raw.extend(struct.pack("<f", float(sample)))
    return base64.b64encode(raw).decode("ascii")


def wav_bytes_from_samples(samples: Iterable[float], sample_rate: int) -> bytes:
    frames = bytearray()
    for sample in samples:
        clipped = max(-1.0, min(1.0, float(sample)))
        value = int(clipped * 32767) if clipped >= 0 else int(clipped * 32768)
        frames.extend(struct.pack("<h", value))
    data_size = len(frames)
    header = bytearray()
    header.extend(b"RIFF")
    header.extend(struct.pack("<I", 36 + data_size))
    header.extend(b"WAVEfmt ")
    header.extend(struct.pack("<IHHIIHH", 16, 1, 1, sample_rate, sample_rate * 2, 2, 16))
    header.extend(b"data")
    header.extend(struct.pack("<I", data_size))
    return bytes(header + frames)


def write_wav(path: Path, samples: Iterable[float], sample_rate: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(wav_bytes_from_samples(samples, sample_rate))


def collect_audio_samples(timed_frames: Iterable[TimedFrame]) -> tuple[list[float], int]:
    samples: list[float] = []
    sample_rate = 0
    for timed in timed_frames:
        frame = timed.frame
        if frame.get("type") != "audio":
            continue
        if isinstance(frame.get("sample_rate"), (int, float)):
            sample_rate = int(frame["sample_rate"])
        audio_data = str(frame.get("audio_data") or "")
        if audio_data:
            samples.extend(float32_base64_to_samples(audio_data))
    return samples, sample_rate


def audio_quality_metrics(
    samples: list[float],
    sample_rate: int,
    text: str = "",
    silence_threshold: float = SILENCE_THRESHOLD,
) -> dict[str, Any]:
    if not samples or sample_rate <= 0:
        return {
            "sample_count": len(samples),
            "audio_duration_ms": 0.0,
            "rms": 0.0,
            "peak": 0.0,
            "clipping_count": 0,
            "clipping_ratio": 0.0,
            "leading_silence_ms": 0.0,
            "trailing_silence_ms": 0.0,
            "max_internal_silence_gap_ms": 0.0,
            "word_count": count_words(text),
            "words_per_minute": 0.0,
        }

    abs_samples = [abs(sample) for sample in samples]
    sample_count = len(samples)
    duration_ms = sample_count / sample_rate * 1000.0
    peak = max(abs_samples)
    rms = math.sqrt(sum(sample * sample for sample in samples) / sample_count)
    clipping_count = sum(1 for sample in abs_samples if sample >= 0.999)
    leading = 0
    for sample in abs_samples:
        if sample >= silence_threshold:
            break
        leading += 1
    trailing = 0
    for sample in reversed(abs_samples):
        if sample >= silence_threshold:
            break
        trailing += 1
    first_sound = leading
    last_sound = sample_count - trailing - 1
    max_gap = 0
    current_gap = 0
    if first_sound <= last_sound:
        for sample in abs_samples[first_sound : last_sound + 1]:
            if sample < silence_threshold:
                current_gap += 1
            else:
                max_gap = max(max_gap, current_gap)
                current_gap = 0
        max_gap = max(max_gap, current_gap)
    words = count_words(text)
    minutes = duration_ms / 60_000.0 if duration_ms > 0 else 0.0
    return {
        "sample_count": sample_count,
        "audio_duration_ms": round(duration_ms, 1),
        "rms": round(rms, 6),
        "peak": round(peak, 6),
        "clipping_count": clipping_count,
        "clipping_ratio": round(clipping_count / sample_count, 6),
        "leading_silence_ms": round(leading / sample_rate * 1000.0, 1),
        "trailing_silence_ms": round(trailing / sample_rate * 1000.0, 1),
        "max_internal_silence_gap_ms": round(max_gap / sample_rate * 1000.0, 1),
        "word_count": words,
        "words_per_minute": round(words / minutes, 1) if minutes else 0.0,
    }


def stream_frame_metrics(
    timed_frames: list[TimedFrame],
    samples: list[float],
    sample_rate: int,
    elapsed_ms: float,
    text: str,
) -> dict[str, Any]:
    audio_frames = [timed for timed in timed_frames if timed.frame.get("type") == "audio"]
    done_frames = [timed.frame for timed in timed_frames if timed.frame.get("type") == "done"]
    error_frames = [timed.frame for timed in timed_frames if timed.frame.get("type") == "error"]
    sequences = [
        int(timed.frame["sequence"])
        for timed in audio_frames
        if isinstance(timed.frame.get("sequence"), (int, float))
    ]
    missing = 0
    non_monotonic = False
    for previous, current in zip(sequences, sequences[1:]):
        if current <= previous:
            non_monotonic = True
        if current > previous + 1:
            missing += current - previous - 1
    cadences = [
        round(current.received_ms - previous.received_ms, 1)
        for previous, current in zip(audio_frames, audio_frames[1:])
    ]
    first_audio_ms = audio_frames[0].received_ms if audio_frames else None
    quality = audio_quality_metrics(samples, sample_rate, text)
    done = done_frames[-1] if done_frames else {}
    audio_duration_ms = float(done.get("audio_duration_ms") or quality["audio_duration_ms"])
    tts_elapsed_ms = float(done.get("tts_elapsed_ms") or elapsed_ms)
    rtf = done.get("tts_rtf")
    if not isinstance(rtf, (int, float)) and audio_duration_ms > 0:
        rtf = (tts_elapsed_ms / 1000.0) / (audio_duration_ms / 1000.0)
    return {
        **quality,
        "first_frame_latency_ms": first_audio_ms,
        "total_elapsed_ms": round(elapsed_ms, 1),
        "tts_elapsed_ms": round(tts_elapsed_ms, 1),
        "rtf": round(float(rtf), 4) if isinstance(rtf, (int, float)) else None,
        "audio_frame_count": len(audio_frames),
        "done_frame_count": len(done_frames),
        "stream_error_count": len(error_frames),
        "missing_audio_frames": missing,
        "sequence_monotonic": not non_monotonic,
        "first_sequence": sequences[0] if sequences else None,
        "last_sequence": sequences[-1] if sequences else None,
        "frame_cadence_ms": summarize_values(cadences),
        "degraded_active": any(bool(frame.get("degraded_active")) for frame in done_frames),
    }


def count_words(text: str) -> int:
    return len(re.findall(r"[A-Za-z0-9]+(?:[-'][A-Za-z0-9]+)?", text))


def percentile(values: list[float], pct: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    index = max(0, min(len(ordered) - 1, math.ceil((pct / 100.0) * len(ordered)) - 1))
    return ordered[index]


def summarize_values(values: list[float]) -> dict[str, Any]:
    if not values:
        return {"count": 0, "min": None, "p50": None, "p95": None, "max": None, "avg": None}
    return {
        "count": len(values),
        "min": round(min(values), 4),
        "p50": round(float(percentile(values, 50) or 0.0), 4),
        "p95": round(float(percentile(values, 95) or 0.0), 4),
        "max": round(max(values), 4),
        "avg": round(sum(values) / len(values), 4),
    }


def endpoint_ok(payload: dict[str, Any], status_code: int = 200) -> bool:
    if status_code >= 400 or status_code == 0:
        return False
    if payload.get("error"):
        return False
    if isinstance(payload.get("ok"), bool):
        return bool(payload["ok"])
    return bool(payload)


def validate_real_tts_health(
    health: dict[str, Any],
    status_code: int,
    expected_profile: str = "jmcp_male_v1",
    expected_sample_rate: int = 48_000,
) -> list[str]:
    failures: list[str] = []
    if not endpoint_ok(health, status_code):
        failures.append(f"TTS health is not OK: status={status_code} error={health.get('error')}")
    active_engine = str(health.get("active_engine") or health.get("voice_engine") or "").lower()
    if active_engine != "voxcpm2":
        failures.append(f"TTS active engine must be voxcpm2, got {active_engine or '<missing>'}")
    if bool(health.get("degraded_active")):
        failures.append("TTS degraded_active must be false for real TTS smoke")
    if health.get("voice_profile") != expected_profile:
        failures.append(f"TTS voice_profile must be {expected_profile}, got {health.get('voice_profile')}")
    if not str(health.get("voice_profile_hash") or ""):
        failures.append("TTS voice_profile_hash is missing")
    if int(health.get("sample_rate") or 0) != expected_sample_rate:
        failures.append(f"TTS sample_rate must be {expected_sample_rate}, got {health.get('sample_rate')}")
    if health.get("loaded") is not True:
        failures.append("TTS loaded must be true")
    if health.get("warmed") is not True:
        failures.append("TTS warmed must be true")
    return failures


def resample_linear(samples: list[float], source_rate: int, target_rate: int) -> list[float]:
    if not samples or source_rate <= 0 or target_rate <= 0:
        return []
    if source_rate == target_rate:
        return list(samples)
    output_len = max(1, int(round(len(samples) * target_rate / source_rate)))
    ratio = source_rate / target_rate
    output: list[float] = []
    for index in range(output_len):
        position = index * ratio
        left = int(position)
        right = min(left + 1, len(samples) - 1)
        frac = position - left
        output.append(samples[left] * (1.0 - frac) + samples[right] * frac)
    return output


def parse_prometheus_metrics(text: str) -> dict[str, float]:
    metrics: dict[str, float] = {}
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        if len(parts) < 2:
            continue
        try:
            value = float(parts[-1])
        except ValueError:
            continue
        metrics[parts[0]] = value
        name = parts[0].split("{", 1)[0]
        metrics[name] = value
    return metrics


def has_raw_audio_keys(value: Any) -> list[str]:
    matches: list[str] = []
    raw_names = {"audiodata", "audio_data", "rawaudio", "raw_audio", "pcmsamples", "pcm_samples"}
    if isinstance(value, dict):
        for key, item in value.items():
            normalized = key.replace("-", "_").lower()
            compact = normalized.replace("_", "")
            if normalized in raw_names or compact in raw_names:
                matches.append(key)
            matches.extend(has_raw_audio_keys(item))
    elif isinstance(value, list):
        for item in value:
            matches.extend(has_raw_audio_keys(item))
    return matches

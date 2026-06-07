#!/usr/bin/env python3
"""JMCP local voice gateway.

Stable browser-facing contract:
  /health
  /events
  /metrics
  /ws/chat

The primary path is ASR -> local text LLM -> streaming TTS. MiniCPM/Comni stays
in its own compatibility gateway and is no longer required for normal cockpit speech.
"""

from __future__ import annotations

import argparse
import asyncio
import base64
import hashlib
import json
import os
import re
import struct
import time
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, AsyncIterator

import httpx
import uvicorn
from fastapi import FastAPI, Request, WebSocket, WebSocketDisconnect
from fastapi.responses import JSONResponse, PlainTextResponse


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def read_str(value: Any) -> str:
    return value if isinstance(value, str) else ""


def read_num(value: Any, default: float = 0.0) -> float:
    return float(value) if isinstance(value, (int, float)) else default


def preview(text: str, limit: int = 80) -> str:
    return " ".join(text.split())[:limit]


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def sha256_file(path: Path) -> str:
    if not path.is_file():
        return ""
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_profile_hash(path: Path) -> str:
    if not path.is_file():
        return ""
    data = json.loads(path.read_text(encoding="utf-8"))
    canonical = json.dumps(data, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def p50(values: list[float]) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    return ordered[len(ordered) // 2]


def elapsed_ms(started: float) -> float:
    return round((time.perf_counter() - started) * 1000, 1)


@dataclass
class Metrics:
    turns_total: int = 0
    failures: dict[str, int] = field(default_factory=dict)
    first_token_latency_ms: list[float] = field(default_factory=list)
    first_audio_latency_ms: list[float] = field(default_factory=list)
    tts_rtf: list[float] = field(default_factory=list)
    dropped_frames: int = 0

    def fail(self, stage: str) -> None:
        self.failures[stage] = self.failures.get(stage, 0) + 1


@dataclass
class GatewayConfig:
    bind: str
    llm_upstream: str
    llm_model: str
    asr_upstream: str
    tts_upstream: str
    jmcp_core: str
    event_log: Path
    audio_dir: Path
    capture_raw_audio: bool
    voice_profile: str
    voice_profile_path: Path
    voice_profile_hash: str
    min_tts_phrase_chars: int
    connect_timeout_s: float
    first_frame_timeout_s: float
    idle_timeout_s: float
    total_turn_timeout_s: float

    @property
    def local_only(self) -> bool:
        host = self.bind.rsplit(":", 1)[0]
        return host in {"127.0.0.1", "localhost", "::1"}


class VoiceGateway:
    def __init__(self, config: GatewayConfig) -> None:
        self.config = config
        self.metrics = Metrics()
        self.http = httpx.AsyncClient(timeout=httpx.Timeout(10.0, read=None))
        self.app = FastAPI(title="JMCP Local Voice Gateway")
        self._routes()

    def _routes(self) -> None:
        self.app.get("/health")(self.health)
        self.app.get("/metrics")(self.metrics_response)
        self.app.post("/events")(self.events)
        self.app.websocket("/ws/chat")(self.chat_ws)

    async def close(self) -> None:
        await self.http.aclose()

    async def health(self) -> JSONResponse:
        tts_health = await self.get_json(f"{self.config.tts_upstream}/health")
        asr_health = await self.get_json(f"{self.config.asr_upstream}/health")
        llm_health = await self.get_json(f"{self.config.llm_upstream.removesuffix('/v1')}/health")
        tts_ok = bool(tts_health.get("ok"))
        llm_ok = not llm_health.get("error") and bool(llm_health)
        ok = tts_ok and llm_ok
        sample_rate = int(read_num(tts_health.get("sample_rate"), 48000))
        return JSONResponse(
            {
                "ok": ok,
                "adapter": "jmcp-local-voice",
                "engine": "local-asr-llm-tts",
                "voice_engine": read_str(tts_health.get("voice_engine"))
                or read_str(tts_health.get("active_engine"))
                or "voxcpm2",
                "voice_profile": read_str(tts_health.get("voice_profile")) or self.config.voice_profile,
                "voice_profile_hash": read_str(tts_health.get("voice_profile_hash"))
                or self.config.voice_profile_hash,
                "sample_rate": sample_rate,
                "audio_format": "f32le",
                "streaming_audio": True,
                "tts_rtf_p50": round(p50(self.metrics.tts_rtf), 4),
                "degraded_active": bool(tts_health.get("degraded_active")),
                "degraded_engine": tts_health.get("degraded_engine"),
                "local_only": self.config.local_only,
                "loaded": ok,
                "llm_model": self.config.llm_model,
                "llm_upstream": self.config.llm_upstream,
                "asr_upstream": self.config.asr_upstream,
                "tts_upstream": self.config.tts_upstream,
                "raw_audio_capture": self.config.capture_raw_audio,
                "audio_dir": str(self.config.audio_dir) if self.config.capture_raw_audio else None,
                "tts_health": tts_health,
                "asr_health": asr_health,
                "llm_health": llm_health,
            },
            status_code=200 if ok else 503,
        )

    async def get_json(self, url: str) -> dict[str, Any]:
        try:
            response = await self.http.get(url)
            if response.status_code >= 400:
                return {"error": f"http_{response.status_code}"}
            try:
                data = response.json()
            except Exception:  # noqa: BLE001
                return {"ok": True, "status": response.text[:80]}
            return data if isinstance(data, dict) else {"ok": True}
        except Exception as exc:  # noqa: BLE001
            return {"error": exc.__class__.__name__}

    async def events(self, request: Request) -> JSONResponse:
        body = await request.json()
        if isinstance(body, list):
            for item in body:
                if isinstance(item, dict):
                    self.write_event(item)
        elif isinstance(body, dict):
            self.write_event(body)
        return JSONResponse({"ok": True})

    async def metrics_response(self) -> PlainTextResponse:
        failure_lines = [
            f'jmcp_voice_failures_total{{stage="{stage}"}} {count}'
            for stage, count in sorted(self.metrics.failures.items())
        ]
        lines = [
            f"jmcp_voice_turns_total {self.metrics.turns_total}",
            f"jmcp_voice_first_audio_latency_ms_p50 {p50(self.metrics.first_audio_latency_ms):.1f}",
            f"jmcp_voice_first_token_latency_ms_p50 {p50(self.metrics.first_token_latency_ms):.1f}",
            f"jmcp_voice_tts_rtf_p50 {p50(self.metrics.tts_rtf):.4f}",
            f"jmcp_voice_dropped_frames_total {self.metrics.dropped_frames}",
            *failure_lines,
            "",
        ]
        return PlainTextResponse("\n".join(lines))

    async def chat_ws(self, ws: WebSocket) -> None:
        await ws.accept()
        started = time.perf_counter()
        turn_id = f"turn_{uuid.uuid4().hex}"
        self.metrics.turns_total += 1
        try:
            raw = await asyncio.wait_for(ws.receive_text(), timeout=self.config.connect_timeout_s)
            request = json.loads(raw)
            if isinstance(request, dict):
                turn_id = read_str(request.get("turn_id")) or turn_id
            else:
                request = {}
            self.write_event({"turn_id": turn_id, "event": "voice.send", "stage": "gateway"})
            messages = await self.local_messages(turn_id, request)
            await self.stream_chat(turn_id, started, messages, ws)
        except WebSocketDisconnect:
            self.write_event({"turn_id": turn_id, "event": "voice.close", "stage": "browser"})
        except Exception as exc:  # noqa: BLE001
            self.metrics.fail("gateway")
            self.write_event(
                {
                    "turn_id": turn_id,
                    "event": "voice.error",
                    "stage": "gateway",
                    "status": "error",
                    "error_class": exc.__class__.__name__,
                }
            )
            try:
                await ws.send_json({"type": "error", "turn_id": turn_id, "error": exc.__class__.__name__})
            except Exception:  # noqa: BLE001
                pass

    async def local_messages(self, turn_id: str, request: dict[str, Any]) -> list[dict[str, str]]:
        raw_messages = request.get("messages")
        if not isinstance(raw_messages, list):
            prompt = read_str(request.get("prompt")) or read_str(request.get("text"))
            raw_messages = [{"role": "user", "content": prompt}]

        messages: list[dict[str, str]] = []
        for raw in raw_messages:
            if not isinstance(raw, dict):
                continue
            role = read_str(raw.get("role")) or "user"
            if role not in {"system", "user", "assistant"}:
                role = "user"
            content = raw.get("content")
            text = await self.content_to_text(turn_id, role, content)
            if text.strip():
                messages.append({"role": role, "content": text.strip()})
        if not messages or messages[-1]["role"] != "user":
            messages.append({"role": "user", "content": "Give a concise spoken JMCP status update."})
        return messages

    async def content_to_text(self, turn_id: str, role: str, content: Any) -> str:
        if isinstance(content, str):
            return content
        if not isinstance(content, list):
            return ""
        parts: list[str] = []
        for item in content:
            if not isinstance(item, dict):
                continue
            if item.get("type") == "text":
                parts.append(read_str(item.get("text")))
            elif item.get("type") == "audio" and role == "user":
                transcript = await self.transcribe_audio(turn_id, read_str(item.get("data")))
                if transcript:
                    parts.append(transcript)
        return "\n".join(part for part in parts if part.strip())

    async def transcribe_audio(self, turn_id: str, audio_data: str) -> str:
        if not audio_data:
            return ""
        wav = wav_bytes_from_float32_base64(audio_data, 16000)
        self.capture_wav_bytes(turn_id, "user_input", "input_000_16k.wav", wav, 16000)
        started = time.perf_counter()
        response = await self.http.post(
            f"{self.config.asr_upstream}/transcribe?language=en&beam_size=1",
            content=wav,
            headers={"content-type": "audio/wav"},
        )
        response.raise_for_status()
        data = response.json()
        transcript = read_str(data.get("text")) if isinstance(data, dict) else ""
        self.write_event(
            {
                "turn_id": turn_id,
                "event": "voice.transcript",
                "stage": "asr",
                "latency_ms": elapsed_ms(started),
                "transcript": transcript,
            }
        )
        return transcript

    async def stream_chat(
        self,
        turn_id: str,
        started: float,
        messages: list[dict[str, str]],
        ws: WebSocket,
    ) -> None:
        send_lock = asyncio.Lock()
        tts_queue: asyncio.Queue[str | None] = asyncio.Queue()
        first_token = False
        full_text = ""
        phrase_buffer = ""
        sequence = 0
        first_audio_seen = False

        async def send_json(payload: dict[str, Any]) -> None:
            async with send_lock:
                await ws.send_json(payload)

        async def tts_worker() -> None:
            nonlocal sequence, first_audio_seen
            while True:
                phrase = await tts_queue.get()
                if phrase is None:
                    tts_queue.task_done()
                    return
                try:
                    async for frame in self.tts_frames(phrase, sequence, tts_queue.qsize() * 160.0):
                        if read_str(frame.get("type")) == "done":
                            rtf = frame.get("tts_rtf")
                            if isinstance(rtf, (int, float)):
                                self.metrics.tts_rtf.append(float(rtf))
                            continue
                        if read_str(frame.get("type")) == "error":
                            self.metrics.fail("tts")
                            await send_json({"type": "error", "turn_id": turn_id, "error": read_str(frame.get("error"))})
                            continue
                        frame["turn_id"] = turn_id
                        frame["type"] = "audio"
                        sequence = int(read_num(frame.get("sequence"), sequence)) + 1
                        if not first_audio_seen:
                            first_audio_seen = True
                            latency = elapsed_ms(started)
                            self.metrics.first_audio_latency_ms.append(latency)
                            self.write_event(
                                {
                                    "turn_id": turn_id,
                                    "event": "voice.first_audio",
                                    "stage": "tts",
                                    "latency_ms": latency,
                                    "bytes": len(read_str(frame.get("audio_data"))),
                                }
                            )
                        self.capture_tts_frame(turn_id, frame)
                        await send_json(frame)
                finally:
                    tts_queue.task_done()

        worker = asyncio.create_task(tts_worker())
        try:
            async for delta in self.llm_deltas(messages):
                if delta:
                    full_text += delta
                    phrase_buffer += delta
                    if not first_token:
                        first_token = True
                        latency = elapsed_ms(started)
                        self.metrics.first_token_latency_ms.append(latency)
                        self.write_event(
                            {
                                "turn_id": turn_id,
                                "event": "voice.first_token",
                                "stage": "llm",
                                "latency_ms": latency,
                                "text": delta,
                            }
                        )
                    await send_json({"type": "chunk", "turn_id": turn_id, "text_delta": delta})
                    segments, phrase_buffer = split_tts_segments(
                        phrase_buffer,
                        self.config.min_tts_phrase_chars,
                    )
                    for segment in segments:
                        await tts_queue.put(segment)

            tail = phrase_buffer.strip()
            if tail:
                await tts_queue.put(tail)
            await tts_queue.join()
            await tts_queue.put(None)
            await worker
            text = full_text.strip()
            await send_json({"type": "done", "turn_id": turn_id, "text": text})
            self.write_event(
                {
                    "turn_id": turn_id,
                    "event": "voice.close",
                    "stage": "gateway",
                    "latency_ms": elapsed_ms(started),
                    "text": text,
                }
            )
        finally:
            if not worker.done():
                worker.cancel()

    async def llm_deltas(self, messages: list[dict[str, str]]) -> AsyncIterator[str]:
        payload = {
            "model": self.config.llm_model,
            "messages": messages,
            "stream": True,
            "temperature": 0.6,
            "max_tokens": 220,
        }
        async with self.http.stream(
            "POST",
            f"{self.config.llm_upstream}/chat/completions",
            json=payload,
            timeout=httpx.Timeout(self.config.total_turn_timeout_s, read=self.config.idle_timeout_s),
        ) as response:
            response.raise_for_status()
            async for line in response.aiter_lines():
                line = line.strip()
                if not line.startswith("data:"):
                    continue
                data = line[5:].strip()
                if data == "[DONE]":
                    return
                try:
                    parsed = json.loads(data)
                except json.JSONDecodeError:
                    continue
                if not isinstance(parsed, dict):
                    continue
                choices = parsed.get("choices")
                if not isinstance(choices, list) or not choices:
                    continue
                choice = choices[0]
                if not isinstance(choice, dict):
                    continue
                delta = choice.get("delta")
                if isinstance(delta, dict):
                    text = read_str(delta.get("content"))
                    if text:
                        yield text

    async def tts_frames(
        self,
        text: str,
        sequence_start: int,
        queue_depth_ms: float,
    ) -> AsyncIterator[dict[str, Any]]:
        payload = {
            "text": text,
            "voice": self.config.voice_profile,
            "sequence_start": sequence_start,
            "queue_depth_ms": queue_depth_ms,
        }
        async with self.http.stream(
            "POST",
            f"{self.config.tts_upstream}/stream",
            json=payload,
            timeout=httpx.Timeout(self.config.total_turn_timeout_s, read=self.config.idle_timeout_s),
        ) as response:
            response.raise_for_status()
            async for line in response.aiter_lines():
                if not line.strip():
                    continue
                try:
                    parsed = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if isinstance(parsed, dict):
                    yield parsed

    def write_event(self, event: dict[str, Any]) -> None:
        sanitized = self.sanitize_event(event)
        self.config.event_log.parent.mkdir(parents=True, exist_ok=True)
        with self.config.event_log.open("a", encoding="utf-8") as file:
            file.write(json.dumps(sanitized, ensure_ascii=False, sort_keys=True) + "\n")
        if self.config.capture_raw_audio:
            turn_id = read_str(sanitized.get("turn_id")) or "turn_unknown"
            turn_dir = self.turn_audio_dir(turn_id)
            with (turn_dir / "events.jsonl").open("a", encoding="utf-8") as file:
                file.write(json.dumps(sanitized, ensure_ascii=False, sort_keys=True) + "\n")

    def sanitize_event(self, event: dict[str, Any]) -> dict[str, Any]:
        sanitized: dict[str, Any] = {
            "ts": event.get("ts") or now_iso(),
            "turn_id": read_str(event.get("turn_id")) or f"turn_{uuid.uuid4().hex}",
            "event": read_str(event.get("event")) or "voice.event",
            "stage": read_str(event.get("stage")) or None,
            "status": read_str(event.get("status")) or "ok",
            "latency_ms": event.get("latency_ms") if isinstance(event.get("latency_ms"), (int, float)) else None,
            "bytes": event.get("bytes") if isinstance(event.get("bytes"), int) else None,
            "error_class": read_str(event.get("error_class")) or None,
            "audio_format": read_str(event.get("audio_format")) or None,
            "sample_rate": event.get("sample_rate") if isinstance(event.get("sample_rate"), int) else None,
            "sequence": event.get("sequence") if isinstance(event.get("sequence"), int) else None,
            "duration_ms": event.get("duration_ms") if isinstance(event.get("duration_ms"), (int, float)) else None,
            "tts_elapsed_ms": event.get("tts_elapsed_ms") if isinstance(event.get("tts_elapsed_ms"), (int, float)) else None,
            "queue_depth_ms": event.get("queue_depth_ms") if isinstance(event.get("queue_depth_ms"), (int, float)) else None,
            "voice_profile_hash": read_str(event.get("voice_profile_hash")) or None,
        }
        transcript = read_str(event.get("transcript"))
        if transcript:
            sanitized["transcript_preview"] = preview(transcript)
            sanitized["transcript_hash"] = sha256_text(transcript)
        text = read_str(event.get("text"))
        if text:
            sanitized["text_preview"] = preview(text)
            sanitized["text_hash"] = sha256_text(text)
        audio_hash = read_str(event.get("audio_hash"))
        if audio_hash:
            sanitized["audio_hash"] = audio_hash
        return {key: value for key, value in sanitized.items() if value is not None}

    def turn_audio_dir(self, turn_id: str) -> Path:
        safe = re.sub(r"[^A-Za-z0-9_.-]+", "_", turn_id)[:96] or f"turn_{uuid.uuid4().hex}"
        path = self.config.audio_dir / safe
        path.mkdir(parents=True, exist_ok=True)
        return path

    def capture_wav_bytes(self, turn_id: str, kind: str, filename: str, wav: bytes, sample_rate: int) -> None:
        if not self.config.capture_raw_audio:
            return
        path = self.turn_audio_dir(turn_id) / filename
        path.write_bytes(wav)
        self.write_event(
            {
                "turn_id": turn_id,
                "event": "voice.audio_snippet",
                "stage": kind,
                "bytes": len(wav),
                "sample_rate": sample_rate,
                "duration_ms": wav_duration_ms(wav, sample_rate),
                "audio_hash": sha256_file(path),
            }
        )

    def capture_tts_frame(self, turn_id: str, frame: dict[str, Any]) -> None:
        event = {
            "turn_id": turn_id,
            "event": "voice.tts_chunk",
            "stage": "tts",
            "audio_format": read_str(frame.get("audio_format")),
            "sample_rate": int(read_num(frame.get("sample_rate"), 0)),
            "sequence": int(read_num(frame.get("sequence"), 0)),
            "duration_ms": read_num(frame.get("duration_ms")),
            "tts_elapsed_ms": read_num(frame.get("tts_elapsed_ms")),
            "queue_depth_ms": read_num(frame.get("queue_depth_ms")),
            "bytes": len(read_str(frame.get("audio_data"))),
            "voice_profile_hash": read_str(frame.get("voice_profile_hash")),
        }
        self.write_event(event)
        if not self.config.capture_raw_audio:
            return
        audio_data = read_str(frame.get("audio_data"))
        sample_rate = int(read_num(frame.get("sample_rate"), 48000))
        sequence = int(read_num(frame.get("sequence"), 0))
        if not audio_data:
            return
        samples = float32_base64_to_samples(audio_data)
        wav = wav_bytes_from_samples(samples, sample_rate)
        path = self.turn_audio_dir(turn_id) / f"output_chunk_{sequence:04d}_{sample_rate // 1000}k.wav"
        path.write_bytes(wav)


def split_tts_segments(buffer: str, min_chars: int) -> tuple[list[str], str]:
    segments: list[str] = []
    cursor = 0
    for match in re.finditer(r"[.!?;:]\s+", buffer):
        end = match.end()
        candidate = buffer[cursor:end].strip()
        if len(candidate) >= min_chars:
            segments.append(candidate)
            cursor = end
    if cursor:
        return segments, buffer[cursor:]
    if len(buffer) >= max(140, min_chars * 3):
        split_at = buffer.rfind(" ", 0, 120)
        if split_at >= min_chars:
            return [buffer[:split_at].strip()], buffer[split_at:].lstrip()
    return [], buffer


def float32_base64_to_samples(data: str) -> list[float]:
    raw = base64.b64decode(data)
    usable = len(raw) - (len(raw) % 4)
    if usable <= 0:
        return []
    return [sample[0] for sample in struct.iter_unpack("<f", raw[:usable])]


def wav_bytes_from_float32_base64(data: str, sample_rate: int) -> bytes:
    return wav_bytes_from_samples(float32_base64_to_samples(data), sample_rate)


def wav_bytes_from_samples(samples: list[float], sample_rate: int) -> bytes:
    frames = bytearray()
    for sample in samples:
        clipped = max(-1.0, min(1.0, sample))
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


def wav_duration_ms(wav: bytes, sample_rate: int) -> float:
    if len(wav) <= 44 or sample_rate <= 0:
        return 0.0
    return round(((len(wav) - 44) / 2) / sample_rate * 1000, 1)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bind", default=os.environ.get("JMCP_TALK_VOICE_BIND", "127.0.0.1:8040"))
    parser.add_argument("--llm-upstream", default=os.environ.get("JMCP_TALK_LLM_UPSTREAM", "http://127.0.0.1:18902/v1"))
    parser.add_argument("--llm-model", default=os.environ.get("JMCP_TALK_LLM_MODEL", "local/qwen2.5-7b-instruct-awq"))
    parser.add_argument("--asr-upstream", default=os.environ.get("JMCP_TALK_ASR_UPSTREAM", "http://127.0.0.1:18878"))
    parser.add_argument("--tts-upstream", default=os.environ.get("JMCP_TALK_TTS_UPSTREAM", "http://127.0.0.1:18901"))
    parser.add_argument("--jmcp-core", default=os.environ.get("JMCP_API_URL", "http://127.0.0.1:18877"))
    parser.add_argument("--voice-profile", default=os.environ.get("JMCP_TALK_VOICE_PROFILE", "jmcp_male_v1"))
    parser.add_argument(
        "--voice-profile-path",
        default=os.environ.get(
            "JMCP_TALK_VOICE_PROFILE_PATH",
            str(Path(__file__).resolve().parent.parent / "speech" / "voice_profiles" / "jmcp_male_v1.json"),
        ),
    )
    parser.add_argument(
        "--event-log",
        default=os.environ.get("JMCP_TALK_VOICE_EVENT_LOG", "/home/ubuntu/jmcp-split/.live/logs/voice-events.jsonl"),
    )
    parser.add_argument(
        "--audio-dir",
        default=os.environ.get("JMCP_TALK_AUDIO_DIR", "/home/ubuntu/jmcp-split/.live/audio"),
    )
    return parser.parse_args()


def build_config(args: argparse.Namespace) -> GatewayConfig:
    profile_path = Path(args.voice_profile_path).expanduser().resolve()
    return GatewayConfig(
        bind=args.bind,
        llm_upstream=args.llm_upstream.rstrip("/"),
        llm_model=args.llm_model,
        asr_upstream=args.asr_upstream.rstrip("/"),
        tts_upstream=args.tts_upstream.rstrip("/"),
        jmcp_core=args.jmcp_core.rstrip("/"),
        event_log=Path(args.event_log),
        audio_dir=Path(args.audio_dir).expanduser().resolve(),
        capture_raw_audio=os.environ.get("JMCP_TALK_CAPTURE_RAW_AUDIO", "1") != "0",
        voice_profile=args.voice_profile,
        voice_profile_path=profile_path,
        voice_profile_hash=load_profile_hash(profile_path),
        min_tts_phrase_chars=int(os.environ.get("JMCP_TALK_MIN_TTS_PHRASE_CHARS", "32")),
        connect_timeout_s=float(os.environ.get("JMCP_TALK_VOICE_CONNECT_TIMEOUT_MS", "10000")) / 1000,
        first_frame_timeout_s=float(os.environ.get("JMCP_TALK_VOICE_FIRST_FRAME_TIMEOUT_MS", "45000")) / 1000,
        idle_timeout_s=float(os.environ.get("JMCP_TALK_VOICE_IDLE_TIMEOUT_MS", "15000")) / 1000,
        total_turn_timeout_s=float(os.environ.get("JMCP_TALK_VOICE_TOTAL_TURN_TIMEOUT_MS", "120000")) / 1000,
    )


def main() -> None:
    args = parse_args()
    config = build_config(args)
    host, port_text = config.bind.rsplit(":", 1)
    gateway = VoiceGateway(config)
    uvicorn.run(gateway.app, host=host, port=int(port_text), log_level="info")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""JMCP MiniCPM-o live voice gateway.

This process is the stable JMCP speech/audio boundary. The cockpit talks to this
gateway through /voice and /voice-ws; the gateway owns Comni protocol details,
timeouts, redacted JSONL events, and metrics.
"""

from __future__ import annotations

import argparse
import asyncio
import base64
import hashlib
import json
import os
import re
import shutil
import struct
import time
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

import httpx
import uvicorn
import websockets
from fastapi import FastAPI, Request, WebSocket, WebSocketDisconnect
from fastapi.responses import JSONResponse, PlainTextResponse


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def is_record(value: Any) -> bool:
    return isinstance(value, dict)


def read_str(value: Any) -> str:
    return value if isinstance(value, str) else ""


def preview(text: str, limit: int = 80) -> str:
    clean = " ".join(text.split())
    return clean[:limit]


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


def ws_url_from_http(base: str, path: str) -> str:
    parsed = urlparse(base)
    scheme = "wss" if parsed.scheme == "https" else "ws"
    netloc = parsed.netloc or parsed.path
    return f"{scheme}://{netloc}{path}"


@dataclass
class Metrics:
    turns_total: int = 0
    failures: dict[str, int] = field(default_factory=dict)
    first_token_latency_ms: list[float] = field(default_factory=list)
    first_audio_latency_ms: list[float] = field(default_factory=list)
    dropped_frames: int = 0

    def fail(self, stage: str) -> None:
        self.failures[stage] = self.failures.get(stage, 0) + 1


@dataclass
class GatewayConfig:
    bind: str
    upstream: str
    jmcp_core: str
    event_log: Path
    voice_profile: str
    ref_audio_path: Path
    ref_audio_hash: str
    ref_audio_data: str
    audio_dir: Path
    capture_raw_audio: bool
    connect_timeout_s: float
    first_frame_timeout_s: float
    idle_timeout_s: float
    total_turn_timeout_s: float


class Gateway:
    def __init__(self, config: GatewayConfig) -> None:
        self.config = config
        self.metrics = Metrics()
        self.http = httpx.AsyncClient(timeout=5.0)
        self.app = FastAPI(title="JMCP MiniCPM-o Live Voice Gateway")
        self._routes()

    def _routes(self) -> None:
        self.app.get("/health")(self.health)
        self.app.get("/status")(self.proxy_status)
        self.app.get("/workers")(self.proxy_workers)
        self.app.get("/metrics")(self.metrics_response)
        self.app.post("/events")(self.events)
        self.app.websocket("/ws/chat")(self.chat_ws)
        self.app.websocket("/ws/duplex/{session_id}")(self.duplex_ws)
        self.app.websocket("/ws/half_duplex/{session_id}")(self.half_duplex_ws)
        self.app.websocket("/ws/half_duplex_omni/{session_id}")(self.half_duplex_omni_ws)

    async def close(self) -> None:
        await self.http.aclose()

    def write_event(self, event: dict[str, Any]) -> None:
        self.config.event_log.parent.mkdir(parents=True, exist_ok=True)
        event = self.sanitize_event(event)
        with self.config.event_log.open("a", encoding="utf-8") as file:
            file.write(json.dumps(event, ensure_ascii=False, sort_keys=True) + "\n")
        self.write_audio_log(event)

    def write_audio_log(self, event: dict[str, Any]) -> None:
        if not self.config.capture_raw_audio:
            return
        self.config.audio_dir.mkdir(parents=True, exist_ok=True)
        with (self.config.audio_dir / "events.jsonl").open("a", encoding="utf-8") as file:
            file.write(json.dumps(event, ensure_ascii=False, sort_keys=True) + "\n")

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
            "route": read_str(event.get("route")) or None,
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

    async def health(self) -> JSONResponse:
        upstream_health: dict[str, Any] = {}
        upstream_status: dict[str, Any] = {}
        ok = False
        try:
            health_response = await self.http.get(f"{self.config.upstream}/health")
            if health_response.status_code < 400:
                upstream_health = health_response.json()
            status_response = await self.http.get(f"{self.config.upstream}/status")
            if status_response.status_code < 400:
                upstream_status = status_response.json()
            ok = read_str(upstream_health.get("status")) == "healthy"
        except Exception as exc:  # noqa: BLE001
            upstream_health = {"error": exc.__class__.__name__}

        return JSONResponse(
            {
                "ok": ok,
                "adapter": "minicpm-o45",
                "engine": "comni",
                "model": "openbmb/MiniCPM-o-4_5-gguf",
                "device": "cuda",
                "quantization": os.environ.get("JMCP_TALK_MINICPM_QUANT", "Q4_K_M"),
                "voice_profile": self.config.voice_profile,
                "ref_audio_path": str(self.config.ref_audio_path),
                "ref_audio_hash": self.config.ref_audio_hash,
                "loaded": ok,
                "raw_audio_capture": self.config.capture_raw_audio,
                "audio_dir": str(self.config.audio_dir) if self.config.capture_raw_audio else None,
                "upstream": self.config.upstream,
                "upstream_health": upstream_health,
                "upstream_status": upstream_status,
            },
            status_code=200 if ok else 503,
        )

    async def proxy_status(self) -> JSONResponse:
        return await self.proxy_json("/status")

    async def proxy_workers(self) -> JSONResponse:
        return await self.proxy_json("/workers")

    async def proxy_json(self, path: str) -> JSONResponse:
        try:
            response = await self.http.get(f"{self.config.upstream}{path}")
            return JSONResponse(response.json(), status_code=response.status_code)
        except Exception as exc:  # noqa: BLE001
            return JSONResponse({"ok": False, "error": exc.__class__.__name__}, status_code=503)

    async def events(self, request: Request) -> JSONResponse:
        body = await request.json()
        if isinstance(body, list):
            for item in body:
                if is_record(item):
                    self.write_event(item)
        elif is_record(body):
            self.write_event(body)
        return JSONResponse({"ok": True})

    async def metrics_response(self) -> PlainTextResponse:
        failure_lines = [
            f'jmcp_voice_failures_total{{stage="{stage}"}} {count}'
            for stage, count in sorted(self.metrics.failures.items())
        ]
        first_audio_avg = avg(self.metrics.first_audio_latency_ms)
        first_token_avg = avg(self.metrics.first_token_latency_ms)
        lines = [
            f"jmcp_voice_turns_total {self.metrics.turns_total}",
            f"jmcp_voice_first_audio_latency_ms_avg {first_audio_avg:.1f}",
            f"jmcp_voice_first_token_latency_ms_avg {first_token_avg:.1f}",
            f"jmcp_voice_dropped_frames_total {self.metrics.dropped_frames}",
            *failure_lines,
            "",
        ]
        return PlainTextResponse("\n".join(lines))

    async def chat_ws(self, ws: WebSocket) -> None:
        await ws.accept()
        turn_id = f"turn_{uuid.uuid4().hex}"
        started = time.perf_counter()
        upstream_ws = None
        first_frame = False
        first_token = False
        first_audio = False
        full_text = ""
        self.metrics.turns_total += 1
        try:
            raw = await asyncio.wait_for(ws.receive_text(), timeout=self.config.connect_timeout_s)
            request = json.loads(raw)
            if isinstance(request, dict):
                turn_id = read_str(request.get("turn_id")) or turn_id
            self.write_event({"turn_id": turn_id, "event": "voice.send", "stage": "gateway"})
            self.capture_request_audio(turn_id, request)
            output_chunks: list[float] = []
            output_chunk_index = 0

            upstream_ws = await asyncio.wait_for(
                websockets.connect(ws_url_from_http(self.config.upstream, "/ws/chat"), max_size=128 * 1024 * 1024),
                timeout=self.config.connect_timeout_s,
            )
            await upstream_ws.send(json.dumps(self.comni_chat_payload(request), ensure_ascii=False))
            self.write_event({"turn_id": turn_id, "event": "voice.backend_connected", "stage": "gateway"})

            deadline = started + self.config.total_turn_timeout_s
            while time.perf_counter() < deadline:
                timeout = self.config.idle_timeout_s if first_frame else self.config.first_frame_timeout_s
                try:
                    frame_raw = await asyncio.wait_for(upstream_ws.recv(), timeout=timeout)
                except asyncio.TimeoutError:
                    stage = "idle_frame" if first_frame else "first_frame"
                    self.metrics.fail(stage)
                    await ws.send_json({"type": "error", "turn_id": turn_id, "error": f"{stage}_timeout"})
                    self.write_event(
                        {"turn_id": turn_id, "event": "voice.timeout", "stage": stage, "status": "error"}
                    )
                    return

                frame = parse_json_frame(frame_raw)
                if not first_frame:
                    first_frame = True
                    self.write_event(
                        {
                            "turn_id": turn_id,
                            "event": "voice.first_model_frame",
                            "stage": "model",
                            "latency_ms": elapsed_ms(started),
                        }
                    )
                text_delta = read_str(frame.get("text_delta"))
                if text_delta:
                    full_text += text_delta
                    if not first_token:
                        first_token = True
                        latency = elapsed_ms(started)
                        self.metrics.first_token_latency_ms.append(latency)
                        self.write_event(
                            {
                                "turn_id": turn_id,
                                "event": "voice.first_token",
                                "stage": "model",
                                "latency_ms": latency,
                                "text": text_delta,
                            }
                        )
                audio_data = read_str(frame.get("audio_data"))
                if audio_data:
                    samples = self.capture_pcm_base64(
                        turn_id,
                        "assistant_output_chunk",
                        f"output_chunk_{output_chunk_index:03d}_24k.wav",
                        audio_data,
                        24000,
                    )
                    output_chunk_index += 1
                    output_chunks.extend(samples)
                if audio_data and not first_audio:
                    first_audio = True
                    latency = elapsed_ms(started)
                    self.metrics.first_audio_latency_ms.append(latency)
                    self.write_event(
                        {
                            "turn_id": turn_id,
                            "event": "voice.first_audio",
                            "stage": "model",
                            "latency_ms": latency,
                            "bytes": len(audio_data),
                        }
                    )

                if read_str(frame.get("type")) == "done":
                    done_text = read_str(frame.get("text"))
                    if done_text:
                        full_text = done_text
                    if output_chunks:
                        self.write_audio_samples(
                            turn_id,
                            "assistant_output_turn",
                            "output_turn_24k.wav",
                            output_chunks,
                            24000,
                        )
                    frame["turn_id"] = turn_id
                    await ws.send_json(frame)
                    self.write_event(
                        {
                            "turn_id": turn_id,
                            "event": "voice.close",
                            "stage": "gateway",
                            "latency_ms": elapsed_ms(started),
                            "text": full_text,
                        }
                    )
                    return

                if read_str(frame.get("type")) == "error":
                    self.metrics.fail("backend")
                    frame["turn_id"] = turn_id
                    await ws.send_json(frame)
                    self.write_event(
                        {"turn_id": turn_id, "event": "voice.error", "stage": "backend", "status": "error"}
                    )
                    return

                frame["turn_id"] = turn_id
                await ws.send_json(frame)

            self.metrics.fail("total_turn")
            await ws.send_json({"type": "error", "turn_id": turn_id, "error": "total_turn_timeout"})
            self.write_event(
                {"turn_id": turn_id, "event": "voice.timeout", "stage": "total_turn", "status": "error"}
            )
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
        finally:
            if upstream_ws is not None:
                await upstream_ws.close()

    def comni_chat_payload(self, request: Any) -> dict[str, Any]:
        if not isinstance(request, dict):
            request = {}
        messages = request.get("messages")
        if not isinstance(messages, list):
            prompt = read_str(request.get("prompt")) or read_str(request.get("text"))
            messages = [{"role": "user", "content": prompt}]
        messages = self.messages_with_ref_audio(messages)
        generation = request.get("generation") if isinstance(request.get("generation"), dict) else {}
        tts = request.get("tts") if isinstance(request.get("tts"), dict) else {}
        tts_payload = {
            "enabled": tts.get("enabled", True),
            "ref_audio_data": self.config.ref_audio_data,
            "voice_profile": self.config.voice_profile,
            **{k: v for k, v in tts.items() if k not in {"enabled", "ref_audio_data", "ref_audio_path", "voice_profile"}},
        }
        return {
            "messages": messages,
            "streaming": request.get("streaming", True),
            "generation": {
                "max_new_tokens": generation.get("max_new_tokens", 240),
                "length_penalty": generation.get("length_penalty", 1.1),
                "temperature": generation.get("temperature", 0.7),
            },
            "audio": {"ref_audio_path": str(self.config.ref_audio_path)},
            "tts": tts_payload,
            "use_tts_template": request.get("use_tts_template", True),
            "omni_mode": request.get("omni_mode", True),
            "enable_thinking": request.get("enable_thinking", False),
        }

    def messages_with_ref_audio(self, messages: list[Any]) -> list[Any]:
        ref_item = {"type": "audio", "data": self.config.ref_audio_data}
        out: list[Any] = []
        injected = False
        for raw in messages:
            if not isinstance(raw, dict):
                out.append(raw)
                continue
            message = dict(raw)
            if message.get("role") == "system" and not injected:
                content = message.get("content", "")
                if isinstance(content, list):
                    has_audio = any(isinstance(item, dict) and item.get("type") == "audio" for item in content)
                    message["content"] = content if has_audio else [*content, ref_item]
                else:
                    text = read_str(content)
                    message["content"] = [
                        {"type": "text", "text": text},
                        ref_item,
                    ]
                injected = True
            out.append(message)
        if not injected:
            out.insert(
                0,
                {
                    "role": "system",
                    "content": [
                        {"type": "text", "text": "Use the configured JMCP friendly male reference voice."},
                        ref_item,
                    ],
                },
            )
        return out

    def turn_audio_dir(self, turn_id: str) -> Path:
        safe = re.sub(r"[^A-Za-z0-9_.-]+", "_", turn_id)[:96] or f"turn_{uuid.uuid4().hex}"
        path = self.config.audio_dir / safe
        path.mkdir(parents=True, exist_ok=True)
        return path

    def capture_request_audio(self, turn_id: str, request: Any) -> None:
        if not self.config.capture_raw_audio or not isinstance(request, dict):
            return
        turn_dir = self.turn_audio_dir(turn_id)
        if self.config.ref_audio_path.is_file():
            shutil.copyfile(self.config.ref_audio_path, turn_dir / "ref_audio_16k.wav")
        input_index = 0
        for message in request.get("messages", []):
            if not isinstance(message, dict) or message.get("role") != "user":
                continue
            content = message.get("content")
            if not isinstance(content, list):
                continue
            for item in content:
                if not isinstance(item, dict) or item.get("type") != "audio":
                    continue
                self.capture_pcm_base64(
                    turn_id,
                    "user_input",
                    f"input_{input_index:03d}_16k.wav",
                    read_str(item.get("data")),
                    16000,
                )
                input_index += 1

    def capture_pcm_base64(
        self,
        turn_id: str,
        kind: str,
        filename: str,
        audio_data: str,
        sample_rate: int,
    ) -> list[float]:
        if not self.config.capture_raw_audio or not audio_data:
            return []
        try:
            samples = float32_base64_to_samples(audio_data)
        except Exception as exc:  # noqa: BLE001
            self.write_event(
                {
                    "turn_id": turn_id,
                    "event": "voice.audio_capture_failed",
                    "stage": kind,
                    "status": "error",
                    "error_class": exc.__class__.__name__,
                }
            )
            return []
        self.write_audio_samples(turn_id, kind, filename, samples, sample_rate)
        return samples

    def write_audio_samples(
        self,
        turn_id: str,
        kind: str,
        filename: str,
        samples: list[float],
        sample_rate: int,
    ) -> None:
        if not self.config.capture_raw_audio or not samples:
            return
        turn_dir = self.turn_audio_dir(turn_id)
        path = turn_dir / filename
        write_wav_pcm16(path, samples, sample_rate)
        audio_hash = sha256_file(path)
        event = {
            "ts": now_iso(),
            "turn_id": turn_id,
            "event": "voice.audio_snippet",
            "stage": kind,
            "status": "ok",
            "path": str(path),
            "bytes": path.stat().st_size,
            "sample_rate": sample_rate,
            "samples": len(samples),
            "duration_ms": round(len(samples) / sample_rate * 1000, 1),
            "audio_hash": audio_hash,
        }
        with (turn_dir / "audio-events.jsonl").open("a", encoding="utf-8") as file:
            file.write(json.dumps(event, ensure_ascii=False, sort_keys=True) + "\n")
        self.write_audio_log(event)

    async def duplex_ws(self, ws: WebSocket, session_id: str) -> None:
        await self.proxy_ws(ws, f"/ws/duplex/{session_id}", session_id)

    async def half_duplex_ws(self, ws: WebSocket, session_id: str) -> None:
        await self.proxy_ws(ws, f"/ws/half_duplex/{session_id}", session_id)

    async def half_duplex_omni_ws(self, ws: WebSocket, session_id: str) -> None:
        await self.proxy_ws(ws, f"/ws/half_duplex_omni/{session_id}", session_id)

    async def proxy_ws(self, ws: WebSocket, upstream_path: str, session_id: str) -> None:
        await ws.accept()
        turn_id = session_id or f"turn_{uuid.uuid4().hex}"
        upstream_ws = None
        try:
            upstream_ws = await asyncio.wait_for(
                websockets.connect(ws_url_from_http(self.config.upstream, upstream_path), max_size=128 * 1024 * 1024),
                timeout=self.config.connect_timeout_s,
            )
            self.write_event({"turn_id": turn_id, "event": "voice.backend_connected", "route": upstream_path})

            async def client_to_backend() -> None:
                async for raw in ws.iter_text():
                    await upstream_ws.send(raw)

            async def backend_to_client() -> None:
                async for raw in upstream_ws:
                    await ws.send_text(raw)

            tasks = [asyncio.create_task(client_to_backend()), asyncio.create_task(backend_to_client())]
            done, pending = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            for task in pending:
                task.cancel()
            for task in done:
                task.result()
        except WebSocketDisconnect:
            self.write_event({"turn_id": turn_id, "event": "voice.close", "route": upstream_path})
        except Exception as exc:  # noqa: BLE001
            self.metrics.fail("gateway_proxy")
            self.write_event(
                {
                    "turn_id": turn_id,
                    "event": "voice.error",
                    "route": upstream_path,
                    "status": "error",
                    "error_class": exc.__class__.__name__,
                }
            )
        finally:
            if upstream_ws is not None:
                await upstream_ws.close()


def parse_json_frame(raw: Any) -> dict[str, Any]:
    if isinstance(raw, bytes):
        raw = raw.decode("utf-8", errors="replace")
    if not isinstance(raw, str):
        return {"type": "unknown"}
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError:
        return {"type": "raw", "text_delta": raw}
    return parsed if isinstance(parsed, dict) else {"type": "unknown"}


def float32_base64_to_samples(data: str) -> list[float]:
    raw = base64.b64decode(data)
    usable = len(raw) - (len(raw) % 4)
    if usable <= 0:
        return []
    return [sample[0] for sample in struct.iter_unpack("<f", raw[:usable])]


def write_wav_pcm16(path: Path, samples: list[float], sample_rate: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with wave_open(path, "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(sample_rate)
        frames = bytearray()
        for sample in samples:
            clipped = max(-1.0, min(1.0, sample))
            value = int(clipped * 32767) if clipped >= 0 else int(clipped * 32768)
            frames.extend(struct.pack("<h", value))
        wav.writeframes(bytes(frames))


def wav_to_float32_base64(path: Path) -> str:
    with wave_open(path, "rb") as wav:
        channels = wav.getnchannels()
        sample_width = wav.getsampwidth()
        sample_rate = wav.getframerate()
        if sample_rate != 16000:
            raise ValueError(f"reference audio must be 16 kHz, got {sample_rate}")
        raw = wav.readframes(wav.getnframes())
    samples: list[float] = []
    if sample_width == 2:
        step = 2 * channels
        for offset in range(0, len(raw) - step + 1, step):
            total = 0.0
            for channel in range(channels):
                total += struct.unpack_from("<h", raw, offset + channel * 2)[0] / 32768.0
            samples.append(total / max(1, channels))
    else:
        raise ValueError(f"reference audio must be 16-bit PCM, got width {sample_width}")
    packed = bytearray()
    for sample in samples:
        packed.extend(struct.pack("<f", max(-1.0, min(1.0, sample))))
    return base64.b64encode(bytes(packed)).decode("ascii")


def wave_open(path: Path, mode: str):
    import wave

    return wave.open(str(path), mode)


def avg(values: list[float]) -> float:
    return sum(values) / len(values) if values else 0.0


def elapsed_ms(started: float) -> float:
    return round((time.perf_counter() - started) * 1000, 1)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bind", default=os.environ.get("JMCP_TALK_MINICPM_BIND", "127.0.0.1:8041"))
    parser.add_argument("--upstream", default=os.environ.get("JMCP_TALK_MINICPM_UPSTREAM", "http://127.0.0.1:18040"))
    parser.add_argument("--jmcp-core", default=os.environ.get("JMCP_API_URL", "http://127.0.0.1:18877"))
    parser.add_argument(
        "--voice-profile",
        default=os.environ.get("JMCP_TALK_MINICPM_VOICE_PROFILE", "jmcp_friendly_male"),
    )
    parser.add_argument(
        "--ref-audio",
        default=os.environ.get(
            "JMCP_TALK_MINICPM_REF_AUDIO",
            str(Path(__file__).resolve().parent / "assets" / "ref_audio" / "jmcp_friendly_male_16k.wav"),
        ),
    )
    parser.add_argument(
        "--event-log",
        default=os.environ.get(
            "JMCP_TALK_MINICPM_EVENT_LOG",
            "/home/ubuntu/jmcp-split/.live/logs/voice-events.jsonl",
        ),
    )
    parser.add_argument(
        "--audio-dir",
        default=os.environ.get("JMCP_TALK_AUDIO_DIR", "/home/ubuntu/jmcp-split/.live/audio"),
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    host, port_text = args.bind.rsplit(":", 1)
    ref_audio_path = Path(args.ref_audio).expanduser().resolve()
    ref_audio_data = wav_to_float32_base64(ref_audio_path)
    capture_raw_audio = os.environ.get("JMCP_TALK_CAPTURE_RAW_AUDIO", "1") != "0"
    config = GatewayConfig(
        bind=args.bind,
        upstream=args.upstream.rstrip("/"),
        jmcp_core=args.jmcp_core.rstrip("/"),
        event_log=Path(args.event_log),
        voice_profile=args.voice_profile,
        ref_audio_path=ref_audio_path,
        ref_audio_hash=sha256_file(ref_audio_path),
        ref_audio_data=ref_audio_data,
        audio_dir=Path(args.audio_dir).expanduser().resolve(),
        capture_raw_audio=capture_raw_audio,
        connect_timeout_s=float(os.environ.get("JMCP_TALK_MINICPM_CONNECT_TIMEOUT_MS", "10000")) / 1000,
        first_frame_timeout_s=float(os.environ.get("JMCP_TALK_MINICPM_FIRST_FRAME_TIMEOUT_MS", "45000")) / 1000,
        idle_timeout_s=float(os.environ.get("JMCP_TALK_MINICPM_IDLE_TIMEOUT_MS", "15000")) / 1000,
        total_turn_timeout_s=float(os.environ.get("JMCP_TALK_MINICPM_TOTAL_TURN_TIMEOUT_MS", "120000")) / 1000,
    )
    gateway = Gateway(config)
    uvicorn.run(gateway.app, host=host, port=int(port_text), log_level="info")


if __name__ == "__main__":
    main()

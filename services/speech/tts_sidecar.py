#!/usr/bin/env python3
"""JMCP local TTS sidecar.

The primary runtime is VoxCPM2, kept hot in this process and exposed only on a
loopback port. Kokoro remains an emergency degraded mode with the same HTTP
contract used by the existing Rust speech adapter.

Endpoints
  GET  /health
  POST /synthesize  JSON {text, voice?, speed?} -> WAV/OGG bytes
  POST /stream      JSON {text, sequence_start?} -> NDJSON float32 PCM frames
"""

from __future__ import annotations

import base64
import hashlib
import io
import json
import os
import threading
import time
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Iterable
from urllib.parse import parse_qs, urlparse

VOICE = os.environ.get("TTS_VOICE", "jmcp_male_v1")
LANG = os.environ.get("TTS_LANG", "a")
DEVICE = os.environ.get("TTS_DEVICE", "auto")
BIND = os.environ.get("TTS_BIND", "127.0.0.1:18901")
ENGINE = os.environ.get("TTS_ENGINE", "voxcpm2").lower()
FALLBACK_ENGINE = os.environ.get("TTS_FALLBACK_ENGINE", "kokoro").lower()
VOXCPM_MODEL = os.environ.get("TTS_VOXCPM_MODEL", "openbmb/VoxCPM2")
VOXCPM_OPTIMIZE = os.environ.get("TTS_VOXCPM_OPTIMIZE", "1") != "0"
VOXCPM_LOAD_DENOISER = os.environ.get("TTS_VOXCPM_LOAD_DENOISER", "0") == "1"
VOXCPM_CFG_VALUE = float(os.environ.get("TTS_VOXCPM_CFG_VALUE", "2.0"))
VOXCPM_TIMESTEPS = int(os.environ.get("TTS_VOXCPM_INFERENCE_TIMESTEPS", "10"))
KOKORO_VOICE = os.environ.get("TTS_KOKORO_VOICE", "am_michael")
KOKORO_SAMPLE_RATE = 24000
VOXCPM_SAMPLE_RATE = 48000
HERE = Path(__file__).resolve().parent
VOICE_PROFILE_PATH = Path(
    os.environ.get(
        "TTS_VOICE_PROFILE_PATH",
        str(HERE / "voice_profiles" / "jmcp_male_v1.json"),
    )
).expanduser()


@dataclass(frozen=True)
class VoiceProfile:
    profile_id: str
    engine: str
    sample_rate: int
    design_prompt: str
    provenance: str
    profile_hash: str
    path: Path


_PROFILE = None
_STATE: dict[str, Any] = {
    "pipeline": None,
    "loaded": False,
    "warmed": False,
    "error": None,
    "primary_error": None,
    "warm_error": None,
    "device": None,
    "engine": ENGINE,
    "active_engine": None,
    "degraded_active": False,
    "sample_rate": None,
    "last_elapsed_ms": None,
    "last_warmup_ms": None,
    "last_audio_seconds": None,
    "last_rtf": None,
}
_LOCK = threading.Lock()


def load_voice_profile(path: Path = VOICE_PROFILE_PATH) -> VoiceProfile:
    data = json.loads(path.read_text(encoding="utf-8"))
    canonical = json.dumps(data, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    profile_hash = hashlib.sha256(canonical.encode("utf-8")).hexdigest()
    return VoiceProfile(
        profile_id=str(data.get("id") or "jmcp_male_v1"),
        engine=str(data.get("engine") or "openbmb/VoxCPM2"),
        sample_rate=int(data.get("sample_rate") or VOXCPM_SAMPLE_RATE),
        design_prompt=str(data.get("design_prompt") or ""),
        provenance=str(data.get("provenance") or ""),
        profile_hash=profile_hash,
        path=path,
    )


def _resolve_device() -> str:
    if DEVICE != "auto":
        return DEVICE
    try:
        import torch

        return "cuda" if torch.cuda.is_available() else "cpu"
    except Exception:  # noqa: BLE001
        return "cpu"


def _load_pipeline() -> None:
    global _PROFILE
    try:
        _PROFILE = load_voice_profile()
    except Exception as exc:  # noqa: BLE001
        with _LOCK:
            _STATE["error"] = f"profile:{type(exc).__name__}: {exc}"
        print(f"[tts] profile load FAILED: {_STATE['error']}", flush=True)
        return

    primary_error = None
    for index, engine in enumerate(engine_candidates()):
        try:
            pipeline, device, sample_rate = load_engine(engine)
            warmup_ms, warm_error = warm_engine(engine, pipeline)
            with _LOCK:
                _STATE["pipeline"] = pipeline
                _STATE["device"] = device
                _STATE["loaded"] = True
                _STATE["warmed"] = warm_error is None
                _STATE["warm_error"] = warm_error
                _STATE["last_warmup_ms"] = warmup_ms
                _STATE["error"] = None
                _STATE["primary_error"] = primary_error
                _STATE["active_engine"] = engine
                _STATE["degraded_active"] = index > 0
                _STATE["sample_rate"] = sample_rate
            degraded = " degraded" if index > 0 else ""
            warm = f", warmup={warmup_ms}ms" if warm_error is None else f", warmup failed: {warm_error}"
            print(f"[tts] {engine}{degraded} loaded on {device} (voice={VOICE}{warm})", flush=True)
            return
        except Exception as exc:  # noqa: BLE001
            message = f"{type(exc).__name__}: {exc}"
            if index == 0:
                primary_error = message
            print(f"[tts] {engine} load FAILED: {message}", flush=True)

    with _LOCK:
        _STATE["error"] = primary_error or "no TTS engine loaded"
        _STATE["primary_error"] = primary_error


def engine_candidates() -> list[str]:
    candidates = [ENGINE]
    if FALLBACK_ENGINE and FALLBACK_ENGINE not in candidates:
        candidates.append(FALLBACK_ENGINE)
    return candidates


def load_engine(engine: str) -> tuple[Any, str, int]:
    if engine == "voxcpm2":
        from voxcpm import VoxCPM

        device = _resolve_device()
        pipeline = VoxCPM.from_pretrained(
            VOXCPM_MODEL,
            load_denoiser=VOXCPM_LOAD_DENOISER,
            optimize=VOXCPM_OPTIMIZE,
            device=device,
        )
        sample_rate = int(getattr(getattr(pipeline, "tts_model", None), "sample_rate", VOXCPM_SAMPLE_RATE))
        return pipeline, device, sample_rate
    if engine == "kokoro":
        from kokoro import KPipeline

        device = _resolve_device()
        return KPipeline(lang_code=LANG, device=device), device, KOKORO_SAMPLE_RATE
    raise ValueError(f"unsupported TTS engine: {engine}")


def warm_engine(engine: str, pipeline: Any) -> tuple[float | None, str | None]:
    started = time.monotonic()
    try:
        list(render_pcm_chunks(engine, pipeline, "Ready.", streaming=False))
        return round((time.monotonic() - started) * 1000, 1), None
    except Exception as exc:  # noqa: BLE001
        return None, f"{type(exc).__name__}: {exc}"


def profile_text(text: str) -> str:
    profile = _PROFILE or load_voice_profile()
    clean = " ".join(text.split())
    if not profile.design_prompt:
        return clean
    return f"({profile.design_prompt}){clean}"


def render_pcm_chunks(
    engine: str,
    pipeline: Any,
    text: str,
    streaming: bool,
    voice: str | None = None,
    speed: float = 1.0,
) -> Iterable[Any]:
    import numpy as np

    if engine == "voxcpm2":
        generate = pipeline.generate_streaming if streaming else pipeline.generate
        kwargs = {
            "text": profile_text(text),
            "cfg_value": VOXCPM_CFG_VALUE,
            "inference_timesteps": VOXCPM_TIMESTEPS,
        }
        if streaming:
            for chunk in generate(**kwargs):
                yield np.asarray(chunk, dtype="float32").reshape(-1)
        else:
            yield np.asarray(generate(**kwargs), dtype="float32").reshape(-1)
        return

    if engine == "kokoro":
        chunks = [
            to_numpy(audio)
            for _, _, audio in pipeline(text, voice=voice or KOKORO_VOICE, speed=speed)
        ]
        if not chunks:
            yield np.zeros(1, dtype="float32")
        elif streaming:
            for chunk in chunks:
                yield chunk
        else:
            yield np.concatenate(chunks)
        return

    raise ValueError(f"unsupported TTS engine: {engine}")


def to_numpy(audio: Any):
    import numpy as np

    if hasattr(audio, "detach"):
        audio = audio.detach().cpu().numpy()
    return np.asarray(audio, dtype="float32").reshape(-1)


_FORMATS = {
    "wav": ("WAV", "PCM_16", "audio/wav"),
    "ogg": ("OGG", "OPUS", "audio/ogg"),
    "opus": ("OGG", "OPUS", "audio/ogg"),
}


def encode_audio(pcm: Any, sample_rate: int, fmt: str) -> tuple[bytes, str]:
    import soundfile as sf

    sf_format, subtype, content_type = _FORMATS.get(fmt, _FORMATS["wav"])
    buf = io.BytesIO()
    sf.write(buf, pcm, sample_rate, format=sf_format, subtype=subtype)
    return buf.getvalue(), content_type


def pcm_frame(
    pcm: Any,
    sample_rate: int,
    sequence: int,
    started: float,
    queue_depth_ms: float = 0.0,
) -> dict[str, Any]:
    import numpy as np

    data = np.asarray(pcm, dtype="float32").reshape(-1)
    return {
        "type": "audio",
        "audio_format": "f32le",
        "sample_rate": sample_rate,
        "sequence": sequence,
        "duration_ms": round((len(data) / sample_rate) * 1000, 1) if sample_rate else 0.0,
        "tts_elapsed_ms": round((time.monotonic() - started) * 1000, 1),
        "queue_depth_ms": round(queue_depth_ms, 1),
        "voice_profile": (_PROFILE.profile_id if _PROFILE else VOICE),
        "voice_profile_hash": (_PROFILE.profile_hash if _PROFILE else ""),
        "audio_data": base64.b64encode(data.tobytes()).decode("ascii"),
    }


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _json(self, code: int, payload: dict[str, Any]) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args: Any) -> None:
        pass

    def do_GET(self) -> None:
        if urlparse(self.path).path == "/health":
            with _LOCK:
                payload = {
                    "ok": _STATE["error"] is None,
                    "model": VOXCPM_MODEL if _STATE["active_engine"] == "voxcpm2" else "kokoro-82M",
                    "engine": _STATE["engine"],
                    "active_engine": _STATE["active_engine"],
                    "voice_engine": _STATE["active_engine"] or _STATE["engine"],
                    "degraded_active": _STATE["degraded_active"],
                    "degraded_engine": FALLBACK_ENGINE,
                    "primary_error": _STATE["primary_error"],
                    "device": _STATE["device"],
                    "loaded": _STATE["loaded"],
                    "warmed": _STATE["warmed"],
                    "voice": VOICE,
                    "voice_profile": _PROFILE.profile_id if _PROFILE else VOICE,
                    "voice_profile_hash": _PROFILE.profile_hash if _PROFILE else "",
                    "voice_profile_path": str(_PROFILE.path) if _PROFILE else str(VOICE_PROFILE_PATH),
                    "sample_rate": _STATE["sample_rate"] or (_PROFILE.sample_rate if _PROFILE else VOXCPM_SAMPLE_RATE),
                    "streaming_audio": True,
                    "audio_format": "f32le",
                    "last_elapsed_ms": _STATE["last_elapsed_ms"],
                    "last_warmup_ms": _STATE["last_warmup_ms"],
                    "last_audio_seconds": _STATE["last_audio_seconds"],
                    "last_rtf": _STATE["last_rtf"],
                    "error": _STATE["error"],
                    "warm_error": _STATE["warm_error"],
                }
            self._json(200, payload)
            return
        self._json(404, {"error": "not found"})

    def do_POST(self) -> None:
        parsed = urlparse(self.path)
        if parsed.path == "/synthesize":
            self.synthesize(parsed)
            return
        if parsed.path == "/stream":
            self.stream(parsed)
            return
        self._json(404, {"error": "not found"})

    def read_request(self) -> dict[str, Any] | None:
        length = int(self.headers.get("content-length", 0))
        raw = self.rfile.read(length) if length > 0 else b"{}"
        try:
            req = json.loads(raw or b"{}")
        except json.JSONDecodeError:
            self._json(400, {"error": "body must be JSON {text, voice?, speed?}"})
            return None
        if not isinstance(req, dict):
            self._json(400, {"error": "body must be JSON {text, voice?, speed?}"})
            return None
        if not str(req.get("text") or "").strip():
            self._json(400, {"error": "missing 'text'"})
            return None
        return req

    def current_pipeline(self) -> tuple[Any, str, int] | None:
        with _LOCK:
            pipeline = _STATE["pipeline"]
            engine = _STATE["active_engine"]
            sample_rate = _STATE["sample_rate"]
            error = _STATE["error"]
        if pipeline is None or not isinstance(engine, str) or not isinstance(sample_rate, int):
            self._json(503, {"error": error or "pipeline still loading"})
            return None
        return pipeline, engine, sample_rate

    def synthesize(self, parsed: Any) -> None:
        req = self.read_request()
        if req is None:
            return
        current = self.current_pipeline()
        if current is None:
            return
        pipeline, engine, sample_rate = current
        fmt = parse_qs(parsed.query).get("format", ["wav"])[0].lower()
        voice = req.get("voice") if isinstance(req.get("voice"), str) else VOICE
        speed = float(req.get("speed") or 1.0)
        text = str(req.get("text") or "").strip()

        try:
            started = time.monotonic()
            chunks = list(render_pcm_chunks(engine, pipeline, text, streaming=False, voice=voice, speed=speed))
            import numpy as np

            pcm = np.concatenate(chunks) if chunks else np.zeros(1, dtype="float32")
            audio, content_type = encode_audio(pcm, sample_rate, fmt)
            elapsed_ms = round((time.monotonic() - started) * 1000, 1)
            seconds = len(pcm) / sample_rate
            update_last_timing(elapsed_ms, seconds)
        except Exception as exc:  # noqa: BLE001
            self._json(500, {"error": f"{type(exc).__name__}: {exc}"})
            return

        self.send_response(200)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(audio)))
        self.send_header("x-audio-seconds", f"{seconds:.3f}")
        self.send_header("x-tts-ms", f"{elapsed_ms:.1f}")
        self.send_header("x-voice", voice)
        self.send_header("x-voice-profile", _PROFILE.profile_id if _PROFILE else VOICE)
        self.send_header("x-voice-profile-hash", _PROFILE.profile_hash if _PROFILE else "")
        self.end_headers()
        self.wfile.write(audio)

    def stream(self, _parsed: Any) -> None:
        req = self.read_request()
        if req is None:
            return
        current = self.current_pipeline()
        if current is None:
            return
        pipeline, engine, sample_rate = current
        text = str(req.get("text") or "").strip()
        voice = req.get("voice") if isinstance(req.get("voice"), str) else VOICE
        speed = float(req.get("speed") or 1.0)
        sequence = int(req.get("sequence_start") or 0)
        queue_depth_ms = float(req.get("queue_depth_ms") or 0.0)
        started = time.monotonic()
        total_samples = 0

        self.close_connection = True
        self.send_response(200)
        self.send_header("content-type", "application/x-ndjson")
        self.send_header("cache-control", "no-store")
        self.send_header("connection", "close")
        self.end_headers()
        try:
            for pcm in render_pcm_chunks(engine, pipeline, text, streaming=True, voice=voice, speed=speed):
                frame = pcm_frame(pcm, sample_rate, sequence, started, queue_depth_ms)
                total_samples += int(len(pcm))
                sequence += 1
                self.wfile.write(json.dumps(frame, sort_keys=True).encode("utf-8") + b"\n")
                self.wfile.flush()
            elapsed_ms = round((time.monotonic() - started) * 1000, 1)
            seconds = total_samples / sample_rate if sample_rate else 0.0
            update_last_timing(elapsed_ms, seconds)
            done = {
                "type": "done",
                "sample_rate": sample_rate,
                "sequence": sequence,
                "audio_duration_ms": round(seconds * 1000, 1),
                "tts_elapsed_ms": elapsed_ms,
                "tts_rtf": round((elapsed_ms / 1000) / seconds, 4) if seconds else None,
                "voice_profile": _PROFILE.profile_id if _PROFILE else VOICE,
                "voice_profile_hash": _PROFILE.profile_hash if _PROFILE else "",
                "degraded_active": bool(_STATE["degraded_active"]),
            }
            self.wfile.write(json.dumps(done, sort_keys=True).encode("utf-8") + b"\n")
            self.wfile.flush()
        except Exception as exc:  # noqa: BLE001
            error = {"type": "error", "error": f"{type(exc).__name__}: {exc}"}
            self.wfile.write(json.dumps(error, sort_keys=True).encode("utf-8") + b"\n")
            self.wfile.flush()


def update_last_timing(elapsed_ms: float, seconds: float) -> None:
    with _LOCK:
        _STATE["last_elapsed_ms"] = elapsed_ms
        _STATE["last_audio_seconds"] = round(seconds, 3)
        _STATE["last_rtf"] = round((elapsed_ms / 1000) / seconds, 4) if seconds else None


def main() -> None:
    host, _, port = BIND.partition(":")
    threading.Thread(target=_load_pipeline, daemon=True).start()
    server = ThreadingHTTPServer((host, int(port)), Handler)
    print(f"[tts] sidecar listening on {BIND} (engine={ENGINE}, voice={VOICE})", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()

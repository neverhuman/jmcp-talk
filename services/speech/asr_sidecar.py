#!/usr/bin/env python3
"""JMCP ASR sidecar: realtime speech-to-text on the local GPU.

A dependency-light HTTP sidecar (stdlib server + faster-whisper/CTranslate2)
that the Rust speech adapter and Cockpit voice loop call. The heavy CUDA/ML
stack stays out of the Rust runtime.

Endpoints:
  GET  /health      -> {ok, model, device, compute_type, beam_size, loaded,
                         warmed, last_elapsed_ms?, error?}
  POST /transcribe  -> raw audio bytes (wav/mp3/flac/ogg, decoded by PyAV)
                       query: ?language=en&beam_size=1
                       200 -> {text, language, confidence, duration,
                               elapsed_ms, rtf, segments:[...]}
"""
import json
import math
import os
import tempfile
import threading
import time
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse


def _env_int(key, default):
    try:
        return int(os.environ.get(key, str(default)))
    except ValueError:
        print(f"[asr] invalid {key}; using {default}", flush=True)
        return default


MODEL_ID = os.environ.get("ASR_MODEL", "distil-small.en")
DEVICE = os.environ.get("ASR_DEVICE", "cuda")
COMPUTE = os.environ.get("ASR_COMPUTE", "float16")
BIND = os.environ.get("ASR_BIND", "127.0.0.1:18878")
DEFAULT_BEAM_SIZE = _env_int("ASR_BEAM_SIZE", 1)

_STATE = {
    "model": None,
    "loaded": False,
    "warmed": False,
    "error": None,
    "warm_error": None,
    "last_elapsed_ms": None,
    "last_warmup_ms": None,
}
_LOCK = threading.Lock()


def _warm_model(model):
    tmp = tempfile.NamedTemporaryFile(suffix=".wav", delete=False)
    path = tmp.name
    tmp.close()
    started = time.monotonic()
    try:
        with wave.open(path, "wb") as wav:
            wav.setnchannels(1)
            wav.setsampwidth(2)
            wav.setframerate(16000)
            wav.writeframes(b"\0\0" * 4000)
        segments, _ = model.transcribe(path, language="en", beam_size=DEFAULT_BEAM_SIZE)
        for _ in segments:
            pass
        return round((time.monotonic() - started) * 1000, 1)
    finally:
        os.unlink(path)


def _load_model():
    try:
        from faster_whisper import WhisperModel

        model = WhisperModel(MODEL_ID, device=DEVICE, compute_type=COMPUTE)
        warmed = False
        warm_error = None
        warmup_ms = None
        try:
            warmup_ms = _warm_model(model)
            warmed = True
        except Exception as exc:  # noqa: BLE001 - warmup is advisory
            warm_error = f"{type(exc).__name__}: {exc}"
        with _LOCK:
            _STATE["model"] = model
            _STATE["loaded"] = True
            _STATE["warmed"] = warmed
            _STATE["warm_error"] = warm_error
            _STATE["last_warmup_ms"] = warmup_ms
        warm_status = f", warmup={warmup_ms}ms" if warmed else f", warmup failed: {warm_error}"
        print(
            f"[asr] model '{MODEL_ID}' loaded on {DEVICE}/{COMPUTE}"
            f" (beam={DEFAULT_BEAM_SIZE}{warm_status})",
            flush=True,
        )
    except Exception as exc:  # noqa: BLE001 - report load failures over /health
        with _LOCK:
            _STATE["error"] = f"{type(exc).__name__}: {exc}"
        print(f"[asr] model load FAILED: {_STATE['error']}", flush=True)


class Handler(BaseHTTPRequestHandler):
    def _json(self, code, payload, headers=None):
        body = json.dumps(payload).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass

    def do_GET(self):
        if urlparse(self.path).path == "/health":
            with _LOCK:
                payload = {
                    "ok": _STATE["error"] is None,
                    "model": MODEL_ID,
                    "device": DEVICE,
                    "compute_type": COMPUTE,
                    "beam_size": DEFAULT_BEAM_SIZE,
                    "loaded": _STATE["loaded"],
                    "warmed": _STATE["warmed"],
                    "last_elapsed_ms": _STATE["last_elapsed_ms"],
                    "last_warmup_ms": _STATE["last_warmup_ms"],
                    "error": _STATE["error"],
                    "warm_error": _STATE["warm_error"],
                }
            self._json(200, payload)
            return
        self._json(404, {"error": "not found"})

    def do_POST(self):
        parsed = urlparse(self.path)
        if parsed.path != "/transcribe":
            self._json(404, {"error": "not found"})
            return
        with _LOCK:
            model = _STATE["model"]
            error = _STATE["error"]
        if model is None:
            self._json(503, {"error": error or "model still loading"})
            return

        length = int(self.headers.get("content-length", 0))
        if length <= 0:
            self._json(400, {"error": "empty audio body"})
            return
        audio = self.rfile.read(length)
        params = parse_qs(parsed.query)
        language = params.get("language", [None])[0]
        try:
            beam_size = int(params.get("beam_size", [str(DEFAULT_BEAM_SIZE)])[0])
        except ValueError:
            self._json(400, {"error": "beam_size must be an integer"})
            return

        tmp = tempfile.NamedTemporaryFile(suffix=".audio", delete=False)
        try:
            tmp.write(audio)
            tmp.flush()
            tmp.close()
            started = time.monotonic()
            segments, info = model.transcribe(tmp.name, language=language, beam_size=beam_size)
            seg_list = []
            seg_confidences = []
            for segment in segments:
                seg_list.append(
                    {
                        "start": round(segment.start, 3),
                        "end": round(segment.end, 3),
                        "text": segment.text,
                    }
                )
                logprob = getattr(segment, "avg_logprob", None)
                if logprob is not None:
                    seg_confidences.append(min(1.0, math.exp(float(logprob))))
            confidence = (
                round(sum(seg_confidences) / len(seg_confidences), 4)
                if seg_confidences
                else None
            )
            elapsed = time.monotonic() - started
            elapsed_ms = round(elapsed * 1000, 1)
            with _LOCK:
                _STATE["last_elapsed_ms"] = elapsed_ms
            text = "".join(segment["text"] for segment in seg_list).strip()
            duration = float(getattr(info, "duration", 0.0) or 0.0)
            self._json(
                200,
                {
                    "text": text,
                    "language": info.language,
                    "language_probability": round(float(info.language_probability), 4),
                    "confidence": confidence,
                    "duration": round(duration, 3),
                    "elapsed_ms": elapsed_ms,
                    "rtf": round(elapsed / duration, 4) if duration else None,
                    "segments": seg_list,
                },
                headers={"x-asr-ms": f"{elapsed_ms:.1f}"},
            )
        except Exception as exc:  # noqa: BLE001
            self._json(500, {"error": f"{type(exc).__name__}: {exc}"})
        finally:
            os.unlink(tmp.name)


def main():
    host, _, port = BIND.partition(":")
    threading.Thread(target=_load_model, daemon=True).start()
    server = ThreadingHTTPServer((host, int(port)), Handler)
    print(f"[asr] sidecar listening on {BIND} (model={MODEL_ID})", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()

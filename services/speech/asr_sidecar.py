#!/usr/bin/env python3
"""JMCP ASR sidecar — master-grade speech-to-text on the local GPU.

A dependency-light HTTP sidecar (stdlib server + faster-whisper/CTranslate2) that
the Rust `jmcp-adapter-asr` client calls, mirroring how `jmcp-adapter-jekko`
shells out to a separate engine. The heavy CUDA/ML stack stays OUT of the Rust
process; this owns it.

Endpoints
  GET  /health      -> {ok, model, device, compute_type, loaded, error?}
  POST /transcribe  -> body = raw audio bytes (wav/mp3/flac/ogg, decoded by PyAV)
                       query: ?language=en (optional), ?beam_size=5 (optional)
                       200  -> {text, language, language_probability, duration,
                                rtf, segments:[{start,end,text}]}

Config (env): ASR_MODEL=large-v3  ASR_DEVICE=cuda  ASR_COMPUTE=float16
              ASR_BIND=127.0.0.1:18878  (must avoid Jeryu-protected ports)

License note: faster-whisper is MIT; Whisper weights are MIT (OpenAI). Swap
ASR_MODEL to nvidia/canary-* or parakeet via the local-speech inventory microtask
once benchmarked — this server is model-agnostic over faster-whisper-loadable ids.
"""
import json
import math
import os
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

MODEL_ID = os.environ.get("ASR_MODEL", "large-v3")
DEVICE = os.environ.get("ASR_DEVICE", "cuda")
COMPUTE = os.environ.get("ASR_COMPUTE", "float16")
BIND = os.environ.get("ASR_BIND", "127.0.0.1:18878")

# Loaded lazily in a background thread so the server binds immediately and
# /health can report progress while the (one-time) weight download runs.
_STATE = {"model": None, "loaded": False, "error": None}
_LOCK = threading.Lock()


def _load_model():
    try:
        from faster_whisper import WhisperModel

        model = WhisperModel(MODEL_ID, device=DEVICE, compute_type=COMPUTE)
        with _LOCK:
            _STATE["model"] = model
            _STATE["loaded"] = True
        print(f"[asr] model '{MODEL_ID}' loaded on {DEVICE}/{COMPUTE}", flush=True)
    except Exception as exc:  # noqa: BLE001 - report any load failure over /health
        with _LOCK:
            _STATE["error"] = f"{type(exc).__name__}: {exc}"
        print(f"[asr] model load FAILED: {_STATE['error']}", flush=True)


class Handler(BaseHTTPRequestHandler):
    def _json(self, code, payload):
        body = json.dumps(payload).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):  # quiet default logging
        pass

    def do_GET(self):
        if urlparse(self.path).path == "/health":
            with _LOCK:
                self._json(
                    200,
                    {
                        "ok": _STATE["error"] is None,
                        "model": MODEL_ID,
                        "device": DEVICE,
                        "compute_type": COMPUTE,
                        "loaded": _STATE["loaded"],
                        "error": _STATE["error"],
                    },
                )
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
        beam_size = int(params.get("beam_size", ["5"])[0])

        tmp = tempfile.NamedTemporaryFile(suffix=".audio", delete=False)
        try:
            tmp.write(audio)
            tmp.flush()
            tmp.close()
            started = time.monotonic()
            segments, info = model.transcribe(
                tmp.name, language=language, beam_size=beam_size
            )
            seg_list = []
            seg_confidences = []
            for s in segments:
                seg_list.append(
                    {"start": round(s.start, 3), "end": round(s.end, 3), "text": s.text}
                )
                logprob = getattr(s, "avg_logprob", None)
                if logprob is not None:
                    # exp(avg token log-prob) -> a per-segment confidence in 0..1.
                    seg_confidences.append(min(1.0, math.exp(float(logprob))))
            # Overall recognizer confidence drives the voice-approval threshold.
            confidence = (
                round(sum(seg_confidences) / len(seg_confidences), 4)
                if seg_confidences
                else None
            )
            elapsed = time.monotonic() - started
            text = "".join(s["text"] for s in seg_list).strip()
            duration = float(getattr(info, "duration", 0.0) or 0.0)
            self._json(
                200,
                {
                    "text": text,
                    "language": info.language,
                    "language_probability": round(float(info.language_probability), 4),
                    "confidence": confidence,
                    "duration": round(duration, 3),
                    "rtf": round(elapsed / duration, 4) if duration else None,
                    "segments": seg_list,
                },
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

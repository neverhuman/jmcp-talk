#!/usr/bin/env python3
"""JMCP TTS sidecar — great text-to-speech on the local GPU/CPU.

A dependency-light HTTP sidecar (stdlib server + Kokoro-82M) that the Rust
`jmcp-adapter-tts` client calls. Kokoro is Apache-2.0 (weights + code), so it
ships commercially clean — unlike XTTS-v2 (non-commercial CPML). Phonemization
uses the bundled espeak-ng via espeakng-loader, so no system package is needed.

Endpoints
  GET  /health      -> {ok, model, device, loaded, warmed, voice, sample_rate,
                         last_elapsed_ms?, error?}
  POST /synthesize  -> body JSON {text, voice?, speed?}
                       200 -> audio/wav bytes (24 kHz, PCM_16)
                       header: x-tts-ms

Config (env): TTS_VOICE=af_heart  TTS_LANG=a (American English)  TTS_DEVICE=auto
              TTS_BIND=127.0.0.1:18901  (a JMCP-safe port)

License note: Kokoro-82M is Apache-2.0. Cloned-voice models are out of scope
here; any future voice cloning requires recorded consent Evidence.
"""
import io
import json
import os
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

VOICE = os.environ.get("TTS_VOICE", "af_heart")
LANG = os.environ.get("TTS_LANG", "a")
DEVICE = os.environ.get("TTS_DEVICE", "auto")
BIND = os.environ.get("TTS_BIND", "127.0.0.1:18901")
SAMPLE_RATE = 24000

_STATE = {
    "pipeline": None,
    "loaded": False,
    "warmed": False,
    "error": None,
    "warm_error": None,
    "device": None,
    "last_elapsed_ms": None,
    "last_warmup_ms": None,
}
_LOCK = threading.Lock()


def _resolve_device():
    if DEVICE != "auto":
        return DEVICE
    try:
        import torch

        return "cuda" if torch.cuda.is_available() else "cpu"
    except Exception:  # noqa: BLE001
        return "cpu"


def _load_pipeline():
    try:
        from kokoro import KPipeline

        device = _resolve_device()
        pipeline = KPipeline(lang_code=LANG, device=device)
        warmed = False
        warm_error = None
        warmup_ms = None
        try:
            started = time.monotonic()
            _render_audio(pipeline, "Ready.", VOICE, 1.0, "wav")
            warmup_ms = round((time.monotonic() - started) * 1000, 1)
            warmed = True
        except Exception as exc:  # noqa: BLE001 - warmup is advisory; serving can proceed
            warm_error = f"{type(exc).__name__}: {exc}"
        with _LOCK:
            _STATE["pipeline"] = pipeline
            _STATE["device"] = device
            _STATE["loaded"] = True
            _STATE["warmed"] = warmed
            _STATE["warm_error"] = warm_error
            _STATE["last_warmup_ms"] = warmup_ms
        warm_status = f", warmup={warmup_ms}ms" if warmed else f", warmup failed: {warm_error}"
        print(f"[tts] Kokoro loaded on {device} (voice={VOICE}{warm_status})", flush=True)
    except Exception as exc:  # noqa: BLE001
        with _LOCK:
            _STATE["error"] = f"{type(exc).__name__}: {exc}"
        print(f"[tts] pipeline load FAILED: {_STATE['error']}", flush=True)


def _to_numpy(audio):
    try:
        import numpy as np

        if hasattr(audio, "detach"):
            audio = audio.detach().cpu().numpy()
        return np.asarray(audio, dtype="float32")
    except Exception:  # noqa: BLE001
        import numpy as np

        return np.asarray(audio, dtype="float32")


# Output formats: WAV (default) and OGG/Opus (what Telegram voice notes require).
_FORMATS = {
    "wav": ("WAV", "PCM_16", "audio/wav"),
    "ogg": ("OGG", "OPUS", "audio/ogg"),
    "opus": ("OGG", "OPUS", "audio/ogg"),
}


def _render_audio(pipeline, text, voice, speed, fmt):
    import numpy as np
    import soundfile as sf

    sf_format, subtype, content_type = _FORMATS.get(fmt, _FORMATS["wav"])
    chunks = [
        _to_numpy(audio) for _, _, audio in pipeline(text, voice=voice, speed=speed)
    ]
    wav = np.concatenate(chunks) if chunks else np.zeros(1, dtype="float32")
    buf = io.BytesIO()
    sf.write(buf, wav, SAMPLE_RATE, format=sf_format, subtype=subtype)
    return buf.getvalue(), len(wav) / SAMPLE_RATE, content_type


def _synthesize(text, voice, speed, fmt):
    with _LOCK:
        pipeline = _STATE["pipeline"]
    return _render_audio(pipeline, text, voice, speed, fmt)


class Handler(BaseHTTPRequestHandler):
    def _json(self, code, payload):
        body = json.dumps(payload).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass

    def do_GET(self):
        if urlparse(self.path).path == "/health":
            with _LOCK:
                payload = {
                    "ok": _STATE["error"] is None,
                    "model": "kokoro-82M",
                    "device": _STATE["device"],
                    "loaded": _STATE["loaded"],
                    "warmed": _STATE["warmed"],
                    "voice": VOICE,
                    "sample_rate": SAMPLE_RATE,
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
        if parsed.path != "/synthesize":
            self._json(404, {"error": "not found"})
            return
        fmt = parse_qs(parsed.query).get("format", ["wav"])[0].lower()
        with _LOCK:
            pipeline = _STATE["pipeline"]
            error = _STATE["error"]
        if pipeline is None:
            self._json(503, {"error": error or "pipeline still loading"})
            return

        length = int(self.headers.get("content-length", 0))
        raw = self.rfile.read(length) if length > 0 else b"{}"
        try:
            req = json.loads(raw or b"{}")
        except json.JSONDecodeError:
            self._json(400, {"error": "body must be JSON {text, voice?, speed?}"})
            return
        text = (req.get("text") or "").strip()
        if not text:
            self._json(400, {"error": "missing 'text'"})
            return
        voice = req.get("voice") or VOICE
        speed = float(req.get("speed") or 1.0)

        try:
            started = time.monotonic()
            audio, seconds, content_type = _synthesize(text, voice, speed, fmt)
            elapsed_ms = round((time.monotonic() - started) * 1000, 1)
            with _LOCK:
                _STATE["last_elapsed_ms"] = elapsed_ms
        except Exception as exc:  # noqa: BLE001
            self._json(500, {"error": f"{type(exc).__name__}: {exc}"})
            return
        self.send_response(200)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(audio)))
        self.send_header("x-audio-seconds", f"{seconds:.3f}")
        self.send_header("x-tts-ms", f"{elapsed_ms:.1f}")
        self.send_header("x-voice", voice)
        self.end_headers()
        self.wfile.write(audio)


def main():
    host, _, port = BIND.partition(":")
    threading.Thread(target=_load_pipeline, daemon=True).start()
    server = ThreadingHTTPServer((host, int(port)), Handler)
    print(f"[tts] sidecar listening on {BIND} (voice={VOICE})", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()

import json
import math
import sys
import unittest
from unittest import mock
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from voice_smoke_common import (  # noqa: E402
    TimedFrame,
    audio_quality_metrics,
    float32_base64_to_samples,
    has_raw_audio_keys,
    parse_json_frame,
    read_json,
    resample_linear,
    samples_to_f32le_base64,
    stream_frame_metrics,
    strip_audio_data,
    validate_real_tts_health,
    wav_bytes_from_samples,
)


class VoiceSmokeCommonTest(unittest.TestCase):
    def test_audio_metrics_detect_silence_rms_and_clipping(self) -> None:
        samples = [0.0, 0.0, 0.2, 0.2, 0.0, 0.0, -1.0, 0.0, 0.0]
        metrics = audio_quality_metrics(samples, 1000, "hello local voice")

        self.assertEqual(metrics["sample_count"], len(samples))
        self.assertEqual(metrics["clipping_count"], 1)
        self.assertEqual(metrics["leading_silence_ms"], 2.0)
        self.assertEqual(metrics["trailing_silence_ms"], 2.0)
        self.assertEqual(metrics["max_internal_silence_gap_ms"], 2.0)
        self.assertGreater(metrics["rms"], 0)
        self.assertEqual(metrics["peak"], 1.0)
        self.assertEqual(metrics["word_count"], 3)

    def test_sine_wave_has_stable_nonzero_metrics(self) -> None:
        samples = [0.25 * math.sin(2 * math.pi * index / 20) for index in range(1000)]
        metrics = audio_quality_metrics(samples, 1000, "one two three four")

        self.assertEqual(metrics["audio_duration_ms"], 1000.0)
        self.assertAlmostEqual(metrics["peak"], 0.25, places=5)
        self.assertEqual(metrics["clipping_count"], 0)
        self.assertGreater(metrics["words_per_minute"], 0)

    def test_stream_metrics_track_timing_sequences_and_rtf(self) -> None:
        first = samples_to_f32le_base64([0.1] * 100)
        second = samples_to_f32le_base64([0.2] * 100)
        frames = [
            TimedFrame(120.0, {"type": "audio", "sample_rate": 1000, "sequence": 0, "audio_data": first}),
            TimedFrame(170.0, {"type": "audio", "sample_rate": 1000, "sequence": 2, "audio_data": second}),
            TimedFrame(220.0, {"type": "done", "audio_duration_ms": 200.0, "tts_elapsed_ms": 100.0, "tts_rtf": 0.5}),
        ]
        samples = [*float32_base64_to_samples(first), *float32_base64_to_samples(second)]

        metrics = stream_frame_metrics(frames, samples, 1000, 220.0, "hello voice")

        self.assertEqual(metrics["first_frame_latency_ms"], 120.0)
        self.assertEqual(metrics["missing_audio_frames"], 1)
        self.assertTrue(metrics["sequence_monotonic"])
        self.assertEqual(metrics["frame_cadence_ms"]["p50"], 50.0)
        self.assertEqual(metrics["rtf"], 0.5)

    def test_stream_metrics_detect_non_monotonic_sequence_and_error(self) -> None:
        audio = samples_to_f32le_base64([0.1] * 10)
        frames = [
            TimedFrame(20.0, {"type": "audio", "sample_rate": 1000, "sequence": 1, "audio_data": audio}),
            TimedFrame(25.0, {"type": "audio", "sample_rate": 1000, "sequence": 1, "audio_data": audio}),
            TimedFrame(30.0, {"type": "error", "error": "bad"}),
        ]
        samples = [*float32_base64_to_samples(audio), *float32_base64_to_samples(audio)]

        metrics = stream_frame_metrics(frames, samples, 1000, 30.0, "bad")

        self.assertFalse(metrics["sequence_monotonic"])
        self.assertEqual(metrics["stream_error_count"], 1)

    def test_json_frame_parsing_and_audio_stripping(self) -> None:
        frame = parse_json_frame(json.dumps({"type": "audio", "audio_data": "abc", "sequence": 1}))

        self.assertIsNotNone(frame)
        self.assertEqual(frame["type"], "audio")
        self.assertEqual(strip_audio_data(frame), {"type": "audio", "sequence": 1})
        self.assertIsNone(parse_json_frame("not json"))

    def test_read_json_accepts_empty_healthy_response(self) -> None:
        class EmptyResponse:
            status = 200

            def __enter__(self) -> "EmptyResponse":
                return self

            def __exit__(self, *_args: object) -> None:
                return None

            def read(self) -> bytes:
                return b""

        with mock.patch("urllib.request.urlopen", return_value=EmptyResponse()):
            response = read_json("http://127.0.0.1:18902/health")

        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.payload, {"ok": True})
        self.assertIsNone(response.error)

    def test_wav_and_resample_helpers(self) -> None:
        samples = [0.0, 0.5, -0.5, 0.25]
        wav = wav_bytes_from_samples(samples, 16000)
        resampled = resample_linear(samples, 16000, 8000)

        self.assertTrue(wav.startswith(b"RIFF"))
        self.assertIn(b"WAVEfmt ", wav[:20])
        self.assertEqual(len(wav), 44 + len(samples) * 2)
        self.assertEqual(len(resampled), 2)

    def test_real_tts_health_contract(self) -> None:
        health = {
            "ok": True,
            "active_engine": "voxcpm2",
            "degraded_active": False,
            "voice_profile": "jmcp_male_v1",
            "voice_profile_hash": "hash",
            "sample_rate": 48000,
            "loaded": True,
            "warmed": True,
        }

        self.assertEqual(validate_real_tts_health(health, 200), [])

        broken = dict(health)
        broken["degraded_active"] = True
        self.assertTrue(validate_real_tts_health(broken, 200))

    def test_raw_audio_key_detection(self) -> None:
        self.assertEqual(has_raw_audio_keys({"audio": {"outputHash": "sha"}}), [])
        self.assertEqual(has_raw_audio_keys({"audio_data": "base64"}), ["audio_data"])


if __name__ == "__main__":
    unittest.main()

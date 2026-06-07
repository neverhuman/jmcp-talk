import argparse
import base64
import json
import struct
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

import voice_gateway  # noqa: E402


class FakeResponse:
    def __init__(self, payload: dict[str, object], status_code: int = 200) -> None:
        self.payload = payload
        self.status_code = status_code

    def json(self) -> dict[str, object]:
        return self.payload


class FakeHttp:
    async def get(self, url: str) -> FakeResponse:
        if url.endswith("/health") and ":18901" in url:
            return FakeResponse(
                {
                    "ok": True,
                    "voice_engine": "voxcpm2",
                    "voice_profile": "jmcp_male_v1",
                    "voice_profile_hash": "profile_hash",
                    "sample_rate": 48000,
                    "degraded_active": False,
                },
            )
        if url.endswith("/health") and ":18878" in url:
            return FakeResponse({"ok": True, "model": "distil-small.en"})
        return FakeResponse({"ok": True, "status": "running"})


class VoiceGatewayContractTest(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.tmpdir = tempfile.TemporaryDirectory()
        self.tmp = self.tmpdir.name

    async def asyncTearDown(self) -> None:
        self.tmpdir.cleanup()

    def build_gateway(self) -> voice_gateway.VoiceGateway:
        profile_path = ROOT.parent / "speech" / "voice_profiles" / "jmcp_male_v1.json"
        args = argparse.Namespace(
            bind="127.0.0.1:8040",
            llm_upstream="http://127.0.0.1:18902/v1",
            llm_model="local/qwen3-30b-a3b",
            asr_upstream="http://127.0.0.1:18878",
            tts_upstream="http://127.0.0.1:18901",
            jmcp_core="http://127.0.0.1:18877",
            voice_profile="jmcp_male_v1",
            voice_profile_path=str(profile_path),
            event_log=str(Path(self.tmp) / "events.jsonl"),
            audio_dir=str(Path(self.tmp) / "audio"),
        )
        gateway = voice_gateway.VoiceGateway(voice_gateway.build_config(args))
        gateway.http = FakeHttp()  # type: ignore[assignment]
        return gateway

    async def test_health_exposes_voice_contract(self) -> None:
        gateway = self.build_gateway()

        response = await gateway.health()
        payload = json.loads(response.body)

        self.assertTrue(payload["ok"])
        self.assertEqual(payload["voice_engine"], "voxcpm2")
        self.assertEqual(payload["voice_profile"], "jmcp_male_v1")
        self.assertEqual(payload["voice_profile_hash"], "profile_hash")
        self.assertEqual(payload["sample_rate"], 48000)
        self.assertEqual(payload["audio_format"], "f32le")
        self.assertTrue(payload["streaming_audio"])
        self.assertTrue(payload["local_only"])

    async def test_events_preserve_playback_timing_fields(self) -> None:
        gateway = self.build_gateway()

        gateway.write_event(
            {
                "turn_id": "turn_test",
                "event": "voice.playback_underrun",
                "stage": "browser",
                "status": "error",
                "sample_rate": 48000,
                "sequence": 3,
                "duration_ms": 25.5,
                "tts_elapsed_ms": 180.0,
                "queue_depth_ms": 40.0,
                "voice_profile_hash": "profile_hash",
            },
        )

        event_log = Path(self.tmp) / "events.jsonl"
        event = json.loads(event_log.read_text(encoding="utf-8").strip())
        self.assertEqual(event["event"], "voice.playback_underrun")
        self.assertEqual(event["sequence"], 3)
        self.assertEqual(event["queue_depth_ms"], 40.0)
        self.assertEqual(event["voice_profile_hash"], "profile_hash")


class VoiceGatewayPureFunctionTest(unittest.TestCase):
    def test_split_tts_segments_waits_for_useful_phrase(self) -> None:
        segments, rest = voice_gateway.split_tts_segments("Short. Still building", 12)
        self.assertEqual(segments, [])
        self.assertEqual(rest, "Short. Still building")

        segments, rest = voice_gateway.split_tts_segments(
            "This is a complete spoken phrase. Next bit",
            12,
        )
        self.assertEqual(segments, ["This is a complete spoken phrase."])
        self.assertEqual(rest, "Next bit")

    def test_float32_base64_audio_becomes_wav(self) -> None:
        raw = b"".join(struct.pack("<f", sample) for sample in [0.0, 0.5, -0.5])
        wav = voice_gateway.wav_bytes_from_float32_base64(base64.b64encode(raw).decode("ascii"), 16000)

        self.assertTrue(wav.startswith(b"RIFF"))
        self.assertIn(b"WAVEfmt ", wav[:20])
        self.assertEqual(len(wav), 44 + 3 * 2)


if __name__ == "__main__":
    unittest.main()

import base64
import hashlib
import json
import sys
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

import tts_sidecar  # noqa: E402


class TtsSidecarContractTest(unittest.TestCase):
    def test_jmcp_male_profile_is_not_minicpm_demo_ref(self) -> None:
        profile = tts_sidecar.load_voice_profile(ROOT / "voice_profiles" / "jmcp_male_v1.json")
        minicpm_ref = ROOT.parent / "llm" / "assets" / "ref_audio" / "jmcp_friendly_male_16k.wav"

        self.assertEqual(profile.profile_id, "jmcp_male_v1")
        self.assertEqual(profile.engine, "openbmb/VoxCPM2")
        self.assertEqual(profile.sample_rate, 48000)
        if minicpm_ref.is_file():
            ref_hash = hashlib.sha256(minicpm_ref.read_bytes()).hexdigest()
            self.assertNotEqual(profile.profile_hash, ref_hash)

    def test_pcm_frame_reports_streaming_audio_metadata(self) -> None:
        frame = tts_sidecar.pcm_frame([0.0, 0.5, -0.5, 0.0], 48000, 7, time.monotonic())

        self.assertEqual(frame["type"], "audio")
        self.assertEqual(frame["audio_format"], "f32le")
        self.assertEqual(frame["sample_rate"], 48000)
        self.assertEqual(frame["sequence"], 7)
        self.assertGreater(frame["duration_ms"], 0)
        decoded = base64.b64decode(frame["audio_data"])
        self.assertEqual(len(decoded), 16)

    def test_profile_hash_is_canonical_json(self) -> None:
        profile_path = ROOT / "voice_profiles" / "jmcp_male_v1.json"
        data = json.loads(profile_path.read_text(encoding="utf-8"))
        expected = hashlib.sha256(
            json.dumps(data, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8"),
        ).hexdigest()

        self.assertEqual(tts_sidecar.load_voice_profile(profile_path).profile_hash, expected)

    def test_streaming_silence_shaper_caps_pause_runs(self) -> None:
        import numpy as np

        chunks = [
            np.array([0.0] * 100 + [0.02] * 10 + [0.0] * 60, dtype="float32"),
            np.array([0.0] * 140 + [0.03] * 10 + [0.0] * 100, dtype="float32"),
        ]

        shaped = list(tts_sidecar.shaped_pcm_chunks(chunks, sample_rate=1000))
        samples = np.concatenate(shaped)

        self.assertEqual(len(samples), 300)
        self.assertTrue(np.all(np.abs(samples[:80]) < tts_sidecar.SILENCE_THRESHOLD))
        self.assertTrue(np.all(np.abs(samples[80:90]) >= tts_sidecar.SILENCE_THRESHOLD))
        self.assertTrue(np.all(np.abs(samples[90:210]) < tts_sidecar.SILENCE_THRESHOLD))
        self.assertTrue(np.all(np.abs(samples[210:220]) >= tts_sidecar.SILENCE_THRESHOLD))
        self.assertTrue(np.all(np.abs(samples[220:]) < tts_sidecar.SILENCE_THRESHOLD))

    def test_voxcpm_max_len_uses_spoken_text_not_profile_prompt(self) -> None:
        short_cap = tts_sidecar.voxcpm_max_len("Ready.")
        short_floor = tts_sidecar.voxcpm_min_len("Ready.")
        status_cap = tts_sidecar.voxcpm_max_len(
            "Local voice is ready. ASR, reasoning, and VoxCPM2 speech are connected."
        )
        status_floor = tts_sidecar.voxcpm_min_len(
            "Local voice is ready. ASR, reasoning, and VoxCPM2 speech are connected."
        )
        prompted_cap = tts_sidecar.voxcpm_max_len(
            f"({tts_sidecar.load_voice_profile().design_prompt})"
            "Local voice is ready. ASR, reasoning, and VoxCPM2 speech are connected."
        )
        prompted_floor = tts_sidecar.voxcpm_min_len(
            f"({tts_sidecar.load_voice_profile().design_prompt})"
            "Local voice is ready. ASR, reasoning, and VoxCPM2 speech are connected."
        )

        self.assertEqual(short_cap, 6)
        self.assertEqual(short_floor, 2)
        self.assertEqual(status_cap, 33)
        self.assertEqual(status_floor, 22)
        self.assertGreater(prompted_cap, status_cap)
        self.assertGreater(prompted_floor, status_floor)

    def test_voxcpm_render_sends_profile_text_with_spoken_cap(self) -> None:
        import numpy as np

        class FakeVoxCpm:
            kwargs = {}

            def generate_streaming(self, **kwargs):
                self.kwargs = kwargs
                yield np.array([0.02] * 160, dtype="float32")

        pipeline = FakeVoxCpm()
        text = "Local voice is ready. ASR, reasoning, and VoxCPM2 speech are connected."

        list(tts_sidecar.render_pcm_chunks("voxcpm2", pipeline, text, streaming=True, sample_rate=1000))

        self.assertIn("150 to 160 words per minute", pipeline.kwargs["text"])
        self.assertTrue(pipeline.kwargs["text"].endswith(text))
        self.assertEqual(pipeline.kwargs["min_len"], 22)
        self.assertEqual(pipeline.kwargs["max_len"], 33)


if __name__ == "__main__":
    unittest.main()

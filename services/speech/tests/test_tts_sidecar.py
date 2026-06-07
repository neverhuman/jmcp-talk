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


if __name__ == "__main__":
    unittest.main()

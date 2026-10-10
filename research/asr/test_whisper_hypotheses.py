"""Checks for the limits in whisper_hypotheses.py. Run with the same interpreter:

    python research/asr/test_whisper_hypotheses.py
"""

import argparse
import sys
import tempfile
import unittest
import wave
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import whisper_hypotheses as wh  # noqa: E402


def write_silence(path, n_samples):
    with wave.open(str(path), "wb") as f:
        f.setnchannels(1)
        f.setsampwidth(2)
        f.setframerate(wh.SAMPLE_RATE)
        f.writeframes(b"\x00\x00" * n_samples)


class WindowLimit(unittest.TestCase):
    def load(self, n_samples):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "silence.wav"
            write_silence(path, n_samples)
            return wh.load_window(path)

    def test_exactly_one_window_is_accepted(self):
        _, duration_s = self.load(int(wh.MAX_AUDIO_S * wh.SAMPLE_RATE))
        self.assertEqual(duration_s, wh.MAX_AUDIO_S)

    def test_one_sample_over_is_refused(self):
        with self.assertRaises(SystemExit) as ctx:
            self.load(int(wh.MAX_AUDIO_S * wh.SAMPLE_RATE) + 1)
        self.assertIn("MAX_AUDIO_S", str(ctx.exception))


class SampleModeArguments(unittest.TestCase):
    def test_zero_temperature_is_refused(self):
        args = argparse.Namespace(
            mode="sample", audio="x.wav", model="base", language="ru",
            temperature=0.0, seed=1, best_of=5, out="x.json",
        )
        with self.assertRaises(SystemExit):
            wh.run(args)


if __name__ == "__main__":
    unittest.main()

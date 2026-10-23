"""Checks for whisper_nbest.py that need no model:

    python research/asr/test_whisper_nbest.py
"""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import whisper_nbest as nb  # noqa: E402


class Plan(unittest.TestCase):
    def test_sealed_k8(self):
        runs = nb.plan(8)
        self.assertEqual(runs[0], ("greedy", 0.0, None, None))
        self.assertEqual([r[3] for r in runs[1:]], [1, 2, 3, 4, 5, 6, 7])
        self.assertTrue(all(r[1] == 0.6 and r[2] == 1 for r in runs[1:]))

    def test_smaller_k_are_prefixes(self):
        for k in (1, 2, 4):
            self.assertEqual(nb.plan(k), nb.plan(8)[:k])

    def test_bad_k(self):
        with self.assertRaises(SystemExit):
            nb.plan(3)


class Guard(unittest.TestCase):
    def test_refuses_test_voice(self):
        with self.assertRaises(SystemExit):
            nb.refuse_test_voice("/tmp/x/tts-dmitri/s0/a.wav")

    def test_allows_train(self):
        nb.refuse_test_voice("/tmp/x/tts-denis/s0/a.wav")


class OutputNames(unittest.TestCase):
    def test_same_stem_different_stage(self):
        names = nb.output_names(["/r/tts-denis/s0/a.wav", "/r/tts-denis/s1/a.wav"])
        self.assertEqual(names, ["s0__a.nbest.json", "s1__a.nbest.json"])

    def test_single_input(self):
        self.assertEqual(nb.output_names(["/r/s0/a.wav"]), ["a.nbest.json"])

    def test_collision_fails(self):
        with self.assertRaises(SystemExit):
            nb.output_names(["/r/s0/a.wav", "/r/s0/a.flac"])


class Canonical(unittest.TestCase):
    def test_order_independent(self):
        self.assertEqual(nb.canonical({"b": 1, "a": 2.5}), nb.canonical({"a": 2.5, "b": 1}))


if __name__ == "__main__":
    unittest.main()

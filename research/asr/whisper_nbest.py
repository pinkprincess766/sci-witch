#!/usr/bin/env python3
"""Reproducible Whisper N-best list for question 1, with an on-disk cache.

Research tool, not the product path. Builds the K-hypothesis list exactly as
sealed in research/protocol/question-1-nbest.md ("Запечатано"):

- hypothesis 1: greedy, T = 0, beam_size = None, best_of = None;
- hypotheses 2..K: `sample`, T = 0.6, best_of = 1, one run per seed 1..K-1,
  in seed order; identical texts are not collapsed.

Each hypothesis is scored with whisper_hypotheses.build_hypothesis (teacher-
forced sum of text-token + EOT log-probs at T = 1). The model is loaded once.

Cache: one JSON file per (audio, model, mode, T, seed, code, versions) key under
--cache-dir. A cache hit returns the stored bytes; --no-cache recomputes and
does not read the cache (use it to check that recomputation is byte-identical).

Output is canonical JSON (sorted keys, fixed separators), so two runs on the
same inputs and environment are expected to be byte-identical.

Guard: refuses audio of the test voice (tts-dmitri). No RQ1 scoring here.
"""

import argparse
import hashlib
import json
import platform
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import whisper_hypotheses as wh  # noqa: E402

SCHEMA_VERSION = 1
PROTOCOL = "question-1-v1"
SEALED_TEMPERATURE = 0.6
SEALED_SEEDS = (1, 2, 3, 4, 5, 6, 7)
ALLOWED_K = (1, 2, 4, 8)
TEST_VOICE_MARKERS = ("tts-dmitri", "dmitri")


def canonical(obj):
    return json.dumps(obj, ensure_ascii=False, sort_keys=True, separators=(",", ":"),
                      allow_nan=False) + "\n"


def plan(k):
    """[(mode, temperature, best_of, seed)] for a list of length k."""
    if k not in ALLOWED_K:
        raise SystemExit(f"K must be one of {ALLOWED_K}, got {k}")
    runs = [("greedy", 0.0, None, None)]
    runs += [("sample", SEALED_TEMPERATURE, 1, s) for s in SEALED_SEEDS[: k - 1]]
    return runs


def refuse_test_voice(path):
    lowered = str(Path(path).resolve()).lower()
    if any(m in lowered for m in TEST_VOICE_MARKERS):
        raise SystemExit(f"refusing test-voice audio: {path} (train only, tts-denis)")


def code_sha256():
    here = Path(__file__).resolve().parent
    digest = hashlib.sha256()
    for name in ("whisper_nbest.py", "whisper_hypotheses.py"):
        digest.update(wh.sha256_of(here / name).encode())
    return digest.hexdigest()


def environment(model_name, model_path):
    return {
        "python_version": platform.python_version(),
        "torch_version": wh.torch.__version__,
        "whisper_version": wh.whisper.__version__,
        "torch_num_threads": wh.torch.get_num_threads(),
        "device": wh.DEVICE,
        "fp16": wh.FP16,
        "model": {"name": model_name, "file": model_path.name, "sha256": wh.sha256_of(model_path)},
        "code_sha256": code_sha256(),
    }


def cache_key(env, audio_sha, language, run):
    mode, temperature, best_of, seed = run
    key = {"env": env, "audio_sha256": audio_sha, "language": language, "mode": mode,
           "temperature": temperature, "best_of": best_of, "beam_size": None, "seed": seed}
    return hashlib.sha256(canonical(key).encode()).hexdigest()


def decode_one(model, mel, language, run):
    mode, temperature, best_of, seed = run
    options = wh.whisper.DecodingOptions(
        language=language, temperature=temperature, beam_size=None, best_of=best_of,
        without_timestamps=True, fp16=wh.FP16,
    )
    if seed is not None:
        wh.torch.manual_seed(seed)
    result = wh.whisper.decode(model, mel, options)
    hyp = wh.build_hypothesis(model, mel, result.language, result)
    hyp.update({"mode": mode, "temperature": temperature, "best_of": best_of,
                "beam_size": None, "seed": seed, "language": result.language})
    return hyp


def nbest(audio, k, model_name, language, cache_dir, use_cache):
    refuse_test_voice(audio)
    wh.torch.use_deterministic_algorithms(True)
    model_path = wh.model_file_path(model_name)
    env = environment(model_name, model_path)
    audio_sha = wh.sha256_of(audio)
    runs = plan(k)
    model = mel = None
    hypotheses = []
    for run in runs:
        key = cache_key(env, audio_sha, language, run)
        path = Path(cache_dir) / key[:2] / f"{key}.json" if cache_dir else None
        if use_cache and path is not None and path.is_file():
            hypotheses.append(json.loads(path.read_text(encoding="utf-8")))
            continue
        if model is None:
            window, _ = wh.load_window(audio)
            model = wh.whisper.load_model(model_name, device=wh.DEVICE,
                                          download_root=str(wh.MODEL_CACHE_DIR))
            mel = wh.whisper.log_mel_spectrogram(window, model.dims.n_mels)
        hyp = decode_one(model, mel, language, run)
        if path is not None and use_cache:
            path.parent.mkdir(parents=True, exist_ok=True)
            tmp = path.with_suffix(".tmp")
            tmp.write_text(canonical(hyp), encoding="utf-8")
            tmp.replace(path)
        hypotheses.append(hyp)
    return {
        "schema_version": SCHEMA_VERSION,
        "protocol": PROTOCOL,
        "k": k,
        "environment": env,
        "audio": {"path": wh.repo_relative(audio), "sha256": audio_sha},
        "language_requested": language,
        "hypotheses": hypotheses,
    }


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("audio", nargs="+", help="train (tts-denis) audio files, each <= 30 s")
    p.add_argument("--k", type=int, default=8)
    p.add_argument("--model", default="base")
    p.add_argument("--language", default=wh.DEFAULT_LANGUAGE)
    p.add_argument("--out-dir", required=True, help="one <audio stem>.nbest.json per input")
    p.add_argument("--cache-dir", default=None)
    p.add_argument("--no-cache", action="store_true", help="recompute; do not read or write cache")
    args = p.parse_args(argv)
    for a in args.audio:
        refuse_test_voice(a)
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    for a in args.audio:
        doc = nbest(Path(a), args.k, args.model, args.language, args.cache_dir, not args.no_cache)
        out = out_dir / f"{Path(a).stem}.nbest.json"
        out.write_text(canonical(doc), encoding="utf-8")
        print(f"{out}: {[h['text'] for h in doc['hypotheses']]!r}", file=sys.stderr)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Whisper hypotheses with per-token log-probabilities, as one JSON file.

Research tool, not the product path. Uses the openai-whisper Python API:
`whisper.decode` on one window of at most 30 s. No temperature fallback, no
beam search, no timestamps. Run it with the interpreter of the Homebrew
openai-whisper package (see research/asr/README.md).

The log-probabilities are ours: each returned hypothesis is teacher-forced
through the model with the same logit filters Whisper applies, at temperature 1
(see docs/research/ASR_SCORES_RU.md, section 1).
"""

import argparse
import hashlib
import json
import math
import os
import platform
import sys
from pathlib import Path

import torch
import whisper
from whisper.audio import CHUNK_LENGTH, N_SAMPLES, SAMPLE_RATE

SCHEMA_VERSION = 1
REPO_ROOT = Path(__file__).resolve().parents[2]

# Longest audio accepted: one Whisper window. Longer audio is refused, not cut.
MAX_AUDIO_S = float(CHUNK_LENGTH)
DEFAULT_LANGUAGE = "ru"
DEFAULT_BEST_OF = 5
DEVICE = "cpu"  # the only device the notes in ASR_SCORES_RU.md were checked on
FP16 = False
MODEL_CACHE_DIR = Path(os.getenv("XDG_CACHE_HOME", Path.home() / ".cache")) / "whisper"


def sha256_of(path):
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def repo_relative(path):
    """Path relative to the repo root; outside the repo only the file name."""
    path = Path(path).resolve()
    if path.is_relative_to(REPO_ROOT):
        return path.relative_to(REPO_ROOT).as_posix()
    return path.name


def model_file_path(model_name):
    """The cached checkpoint. Never downloads: a missing file is an error."""
    if model_name not in whisper._MODELS:
        raise SystemExit(f"unknown model {model_name!r}; known: {whisper.available_models()}")
    path = MODEL_CACHE_DIR / os.path.basename(whisper._MODELS[model_name])
    if not path.is_file():
        raise SystemExit(f"model file not in cache: {path.name} (this script does not download)")
    return path


def load_window(audio_path):
    """Mono 16 kHz samples of at most one window, and the real duration in seconds."""
    audio = whisper.load_audio(str(audio_path))
    duration_s = len(audio) / SAMPLE_RATE
    if len(audio) > N_SAMPLES:
        raise SystemExit(
            f"audio is {duration_s:.2f} s, longer than one window ({MAX_AUDIO_S:.0f} s, MAX_AUDIO_S); "
            "cut it first"
        )
    return whisper.pad_or_trim(audio), duration_s


def token_logprobs(model, mel, language, token_ids):
    """Log-probability of each of `token_ids` plus EOT at T=1 after Whisper's logit filters.

    Returns (list of logprobs for token_ids, logprob of EOT).
    """
    options = whisper.DecodingOptions(
        language=language, temperature=0.0, beam_size=None, best_of=None,
        without_timestamps=True, fp16=FP16,
    )
    task = whisper.decoding.DecodingTask(model, options)
    prompt = list(task.initial_tokens)
    sequence = prompt + list(token_ids) + [task.tokenizer.eot]
    tokens = torch.tensor([sequence])
    with torch.no_grad():
        audio_features = task._get_audio_features(mel.unsqueeze(0))
        all_logits = model.decoder(tokens, audio_features)
        logprobs = []
        for step in range(len(token_ids) + 1):
            position = len(prompt) - 1 + step
            logits = all_logits[:, position].clone()
            for logit_filter in task.logit_filters:
                logit_filter.apply(logits, tokens[:, : position + 1])
            log_probs = torch.log_softmax(logits.float(), dim=-1)
            logprobs.append(log_probs[0, sequence[position + 1]].item())
    return logprobs[:-1], logprobs[-1]


def build_hypothesis(model, mel, tokenizer_language, result):
    ids = list(result.tokens)
    id_logprobs, eot_logprob = token_logprobs(model, mel, tokenizer_language, ids)
    tokenizer = whisper.tokenizer.get_tokenizer(
        model.is_multilingual, num_languages=model.num_languages, language=tokenizer_language
    )
    tokens = [
        {"id": i, "text": tokenizer.decode([i]), "logprob": lp}
        for i, lp in zip(ids, id_logprobs)
    ]
    return {
        "text": result.text,
        "tokens": tokens,
        "eot_logprob": eot_logprob,
        "sum_logprob": math.fsum(id_logprobs) + eot_logprob,
        "mean_logprob": math.fsum(id_logprobs) / len(ids) if ids else None,
        "min_logprob": min(id_logprobs) if ids else None,
        "window_avg_logprob": result.avg_logprob,
        "no_speech_prob": result.no_speech_prob,
    }


def run(args):
    audio_path = Path(args.audio)
    mode = args.mode
    temperature = 0.0 if mode == "greedy" else args.temperature
    best_of = None if mode == "greedy" else args.best_of
    seed = None if mode == "greedy" else args.seed
    if mode == "sample" and temperature <= 0.0:
        raise SystemExit("sample mode needs --temperature > 0")
    if mode == "sample" and best_of < 1:
        raise SystemExit("--best-of must be at least 1")

    model_path = model_file_path(args.model)
    audio, duration_s = load_window(audio_path)
    model = whisper.load_model(args.model, device=DEVICE, download_root=str(MODEL_CACHE_DIR))
    mel = whisper.log_mel_spectrogram(audio, model.dims.n_mels)

    options = whisper.DecodingOptions(
        language=args.language, temperature=temperature, beam_size=None, best_of=best_of,
        without_timestamps=True, fp16=FP16,
    )
    if seed is not None:
        torch.manual_seed(seed)
    result = whisper.decode(model, mel, options)

    hypothesis = build_hypothesis(model, mel, result.language, result)
    script_path = Path(__file__).resolve()
    manifest = {
        "script": {"path": repo_relative(script_path), "sha256": sha256_of(script_path)},
        "python_version": platform.python_version(),
        "torch_version": torch.__version__,
        "whisper_version": whisper.__version__,
        "model": {"name": args.model, "file": model_path.name, "sha256": sha256_of(model_path)},
        "mode": mode,
        "temperature": temperature,
        "best_of": best_of,
        "beam_size": None,
        "seed": seed,
        "device": DEVICE,
        "fp16": FP16,
        "language_requested": args.language,
        "audio": {"path": repo_relative(audio_path), "sha256": sha256_of(audio_path)},
        "sample_rate_hz": SAMPLE_RATE,
        "audio_duration_s": duration_s,
    }
    return {
        "schema_version": SCHEMA_VERSION,
        "manifest": manifest,
        "language": result.language,
        "hypotheses": [hypothesis],
    }


def build_parser():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = parser.add_subparsers(dest="mode", required=True)
    for mode in ("greedy", "sample"):
        p = sub.add_parser(mode)
        p.add_argument("audio", help="audio file of at most 30 s")
        p.add_argument("--model", default="base")
        p.add_argument("--language", default=DEFAULT_LANGUAGE, help="Whisper language code")
        p.add_argument("--out", required=True, help="output JSON path")
        if mode == "sample":
            p.add_argument("--temperature", type=float, required=True)
            p.add_argument("--seed", type=int, required=True)
            p.add_argument("--best-of", type=int, default=DEFAULT_BEST_OF)
    return parser


def main(argv=None):
    args = build_parser().parse_args(argv)
    document = run(args)
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(document, f, ensure_ascii=False, indent=2, allow_nan=False)
        f.write("\n")
    hyp = document["hypotheses"][0]
    print(f"{args.out}: {hyp['text']!r} sum_logprob={hyp['sum_logprob']:.6f}", file=sys.stderr)


if __name__ == "__main__":
    main()

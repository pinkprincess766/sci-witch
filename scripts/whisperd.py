#!/usr/bin/env python3
"""Long-lived local Whisper worker. Loads the model once. No network."""

from __future__ import annotations

import json
import os
import sys

os.environ.setdefault("HF_HUB_OFFLINE", "1")
os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")

try:
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
except Exception:
    pass


def fail(msg: str, **extra) -> None:
    rec = {"ok": False, "error": msg}
    rec.update(extra)
    print(json.dumps(rec, ensure_ascii=False), flush=True)


def main() -> int:
    model_name = os.environ.get("SCIWHISPER_MODEL", "base")
    cache = os.path.expanduser("~/.cache/whisper")
    try:
        import whisper
    except ImportError:
        fail("python package 'whisper' is not installed")
        return 1

    names = {
        "turbo": ["large-v3-turbo.pt", "turbo.pt"],
        "large": ["large-v3.pt", "large.pt"],
    }.get(model_name, [f"{model_name}.pt"])
    if not any(os.path.isfile(os.path.join(cache, n)) for n in names):
        fail(f"local model '{model_name}' not found in {cache}")
        return 1

    print(
        json.dumps({"ok": True, "event": "loading", "model": model_name}, ensure_ascii=False),
        flush=True,
    )
    try:
        model = whisper.load_model(model_name, download_root=cache, in_memory=False)
    except Exception as e:
        fail(f"failed to load model: {e}")
        return 1
    print(
        json.dumps({"ok": True, "event": "ready", "model": model_name}, ensure_ascii=False),
        flush=True,
    )

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError as e:
            fail(f"bad json: {e}")
            continue
        cmd = req.get("cmd")
        if cmd == "quit":
            print(json.dumps({"ok": True, "event": "bye"}), flush=True)
            return 0
        if cmd == "ping":
            print(json.dumps({"ok": True, "event": "pong"}), flush=True)
            continue
        if cmd != "transcribe":
            fail(f"unknown cmd {cmd}")
            continue
        path = req.get("path")
        if not path or not os.path.isfile(path):
            fail("audio path missing")
            continue
        language = req.get("language") or None
        prompt = req.get("prompt") or None
        try:
            result = model.transcribe(
                path,
                language=language,
                initial_prompt=prompt,
                temperature=0.0,
                fp16=False,
                condition_on_previous_text=False,
                verbose=False,
            )
        except Exception as e:
            fail(f"transcribe failed: {e}")
            continue
        text = (result.get("text") or "").strip()
        segs = []
        no_speech = not text
        for s in result.get("segments") or []:
            nsp = float(s.get("no_speech_prob") or 0.0)
            segs.append(
                {
                    "text": s.get("text") or "",
                    "start": s.get("start"),
                    "end": s.get("end"),
                    "no_speech_prob": nsp,
                    "avg_logprob": s.get("avg_logprob"),
                }
            )
        if segs and all((s.get("no_speech_prob") or 0) > 0.6 for s in segs) and len(text) < 3:
            no_speech = True
        print(
            json.dumps(
                {
                    "ok": True,
                    "event": "transcript",
                    "text": text,
                    "language": result.get("language"),
                    "no_speech": no_speech,
                    "segments": segs,
                },
                ensure_ascii=False,
            ),
            flush=True,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())

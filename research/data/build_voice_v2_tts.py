#!/usr/bin/env python3
"""Синтез TTS-части корпуса voice-v2 (research/protocol/voice-v2.md).

Только стандартная библиотека Python плюс внешний инструмент Piper
(`piper-tts==1.8.0`, `onnxruntime==1.31.0`). Piper в манифесты репозитория
не входит: его ставят во временное окружение вне проекта. WAV пишутся в
каталог вне репозитория или в `voice-v2-audio/`, который в `.gitignore`.

По умолчанию синтезируется только train (`tts-denis`). Test (`tts-dmitri`)
синтезируется только флагом `--test-final --stop-stage s`, один раз, после
того как ступень стопа s определена на train (правило стопа в протоколе).

Манифест не перезаписывается: новые клипы сливаются с уже записанными по
ключу (голос, ступень, id). `--verify` ничего не пишет в манифест, а
синтезирует заново и сверяет SHA-256 с манифестом.

Пример (пути — только пример):

    /tmp/piper-venv/bin/python research/data/build_voice_v2_tts.py \
        --models /tmp/piper-voices --out /tmp/voice-v2-audio \
        --stages 0,1,2 --manifest research/data/voice-v2-tts.manifest.json
    # проверка детерминизма по всем ступеням train:
    ... --stages 0,1,2,3,4,5,6,7 --verify
    # test, один раз, после стопа на ступени s:
    ... --test-final --stop-stage 3
"""

from __future__ import annotations

import argparse
import array
import hashlib
import io
import platform
import json
import math
import random
import sys
import wave
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FAMILIES = ROOT / "research" / "protocol" / "voice-v1-families.json"

PIPER_VERSION = "1.8.0"
ONNXRUNTIME_VERSION = "1.31.0"
NOISE_SCALE = 0.667
NOISE_W = 0.8
PIPER_VOICES_REVISION = "5041a7a49ae5b3a0f30e6837769eb03fdbb231bd"
TRAIN_VOICE = "ru_RU-denis-medium"
TEST_VOICE = "ru_RU-dmitri-medium"

# Голос -> (блок семейств, split, SHA-256 .onnx, SHA-256 .onnx.json)
VOICES = {
    "ru_RU-denis-medium": (
        "I",
        "train",
        "15fab56e11a097858ee115545d0f697fc2a316c41a291a5362349fb870411b0a",
        "831c860dac0b5073eaa81610a0a638ec23d90a6cf8e5f871b4485c2cec3767c8",
    ),
    "ru_RU-dmitri-medium": (
        "II",
        "dev_holdout",
        "f073356ebc4bd0f80c5af58df2953a5988bd5bdab1eb38635ce960b071fbefcb",
        "667ef3117bc642c2892dff7690d8bdc8ca4228aeaa783b2dc1416df632855e0d",
    ),
}

# Ступень -> (length_scale, SNR в дБ или None). Зафиксировано в протоколе.
STAGES = {
    0: (1.00, None),
    1: (0.80, None),
    2: (1.25, None),
    3: (1.00, 20.0),
    4: (1.00, 10.0),
    5: (0.80, 10.0),
    6: (1.25, 10.0),
    7: (1.00, 5.0),
}


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def clip_seed(*parts: object) -> int:
    key = ":".join(["voice-v2", *map(str, parts)]).encode("utf-8")
    return int.from_bytes(hashlib.sha256(key).digest()[:4], "big")


def add_white_noise(pcm: bytes, snr_db: float, seed: int) -> bytes:
    samples = array.array("h", pcm)
    if sys.byteorder != "little":
        samples.byteswap()
    power = sum(s * s for s in samples) / max(len(samples), 1)
    sigma = math.sqrt(power / (10 ** (snr_db / 10))) if power > 0 else 0.0
    rng = random.Random(seed)
    out = array.array(
        "h",
        (max(-32768, min(32767, round(s + rng.gauss(0.0, sigma)))) for s in samples),
    )
    if sys.byteorder != "little":
        out.byteswap()
    return out.tobytes()


def load_voice(models: Path, name: str, seed: int):
    import onnxruntime  # внешний инструмент, не зависимость репозитория
    from piper import PiperVoice
    from piper.config import PiperConfig

    from importlib.metadata import version

    got = version("piper-tts")
    if got != PIPER_VERSION or onnxruntime.__version__ != ONNXRUNTIME_VERSION:
        sys.exit(
            f"нужны piper-tts {PIPER_VERSION} и onnxruntime {ONNXRUNTIME_VERSION}, "
            f"найдены {got} и {onnxruntime.__version__}"
        )
    _, _, onnx_sha, json_sha = VOICES[name]
    onnx_path = models / f"{name}.onnx"
    json_path = models / f"{name}.onnx.json"
    for path, want in ((onnx_path, onnx_sha), (json_path, json_sha)):
        got_sha = sha256_file(path)
        if got_sha != want:
            sys.exit(f"{path}: SHA-256 {got_sha}, ожидался {want}")
    # Шум Piper (RandomNormalLike внутри графа) берёт seed при создании
    # сессии из глобального генератора onnxruntime. Поэтому seed ставится
    # до создания сессии, а не перед фразой: set_seed после создания на
    # уже созданное ядро не действует (проверено 23.10.2026).
    onnxruntime.set_seed(seed)
    opts = onnxruntime.SessionOptions()
    opts.intra_op_num_threads = 1
    opts.inter_op_num_threads = 1
    opts.execution_mode = onnxruntime.ExecutionMode.ORT_SEQUENTIAL
    session = onnxruntime.InferenceSession(
        str(onnx_path), sess_options=opts, providers=["CPUExecutionProvider"]
    )
    config = PiperConfig.from_dict(json.loads(json_path.read_text(encoding="utf-8")))
    return onnxruntime, PiperVoice(session=session, config=config)


def synthesize(voice, text: str, length_scale: float) -> tuple[bytes, int]:
    from piper import SynthesisConfig

    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        voice.synthesize_wav(
            text,
            w,
            SynthesisConfig(
                noise_scale=NOISE_SCALE, noise_w_scale=NOISE_W, length_scale=length_scale
            ),
        )
    buf.seek(0)
    with wave.open(buf, "rb") as r:
        assert r.getnchannels() == 1 and r.getsampwidth() == 2
        return r.readframes(r.getnframes()), r.getframerate()


def clip_key(c: dict) -> tuple:
    return (c["speaker"], c["stage"], c["id"])


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--models", type=Path, required=True, help="каталог с .onnx и .onnx.json")
    ap.add_argument("--out", type=Path, required=True, help="куда писать WAV (вне git)")
    ap.add_argument("--stages", default="0", help="ступени train через запятую, например 0,1,2")
    ap.add_argument("--manifest", type=Path, required=True)
    ap.add_argument("--verify", action="store_true", help="сверить SHA-256 с манифестом, не меняя его")
    ap.add_argument("--test-final", action="store_true", help="синтез test (tts-dmitri), один раз")
    ap.add_argument("--stop-stage", type=int, help="ступень стопа s, определённая на train")
    args = ap.parse_args()

    if args.test_final:
        if args.stop_stage is None or args.stop_stage not in STAGES:
            sys.exit("--test-final требует --stop-stage s из 0..7")
        name = TEST_VOICE
        stages = list(range(args.stop_stage + 1))
    else:
        if args.stop_stage is not None:
            sys.exit("--stop-stage только вместе с --test-final")
        name = TRAIN_VOICE
        stages = [int(s) for s in args.stages.split(",")]
        for s in stages:
            if s not in STAGES:
                sys.exit(f"нет ступени {s}")

    old = {}
    old_manifest = {}
    if args.manifest.exists():
        old_manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
        old = {clip_key(c): c for c in old_manifest.get("clips", [])}
    old_python = old_manifest.get("python")
    if old_python is not None and old_python != platform.python_version():
        sys.exit(
            f"Python {platform.python_version()} не совпадает с версией в манифесте "
            f"{old_python}: random.gauss может дать другой шум, запуск запрещён"
        )
    test_in_manifest = {k[1] for k in old if k[0] == "tts-dmitri"}
    if args.test_final:
        if args.verify:
            # --verify для test разрешён только как повторная сверка уже
            # записанного test: иначе это был бы незарегистрированный синтез test.
            lacking = [s for s in stages if s not in test_in_manifest]
            if lacking:
                sys.exit(
                    "--test-final --verify запрещён: test для ступеней "
                    f"{lacking} нет в манифесте (test синтезируется только один раз, "
                    "без --verify, после определения s)"
                )
        elif test_in_manifest:
            sys.exit("test уже синтезирован (есть в манифесте): второй синтез запрещён протоколом")
    elif not args.verify:
        # Ступени train синтезируются строго по порядку 0..7.
        have = {k[1] for k in old if k[0] == "tts-denis"} | set(stages)
        for s in stages:
            gap = [k for k in range(s) if k not in have]
            if gap:
                sys.exit(f"ступень {s} запрещена: нет ступеней train {gap} (порядок 0..7)")

    families = json.loads(FAMILIES.read_text(encoding="utf-8"))
    block, split, _, _ = VOICES[name]
    speaker = "tts-" + name.split("-")[1]
    clips = []
    for stage in stages:
        length_scale, snr = STAGES[stage]
        # Новая сессия на каждую пару (голос, ступень): шум фразы зависит
        # от seed пары и от порядка фраз в блоке, порядок задан файлом семейств.
        seed = clip_seed(name, stage)
        voice = load_voice(args.models, name, seed)[1]
        for index, row in enumerate(families["blocks"][block]):
            pcm, rate = synthesize(voice, row["human_transcript"], length_scale)
            if snr is not None:
                pcm = add_white_noise(pcm, snr, clip_seed(name, row["id"], stage, "noise"))
            rel = Path(speaker) / f"s{stage}" / f"{row['id']}.wav"
            path = args.out / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            with wave.open(str(path), "wb") as w:
                w.setnchannels(1)
                w.setsampwidth(2)
                w.setframerate(rate)
                w.writeframes(pcm)
            clips.append(
                {
                    "path": rel.as_posix(),
                    "id": row["id"],
                    "family_id": row["family_id"],
                    "domain": row["domain"],
                    "speaker": speaker,
                    "split": split,
                    "stage": stage,
                    "length_scale": length_scale,
                    "snr_db": snr,
                    "session_seed": seed,
                    "order_in_session": index,
                    "python": platform.python_version(),
                    "sha256": sha256_file(path),
                }
            )

    if args.verify:
        bad = missing = 0
        for c in clips:
            o = old.get(clip_key(c))
            if o is None:
                missing += 1
            elif o["sha256"] != c["sha256"]:
                bad += 1
                print(f"РАСХОЖДЕНИЕ {c['path']}")
        ok = len(clips) - bad - missing
        print(f"verify: совпало {ok}, расходится {bad}, нет в манифесте {missing}")
        return 1 if bad or missing else 0

    for c in clips:
        old[clip_key(c)] = c
    merged = sorted(old.values(), key=lambda c: (c["speaker"] != "tts-denis", c["speaker"], c["stage"], c["order_in_session"]))
    manifest = {
        "corpus": "voice-v2-tts",
        "protocol": "research/protocol/voice-v2.md",
        "families": "research/protocol/voice-v1-families.json",
        "families_sha256": sha256_file(FAMILIES),
        "piper_tts": PIPER_VERSION,
        "onnxruntime": ONNXRUNTIME_VERSION,
        # Версия Python, с которой синтезированы все клипы; при несовпадении
        # запуск выше отказывает, поэтому поле не перезаписывается другой версией.
        "python": old_python or platform.python_version(),
        "piper_voices_revision": PIPER_VOICES_REVISION,
        "provider": "CPUExecutionProvider",
        "threads": 1,
        "noise_scale": NOISE_SCALE,
        "noise_w": NOISE_W,
        "models": {
            n: {"onnx_sha256": v[2], "onnx_json_sha256": v[3]}
            for n, v in VOICES.items()
            if any(c["speaker"] == "tts-" + n.split("-")[1] for c in merged)
        },
        "stages": sorted({c["stage"] for c in merged}),
        "clips": merged,
    }
    args.manifest.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(f"{len(clips)} новых клипов, всего {len(merged)}, манифест {args.manifest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

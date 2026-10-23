#!/usr/bin/env python3
"""Загрузка внешней части voice-v2 (FLEURS ru, Common Voice ru) по списку ID.

Только стандартная библиотека. Аудио кладётся в каталог вне git
(или `voice-v2-audio/`, он в `.gitignore`) и не перевыкладывается.

FLEURS:        fetch_voice_v2_external.py fleurs --out /tmp/voice-v2-audio
Common Voice:  архив скачивает человек с commonvoice.mozilla.org (вход и
               согласие с условиями — его шаг, скрипт их не обходит), затем
               fetch_voice_v2_external.py select-cv --archive <путь>   # выбрать ID
               fetch_voice_v2_external.py cv --archive <путь> --out ...  # извлечь
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import io
import json
import os
import sys
import tarfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CLIPS = ROOT / "research" / "data" / "voice-v2-external-clips.json"
TIMEOUT = 60  # секунд на сетевую операцию
HF = "https://huggingface.co/datasets/google/fleurs/resolve/{rev}/{path}"


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def fetch(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=TIMEOUT) as r:  # noqa: S310 - фиксированный URL
        return r.read()


def cmd_fleurs(spec: dict, out: Path, cache: Path) -> None:
    f = spec["fleurs"]
    tsv = fetch(HF.format(rev=f["revision"], path=f["tsv"]))
    if sha256_bytes(tsv) != f["tsv_sha256"]:
        sys.exit("test.tsv FLEURS не совпал по SHA-256: выпуск изменился")
    want = {c["file"] for c in f["clips"]}
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / "fleurs-ru_ru-test.tar.gz"
    if not archive.exists():
        part = archive.with_suffix(".part")
        url = HF.format(rev=f["revision"], path=f["archive"])
        with urllib.request.urlopen(url, timeout=TIMEOUT) as r, part.open("wb") as out_f:  # noqa: S310
            for chunk in iter(lambda: r.read(1 << 20), b""):
                out_f.write(chunk)
        part.rename(archive)
    if sha256_file(archive) != f["archive_sha256"]:
        sys.exit(f"{archive}: SHA-256 архива FLEURS не совпал с зафиксированным")
    target = out / "fleurs-ru"
    target.mkdir(parents=True, exist_ok=True)
    found = {}
    with tarfile.open(archive) as tar:
        for m in tar:
            name = Path(m.name).name
            if m.isfile() and name in want:
                data = tar.extractfile(m).read()
                (target / name).write_bytes(data)
                found[name] = sha256_bytes(data)
    missing = want - found.keys()
    if missing:
        sys.exit(f"в архиве нет {len(missing)} файлов: {sorted(missing)[:3]}…")
    bad = [c["file"] for c in f["clips"] if c.get("sha256") and found[c["file"]] != c["sha256"]]
    if bad:
        sys.exit(f"SHA-256 клипов FLEURS не совпал с зафиксированным: {bad[:3]}…")
    if any(not c.get("sha256") for c in f["clips"]):
        sys.exit("в voice-v2-external-clips.json нет SHA-256 части клипов FLEURS")
    write_hashes(out / "fleurs-ru.sha256.json", found)


def cv_rows(archive: Path) -> tuple[tarfile.TarFile, list[dict]]:
    tar = tarfile.open(archive)
    member = next(m for m in tar if m.name.endswith("/test.tsv"))
    text = tar.extractfile(member).read().decode("utf-8")
    return tar, list(csv.DictReader(io.StringIO(text), delimiter="\t", quoting=csv.QUOTE_NONE))


def cmd_select_cv(spec: dict, archive: Path) -> None:
    _, rows = cv_rows(archive)
    ok = [r for r in rows if int(r["up_votes"]) >= 2 and int(r["down_votes"]) == 0]
    per_client: dict[str, dict] = {}
    for r in sorted(ok, key=lambda r: r["path"]):
        per_client.setdefault(r["client_id"], r)
    chosen = sorted(per_client.values(), key=lambda r: r["path"])[:40]
    spec["common_voice"]["archive_sha256"] = sha256_file(archive)
    spec["common_voice"]["clips"] = [{"path": r["path"], "sentence": r["sentence"]} for r in chosen]
    spec["common_voice"]["status"] = "ID выбраны по правилу из архива с указанным SHA-256"
    CLIPS.write_text(json.dumps(spec, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"выбрано {len(chosen)} клипов Common Voice")


def cmd_cv(spec: dict, archive: Path, out: Path) -> None:
    cv = spec["common_voice"]
    if not cv["clips"]:
        sys.exit("список Common Voice пуст: сначала select-cv")
    if sha256_file(archive) != cv["archive_sha256"]:
        sys.exit("архив Common Voice не тот, по которому выбраны ID")
    want = {c["path"] for c in cv["clips"]}
    tar, _ = cv_rows(archive)
    target = out / "common-voice-ru"
    target.mkdir(parents=True, exist_ok=True)
    found = {}
    for m in tar:
        name = Path(m.name).name
        if m.isfile() and name in want:
            data = tar.extractfile(m).read()
            (target / name).write_bytes(data)
            found[name] = sha256_bytes(data)
    if want - found.keys():
        sys.exit("в архиве нет части клипов")
    write_hashes(out / "common-voice-ru.sha256.json", found)


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def write_hashes(path: Path, found: dict) -> None:
    path.write_text(json.dumps(dict(sorted(found.items())), indent=2) + "\n", encoding="utf-8")
    print(f"{len(found)} файлов, SHA-256 в {path}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("what", choices=["fleurs", "select-cv", "cv"])
    ap.add_argument("--out", type=Path)
    ap.add_argument(
        "--cache",
        type=Path,
        default=Path(os.environ.get("VOICE_V2_CACHE", "/tmp/voice-v2-cache")),
        help="кэш архивов; по умолчанию $VOICE_V2_CACHE или /tmp/voice-v2-cache",
    )
    ap.add_argument("--archive", type=Path)
    args = ap.parse_args()
    spec = json.loads(CLIPS.read_text(encoding="utf-8"))
    if args.what == "fleurs":
        cmd_fleurs(spec, args.out, args.cache)
    elif args.what == "select-cv":
        cmd_select_cv(spec, args.archive)
    else:
        cmd_cv(spec, args.archive, args.out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

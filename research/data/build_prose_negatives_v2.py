#!/usr/bin/env python3
"""Builds research/data/prose-negatives-v2.jsonl and its manifest.

v2 is v1 plus the traps found on 2026-09-30. Every v1 record is carried over
byte for byte, with its split; the v1 file is read, checked against the
SHA-256 in its manifest, and never written. New sentences get new families,
and their splits are assigned by the v1 rule over the new families only, so
no v1 record moves between splits.

The claim of every record is the v1 claim: nothing here is dictated
notation, the gold is the transcript itself, any change is an S4 defect.

What is new, and where it came from:

* **Amounts in prose.** The owner's own coursework (a private corpus,
  rgr-prose-v1, not included here) showed «один моль» becoming «1 моль»
  inside a sentence. The owner decided that an amount in a sentence stays
  words, and that several amounts in a row are not a dictated list.
* **New units.** грамм, литр, час, минута and градус Цельсия were added the
  same day; each of them now has to stay words in a sentence, and words that
  merely resemble a unit («час пик», «ни грамма») must not become one.
* **A conjunction between two operands.** «икс и игрек» was `xiy` and «два
  моля и три моля» was `2 мольi3 моль`: the conjunction was read as the
  letter i. A conjunction after a number («И два дня спустя…») is a sentence.
* **Letter names that are also element names.** «ка» (potassium) and «це»
  (carbon, as a recogniser writes «цэ») became element evidence the same day;
  «це» is also Ukrainian for "this".

The sentences were written before any of them was run through the system,
and none is removed or edited after seeing the result. This builder never
calls sciwhisper-core.
"""

import hashlib
import json
import pathlib
from collections import Counter

OUT = pathlib.Path(__file__).resolve().parent
PARENT = OUT / "prose-negatives-v1.jsonl"
PARENT_MANIFEST = OUT / "prose-negatives-v1.manifest.json"
JSONL = OUT / "prose-negatives-v2.jsonl"
MANIFEST = OUT / "prose-negatives-v2.manifest.json"
SCHEMA_VERSION = 1
CORPUS_ID = "prose-negatives-v2"
CREATED = "2026-09-30"

parent_bytes = PARENT.read_bytes()
parent_manifest = json.loads(PARENT_MANIFEST.read_text(encoding="utf-8"))
parent_sha = hashlib.sha256(parent_bytes).hexdigest()
assert parent_sha == parent_manifest["sha256"], "prose-negatives-v1 differs from its manifest"
PARENT_RECORDS = [json.loads(line) for line in parent_bytes.decode("utf-8").splitlines() if line]
assert len(PARENT_RECORDS) == parent_manifest["records"]
parent_families = {r["family_id"] for r in PARENT_RECORDS}

NEW = []
_counters = Counter()


def prose(section, text, *tags):
    """One new sentence of ordinary prose; the family is the first tag's trap."""
    family = "%s-%s" % (section, tags[0])
    assert family not in parent_families, family
    _counters[family] += 1
    NEW.append({
        "dataset_schema_version": SCHEMA_VERSION,
        "id": "%s-%03d" % (family, _counters[family]),
        "family_id": family,
        "provenance": "handcrafted_text",
        "human_transcript": text,
        "asr_hypotheses": [],
        "target_domain": "plain",
        "target_action": "raw",
        "target_ast": None,
        "split": None,
        "tags": list(tags) + ["prose-negative", "v2"],
        "speaker_id": None,
    })


# ------------------------------------------------------------------------
# 1. Количество внутри обычной фразы.
# ------------------------------------------------------------------------
A = "prose-amount"
prose(A, "На каждый моль хлора приходится один моль водорода.", "mole")
prose(A, "Для первой стадии хватит двух молей, для второй нужно больше.", "mole")
prose(A, "В пробирку добавили пять граммов соли и перемешали.", "gram")
prose(A, "На весах было двести граммов, хотя ждали больше.", "gram")
prose(A, "Налей два литра воды в кастрюлю.", "litre")
prose(A, "Эксперимент занял три часа без перерыва.", "hour")
prose(A, "Раствор настаивали пять минут и сразу фильтровали.", "minute")
prose(A, "Встреча длилась два часа тридцать минут.", "compound")
prose(A, "Шланг оказался длиной три метра пять сантиметров.", "compound")
prose(A, "Температура в комнате держалась двадцать градусов Цельсия.", "celsius")
prose(A, "Ночью было минус пять градусов, и лужи замёрзли.", "degree-bare")
prose(A, "Скорость была десять километров в час, не больше.", "speed")
prose(A, "Он пробежал пять километров до лаборатории.", "length")
prose(A, "Мы добавили два моля серной кислоты в колбу.", "with-substance")
prose(A, "Для реакции нужно пять граммов хлорида натрия и немного воды.", "with-substance")
prose(A, "Налили три литра, потом ещё два литра.", "run")
prose(A, "Нагревали пять минут, потом десять минут, потом два часа.", "run")
prose(A, "Навески: два моля, три моля, пять молей.", "list")

# ------------------------------------------------------------------------
# 2. Слова, похожие на единицы, но не единицы.
# ------------------------------------------------------------------------
U = "prose-unit-homonym"
prose(U, "Час пик в этот раз начался раньше обычного.", "hour")
prose(U, "Который час, не подскажешь?", "hour")
prose(U, "Минута молчания прошла в полной тишине.", "minute")
prose(U, "У него нет ни грамма совести.", "gram")
prose(U, "Градус напряжения на собрании рос с каждой минутой.", "degree")
prose(U, "Литр за литром вода уходила в песок.", "litre")

# ------------------------------------------------------------------------
# 3. Союз между двумя операндами.
# ------------------------------------------------------------------------
C = "prose-conjunction-operand"
prose(C, "Переменные икс и игрек здесь независимы.", "letters")
prose(C, "Отрезок был три метра и ещё четыре метра запаса.", "amounts")
prose(C, "Мы взяли два моля и три моля соответственно.", "amounts")
prose(C, "Два и три в сумме дают пять, это знает каждый.", "numbers")
prose(C, "У нас было пять, а у них шесть.", "numbers-a")
prose(C, "И два дня спустя всё повторилось.", "sentence-initial")
prose(C, "А два года назад такого не было.", "sentence-initial")
prose(C, "Мы взяли и два образца, и три пробы.", "numbers")

# ------------------------------------------------------------------------
# 4. Буквенные имена, которые стали именами элементов.
# ------------------------------------------------------------------------
L = "prose-letter-name"
prose(L, "Бабушка по-украински говорит: це добре.", "tse")
prose(L, "Рядом стояла буква ка, а за ней пустое место.", "ka")

# --------------------------------------------------------------- splits

new_families = []
for record in NEW:
    if record["family_id"] not in new_families:
        new_families.append(record["family_id"])
new_families.sort()
split_of = {}
for index, family in enumerate(new_families):
    bucket = index % 10
    split_of[family] = "train" if bucket < 5 else ("validation" if bucket < 8 else "dev_holdout")
for record in NEW:
    record["split"] = split_of[record["family_id"]]

# ---------------------------------------------------------------- write


def canonical_line(record):
    return json.dumps(record, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


# v1 lines are kept exactly as they were; the new ones follow, sorted by id.
parent_lines = [line for line in parent_bytes.decode("utf-8").splitlines() if line]
assert parent_lines == [canonical_line(r) for r in PARENT_RECORDS], "v1 is not canonical"
NEW.sort(key=lambda record: record["id"])
RECORDS = PARENT_RECORDS + NEW
ids = [record["id"] for record in RECORDS]
assert len(ids) == len(set(ids)), "duplicate id"
texts = [record["human_transcript"] for record in RECORDS]
assert len(texts) == len(set(texts)), "duplicate sentence"
for record in RECORDS:
    assert record["target_action"] == "raw", record["id"]
    assert record["target_ast"] is None, record["id"]
    assert "expected_mixed_output" not in record, record["id"]

text = "\n".join(parent_lines + [canonical_line(r) for r in NEW]) + "\n"
JSONL.write_text(text, encoding="utf-8")

all_families = sorted({r["family_id"] for r in RECORDS})
family_split = {r["family_id"]: r["split"] for r in RECORDS}
manifest = {
    "manifest_schema_version": 1,
    "corpus_id": CORPUS_ID,
    "created": CREATED,
    "file": JSONL.name,
    "sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
    "dataset_schema_version": SCHEMA_VERSION,
    "records": len(RECORDS),
    "families": len(all_families),
    "counts_by_domain": dict(sorted(Counter(r["target_domain"] for r in RECORDS).items())),
    "counts_by_split": dict(sorted(Counter(r["split"] for r in RECORDS).items())),
    "counts_by_action": dict(sorted(Counter(r["target_action"] for r in RECORDS).items())),
    "counts_by_provenance": dict(sorted(Counter(r["provenance"] for r in RECORDS).items())),
    "counts_by_tag": dict(sorted(Counter(t for r in RECORDS for t in r["tags"]).items())),
    "families_by_split": {
        split: sorted(f for f in all_families if family_split[f] == split)
        for split in ("train", "validation", "dev_holdout")
    },
    "derived_from": {
        "corpus_id": "prose-negatives-v1",
        "sha256": parent_sha,
        "by": "research/data/build_prose_negatives_v2.py",
        "change": (
            "Every v1 record is carried over byte for byte with its split. %d new records in "
            "%d new families, tagged v2, are appended; their splits follow the v1 rule over "
            "the new families only." % (len(NEW), len(new_families))
        ),
    },
    "claim": parent_manifest["claim"],
    "why": (
        "Four traps found on 2026-09-30 that no record in any corpus covered: an amount "
        "inside a sentence («один моль» -> «1 моль», found in the owner's coursework), "
        "the units added that day (грамм, литр, час, минута, градус Цельсия), a conjunction "
        "read as a letter between two operands («икс и игрек» -> xiy, «два моля и три моля» "
        "-> 2 мольi3 моль), and the letter names «ка» and «це» that became element evidence."
    ),
    "gold_provenance": (
        "There is no annotation in this corpus and nothing to get wrong: the gold is the "
        "transcript itself. This builder never calls sciwhisper-core. The new sentences were "
        "written before any of them was run through the system, and no record was removed "
        "or edited after seeing what the system does with it."
    ),
    "amount_policy": (
        "Decided by the owner on 2026-09-30: an amount (a number with a unit) inside a "
        "sentence is mentioned, not dictated, and stays words; several amounts in a row are "
        "not a dictated list («Навески: два моля, три моля, пять молей» stays words, a "
        "deliberate price). See is_bare_quantity and holds_an_enumeration in "
        "crates/sciwhisper-core/src/utterance.rs."
    ),
    "substitution_policy": parent_manifest["substitution_policy"],
    "audio": "none",
    "asr": "none: this is a text-level corpus and no record carries a speaker_id.",
    "limitations": parent_manifest["limitations"] + [
        "The v2 sentences were written by the agents working on the parser the same day the "
        "traps were found and closed; they pin those traps, they do not search for new ones.",
        "The owner's coursework corpus (rgr-prose-v1), the only text so far not written to "
        "test the parser, is private and is not included.",
    ],
}
MANIFEST.write_text(json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
                    encoding="utf-8")

print("%d records (%d from v1, %d new), %d families" % (
    len(RECORDS), len(PARENT_RECORDS), len(NEW), len(all_families)))
print("splits :", dict(sorted(Counter(r["split"] for r in RECORDS).items())))
print("sha256 :", manifest["sha256"])

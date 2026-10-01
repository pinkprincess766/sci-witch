#!/usr/bin/env python3
"""Builds research/data/ambiguous-v1.jsonl and its manifest.

This corpus exists for one question that `dev-seed-v2` cannot answer:
**when the right reading is not the first one the parser produces, is it
produced at all?** Every `Recall@K` on `dev-seed-v2` is 100% from K=2 upward,
so that corpus can neither reward a wider candidate set nor punish one.

Two rules govern the gold here, and both are worth stating because they are
the ones that could silently turn a benchmark into a self-portrait:

1. **Gold is written from the intended meaning, before the algorithm was
   changed.** This script never imports, calls or shells out to
   sciwhisper-core. Where the current system is known to disagree, the gold
   still says what the speaker meant; that disagreement is the finding.

2. **Gold transcribes what was said, not what is chemically true.** A speaker
   who says «реакция между натрием и хлором с образованием хлорида натрия»
   dictated no coefficients, so the gold equation carries none. Balancing is
   a separate, already-measured concern (`chemistry.balance_suggestion`), and
   folding it in here would score the balancer inside a parser metric.

`dev-seed-v1` and `dev-seed-v2` are not touched by this script.
"""

import hashlib
import json
import pathlib
from collections import Counter

import ast_helpers as H

OUT = pathlib.Path(__file__).resolve().parent
JSONL = OUT / "ambiguous-v1.jsonl"
MANIFEST = OUT / "ambiguous-v1.manifest.json"
SCHEMA_VERSION = 1
CORPUS_ID = "ambiguous-v1"
CREATED = "2026-09-12"

RECORDS = []

def ast_record(family, suffix, transcript, domain, target, tags, render=None, notes=None):
    entry = {
        "dataset_schema_version": SCHEMA_VERSION,
        "id": f"{family}-{suffix}",
        "family_id": family,
        "provenance": "handcrafted_text",
        "human_transcript": transcript,
        "asr_hypotheses": [],
        "target_domain": domain,
        "target_action": "ast",
        "target_ast": target,
        "split": None,
        "tags": tags,
        "speaker_id": None,
    }
    if render:
        entry["expected_render"] = {"unicode": render}
    if notes:
        entry["notes"] = notes
    RECORDS.append(entry)

def raw_record(family, suffix, transcript, tags, mixed=None, notes=None):
    entry = {
        "dataset_schema_version": SCHEMA_VERSION,
        "id": f"{family}-{suffix}",
        "family_id": family,
        "provenance": "handcrafted_text",
        "human_transcript": transcript,
        "asr_hypotheses": [],
        "target_domain": "plain",
        "target_action": "raw",
        "target_ast": None,
        "split": None,
        "tags": tags,
        "speaker_id": None,
    }
    if mixed is not None:
        entry["expected_mixed_output"] = mixed
    if notes:
        entry["notes"] = notes
    RECORDS.append(entry)

C, M, P = "chemistry", "mathematics", "physics"

# ------------------------------------------------------------- substances

Na   = H.formula(H.atom("Na"))
Cl2  = H.formula(H.atom("Cl", 2))
NaCl = H.formula(H.atom("Na"), H.atom("Cl"))
Zn    = H.formula(H.atom("Zn"))
HCl   = H.formula(H.atom("H"), H.atom("Cl"))
ZnCl2 = H.formula(H.atom("Zn"), H.atom("Cl", 2))
H2    = H.formula(H.atom("H", 2))
O2    = H.formula(H.atom("O", 2))
H2O   = H.formula(H.atom("H", 2), H.atom("O"))
CaCO3 = H.formula(H.atom("Ca"), H.atom("C"), H.atom("O", 3))
CaO   = H.formula(H.atom("Ca"), H.atom("O"))
CO2   = H.formula(H.atom("C"), H.atom("O", 2))
CuO   = H.formula(H.atom("Cu"), H.atom("O"))
Cu2O  = H.formula(H.atom("Cu", 2), H.atom("O"))
FeOH3 = H.formula(H.atom("Fe"), H.group([H.atom("O"), H.atom("H")], 3))
FeOH2 = H.formula(H.atom("Fe"), H.group([H.atom("O"), H.atom("H")], 2))
K2CO3 = H.formula(H.atom("K", 2), H.atom("C"), H.atom("O", 3))
Na2CO3 = H.formula(H.atom("Na", 2), H.atom("C"), H.atom("O", 3))
H2SO4 = H.formula(H.atom("H", 2), H.atom("S"), H.atom("O", 4))

def eq(left, right, condition=None):
    return H.reaction([H.species(f) for f in left], [H.species(f) for f in right],
                      condition=condition)

NACL_EQ  = eq([Na, Cl2], [NaCl])
ZNCL_EQ  = eq([Zn, HCl], [ZnCl2, H2])
CACO3_EQ = eq([CaCO3], [CaO, CO2])
# «если нагреть…» and «при нагревании…» state a condition out loud. The gold
# has to carry it for the same reason it carries no coefficients: it
# transcribes what was said. The first version of this file omitted the
# condition on both, which made two correct parses read as misses.
CACO3_HEAT_EQ = eq([CaCO3], [CaO, CO2], condition="heat")
WATER_EQ = eq([H2, O2], [H2O])

SAID_NOT_BALANCED = ("The speaker dictated no coefficients, so the gold carries none; "
                     "the balance warning is a separate, already-measured concern.")

# ------------------------------------- A. «между X и Y с образованием Z»

ast_record("amb-frame-between-nacl-001", "a",
           "реакция идёт между натрием и хлором с образованием хлорида натрия",
           C, NACL_EQ, ["reaction-frame", "between-with", "equation"], notes=SAID_NOT_BALANCED)
ast_record("amb-frame-between-nacl-001", "b",
           "реакция между натрием и хлором с образованием хлорида натрия",
           C, NACL_EQ, ["reaction-frame", "between-with", "equation", "paraphrase"])
ast_record("amb-frame-between-zn-001", "a",
           "реакция между цинком и соляной кислотой с образованием хлорида цинка и водорода",
           C, ZNCL_EQ, ["reaction-frame", "between-with", "equation", "two-products"],
           notes=SAID_NOT_BALANCED)
ast_record("amb-frame-between-water-001", "a",
           "реакция идёт между водородом и кислородом с образованием воды",
           C, WATER_EQ, ["reaction-frame", "between-with", "equation", "conjunction-trap"],
           notes="«и» between two substance names must stay a conjunction, never iodine.")

# ---------------------------- B. «X взаимодействует с Y, получается Z»

ast_record("amb-frame-result-nacl-001", "a",
           "натрий взаимодействует с хлором, в результате получается хлорид натрия",
           C, NACL_EQ, ["reaction-frame", "interacts-result", "equation"])
ast_record("amb-frame-result-zn-001", "a",
           "цинк взаимодействует с соляной кислотой, в результате получается хлорид цинка и водород",
           C, ZNCL_EQ, ["reaction-frame", "interacts-result", "equation", "two-products"])
ast_record("amb-frame-result-zn-001", "b",
           "цинк реагирует с соляной кислотой, получается хлорид цинка и водород",
           C, ZNCL_EQ, ["reaction-frame", "interacts-result", "equation", "paraphrase"])
ast_record("amb-frame-result-water-001", "a",
           "водород взаимодействует с кислородом, в результате получается вода",
           C, WATER_EQ, ["reaction-frame", "interacts-result", "equation"])

# ------------------------ C. «если нагреть X, он разлагается на Y и Z»

ast_record("amb-frame-decompose-001", "a",
           "если нагреть карбонат кальция, он разлагается на оксид кальция и углекислый газ",
           C, CACO3_HEAT_EQ, ["reaction-frame", "decomposition", "equation", "conditional"],
           notes="«если нагреть» is a spoken condition and belongs in the gold.")
ast_record("amb-frame-decompose-001", "b",
           "при нагревании карбонат кальция разлагается на оксид кальция и углекислый газ",
           C, CACO3_HEAT_EQ, ["reaction-frame", "decomposition", "equation", "paraphrase"])
ast_record("amb-frame-decompose-001", "c",
           "карбонат кальция разлагается на оксид кальция и углекислый газ",
           C, CACO3_EQ, ["reaction-frame", "decomposition", "equation", "paraphrase"])

# ------------------------------------ D. permutations and parentheticals

ast_record("amb-order-fronted-001", "a",
           "с образованием хлорида натрия идёт реакция между натрием и хлором",
           C, NACL_EQ, ["reaction-frame", "permutation", "equation"],
           notes="The product is stated first; the equation still reads left to right.")
ast_record("amb-order-parenthetical-001", "a",
           "реакция, как мы вчера обсуждали, идёт между натрием и хлором с образованием хлорида натрия",
           C, NACL_EQ, ["reaction-frame", "parenthetical", "equation"])
ast_record("amb-order-filler-001", "a",
           "ну значит карбонат кальция, вот, разлагается на оксид кальция и углекислый газ",
           C, CACO3_EQ, ["reaction-frame", "filler", "equation"])
raw_record("amb-order-noproduct-001", "a",
           "ну значит натрий, вот, реагирует с хлором",
           ["reaction-frame", "incomplete", "raw"],
           notes="A reaction with no product named cannot be written as an equation; "
                 "keeping the words is the right answer.")

# ------------------------------------------- E. explicit self-corrections

ast_record("amb-fix-hydroxide-001", "a", "гидроксид железа два, нет, железа три",
           C, H.chem(H.species(FeOH3)), ["self-correction", "restate"], "Fe(OH)₃")
ast_record("amb-fix-hydroxide-001", "b", "гидроксид железа три, точнее железа два",
           C, H.chem(H.species(FeOH2)), ["self-correction", "restate"], "Fe(OH)₂")
ast_record("amb-fix-oxide-001", "a", "оксид меди один, точнее, меди два",
           C, H.chem(H.species(CuO)), ["self-correction", "restate", "oxidation-state"], "CuO")
ast_record("amb-fix-oxide-001", "b", "оксид меди два, нет, меди один",
           C, H.chem(H.species(Cu2O)), ["self-correction", "restate", "oxidation-state"], "Cu₂O")
ast_record("amb-fix-carbonate-001", "a", "карбонат натрия, нет, карбонат калия",
           C, H.chem(H.species(K2CO3)), ["self-correction", "restate", "whole-term"], "K₂CO₃")
ast_record("amb-fix-substitute-001", "a", "не два икс, а три икс плюс один",
           M, H.math(H.binary("Add", H.juxt(H.num("3"), H.sym("x")), H.num("1"))),
           ["self-correction", "substitute"], "3x + 1",
           notes="«не A, а B» is a token-for-token substitution: the phrase is «три икс плюс один».")
ast_record("amb-fix-substitute-001", "b", "не два, а три икс",
           M, H.math(H.juxt(H.num("3"), H.sym("x"))), ["self-correction", "substitute"], "3x")
ast_record("amb-fix-restate-math-001", "a", "два икс плюс один, нет, три икс плюс один",
           M, H.math(H.binary("Add", H.juxt(H.num("3"), H.sym("x")), H.num("1"))),
           ["self-correction", "restate"], "3x + 1",
           notes="The restated tail replaces the whole phrase; no «два» may survive outside it.")
ast_record("amb-fix-restate-math-001", "b", "икс в квадрате, нет, икс в кубе",
           M, H.math(H.power(H.sym("x"), H.num("3"))), ["self-correction", "restate"], "x³")
# Four corrections is exactly MAX_CORRECTIONS, so all of them apply and the
# answer is the value the speaker landed on. The first version of this record
# claimed the transcript would be kept instead; that was an assumption about a
# bound, not a reading of one, and it was wrong.
ast_record("amb-fix-too-many-001", "a",
           "гидроксид железа два, нет, железа три, нет, железа два, нет, железа три, нет, железа два",
           C, H.chem(H.species(FeOH2)), ["self-correction", "bounded"], "Fe(OH)\u2082",
           notes="Exactly MAX_CORRECTIONS corrections; the last value said is the answer.")

# --------------------------------------- F. ambiguous math boundaries

ast_record("amb-root-bind-001", "a", "корень из икс плюс один",
           M, H.math(H.binary("Add", H.root(H.sym("x")), H.num("1"))),
           ["boundary", "root", "ambiguous"], "√x + 1",
           notes="The competing reading √(x + 1) must be reachable as a candidate; "
                 "the narrow one is gold because it is the documented default.")
ast_record("amb-root-bind-001", "b", "начало корня икс плюс один конец корня",
           M, H.math(H.root(H.binary("Add", H.sym("x"), H.num("1")))),
           ["boundary", "root", "explicit"], "√(x + 1)",
           notes="With explicit boundary commands there is no ambiguity and no second candidate.")
ast_record("amb-root-bind-001", "c", "корень из икс плюс один равно два",
           M, H.math(H.binary("Eq", H.binary("Add", H.root(H.sym("x")), H.num("1")), H.num("2"))),
           ["boundary", "root", "ambiguous", "equation"], "√x + 1 = 2")
ast_record("amb-sine-bind-001", "a", "синус икс плюс игрек",
           M, H.math(H.binary("Add", H.fn("Sin", H.sym("x")), H.sym("y"))),
           ["boundary", "function", "ambiguous"], "sin(x) + y",
           notes="sin(x + y) is the competing reading and must be a candidate.")
ast_record("amb-div-bind-001", "a", "один делить на икс плюс два",
           M, H.math(H.binary("Add", H.fraction(H.num("1"), H.sym("x")), H.num("2"))),
           ["boundary", "fraction", "ambiguous"],
           notes="1/(x + 2) is the competing reading.")
ast_record("amb-log-bind-001", "a", "логарифм икс плюс один",
           M, H.math(H.binary("Add", H.fn("Log", H.sym("x")), H.num("1"))),
           ["boundary", "function", "ambiguous"])

# ------------------------------------------------- G. domain competition

ast_record("amb-domain-delta-g-001", "a", "дельта же равно минус эн эф е",
           P, H.math(H.binary("Eq", H.delta(H.sym("G", "Upper")),
                              H.neg(H.juxt(H.sym("n"), H.sym("F", "Upper"), H.sym("E", "Upper"))))),
           ["domain-competition", "delta", "physics"],
           notes="Δ and the spelled symbols are physics notation, not a chemistry species.")
ast_record("amb-domain-spelled-001", "a", "аш два эс о четыре",
           C, H.chem(H.species(H2SO4)), ["domain-competition", "spelled", "chemistry"], "H₂SO₄",
           notes="A run of spelled letters is a formula; a lone spelled letter between two "
                 "substance names is not.")
ast_record("amb-domain-spelled-001", "b", "цэ о два",
           C, H.chem(H.species(CO2)), ["domain-competition", "spelled", "chemistry"], "CO₂")

# ------------------------------ H. bounded ASR text errors (no open dictionary)

ast_record("amb-asr-hydroxide-001", "a", "гидраксид железа три",
           C, H.chem(H.species(FeOH3)), ["asr-error", "vowel", "chemistry"], "Fe(OH)₃")
ast_record("amb-asr-hydroxide-001", "b", "кидроксидж лезо три",
           C, H.chem(H.species(FeOH3)), ["asr-error", "known-repair", "chemistry"], "Fe(OH)₃")
ast_record("amb-asr-carbonate-001", "a", "карбанат кальция",
           C, H.chem(H.species(CaCO3)), ["asr-error", "vowel", "chemistry"], "CaCO₃")
ast_record("amb-asr-chloride-001", "a", "хларид натрия",
           C, H.chem(H.species(NaCl)), ["asr-error", "vowel", "chemistry"], "NaCl")
ast_record("amb-asr-sulfuric-001", "a", "сернея кислота",
           C, H.chem(H.species(H2SO4)), ["asr-error", "vowel", "chemistry"], "H₂SO₄")
raw_record("amb-asr-unrepairable-001", "a", "гырдксд жлз тр",
           ["asr-error", "unrepairable", "raw"],
           notes="Past one vowel substitution per word there is no bounded repair; "
                 "the words are kept rather than guessed at.")

# --------------------------- negatives: ordinary speech must stay ordinary

raw_record("amb-neg-copper-prose-001", "a", "медь, о которой я говорил, лежит в шкафу",
           ["negative", "conjunction-trap", "raw"],
           mixed="Cu, о которой я говорил, лежит в шкафу",
           notes="«о» here is a preposition. Reading it as oxygen produces CuO — a different "
                 "substance — at full confidence.")
raw_record("amb-neg-hydrogen-prose-001", "a", "водород у нас закончился",
           ["negative", "conjunction-trap", "raw"], mixed="H₂ у нас закончился",
           notes="«у» must not become uranium.")
raw_record("amb-neg-sodium-and-001", "a", "натрий и калий стоят рядом в таблице",
           ["negative", "conjunction-trap", "raw"],
           notes="«и» must not become iodine.")
raw_record("amb-neg-sodium-with-001", "a", "натрий с хлором реагируют бурно",
           ["negative", "conjunction-trap", "raw"],
           notes="«с» must not become sulfur; no product is named, so no equation exists.")
raw_record("amb-neg-reaction-001", "a", "реакция была бурной", ["negative", "homonym", "raw"])
raw_record("amb-neg-limit-001", "a", "предел терпения не резиновый", ["negative", "homonym", "raw"])
raw_record("amb-neg-order-001", "a", "порядок величины имеет значение", ["negative", "homonym", "raw"])
raw_record("amb-neg-integral-001", "a", "интегральная оценка курса", ["negative", "homonym", "raw"])
raw_record("amb-neg-degree-001", "a", "степень доверия к источнику", ["negative", "homonym", "raw"])
raw_record("amb-neg-root-001", "a", "корень проблемы в другом", ["negative", "homonym", "raw"])
raw_record("amb-neg-decompose-001", "a", "команда разлагается на две группы",
           ["negative", "reaction-frame", "raw"],
           notes="The decomposition frame must not fire when neither side is a substance.")
raw_record("amb-neg-between-001", "a", "разговор между коллегами закончился ничем",
           ["negative", "reaction-frame", "raw"])
raw_record("amb-neg-result-001", "a", "он взаимодействует с людьми, в результате получается неплохо",
           ["negative", "reaction-frame", "raw"])
raw_record("amb-neg-sample-001", "a", "я взял пробу оксида", ["negative", "homonym", "raw"])
raw_record("amb-neg-not-correction-001", "a", "не всё так просто, а вот хлорид натрия мы уже обсудили",
           ["negative", "self-correction", "raw"],
           mixed="не всё так просто, а вот NaCl мы уже обсудили",
           notes="«не … а …» here is ordinary contrast, not a dictated substitution.")

# ------------------------------------------------------------- stress

ast_record("amb-stress-unicode-001", "a", "гидроксид железа три — «ключевой» реагент",
           C, H.chem(H.species(FeOH3)), ["stress", "unicode"],
           notes="Dashes and guillemets must not break span boundaries.")
# 150 × 3 = 450 words, past MAX_UTTERANCE_WORDS (400). The first version
# repeated the phrase 40 times — 120 words — and so exercised nothing while
# claiming to exercise the bound.
raw_record("amb-stress-long-001", "a", " ".join(["карбонат кальция разлагается"] * 150),
           ["stress", "long", "raw"],
           notes="Past MAX_UTTERANCE_WORDS (400 words) the text is kept verbatim; this is the "
                 "bound being exercised, not a parse failure.")
raw_record("amb-stress-overflow-001", "a", "гидроксид железа девятьсот девяносто девять тысяч",
           ["stress", "overflow", "raw"],
           notes="A count past MAX_ATOM_COUNT must refuse, not wrap.")

# ------------------------------------------------------------- splits

families = []
for record in RECORDS:
    if record["family_id"] not in families:
        families.append(record["family_id"])
families.sort()
split_of = {}
for index, family in enumerate(families):
    bucket = index % 10
    split_of[family] = "train" if bucket < 5 else ("validation" if bucket < 8 else "dev_holdout")
for record in RECORDS:
    record["split"] = split_of[record["family_id"]]

# --------------------------------------------------------------- write

def canonical_line(record):
    return json.dumps(record, ensure_ascii=False, sort_keys=True, separators=(",", ":"))

RECORDS.sort(key=lambda record: record["id"])
ids = [record["id"] for record in RECORDS]
assert len(ids) == len(set(ids)), "duplicate id in the generated corpus"
text = "\n".join(canonical_line(record) for record in RECORDS) + "\n"
JSONL.write_text(text, encoding="utf-8")

domain_counts = Counter(record["target_domain"] for record in RECORDS)
manifest = {
    "manifest_schema_version": 1,
    "corpus_id": CORPUS_ID,
    "created": CREATED,
    "file": JSONL.name,
    "sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
    "dataset_schema_version": SCHEMA_VERSION,
    "records": len(RECORDS),
    "families": len(families),
    "counts_by_domain": dict(sorted(domain_counts.items())),
    "counts_by_split": dict(sorted(Counter(r["split"] for r in RECORDS).items())),
    "counts_by_action": dict(sorted(Counter(r["target_action"] for r in RECORDS).items())),
    "counts_by_provenance": dict(sorted(Counter(r["provenance"] for r in RECORDS).items())),
    "counts_by_tag": dict(sorted(Counter(t for r in RECORDS for t in r["tags"]).items())),
    "families_by_split": {
        split: sorted(f for f in families if split_of[f] == split)
        for split in ("train", "validation", "dev_holdout")
    },
    "purpose": (
        "Candidate-lattice evaluation. dev-seed-v2 reaches Recall@2 = 100%, so it can neither "
        "reward a wider candidate set nor punish one. Every record here is a case where the "
        "intended reading is plausibly not the first one produced."
    ),
    "gold_provenance": (
        "Every target_ast was written by hand from the intended meaning, before the candidate "
        "lattice existed. This builder never calls sciwhisper-core, so no gold answer is a copy "
        "of the parser's output. Where the system disagreed at annotation time, the gold still "
        "states the intent and the disagreement is reported as a finding."
    ),
    "annotation_rules": [
        "Gold transcribes what was said. A dictated reaction carries no coefficients the speaker "
        "did not say; balancing is reported separately.",
        "Where speech is genuinely ambiguous, gold is the documented default reading and the "
        "competing reading is named in the record's notes. The competing reading is expected to "
        "be reachable as a candidate, not to be first.",
        "A negative record is ordinary speech that must stay ordinary. expected_mixed_output, "
        "where present, states what the application may substitute inside the sentence.",
        "A spoken reaction condition («при нагревании», «если нагреть») is part of what was "
        "said and is carried in the gold equation.",
    ],
    "audio": "none",
    "asr": "none: this is a text-level corpus. The asr-error records are hand-written spellings "
           "of mistakes observed in Russian Whisper output, not recognizer samples.",
    "limitations": [
        "This is a development corpus, not a frozen test set.",
        "It is small and deliberately adversarial: its rates are not comparable with dev-seed-v2 "
        "and must never be quoted as end-to-end product accuracy.",
        "Records tagged known-gap or stress encode answers or bounds the current grammar may not "
        "reach; those are findings, not corpus errors.",
    ],
}
MANIFEST.write_text(json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
                    encoding="utf-8")

print(f"{len(RECORDS)} records, {len(families)} families")
print("domains:", dict(sorted(domain_counts.items())))
print("actions:", dict(sorted(Counter(r['target_action'] for r in RECORDS).items())))
print("splits :", dict(sorted(Counter(r['split'] for r in RECORDS).items())))
print("sha256 :", manifest["sha256"])

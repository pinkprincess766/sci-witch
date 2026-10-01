#!/usr/bin/env python3
"""Derives the corpus versions that follow the substance-in-prose policy.

The policy: **a substance name inside ordinary prose is not substituted.**
It is mentioned, not dictated, so the sentence comes back as it was said.

Five records across two corpora were written before that was decided and
declare the opposite. Editing them in place would move a published SHA-256,
so this script derives new versions instead and leaves the originals exactly
as they are:

    ambiguous-v1.jsonl  ->  ambiguous-v2.jsonl   (4 records)
    dev-seed-v2.jsonl   ->  dev-seed-v3.jsonl    (1 record)

Two kinds of change, both named per record below:

* `expected_mixed_output` removed. Absent means "the application is expected
  to hand the sentence back unchanged", which is what the policy now asks.
* a substance named inside a sentence re-annotated as ordinary speech:
  `target_action` raw, no `target_ast`. One record needed this, and the first
  version of this script missed it — `amb-stress-unicode-001-a`, «гидроксид
  железа три — «ключевой» реагент», whose gold still said Fe(OH)₃. It was
  found because the candidate lattice, which reuses the shipped span search,
  stopped offering Fe(OH)₃ for it, and recall dropped by one.

Nothing else in any record is touched, and the script proves it: every field
not named here, of every record, must compare equal to the parent, or it
refuses to write.

The original builders are untouched and still reproduce their own files.
"""

import hashlib
import json
import pathlib
import sys
from collections import Counter

HERE = pathlib.Path(__file__).resolve().parent

# The records that declared a substitution inside prose, with the text that
# was expected, so the delta is readable without diffing two JSONL files.
DERIVATIONS = [
    {
        "parent": "ambiguous-v1",
        "child": "ambiguous-v2",
        "created": "2026-09-15",
        "drop_mixed": {
            "amb-neg-copper-prose-001-a": "Cu, о которой я говорил, лежит в шкафу",
            "amb-neg-hydrogen-prose-001-a": "H₂ у нас закончился",
            "amb-neg-not-correction-001-a": "не всё так просто, а вот NaCl мы уже обсудили",
        },
        "to_raw": {
            "amb-stress-unicode-001-a": {
                "was": "chemistry",
                "notes": (
                    "A substance named inside a sentence, with a comment after it. Under the "
                    "substance-in-prose policy it is mentioned, not dictated, and the words are "
                    "the right answer. The record still checks that a dash and guillemets do not "
                    "open a span of their own."
                ),
            },
        },
    },
    {
        "parent": "dev-seed-v2",
        "child": "dev-seed-v3",
        "created": "2026-09-15",
        "drop_mixed": {
            "raw-acid-storage-001-a": "H₂SO₄ хранится в лаборатории",
        },
    },
]


def canonical_line(record):
    return json.dumps(record, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def read(name):
    path = HERE / ("%s.jsonl" % name)
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]


def derive(spec):
    parent_records = read(spec["parent"])
    parent_manifest = json.loads(
        (HERE / ("%s.manifest.json" % spec["parent"])).read_text(encoding="utf-8")
    )
    expected = dict(spec["drop_mixed"])
    to_raw = dict(spec.get("to_raw", {}))
    child_records = []
    for record in parent_records:
        record = json.loads(json.dumps(record))  # deep copy, no shared state
        declared = expected.pop(record["id"], None)
        if declared is not None:
            actual = record.get("expected_mixed_output")
            if actual != declared:
                sys.exit(
                    "%s: %s declares %r, not the %r this script was written for"
                    % (spec["parent"], record["id"], actual, declared)
                )
            del record["expected_mixed_output"]
        change = to_raw.pop(record["id"], None)
        if change is not None:
            if record["target_action"] != "ast" or record["target_domain"] != change["was"]:
                sys.exit(
                    "%s: %s is not the %s formula this script was written for"
                    % (spec["parent"], record["id"], change["was"])
                )
            record["target_action"] = "raw"
            record["target_ast"] = None
            record["target_domain"] = "plain"
            record.pop("expected_render", None)
            record["notes"] = change["notes"]
        child_records.append(record)
    if expected or to_raw:
        sys.exit("%s: records not found: %s" % (spec["parent"], sorted(expected) + sorted(to_raw)))

    # Nothing but the named fields on the named records may differ.
    renamed = {"target_action", "target_ast", "target_domain", "expected_render", "notes"}
    for before, after in zip(parent_records, child_records):
        before = dict(before)
        after = dict(after)
        if before["id"] in spec["drop_mixed"]:
            before.pop("expected_mixed_output", None)
        if before["id"] in spec.get("to_raw", {}):
            for field in renamed:
                before.pop(field, None)
                after.pop(field, None)
        if before != after:
            sys.exit("%s: unintended change in %s" % (spec["parent"], after["id"]))

    text = "\n".join(canonical_line(r) for r in child_records) + "\n"
    (HERE / ("%s.jsonl" % spec["child"])).write_text(text, encoding="utf-8")

    families = sorted({r["family_id"] for r in child_records})
    manifest = dict(parent_manifest)
    manifest.update(
        {
            "corpus_id": spec["child"],
            "created": spec["created"],
            "file": "%s.jsonl" % spec["child"],
            "sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
            "records": len(child_records),
            "families": len(families),
            "counts_by_tag": dict(
                sorted(Counter(t for r in child_records for t in r["tags"]).items())
            ),
            "derived_from": {
                "corpus_id": parent_manifest["corpus_id"],
                "sha256": parent_manifest["sha256"],
                "by": "research/data/apply_prose_policy.py",
                "change": (
                    "A substance name inside ordinary prose is mentioned rather than dictated "
                    "and is no longer substituted. `expected_mixed_output` is removed from the "
                    "records under `records` (absent means the application returns the "
                    "sentence unchanged), and the records under `re_annotated_as_raw` had a "
                    "formula gold for a substance named inside a sentence and are ordinary "
                    "speech now. No other field of any record differs from the parent, and "
                    "the script refuses to write if one does."
                ),
                "records": dict(sorted(spec["drop_mixed"].items())),
                "re_annotated_as_raw": sorted(spec.get("to_raw", {})),
            },
        }
    )
    (HERE / ("%s.manifest.json" % spec["child"])).write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(
        "%s -> %s: %d records, %d changed, sha256 %s"
        % (
            spec["parent"],
            spec["child"],
            len(child_records),
            len(spec["drop_mixed"]) + len(spec.get("to_raw", {})),
            manifest["sha256"],
        )
    )


for spec in DERIVATIONS:
    derive(spec)

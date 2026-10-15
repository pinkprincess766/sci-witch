//! Would word-level repair fix the `asr-error` phrases, and what does it cost
//! in false scientific rewrites of ordinary speech?
//!
//! The rule under test is `sciwhisper_core::research::nearest_lexicon_words`
//! applied to each word of a transcript before `interpret`. It is not wired
//! into the product. `WORD_REPAIR_WRITE=1` rewrites
//! `research/results/word-repair-v1.json`; otherwise the file is compared.
//!
//! "Correct" is what the evaluation harness means by it (`evaluate.rs` of
//! `sciwhisper-eval`, deterministic baseline, `Domain::Auto`): the first
//! candidate the parser offers, kept only at confidence >= 0.9, otherwise the
//! words stay (`RAW`); the decision is correct when its canonical form equals
//! the canonical form of the corpus target. The core tests have no such
//! comparison, so it is restated here.

use std::collections::BTreeMap;

use sciwhisper_core::research::nearest_lexicon_words;
use sciwhisper_core::{interpret, Domain, InterpretOptions, Node};
use serde_json::{json, Value};

const CORPORA: [&str; 4] = [
    "dev-seed-v3",
    "ambiguous-v2",
    "nomenclature-v1",
    "prose-negatives-v2",
];

/// The split that stays closed.
const CLOSED_SPLIT: &str = "dev_holdout";

/// Same gate as `EvalConfig::auto_insert_threshold`.
const AUTO_INSERT_THRESHOLD: f32 = 0.9;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Variant {
    None,
    D1Unique,
    D2Unique,
}

const VARIANTS: [Variant; 3] = [Variant::None, Variant::D1Unique, Variant::D2Unique];

impl Variant {
    fn name(self) -> &'static str {
        match self {
            Variant::None => "none",
            Variant::D1Unique => "d1_unique",
            Variant::D2Unique => "d2_unique",
        }
    }

    /// Largest distance the variant replaces at; `None` replaces nothing.
    fn max_distance(self) -> usize {
        match self {
            Variant::None => 0,
            Variant::D1Unique => 1,
            Variant::D2Unique => 2,
        }
    }
}

struct Row {
    id: String,
    transcript: String,
    tags: Vec<String>,
    target_is_raw: bool,
    /// Canonical form of the target AST; `None` for a `raw` target.
    target_key: Option<String>,
}

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Rows of one corpus without `dev_holdout`, and how many were skipped. The
/// split is read first; a closed row's text is never looked at.
fn read_corpus(name: &str) -> (Vec<Row>, usize) {
    let path = root().join(format!("research/data/{name}.jsonl"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let mut rows = Vec::new();
    let mut skipped = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let row: Value = serde_json::from_str(line).expect("a JSON row");
        if row["split"].as_str().expect("split") == CLOSED_SPLIT {
            skipped += 1;
            continue;
        }
        let target_is_raw = match row["target_action"].as_str().expect("target_action") {
            "raw" => true,
            "ast" => false,
            other => panic!("unknown target_action {other}"),
        };
        let target_key = (!target_is_raw).then(|| {
            let node: Node = serde_json::from_value(row["target_ast"].clone()).expect("target_ast");
            canonical_key(&node)
        });
        rows.push(Row {
            id: row["id"].as_str().expect("id").to_string(),
            transcript: row["human_transcript"]
                .as_str()
                .expect("human_transcript")
                .to_string(),
            tags: row["tags"]
                .as_array()
                .expect("tags")
                .iter()
                .map(|t| t.as_str().expect("tag").to_string())
                .collect(),
            target_is_raw,
            target_key,
        });
    }
    (rows, skipped)
}

/// `canonical_node_v1` of the harness: sorted keys, `Document([x])` is `x`,
/// a plain decimal number uses `.` and has no leading `+`.
fn canonical_key(node: &Node) -> String {
    fn normalise(value: Value) -> Value {
        match value {
            Value::Object(map) => {
                if let Some(Value::Array(children)) = map.get("Document") {
                    if children.len() == 1 {
                        return normalise(children[0].clone());
                    }
                }
                if let (1, Some(Value::String(text))) = (map.len(), map.get("Number")) {
                    let unsigned = text.strip_prefix('+').unwrap_or(text);
                    let (integer, fraction) = match unsigned.split_once([',', '.']) {
                        Some((i, f)) => (i, Some(f)),
                        None => (unsigned, None),
                    };
                    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
                    let body = integer.strip_prefix('-').unwrap_or(integer);
                    if digits(body) && fraction.is_none_or(digits) {
                        let number = match fraction {
                            Some(f) => format!("{integer}.{f}"),
                            None => integer.to_string(),
                        };
                        return json!({ "Number": number });
                    }
                }
                Value::Object(map.into_iter().map(|(k, v)| (k, normalise(v))).collect())
            }
            Value::Array(items) => Value::Array(items.into_iter().map(normalise).collect()),
            other => other,
        }
    }
    normalise(serde_json::to_value(node).expect("serialisable")).to_string()
}

/// What the deterministic baseline emits for `text`: `Some(key)` for a
/// scientific AST, `None` for `RAW`.
fn emitted_key(text: &str) -> Option<String> {
    let result = interpret(
        text,
        InterpretOptions {
            domain: Domain::Auto,
            allow_shortcuts: true,
        },
    );
    // The first candidate: the primary parse if it is a structure, then the
    // parser's own alternatives (`Text` is not a structure).
    let primary = (result.confidence > 0.0 && !matches!(result.ast, Node::Text(_)))
        .then(|| result.ast.clone());
    let first = primary.or_else(|| {
        result
            .alternatives
            .iter()
            .find(|a| !matches!(a, Node::Text(_)))
            .cloned()
    })?;
    (result.confidence >= AUTO_INSERT_THRESHOLD).then(|| canonical_key(&first))
}

fn is_correct(row: &Row, text: &str) -> bool {
    emitted_key(text) == row.target_key
}

/// Maximal runs of letters and the text between them, in order. Joining the
/// pieces gives the input back.
fn pieces(text: &str) -> Vec<(bool, String)> {
    let mut out: Vec<(bool, String)> = Vec::new();
    for c in text.chars() {
        let is_word = c.is_alphabetic();
        match out.last_mut() {
            Some((kind, piece)) if *kind == is_word => piece.push(c),
            _ => out.push((is_word, c.to_string())),
        }
    }
    out
}

/// What a variant does with one word.
#[derive(Debug, PartialEq)]
enum Pick {
    Leave,
    Replace(String),
    Tie,
}

fn pick(word: &str, variant: Variant) -> Pick {
    match nearest_lexicon_words(word) {
        Some((distance, mut words)) if distance <= variant.max_distance() => {
            if words.len() == 1 {
                Pick::Replace(words.remove(0))
            } else {
                Pick::Tie
            }
        }
        _ => Pick::Leave,
    }
}

/// The repaired text, how many words were replaced, how many were left
/// alone because of a tie.
fn repair(text: &str, variant: Variant) -> (String, usize, usize) {
    let (mut replaced, mut ties) = (0, 0);
    let mut out = String::new();
    for (is_word, piece) in pieces(text) {
        match is_word.then(|| pick(&piece, variant)) {
            Some(Pick::Replace(word)) => {
                replaced += 1;
                out.push_str(&word);
            }
            Some(Pick::Tie) => {
                ties += 1;
                out.push_str(&piece);
            }
            _ => out.push_str(&piece),
        }
    }
    (out, replaced, ties)
}

fn report() -> Value {
    let mut corpora = BTreeMap::new();
    let mut asr_error_phrases = Vec::new();
    for name in CORPORA {
        let (rows, skipped) = read_corpus(name);
        let mut variants = BTreeMap::new();
        for variant in VARIANTS {
            let (mut correct, mut replaced, mut ties) = (0, 0, 0);
            let (mut false_rewrites, mut newly_correct, mut newly_wrong) = (0, 0, 0);
            let mut changed = Vec::new();
            for row in &rows {
                let before = is_correct(row, &row.transcript);
                let (text, r, t) = repair(&row.transcript, variant);
                let after = is_correct(row, &text);
                correct += usize::from(after);
                replaced += r;
                ties += t;
                false_rewrites += usize::from(row.target_is_raw && emitted_key(&text).is_some());
                newly_correct += usize::from(after && !before);
                newly_wrong += usize::from(before && !after);
                if text != row.transcript {
                    changed.push(json!({
                        "id": row.id,
                        "repaired": text,
                        "correct_before": before,
                        "correct_after": after,
                    }));
                }
            }
            variants.insert(
                variant.name(),
                json!({
                    "phrases": rows.len(),
                    "correct": correct,
                    "words_replaced": replaced,
                    "ties_left": ties,
                    "false_scientific_rewrites": false_rewrites,
                    "newly_correct": newly_correct,
                    "newly_wrong": newly_wrong,
                    "changed_phrases": changed,
                }),
            );
        }
        corpora.insert(
            name,
            json!({ "dev_holdout_skipped": skipped, "variants": variants }),
        );
        if name == "ambiguous-v2" {
            for row in rows
                .iter()
                .filter(|r| r.tags.iter().any(|t| t == "asr-error"))
            {
                let per_variant: BTreeMap<_, _> = VARIANTS
                    .iter()
                    .map(|&v| {
                        let (text, _, _) = repair(&row.transcript, v);
                        let correct = is_correct(row, &text);
                        (v.name(), json!({ "text": text, "correct": correct }))
                    })
                    .collect();
                asr_error_phrases.push(json!({
                    "id": row.id,
                    "human_transcript": row.transcript,
                    "target_action": if row.target_is_raw { "raw" } else { "ast" },
                    "variants": per_variant,
                }));
            }
        }
    }
    json!({
        "report": "word-repair-v1",
        "rule": {
            "function": "sciwhisper_core::research::nearest_lexicon_words",
            "max_word_edits": sciwhisper_core::research::MAX_WORD_EDITS,
            "min_corrected_word_chars": sciwhisper_core::research::MIN_CORRECTED_WORD_CHARS,
            "domain": "auto",
            "auto_insert_threshold": AUTO_INSERT_THRESHOLD,
        },
        "corpora": corpora,
        "asr_error_phrases": asr_error_phrases,
        "note": "Each letter-run of human_transcript is replaced by its nearest lexicon word before interpret, at distance 1 (d1_unique) or at most 2 (d2_unique), only when exactly one word is nearest. Ties are never resolved: ties_left counts the words left alone because several lexicon words were equally near within the variant's limit. Correct means the harness's deterministic decision (first candidate, confidence at least 0.9, otherwise the words stay) equals the corpus target. The dev_holdout split was skipped before its text was read, so this is an evaluation of a fixed rule on the other splits, not a fit. The rule is research code and is not wired into lattice or interpret.",
    })
}

fn result_path() -> std::path::PathBuf {
    root().join("research/results/word-repair-v1.json")
}

#[test]
fn word_repair_report_matches_the_committed_file() {
    let fresh = report();
    let path = result_path();
    if std::env::var_os("WORD_REPAIR_WRITE").is_some_and(|v| v == "1") {
        let text = serde_json::to_string_pretty(&fresh).expect("serialisable") + "\n";
        std::fs::write(&path, text).expect("write the report");
        return;
    }
    let committed: Value = serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}")),
    )
    .expect("the committed report is JSON");
    for name in CORPORA {
        for variant in VARIANTS {
            let at = |v: &Value| v["corpora"][name]["variants"][variant.name()].clone();
            let (a, b) = (at(&committed), at(&fresh));
            for key in ["ties_left", "words_replaced", "phrases", "correct"] {
                assert_eq!(a[key], b[key], "{name} {} {key}", variant.name());
            }
        }
    }
    assert_eq!(committed, fresh);
}

#[test]
fn none_variant_matches_the_plain_interpreter() {
    // A direct loop with no repair code on the path: `interpret` on the
    // transcript as read from the file.
    for name in CORPORA {
        let (rows, _) = read_corpus(name);
        let direct = rows
            .iter()
            .filter(|row| emitted_key(&row.transcript) == row.target_key)
            .count();
        let (mut via_repair, mut touched) = (0, 0);
        for row in &rows {
            let (text, replaced, ties) = repair(&row.transcript, Variant::None);
            touched += replaced + ties + usize::from(text != row.transcript);
            via_repair += usize::from(is_correct(row, &text));
        }
        assert_eq!(touched, 0, "{name}: `none` changed a transcript");
        assert_eq!(via_repair, direct, "{name}");
        let committed: Value =
            serde_json::from_str(&std::fs::read_to_string(result_path()).expect("report"))
                .expect("JSON");
        assert_eq!(
            committed["corpora"][name]["variants"]["none"]["correct"],
            json!(direct),
            "{name}: committed `none` differs from the plain interpreter"
        );
    }
}

#[test]
fn no_variant_touches_a_vocabulary_word() {
    // A word the research function has no answer for (a vocabulary word, a
    // short word, a far word) must come out byte for byte. Checked by diffing
    // the pieces of the input and of the output, not by the repair code.
    for known in ["гидроксид", "хлорид", "серная", "кислота", "натрия"]
    {
        assert_eq!(nearest_lexicon_words(known), None, "{known}");
    }
    let (mut words, mut replaced) = (0, 0);
    for name in CORPORA {
        let (rows, _) = read_corpus(name);
        for row in &rows {
            for variant in VARIANTS {
                let (text, _, _) = repair(&row.transcript, variant);
                let (before, after) = (pieces(&row.transcript), pieces(&text));
                assert_eq!(before.len(), after.len(), "{}: pieces moved", row.id);
                for ((is_word, old), (_, new)) in before.iter().zip(&after) {
                    if !is_word {
                        assert_eq!(old, new, "{}: a separator changed", row.id);
                        continue;
                    }
                    words += 1;
                    if old == new {
                        continue;
                    }
                    replaced += 1;
                    let (distance, near) = nearest_lexicon_words(old)
                        .unwrap_or_else(|| panic!("{}: «{old}» has no answer", row.id));
                    assert!(distance <= variant.max_distance(), "{}: «{old}»", row.id);
                    assert_eq!(near, vec![new.clone()], "{}: «{old}»", row.id);
                }
            }
        }
    }
    // A loop that saw nothing would report "no vocabulary word touched".
    assert!(words > 1000, "only {words} words read");
    assert!(replaced > 0, "no word was replaced at all");
}

#[test]
fn closed_split_is_skipped_and_counted() {
    let expected = [
        ("dev-seed-v3", 21),
        ("ambiguous-v2", 9),
        ("nomenclature-v1", 0),
        ("prose-negatives-v2", 34),
    ];
    for (name, closed) in expected {
        let (rows, skipped) = read_corpus(name);
        let total = std::fs::read_to_string(root().join(format!("research/data/{name}.jsonl")))
            .expect("corpus")
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count();
        assert_eq!(skipped, closed, "{name}");
        assert_eq!(rows.len() + skipped, total, "{name}");
    }
}

#[test]
fn a_tie_is_left_alone_and_counted() {
    // «кальции» is one edit from «кальций» and «кальция»; «между» is two from
    // «меди» and «медь»; «сернея» has one nearest word, «серная».
    let tie = |text, variant| repair(text, variant);
    assert_eq!(tie("кальции", Variant::D1Unique), ("кальции".into(), 0, 1));
    assert_eq!(tie("кальции", Variant::D2Unique), ("кальции".into(), 0, 1));
    assert_eq!(tie("между", Variant::D1Unique), ("между".into(), 0, 0));
    assert_eq!(tie("между", Variant::D2Unique), ("между".into(), 0, 1));
    assert_eq!(tie("сернея", Variant::D1Unique), ("серная".into(), 1, 0));
    assert_eq!(tie("сернея", Variant::None), ("сернея".into(), 0, 0));
}

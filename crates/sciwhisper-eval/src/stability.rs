//! Does the decoder move when the meaning moves, and stay still when it does
//! not?
//!
//! ADR-0003 proposed a Lipschitz bound, `d_Y(f(x), f(x')) ≤ L·d_X(x, x')`,
//! and the constant is the part that was dropped. There is no natural
//! `d_X` on spoken Russian — word-level edit distance? phoneme-level? — so
//! any `L` would be chosen by eye and could never be falsified.
//!
//! What survives is the two-sided property, which has teeth and needs no
//! `d_X` at all:
//!
//! * **invariance** — a perturbation that does not change the meaning must
//!   not change the answer. Asserted as `d_sci = 0` exactly, which is both
//!   stronger than a bound and cheaper to check.
//! * **equivariance** — a controlled change of meaning must move the answer,
//!   and move it *locally*: «железа два» → «железа три» must land nearer to
//!   where it started than an unrelated substance does.
//!
//! Both are checked over the corpus rather than a hand-picked list, so the
//! coverage grows with the corpus instead of with somebody's memory.

#![cfg(test)]

use sciwhisper_core::{
    interpret, interpret_utterance, render, Domain, InterpretOptions, Node, Renderer,
    UtteranceMode, UtteranceOptions,
};
use serde_json::Value;

use crate::distance::distance;
use crate::schema::Dataset;

fn corpus() -> Dataset {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../research/data/dev-seed-v2.jsonl"
    );
    let text = std::fs::read_to_string(path).expect("dev-seed-v2.jsonl must exist");
    Dataset::parse_jsonl(&text).expect("the corpus must load")
}

/// The AST the decoder produces, or `None` when it declines to produce one.
fn answer(spoken: &str) -> Option<Value> {
    let result = interpret(
        spoken,
        InterpretOptions {
            domain: Domain::Auto,
            ..Default::default()
        },
    );
    if result.confidence <= 0.0 || matches!(result.ast, Node::Text(_)) {
        return None;
    }
    serde_json::to_value(&result.ast).ok()
}

/// Perturbations that change how something is written, not what it means.
///
/// Each is a claim the rest of the system already makes: the normaliser
/// folds case, treats `ё` and `е` as the same letter, and collapses
/// whitespace. This is where those claims are tested end to end rather than
/// unit by unit.
fn meaning_preserving(spoken: &str) -> Vec<(&'static str, String)> {
    let mut out = vec![
        ("верхний регистр", spoken.to_uppercase()),
        ("нижний регистр", spoken.to_lowercase()),
        ("пробелы по краям", format!("  {spoken}  ")),
        ("двойные пробелы", spoken.replace(' ', "  ")),
    ];
    if spoken.contains('ё') {
        out.push(("ё как е", spoken.replace('ё', "е")));
    }
    if spoken.contains('Ё') {
        out.push(("Ё как Е", spoken.replace('Ё', "Е")));
    }
    out
}

#[test]
fn writing_the_same_thing_differently_gives_the_same_answer() {
    let corpus = corpus();
    let mut checked = 0usize;
    let mut failures = Vec::new();
    for record in &corpus.records {
        let Some(original) = answer(&record.human_transcript) else {
            // Nothing compiled, so there is no answer to be stable about.
            continue;
        };
        for (what, perturbed) in meaning_preserving(&record.human_transcript) {
            checked += 1;
            match answer(&perturbed) {
                Some(other) => {
                    let moved = distance(&original, &other);
                    if moved != 0.0 {
                        failures.push(format!(
                            "  {} [{what}] {:?} → расстояние {moved}",
                            record.id, perturbed
                        ));
                    }
                }
                None => failures.push(format!(
                    "  {} [{what}] {:?} перестало разбираться",
                    record.id, perturbed
                )),
            }
        }
    }
    assert!(checked > 200, "only {checked} perturbations were checked");
    assert!(
        failures.is_empty(),
        "{} из {checked} безобидных перестановок сдвинули ответ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// A family is a set of paraphrases of one construction, and the corpus
/// gives them all the same gold AST. The decoder must agree: saying it
/// another way is not saying another thing.
#[test]
fn paraphrases_of_one_construction_reach_one_answer() {
    let corpus = corpus();
    let mut by_family: std::collections::BTreeMap<&str, Vec<&crate::schema::Record>> =
        std::collections::BTreeMap::new();
    for record in &corpus.records {
        by_family
            .entry(record.family_id.as_str())
            .or_default()
            .push(record);
    }

    let mut compared = 0usize;
    let mut failures = Vec::new();
    for (family, records) in by_family {
        let answers: Vec<(&str, Value)> = records
            .iter()
            .filter_map(|record| {
                answer(&record.human_transcript)
                    .map(|value| (record.human_transcript.as_str(), value))
            })
            .collect();
        for pair in answers.windows(2) {
            compared += 1;
            let moved = distance(&pair[0].1, &pair[1].1);
            if moved != 0.0 {
                failures.push(format!(
                    "  {family}: {:?} и {:?} разошлись на {moved}",
                    pair[0].0, pair[1].0
                ));
            }
        }
    }
    assert!(
        compared > 0,
        "the corpus has no paraphrase pairs to compare"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The other side of the property. A perturbation that *does* change the
/// meaning must change the answer — otherwise "the decoder is stable" would
/// be satisfied by a decoder that ignores its input.
#[test]
fn a_change_of_meaning_changes_the_answer() {
    let pairs = [
        ("вода", "перекись водорода"),
        ("гидроксид железа два", "гидроксид железа три"),
        ("икс в квадрате", "икс в кубе"),
        ("синус икс", "косинус икс"),
        ("сто паскалей", "сто кельвинов"),
    ];
    for (before, after) in pairs {
        let (a, b) = (
            answer(before).unwrap_or_else(|| panic!("{before:?} did not compile")),
            answer(after).unwrap_or_else(|| panic!("{after:?} did not compile")),
        );
        assert!(
            distance(&a, &b) > 0.0,
            "{before:?} and {after:?} mean different things but gave the same answer"
        );
    }
}

/// Equivariance, stated as an ordering rather than a magnitude.
///
/// A magnitude assertion would only read this project's own weights back.
/// An ordering says something about the decoder: a small controlled change
/// of meaning must land nearer to where it started than an unrelated answer
/// does. That is what makes the space usable for anything later — nearest
/// neighbours, ambiguity, ranking.
#[test]
fn a_small_change_of_meaning_lands_nearer_than_an_unrelated_answer() {
    let cases = [
        // (starting point, one step away, somewhere else entirely)
        ("вода", "перекись водорода", "серная кислота"),
        ("гидроксид железа два", "гидроксид железа три", "метан"),
        ("икс в квадрате", "икс в кубе", "синус икс"),
        ("три кельвина", "четыре кельвина", "икс в квадрате"),
    ];
    for (origin, near, far) in cases {
        let get =
            |spoken: &str| answer(spoken).unwrap_or_else(|| panic!("{spoken:?} did not compile"));
        let (origin_ast, near_ast, far_ast) = (get(origin), get(near), get(far));
        let close = distance(&origin_ast, &near_ast);
        let distant = distance(&origin_ast, &far_ast);
        assert!(
            close < distant,
            "{origin:?}→{near:?} is {close}, but {origin:?}→{far:?} is only {distant}"
        );
    }
}

/// What the shipped decoder puts on screen, with whitespace collapsed so a
/// perturbation that only added spaces does not look like a rewrite.
fn inserted(spoken: &str) -> String {
    let result = interpret_utterance(
        spoken,
        UtteranceOptions {
            domain: Domain::Auto,
            mode: UtteranceMode::MixedText,
            allow_shortcuts: true,
        },
    );
    render(&result.document, Renderer::Unicode)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The decoder must not be stable in the one place stability would be a
/// defect — but the question here is narrower and unambiguous: whatever the
/// shipped decoder decides about a sentence, it must decide the same thing
/// when the sentence is written in capitals or with doubled spaces.
///
/// What it *should* decide is a separate question, and a contested one — see
/// `the_lab_and_the_application_agree_about_all_but_one_record` below.
#[test]
fn ordinary_speech_is_treated_the_same_however_it_is_written() {
    let corpus = corpus();
    let mut checked = 0usize;
    let mut failures = Vec::new();
    for record in &corpus.records {
        if record.target_action != crate::schema::TargetAction::Raw {
            continue;
        }
        let original = inserted(&record.human_transcript);
        for (what, perturbed) in meaning_preserving(&record.human_transcript) {
            checked += 1;
            let produced = inserted(&perturbed);
            // Case folding is part of the perturbation, so the comparison
            // is case-insensitive; what must not change is the decision.
            if produced.to_lowercase() != original.to_lowercase() {
                failures.push(format!(
                    "  {} [{what}] {:?} → {:?}, а без перестановки → {:?}",
                    record.id, perturbed, produced, original
                ));
            }
        }
    }
    assert!(
        checked > 100,
        "only {checked} ordinary sentences were perturbed"
    );
    assert!(
        failures.is_empty(),
        "{} из {checked} перестановок изменили решение:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// **The lab and the application do not run the same decoder.**
///
/// `candidates.rs` calls `interpret`, which either parses the whole
/// utterance or offers nothing the decision layer will accept.
/// `pipeline.rs` — the code a user actually runs — calls
/// `interpret_utterance`, which in its default `MixedText` mode keeps the
/// sentence and replaces the spans it can prove *inside* it. Span
/// replacement has no counterpart in the lab at all.
///
/// On this corpus they agree about every ordinary sentence but one:
///
/// | | lab (`interpret`) | application (`interpret_utterance`) |
/// |---|---|---|
/// | «серная кислота хранится в лаборатории» | words kept | «H₂SO₄ хранится в лаборатории» |
///
/// That matters because `false_scientific_rewrite_rate` is a headline safety
/// number and a 0.5 release gate. Measured on `interpret` it is 0/28;
/// measured on the path that ships it is 1/28 — if the corpus's intent for
/// that record is taken at face value, and its `substance-mentioned` tag
/// says it should be.
///
/// Which behaviour is right is a product decision, not one this test can
/// make: `NATURAL_DICTATION_RU.md` asks `MixedText` to replace proven spans
/// inside prose, and this is exactly that feature working. The test does not
/// take a side. It pins the divergence at the single record it is, so that
/// it cannot grow quietly while a gate reports zero.
#[test]
fn the_lab_and_the_application_agree_about_all_but_one_record() {
    let corpus = corpus();
    let mut rewritten = Vec::new();
    let mut ordinary = 0usize;
    for record in &corpus.records {
        if record.target_action != crate::schema::TargetAction::Raw {
            continue;
        }
        ordinary += 1;
        // The lab's view is recorded in the report as `raw_accuracy` 28/28:
        // its decision layer keeps every one of these sentences. It is not
        // re-derived here, because the point of this test is the other side.
        let produced = inserted(&record.human_transcript);
        let kept: String = record
            .human_transcript
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if produced != kept {
            rewritten.push(format!("{}: {:?} → {:?}", record.id, kept, produced));
        }
    }
    // 29 since «феррит бария» was corrected from a demanded formula to an
    // ambiguous name whose right answer is to keep the words.
    assert_eq!(ordinary, 29, "the corpus changed size; re-check this pin");
    assert_eq!(
        rewritten,
        vec![
            "raw-acid-storage-001-a: \"серная кислота хранится в лаборатории\" → \"H₂SO₄ хранится в лаборатории\"".to_string()
        ],
        "the two decoders now disagree about a different set of sentences than when this was \
         measured. Either the application changed, or the lab did, and the safety metric no \
         longer means what the release gate thinks it means."
    );
}

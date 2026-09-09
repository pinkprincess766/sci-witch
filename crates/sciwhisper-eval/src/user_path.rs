//! What the **application** does, measured separately from what the lab
//! measures.
//!
//! `candidates.rs` calls `interpret`: the whole utterance parses or it does
//! not. `pipeline.rs` — the code a user actually runs — calls
//! `interpret_utterance` in `MixedText`, which keeps the sentence and
//! substitutes the spans it can prove. Span substitution has no counterpart
//! in the lab at all, so a safety number measured there is not a safety
//! number about the product.
//!
//! Both are now reported, side by side and never added together. Where they
//! disagree, that disagreement is the finding.

use serde::Serialize;

use sciwhisper_core::{
    interpret_utterance, render, Domain, Renderer, UtteranceMode, UtteranceOptions,
};

use crate::metrics::{proportion, Proportion};
use crate::schema::{Record, TargetAction};

#[derive(Clone, Debug, Serialize)]
pub struct UserPathReport {
    /// Records that explicitly override what the application should show.
    /// AST records without an override derive their expected Unicode from
    /// the hand-written target AST and are still included below.
    pub declared_expectations: usize,
    /// Every selected record exercised through the shipped mixed-text path.
    pub evaluated_records: usize,
    /// Of all evaluated records, how many matched the expected document.
    pub mixed_exact_match: Proportion,
    /// Ordinary speech that the application turned into something else.
    ///
    /// Counted over `raw` records whose expected document is the original
    /// transcript. A record may explicitly declare a deliberate inline
    /// substitution; that is still scored by exact match, but is not a false
    /// rewrite by the corpus policy.
    pub false_scientific_rewrite_rate: Proportion,
    /// Every disagreement, so the number can be read rather than trusted.
    pub rewritten: Vec<Rewritten>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Rewritten {
    pub id: String,
    pub said: String,
    pub shown: String,
}

/// The document the application would insert, whitespace-normalised so a
/// difference in spacing is not read as a rewrite.
pub fn shown_to_user(text: &str) -> String {
    let result = interpret_utterance(
        text,
        UtteranceOptions {
            domain: Domain::Auto,
            mode: UtteranceMode::MixedText,
            allow_shortcuts: true,
        },
    );
    normalise(&render(&result.document, Renderer::Unicode))
}

fn normalise(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Measures the records the run actually selected.
///
/// Taking the whole dataset made `--split validation` report the same
/// numbers as `--split all`: the lab metrics narrowed and this one did not,
/// so a five-record split silently published a twenty-seven-record result.
pub fn evaluate(records: &[&Record]) -> UserPathReport {
    let mut declared = 0usize;
    let mut exact_hits = 0usize;
    let mut unchanged_expected = 0usize;
    let mut unexpected_rewrites = 0usize;
    let mut rewritten = Vec::new();

    for record in records {
        let shown = shown_to_user(&record.human_transcript);
        if record.expected_mixed_output.is_some() {
            declared += 1;
        }
        let expected = expected_output(record);
        let expects_unchanged = record.target_action == TargetAction::Raw
            && expected == normalise(&record.human_transcript);
        if expects_unchanged {
            unchanged_expected += 1;
        }
        if shown == expected {
            exact_hits += 1;
        } else {
            if expects_unchanged {
                unexpected_rewrites += 1;
            }
            rewritten.push(Rewritten {
                id: record.id.clone(),
                said: record.human_transcript.clone(),
                shown,
            });
        }
    }

    UserPathReport {
        declared_expectations: declared,
        evaluated_records: records.len(),
        mixed_exact_match: proportion(exact_hits, records.len()),
        false_scientific_rewrite_rate: proportion(unexpected_rewrites, unchanged_expected),
        rewritten,
    }
}

fn expected_output(record: &Record) -> String {
    if let Some(expected) = &record.expected_mixed_output {
        return normalise(expected);
    }
    match (&record.target_action, &record.target_ast) {
        (TargetAction::Ast, Some(ast)) => normalise(&render(ast, Renderer::Unicode)),
        _ => normalise(&record.human_transcript),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus(lines: &[String]) -> crate::schema::Dataset {
        crate::schema::Dataset::parse_jsonl(&lines.join("\n")).expect("must load")
    }

    fn all(dataset: &crate::schema::Dataset) -> Vec<&Record> {
        dataset.records.iter().collect()
    }

    fn raw_record(id: &str, text: &str, expected: Option<&str>) -> String {
        let extra = match expected {
            Some(value) => format!(r#","expected_mixed_output":"{value}""#),
            None => String::new(),
        };
        format!(
            r#"{{"dataset_schema_version":1,"id":"{id}-a","family_id":"{id}","provenance":"handcrafted_text","human_transcript":"{text}","asr_hypotheses":[],"target_domain":"plain","target_action":"raw","target_ast":null,"split":"train","tags":[],"speaker_id":null{extra}}}"#
        )
    }

    fn ast_record(id: &str, text: &str) -> String {
        format!(
            r#"{{"dataset_schema_version":1,"id":"{id}-a","family_id":"{id}","provenance":"handcrafted_text","human_transcript":"{text}","asr_hypotheses":[],"target_domain":"chemistry","target_action":"ast","target_ast":{{"Chemical":{{"Species":{{"coefficient":1,"formula":{{"parts":[{{"Atom":{{"symbol":"H","count":2}}}},{{"Atom":{{"symbol":"O","count":1}}}}]}},"charge":null,"marker":null}}}}}},"split":"train","tags":[],"speaker_id":null}}"#
        )
    }

    /// Ordinary speech that the application leaves alone.
    #[test]
    fn a_sentence_the_application_does_not_touch_is_not_a_rewrite() {
        let dataset = corpus(&[raw_record("prose", "феррит оказался нестабильным", None)]);
        let report = evaluate(&all(&dataset));
        assert_eq!(report.false_scientific_rewrite_rate.numerator, 0);
        assert_eq!(report.false_scientific_rewrite_rate.denominator, 1);
        assert!(report.rewritten.is_empty());
    }

    /// A record that states what the application should show is scored
    /// against that, not against its own words. Collapsing it to `RAW` would
    /// mark a correct substitution wrong.
    #[test]
    fn a_declared_document_is_scored_as_a_document() {
        let dataset = corpus(&[raw_record(
            "mention",
            "сульфат меди был куплен вчера",
            Some("CuSO₄ был куплен вчера"),
        )]);
        let report = evaluate(&all(&dataset));
        assert_eq!(report.declared_expectations, 1);
        assert_eq!(report.evaluated_records, 1);
        assert_eq!(report.mixed_exact_match.numerator, 1);
        // It made a claim, so it is not counted in the unclaimed-raw rate.
        assert_eq!(report.false_scientific_rewrite_rate.denominator, 0);
    }

    /// Positive scientific examples are part of the end-to-end metric even
    /// when the corpus does not repeat the Unicode rendering by hand.
    #[test]
    fn an_ast_record_derives_its_expected_document_from_the_gold_tree() {
        let dataset = corpus(&[ast_record("water", "вода")]);
        let report = evaluate(&all(&dataset));
        assert_eq!(report.declared_expectations, 0);
        assert_eq!(report.evaluated_records, 1);
        assert_eq!(report.mixed_exact_match.numerator, 1);
        assert_eq!(report.mixed_exact_match.denominator, 1);
    }

    /// The same sentence without a claim is a rewrite, and is named.
    #[test]
    fn an_unclaimed_substitution_is_counted_and_named() {
        let dataset = corpus(&[raw_record("mention", "сульфат меди был куплен вчера", None)]);
        let report = evaluate(&all(&dataset));
        assert_eq!(report.false_scientific_rewrite_rate.numerator, 1);
        assert_eq!(report.rewritten.len(), 1);
        assert_eq!(report.rewritten[0].shown, "CuSO₄ был куплен вчера");
    }

    /// A declared document that does not match is reported, so the metric
    /// cannot be satisfied by declaring whatever the code happens to do.
    #[test]
    fn a_wrong_declaration_fails_rather_than_being_absorbed() {
        let dataset = corpus(&[raw_record(
            "mention",
            "сульфат меди был куплен вчера",
            Some("совсем не то"),
        )]);
        let report = evaluate(&all(&dataset));
        assert_eq!(report.mixed_exact_match.numerator, 0);
        assert_eq!(report.mixed_exact_match.denominator, 1);
    }

    /// The metric must narrow with the split it was asked for. Handing it
    /// the whole dataset made `--split validation` publish the numbers of
    /// `--split all`.
    #[test]
    fn only_the_selected_records_are_measured() {
        let dataset = corpus(&[
            raw_record("a", "феррит оказался нестабильным", None),
            raw_record("b", "координационное число неизвестно", None),
            raw_record("c", "лиганд был выбран заранее", None),
        ]);
        assert_eq!(
            evaluate(&all(&dataset))
                .false_scientific_rewrite_rate
                .denominator,
            3
        );
        assert_eq!(evaluate(&all(&dataset)).mixed_exact_match.denominator, 3);

        let one: Vec<&Record> = dataset.records.iter().take(1).collect();
        assert_eq!(evaluate(&one).false_scientific_rewrite_rate.denominator, 1);
        assert_eq!(evaluate(&one).mixed_exact_match.denominator, 1);
    }
}

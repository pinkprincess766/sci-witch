//! Admission gates, evaluated rather than read.
//!
//! A release checklist with a line like «точность не ниже 90%» is a line a
//! person ticks. This module makes it a program that reads the report and
//! refuses.
//!
//! Three rules give the gates their teeth, and each one exists because the
//! obvious alternative would let a release through on evidence that does not
//! support it:
//!
//! 1. **Bounds, not point estimates.** 90% of twenty examples is not 90%.
//!    Every proportional gate is judged on the end of the confidence
//!    interval that could embarrass us — the lower bound for an accuracy,
//!    the upper bound for a failure rate.
//! 2. **An unmet precondition is not a pass.** A gate about speech,
//!    evaluated on a corpus of written text, reports *not measurable*, and
//!    that blocks the release exactly like a failure. Otherwise 98% on
//!    handcrafted sentences would quietly become a claim about microphones.
//! 3. **A zero is judged on its upper bound.** Zero false rewrites in 28
//!    sentences is compatible with a true rate of 10%. Shipping on that
//!    would be claiming something nobody measured.
//!
//! The gates themselves live in `research/schema/release-gates-v1.json`, so
//! changing a threshold is a visible edit to a versioned file rather than a
//! constant somebody moved.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const GATES_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize)]
pub struct GateFile {
    pub gates_schema_version: u32,
    pub profile: String,
    #[serde(default)]
    pub gates: Vec<Gate>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Gate {
    pub id: String,
    pub title: String,
    /// Dotted path into the report JSON, e.g.
    /// `metrics.pre_insertion_end_to_end_exact_match.unicode`.
    pub metric: String,
    pub judge: Judge,
    pub comparison: Comparison,
    #[serde(default)]
    pub threshold: f64,
    #[serde(default)]
    pub requires: Requirements,
    #[serde(default)]
    pub rationale: String,
}

/// Which number of a measurement the gate reads.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Judge {
    /// The optimistic end. Used for accuracies, where the question is "can
    /// the data support at least this?"
    Ci95Low,
    /// The pessimistic end. Used for failure rates, where the question is
    /// "could the true rate still be worse than this?"
    Ci95High,
    /// The value itself. For counts and flags, which have no interval.
    Value,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    AtLeast,
    AtMost,
    IsTrue,
}

/// What a corpus must be before a gate about it means anything.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Requirements {
    /// Minimum count per provenance, e.g. `{"real_audio": 200}`.
    #[serde(default)]
    pub provenance_at_least: BTreeMap<String, u64>,
    #[serde(default)]
    pub min_speakers: Option<u64>,
    /// Whether the corpus must be the sealed frozen test.
    #[serde(default)]
    pub frozen: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    Pass {
        observed: String,
    },
    Fail {
        observed: String,
        reason: String,
    },
    /// The corpus cannot support this gate. Blocks a release just like a
    /// failure; kept as its own outcome because the fix is different —
    /// nothing about the software will change it.
    NotMeasurable {
        reason: String,
    },
}

impl Outcome {
    pub fn admits_release(&self) -> bool {
        matches!(self, Outcome::Pass { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Outcome::Pass { .. } => "ПРОЙДЕНО",
            Outcome::Fail { .. } => "НЕ ПРОЙДЕНО",
            Outcome::NotMeasurable { .. } => "НЕ ИЗМЕРИМО",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct GateResult {
    pub id: String,
    pub title: String,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Clone, Debug, Serialize)]
pub struct GateReport {
    pub profile: String,
    pub results: Vec<GateResult>,
}

impl GateReport {
    pub fn admits_release(&self) -> bool {
        self.results
            .iter()
            .all(|result| result.outcome.admits_release())
    }

    pub fn blocking(&self) -> Vec<&GateResult> {
        self.results
            .iter()
            .filter(|result| !result.outcome.admits_release())
            .collect()
    }
}

impl fmt::Display for GateReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "ворота допуска: {}", self.profile)?;
        for result in &self.results {
            let detail = match &result.outcome {
                Outcome::Pass { observed } => observed.clone(),
                Outcome::Fail { observed, reason } => format!("{observed} — {reason}"),
                Outcome::NotMeasurable { reason } => reason.clone(),
            };
            writeln!(
                f,
                "  {:<12} {:<24} {}",
                result.outcome.label(),
                result.id,
                detail
            )?;
        }
        Ok(())
    }
}

/// A sealed corpus: the file a release was measured on, named by digest so
/// that editing it after the fact is visible.
///
/// The seal is written **once**, before the numbers are known. A frozen test
/// that can be re-sealed after seeing a result is not frozen — it is a test
/// that gets adjusted until it agrees, which is the failure mode the whole
/// research protocol exists to prevent. [`seal`] therefore refuses to
/// overwrite.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FrozenSeal {
    pub file: String,
    pub sha256: String,
    #[serde(default)]
    pub sealed_at: String,
    #[serde(default)]
    pub sealed_for: String,
}

/// Seals `dataset` as the frozen test for `release`.
///
/// Refuses if a seal already exists, whatever it says. Re-sealing is not an
/// operation this program offers: a corpus that was already frozen and now
/// needs freezing again is a different corpus, and it needs a different file
/// and a visible decision to use it.
pub fn seal(
    dataset: &std::path::Path,
    out: &std::path::Path,
    release: &str,
    today: &str,
) -> Result<FrozenSeal, String> {
    if out.exists() {
        return Err(format!(
            "{} уже существует. Перепечатывание frozen test не предусмотрено: тест, который \
             можно запечатать заново после того, как результат стал известен, — это тест, \
             который подгоняют. Если корпус действительно другой, дайте ему другое имя.",
            out.display()
        ));
    }
    let bytes =
        std::fs::read(dataset).map_err(|error| format!("{}: {error}", dataset.display()))?;
    // The corpus must at least load before it can be frozen; sealing an
    // unparsable file would produce a digest nobody can ever evaluate.
    let text =
        String::from_utf8(bytes.clone()).map_err(|_| format!("{} не UTF-8", dataset.display()))?;
    let parsed = crate::schema::Dataset::parse_jsonl(&text).map_err(|error| error.to_string())?;
    let audit = crate::split::audit_splits(&parsed);
    if !audit.clean {
        return Err(format!(
            "{} не проходит проверку split: {} семейств и {} дикторов пересекают split. \
             Запечатывать корпус с утечкой — значит зафиксировать её навсегда.",
            dataset.display(),
            audit.leaking_families.len(),
            audit.leaking_speakers.len()
        ));
    }

    let sealed = FrozenSeal {
        file: dataset
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| dataset.display().to_string()),
        sha256: crate::report::sha256_hex(&bytes),
        sealed_at: today.to_string(),
        sealed_for: release.to_string(),
    };
    let json = serde_json::to_string_pretty(&sealed).map_err(|error| error.to_string())?;
    std::fs::write(out, format!("{json}\n"))
        .map_err(|error| format!("{}: {error}", out.display()))?;
    Ok(sealed)
}

/// Runs every gate against one report.
///
/// `seal` is the sealed frozen test, if there is one. `None` means no corpus
/// has been sealed yet, and every gate that requires one reports *not
/// measurable* rather than being skipped.
pub fn evaluate(
    file: &GateFile,
    report: &serde_json::Value,
    seal: Option<&FrozenSeal>,
) -> Result<GateReport, String> {
    if file.gates_schema_version != GATES_SCHEMA_VERSION {
        return Err(format!(
            "gates_schema_version {} не поддерживается этой сборкой (нужна {GATES_SCHEMA_VERSION})",
            file.gates_schema_version
        ));
    }
    let results = file
        .gates
        .iter()
        .map(|gate| GateResult {
            id: gate.id.clone(),
            title: gate.title.clone(),
            outcome: evaluate_one(gate, report, seal),
        })
        .collect();
    Ok(GateReport {
        profile: file.profile.clone(),
        results,
    })
}

fn evaluate_one(gate: &Gate, report: &serde_json::Value, seal: Option<&FrozenSeal>) -> Outcome {
    if let Some(reason) = unmet_requirement(&gate.requires, report, seal) {
        return Outcome::NotMeasurable { reason };
    }
    let Some(node) = lookup(report, &gate.metric) else {
        return Outcome::NotMeasurable {
            reason: format!("в отчёте нет {}", gate.metric),
        };
    };
    match gate.comparison {
        Comparison::IsTrue => match node.as_bool() {
            Some(true) => Outcome::Pass {
                observed: format!("{} = true", gate.metric),
            },
            Some(false) => Outcome::Fail {
                observed: format!("{} = false", gate.metric),
                reason: "требуется true".into(),
            },
            None => Outcome::NotMeasurable {
                reason: format!("{} не логическое значение", gate.metric),
            },
        },
        Comparison::AtLeast | Comparison::AtMost => {
            let Some((observed, note)) = judged_value(node, gate.judge) else {
                return Outcome::NotMeasurable {
                    reason: format!("{} не содержит {:?}", gate.metric, gate.judge),
                };
            };
            let ok = match gate.comparison {
                Comparison::AtLeast => observed >= gate.threshold,
                Comparison::AtMost => observed <= gate.threshold,
                Comparison::IsTrue => unreachable!(),
            };
            let shown = format!("{observed:.4}{note}");
            if ok {
                Outcome::Pass { observed: shown }
            } else {
                Outcome::Fail {
                    observed: shown,
                    reason: match gate.comparison {
                        Comparison::AtLeast => format!("нужно ≥ {:.4}", gate.threshold),
                        _ => format!("нужно ≤ {:.4}", gate.threshold),
                    },
                }
            }
        }
    }
}

/// Reads the number a gate judges on, plus a note naming which end of the
/// interval it was — so a printed result cannot be mistaken for the point
/// estimate.
fn judged_value(node: &serde_json::Value, judge: Judge) -> Option<(f64, String)> {
    match judge {
        Judge::Value => {
            let value = node
                .as_f64()
                .or_else(|| node.get("value").and_then(serde_json::Value::as_f64))?;
            Some((value, String::new()))
        }
        Judge::Ci95Low => {
            let value = node.get("ci95_low")?.as_f64()?;
            let point = node.get("value").and_then(serde_json::Value::as_f64);
            Some((
                value,
                match point {
                    Some(point) => format!(" (нижняя граница 95% CI; наблюдалось {point:.4})"),
                    None => " (нижняя граница 95% CI)".into(),
                },
            ))
        }
        Judge::Ci95High => {
            let value = node.get("ci95_high")?.as_f64()?;
            let point = node.get("value").and_then(serde_json::Value::as_f64);
            Some((
                value,
                match point {
                    Some(point) => format!(" (верхняя граница 95% CI; наблюдалось {point:.4})"),
                    None => " (верхняя граница 95% CI)".into(),
                },
            ))
        }
    }
}

/// The first unmet precondition, in words a person can act on.
fn unmet_requirement(
    requires: &Requirements,
    report: &serde_json::Value,
    seal: Option<&FrozenSeal>,
) -> Option<String> {
    for (provenance, wanted) in &requires.provenance_at_least {
        let have = lookup(
            report,
            &format!("dataset.counts_by_provenance.{provenance}"),
        )
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
        if have < *wanted {
            return Some(format!(
                "корпус содержит {have} записей provenance={provenance}, нужно {wanted}"
            ));
        }
    }
    if let Some(wanted) = requires.min_speakers {
        let have = lookup(report, "split_audit.speakers")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if have < wanted {
            return Some(format!("дикторов в корпусе {have}, нужно {wanted}"));
        }
    }
    if requires.frozen {
        let Some(seal) = seal else {
            return Some("frozen test не запечатан: нечего сверять".into());
        };
        let digest = lookup(report, "dataset.sha256").and_then(serde_json::Value::as_str);
        match digest {
            Some(digest) if digest == seal.sha256 => {}
            Some(digest) => {
                return Some(format!(
                    "отчёт получен на корпусе {digest}, а запечатан {} ({})",
                    seal.sha256, seal.file
                ))
            }
            None => return Some("в отчёте нет dataset.sha256".into()),
        }
    }
    None
}

/// Dotted path lookup. Deliberately dumb: a gate names a field, and if the
/// field is not there the gate is not measurable rather than silently zero.
fn lookup<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut node = root;
    for part in path.split('.') {
        node = node.get(part)?;
    }
    Some(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn gates() -> GateFile {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../research/schema/release-gates-v1.json");
        let text = std::fs::read_to_string(&path).expect("release-gates-v1.json must exist");
        serde_json::from_str(&text).expect("release-gates-v1.json must parse")
    }

    fn seal() -> FrozenSeal {
        FrozenSeal {
            file: "voice-frozen-v1.jsonl".into(),
            sha256: "a".repeat(64),
            sealed_at: "2026-09-06".into(),
            sealed_for: "0.5".into(),
        }
    }

    /// A corpus that satisfies every precondition, so individual gates can be
    /// tested on their numbers rather than on their requirements.
    fn voice_report(overrides: serde_json::Value) -> serde_json::Value {
        let mut report = json!({
            "dataset": {
                "sha256": "a".repeat(64),
                "counts_by_provenance": { "real_audio": 400 }
            },
            "split_audit": { "speakers": 12, "clean": true },
            "severity": { "count_by_severity": { "S4": 0 } },
            "metrics": {
                "pre_insertion_end_to_end_exact_match": {
                    "unicode": { "value": 0.95, "ci95_low": 0.93, "ci95_high": 0.97 }
                },
                "auto_insert_exact_match": { "value": 0.99, "ci95_low": 0.98, "ci95_high": 0.999 },
                "false_scientific_rewrite_rate": { "value": 0.0, "ci95_low": 0.0, "ci95_high": 0.01 },
                "ast_validity": { "value": 1.0, "ci95_low": 0.995, "ci95_high": 1.0 }
            }
        });
        merge(&mut report, &overrides);
        report
    }

    fn merge(into: &mut serde_json::Value, from: &serde_json::Value) {
        match (into, from) {
            (serde_json::Value::Object(target), serde_json::Value::Object(source)) => {
                for (key, value) in source {
                    merge(target.entry(key.clone()).or_insert(json!(null)), value);
                }
            }
            (target, source) => *target = source.clone(),
        }
    }

    fn outcome<'a>(report: &'a GateReport, id: &str) -> &'a Outcome {
        &report
            .results
            .iter()
            .find(|result| result.id == id)
            .unwrap_or_else(|| panic!("no gate {id}"))
            .outcome
    }

    #[test]
    fn the_shipped_gate_file_loads_and_covers_the_roadmap_criteria() {
        let file = gates();
        assert_eq!(file.gates_schema_version, GATES_SCHEMA_VERSION);
        let ids: Vec<&str> = file.gates.iter().map(|gate| gate.id.as_str()).collect();
        for required in [
            "end-to-end-accuracy",
            "auto-insert-precision",
            "no-dangerous-rewrites",
            "no-s4-errors",
        ] {
            assert!(ids.contains(&required), "missing gate {required}: {ids:?}");
        }
    }

    /// The gate that makes the rest honest: a corpus of written text cannot
    /// answer a question about speech, and reporting that as a pass is how a
    /// benchmark becomes a lie.
    #[test]
    fn a_text_corpus_cannot_pass_a_gate_about_speech() {
        let text_report = json!({
            "dataset": {
                "sha256": "b".repeat(64),
                "counts_by_provenance": { "handcrafted_text": 113 }
            },
            "split_audit": { "speakers": 0, "clean": true },
            "severity": { "count_by_severity": { "S4": 0 } },
            "metrics": {
                "pre_insertion_end_to_end_exact_match": {
                    "unicode": { "value": 0.982, "ci95_low": 0.938, "ci95_high": 0.995 }
                },
                "auto_insert_exact_match": { "value": 1.0, "ci95_low": 0.956, "ci95_high": 1.0 },
                "false_scientific_rewrite_rate": { "value": 0.0, "ci95_low": 0.0, "ci95_high": 0.121 },
                "ast_validity": { "value": 1.0, "ci95_low": 0.956, "ci95_high": 1.0 }
            }
        });
        let report = evaluate(&gates(), &text_report, None).unwrap();
        assert!(!report.admits_release());
        for id in [
            "end-to-end-accuracy",
            "auto-insert-precision",
            "no-dangerous-rewrites",
        ] {
            assert!(
                matches!(outcome(&report, id), Outcome::NotMeasurable { .. }),
                "{id}: {:?}",
                outcome(&report, id)
            );
        }
        // Split hygiene needs no voice, so it is answered on any corpus.
        assert!(outcome(&report, "split-hygiene").admits_release());
    }

    /// A point estimate above the threshold is not enough when the interval
    /// says the truth could be far below it.
    #[test]
    fn a_thin_sample_does_not_clear_a_gate_it_only_appears_to_meet() {
        let thin = voice_report(json!({
            "metrics": {
                "pre_insertion_end_to_end_exact_match": {
                    // 19 of 20: looks like 95%, supports as little as 76%.
                    "unicode": { "value": 0.95, "ci95_low": 0.764, "ci95_high": 0.992 }
                }
            }
        }));
        let report = evaluate(&gates(), &thin, Some(&seal())).unwrap();
        let outcome = outcome(&report, "end-to-end-accuracy");
        assert!(matches!(outcome, Outcome::Fail { .. }), "{outcome:?}");
        let Outcome::Fail { observed, .. } = outcome else {
            unreachable!()
        };
        // The message must not let 0.95 be mistaken for what was judged.
        assert!(observed.contains("нижняя граница"), "{observed}");
        assert!(observed.contains("0.7640"), "{observed}");
    }

    /// Zero observed failures is not evidence of safety on a small sample.
    #[test]
    fn an_observed_zero_is_judged_on_what_it_could_still_hide() {
        let few = voice_report(json!({
            "metrics": {
                // 0/28: the true rate could be 10%.
                "false_scientific_rewrite_rate": { "value": 0.0, "ci95_low": 0.0, "ci95_high": 0.121 }
            }
        }));
        let report = evaluate(&gates(), &few, Some(&seal())).unwrap();
        let outcome = outcome(&report, "no-dangerous-rewrites");
        assert!(matches!(outcome, Outcome::Fail { .. }), "{outcome:?}");
        let Outcome::Fail { observed, .. } = outcome else {
            unreachable!()
        };
        assert!(observed.contains("верхняя граница"), "{observed}");
    }

    #[test]
    fn a_corpus_that_meets_every_precondition_can_pass() {
        let report = evaluate(&gates(), &voice_report(json!({})), Some(&seal())).unwrap();
        assert!(report.admits_release(), "{report}");
        assert!(report.blocking().is_empty());
    }

    /// One dangerous error is one too many, and it is counted exactly rather
    /// than smoothed into a rate.
    #[test]
    fn a_single_s4_error_blocks_the_release() {
        let with_s4 = voice_report(json!({ "severity": { "count_by_severity": { "S4": 1 } } }));
        let report = evaluate(&gates(), &with_s4, Some(&seal())).unwrap();
        assert!(!report.admits_release());
        assert!(matches!(
            outcome(&report, "no-s4-errors"),
            Outcome::Fail { .. }
        ));
    }

    /// The frozen test is identified by digest. A corpus edited after
    /// sealing is a different corpus, whatever it is called.
    #[test]
    fn a_corpus_edited_after_sealing_is_not_the_frozen_test() {
        let edited = voice_report(json!({ "dataset": { "sha256": "c".repeat(64) } }));
        let report = evaluate(&gates(), &edited, Some(&seal())).unwrap();
        let outcome = outcome(&report, "end-to-end-accuracy");
        let Outcome::NotMeasurable { reason } = outcome else {
            panic!("{outcome:?}")
        };
        assert!(reason.contains("запечатан"), "{reason}");
    }

    #[test]
    fn nothing_is_frozen_until_something_is_sealed() {
        let report = evaluate(&gates(), &voice_report(json!({})), None).unwrap();
        assert!(!report.admits_release());
        let outcome = outcome(&report, "end-to-end-accuracy");
        assert!(
            matches!(outcome, Outcome::NotMeasurable { reason } if reason.contains("не запечатан")),
            "{outcome:?}"
        );
    }

    /// A metric the report does not carry is not zero, and not a pass.
    #[test]
    fn a_missing_metric_is_reported_rather_than_assumed() {
        let mut without = voice_report(json!({}));
        without["metrics"]
            .as_object_mut()
            .unwrap()
            .remove("ast_validity");
        let report = evaluate(&gates(), &without, Some(&seal())).unwrap();
        let outcome = outcome(&report, "structural-validity");
        assert!(
            matches!(outcome, Outcome::NotMeasurable { reason } if reason.contains("нет metrics.ast_validity")),
            "{outcome:?}"
        );
    }

    #[test]
    fn a_gate_file_from_a_future_build_is_refused() {
        let mut file = gates();
        file.gates_schema_version = 99;
        let error = evaluate(&file, &voice_report(json!({})), Some(&seal())).unwrap_err();
        assert!(error.contains("не поддерживается"), "{error}");
    }
}

#[cfg(test)]
mod seal_tests {
    use super::*;

    const RECORD: &str = r#"{"dataset_schema_version":1,"id":"fam-1-a","family_id":"fam-1","provenance":"handcrafted_text","human_transcript":"вода","asr_hypotheses":[],"target_domain":"plain","target_action":"raw","target_ast":null,"split":"train","tags":[],"speaker_id":null}"#;

    fn leaking() -> String {
        let a = RECORD.to_string();
        let b = RECORD
            .replace("fam-1-a", "fam-1-b")
            .replace("\"train\"", "\"dev_holdout\"");
        format!("{a}\n{b}")
    }

    #[test]
    fn sealing_records_the_digest_of_what_was_frozen() {
        let dir = tempfile::tempdir().unwrap();
        let dataset = dir.path().join("voice-frozen-v1.jsonl");
        std::fs::write(&dataset, RECORD).unwrap();
        let out = dir.path().join("seal.json");

        let sealed = seal(&dataset, &out, "0.5", "2026-09-06").unwrap();
        assert_eq!(sealed.file, "voice-frozen-v1.jsonl");
        assert_eq!(sealed.sha256.len(), 64);
        assert_eq!(sealed.sealed_for, "0.5");

        // What was written is what a later gate run will read.
        let written: FrozenSeal =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(written.sha256, sealed.sha256);
    }

    /// The property that makes "frozen" mean anything: a test that can be
    /// re-sealed after the result is known is a test that gets adjusted.
    #[test]
    fn a_frozen_test_cannot_be_sealed_a_second_time() {
        let dir = tempfile::tempdir().unwrap();
        let dataset = dir.path().join("voice-frozen-v1.jsonl");
        std::fs::write(&dataset, RECORD).unwrap();
        let out = dir.path().join("seal.json");
        let first = seal(&dataset, &out, "0.5", "2026-09-06").unwrap();

        // Even with a different corpus, and even for a different release.
        std::fs::write(
            &dataset,
            format!("{RECORD}\n{RECORD}").replace("fam-1-a", "fam-1-b"),
        )
        .unwrap();
        let error = seal(&dataset, &out, "0.6", "2026-10-01").unwrap_err();
        assert!(error.contains("уже существует"), "{error}");

        let unchanged: FrozenSeal =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(unchanged.sha256, first.sha256);
        assert_eq!(unchanged.sealed_for, "0.5");
    }

    /// Freezing a corpus with a split leak would fix the leak in place
    /// forever, and every number measured on it afterwards would be wrong in
    /// the same way.
    #[test]
    fn a_corpus_with_a_split_leak_is_not_sealed() {
        let dir = tempfile::tempdir().unwrap();
        let dataset = dir.path().join("leaky.jsonl");
        std::fs::write(&dataset, leaking()).unwrap();
        let out = dir.path().join("seal.json");
        let error = seal(&dataset, &out, "0.5", "2026-09-06").unwrap_err();
        assert!(error.contains("split"), "{error}");
        assert!(!out.exists(), "a refused seal must leave no file behind");
    }

    #[test]
    fn a_corpus_that_does_not_load_is_not_sealed() {
        let dir = tempfile::tempdir().unwrap();
        let dataset = dir.path().join("broken.jsonl");
        std::fs::write(&dataset, "{not json").unwrap();
        let out = dir.path().join("seal.json");
        assert!(seal(&dataset, &out, "0.5", "2026-09-06").is_err());
        assert!(!out.exists());
    }

    /// Sealing the same bytes twice, in different places, gives the same
    /// digest — the seal names the content, not the run.
    #[test]
    fn the_seal_names_the_content_not_the_moment() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jsonl");
        let b = dir.path().join("b.jsonl");
        std::fs::write(&a, RECORD).unwrap();
        std::fs::write(&b, RECORD).unwrap();
        let one = seal(&a, &dir.path().join("one.json"), "0.5", "2026-09-06").unwrap();
        let two = seal(&b, &dir.path().join("two.json"), "0.5", "2027-01-01").unwrap();
        assert_eq!(one.sha256, two.sha256);
    }
}

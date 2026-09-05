//! Versioned research corpus format (`dataset_schema_version: 1`).
//!
//! The loader is deliberately strict: an unknown schema version, a repeated
//! `id`, an unknown enum value or a record whose action and AST disagree is a
//! hard error. A corpus that silently loads with a different meaning than the
//! author intended is worse than one that refuses to load.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use sciwhisper_core::{Domain, Node};
use serde::{Deserialize, Serialize};

pub const DATASET_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Written by hand as text. There is no audio and no ASR behind it.
    HandcraftedText,
    /// Generated from an AST by a program, still text only.
    SyntheticText,
    /// Text-to-speech audio put through a real ASR backend.
    Tts,
    /// A recording of a real person.
    RealAudio,
}

impl Provenance {
    /// Whether a record of this provenance may carry ASR hypotheses at all.
    /// Text that never passed through a recogniser must not be dressed up as
    /// a voice sample.
    pub fn has_audio(self) -> bool {
        matches!(self, Provenance::Tts | Provenance::RealAudio)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Provenance::HandcraftedText => "handcrafted_text",
            Provenance::SyntheticText => "synthetic_text",
            Provenance::Tts => "tts",
            Provenance::RealAudio => "real_audio",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Split {
    Train,
    Validation,
    DevHoldout,
}

impl Split {
    pub fn as_str(self) -> &'static str {
        match self {
            Split::Train => "train",
            Split::Validation => "validation",
            Split::DevHoldout => "dev_holdout",
        }
    }
}

/// What the system is supposed to do with the utterance. `Raw` is a first
/// class action — the correct answer for ordinary speech — and never a
/// `Node::Text` smuggled into the scientific AST space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetAction {
    Ast,
    Raw,
}

impl TargetAction {
    pub fn as_str(self) -> &'static str {
        match self {
            TargetAction::Ast => "ast",
            TargetAction::Raw => "raw",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AsrHypothesis {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

/// Renders the corpus author fixed by hand, used only for render-first error
/// attribution. Absent for most records; never generated from the parser.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExpectedRender {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unicode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omml: Option<String>,
}

impl ExpectedRender {
    pub fn is_empty(&self) -> bool {
        self.unicode.is_none() && self.latex.is_none() && self.omml.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub dataset_schema_version: u32,
    pub id: String,
    pub family_id: String,
    pub provenance: Provenance,
    pub human_transcript: String,
    #[serde(default)]
    pub asr_hypotheses: Vec<AsrHypothesis>,
    pub target_domain: String,
    pub target_action: TargetAction,
    pub target_ast: Option<Node>,
    #[serde(default, skip_serializing_if = "ExpectedRender::is_empty")]
    pub expected_render: ExpectedRender,
    pub split: Split,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub speaker_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl Record {
    /// The domain the router is supposed to pick. `None` for ordinary text:
    /// there is no scientific domain to get right, so such records are
    /// excluded from routing accuracy instead of being scored against a
    /// domain nobody intended.
    pub fn gold_domain(&self) -> Option<Domain> {
        match self.target_domain.as_str() {
            "chemistry" => Some(Domain::Chemistry),
            "mathematics" => Some(Domain::Mathematics),
            "physics" => Some(Domain::Physics),
            _ => None,
        }
    }

    /// The transcript the real system would see: the best ASR hypothesis if
    /// the record has audio, otherwise the written text itself.
    pub fn system_transcript(&self) -> &str {
        self.asr_hypotheses
            .first()
            .map(|hypothesis| hypothesis.text.as_str())
            .unwrap_or(&self.human_transcript)
    }
}

#[derive(Debug)]
pub struct SchemaError {
    pub line: usize,
    pub id: Option<String>,
    pub message: String,
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.id {
            Some(id) => write!(f, "line {}: record '{}': {}", self.line, id, self.message),
            None => write!(f, "line {}: {}", self.line, self.message),
        }
    }
}

impl std::error::Error for SchemaError {}

#[derive(Clone, Debug, Default)]
pub struct Dataset {
    pub records: Vec<Record>,
}

impl Dataset {
    /// Parses a JSONL corpus. Blank lines are skipped; everything else must
    /// be a valid record, and the whole file is rejected on the first fault.
    pub fn parse_jsonl(text: &str) -> Result<Dataset, SchemaError> {
        let mut records = Vec::new();
        let mut seen_ids: BTreeSet<String> = BTreeSet::new();
        for (index, line) in text.lines().enumerate() {
            let line_number = index + 1;
            if line.trim().is_empty() {
                continue;
            }
            let value: serde_json::Value =
                serde_json::from_str(line).map_err(|error| SchemaError {
                    line: line_number,
                    id: None,
                    message: format!("not valid JSON: {error}"),
                })?;
            // The id is read first so that every later complaint can name the
            // record it is about.
            let id = value
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let version = value
                .get("dataset_schema_version")
                .and_then(serde_json::Value::as_u64);
            match version {
                Some(version) if version == u64::from(DATASET_SCHEMA_VERSION) => {}
                Some(version) => {
                    return Err(SchemaError {
                        line: line_number,
                        id,
                        message: format!(
                            "unsupported dataset_schema_version {version}; this build reads {DATASET_SCHEMA_VERSION}"
                        ),
                    })
                }
                None => {
                    return Err(SchemaError {
                        line: line_number,
                        id,
                        message: "missing dataset_schema_version".into(),
                    })
                }
            }
            let record: Record = serde_json::from_value(value).map_err(|error| SchemaError {
                line: line_number,
                id: id.clone(),
                message: format!("does not match the record schema: {error}"),
            })?;
            validate_record(&record).map_err(|message| SchemaError {
                line: line_number,
                id: id.clone(),
                message,
            })?;
            if !seen_ids.insert(record.id.clone()) {
                return Err(SchemaError {
                    line: line_number,
                    id,
                    message: "duplicate id".into(),
                });
            }
            records.push(record);
        }
        if records.is_empty() {
            return Err(SchemaError {
                line: 0,
                id: None,
                message: "the corpus contains no records".into(),
            });
        }
        Ok(Dataset { records })
    }

    pub fn split_counts(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for record in &self.records {
            *counts.entry(record.split.as_str()).or_insert(0) += 1;
        }
        counts
    }

    pub fn domain_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for record in &self.records {
            *counts.entry(record.target_domain.clone()).or_insert(0) += 1;
        }
        counts
    }

    pub fn action_counts(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for record in &self.records {
            *counts.entry(record.target_action.as_str()).or_insert(0) += 1;
        }
        counts
    }

    pub fn provenance_counts(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for record in &self.records {
            *counts.entry(record.provenance.as_str()).or_insert(0) += 1;
        }
        counts
    }

    pub fn family_count(&self) -> usize {
        self.records
            .iter()
            .map(|record| record.family_id.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

fn validate_record(record: &Record) -> Result<(), String> {
    if record.id.trim().is_empty() {
        return Err("empty id".into());
    }
    if record.family_id.trim().is_empty() {
        return Err("empty family_id".into());
    }
    if !record.id.starts_with(&record.family_id) {
        return Err(format!(
            "id '{}' must start with its family_id '{}', so that family leakage stays visible by eye",
            record.id, record.family_id
        ));
    }
    if record.human_transcript.trim().is_empty() {
        return Err("empty human_transcript".into());
    }
    if !matches!(
        record.target_domain.as_str(),
        "chemistry" | "mathematics" | "physics" | "plain"
    ) {
        return Err(format!("unknown target_domain '{}'", record.target_domain));
    }
    match (record.target_action, &record.target_ast) {
        (TargetAction::Ast, None) => {
            return Err("target_action=ast requires a target_ast".into());
        }
        (TargetAction::Raw, Some(_)) => {
            return Err("target_action=raw must not carry a target_ast".into());
        }
        _ => {}
    }
    if record.target_action == TargetAction::Raw && record.target_domain != "plain" {
        return Err(format!(
            "target_action=raw must use target_domain 'plain', not '{}'",
            record.target_domain
        ));
    }
    if record.target_action == TargetAction::Ast && record.target_domain == "plain" {
        return Err("target_action=ast needs a scientific target_domain".into());
    }
    if !record.provenance.has_audio() {
        if !record.asr_hypotheses.is_empty() {
            return Err(format!(
                "provenance '{}' has no audio, so it cannot carry asr_hypotheses",
                record.provenance.as_str()
            ));
        }
        if record.speaker_id.is_some() {
            return Err(format!(
                "provenance '{}' has no audio, so a speaker_id would be invented",
                record.provenance.as_str()
            ));
        }
    }
    for hypothesis in &record.asr_hypotheses {
        if hypothesis.text.trim().is_empty() {
            return Err("an asr hypothesis has empty text".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MATH: &str = r#"{"Math":{"Number":"1"}}"#;

    fn record_json(overrides: &[(&str, &str)]) -> String {
        let mut fields: BTreeMap<&str, String> = BTreeMap::new();
        fields.insert("dataset_schema_version", "1".into());
        fields.insert("id", "\"fam-001-a\"".into());
        fields.insert("family_id", "\"fam-001\"".into());
        fields.insert("provenance", "\"handcrafted_text\"".into());
        fields.insert("human_transcript", "\"один\"".into());
        fields.insert("asr_hypotheses", "[]".into());
        fields.insert("target_domain", "\"mathematics\"".into());
        fields.insert("target_action", "\"ast\"".into());
        fields.insert("target_ast", MATH.into());
        fields.insert("split", "\"train\"".into());
        fields.insert("tags", "[]".into());
        fields.insert("speaker_id", "null".into());
        for (key, value) in overrides {
            fields.insert(key, (*value).into());
        }
        let body = fields
            .iter()
            .map(|(key, value)| format!("\"{key}\":{value}"))
            .collect::<Vec<_>>()
            .join(",");
        format!("{{{body}}}")
    }

    #[test]
    fn a_well_formed_record_loads() {
        let dataset = Dataset::parse_jsonl(&record_json(&[])).expect("must load");
        assert_eq!(dataset.records.len(), 1);
        assert_eq!(dataset.records[0].id, "fam-001-a");
        assert_eq!(dataset.records[0].gold_domain(), Some(Domain::Mathematics));
    }

    #[test]
    fn an_unknown_schema_version_is_rejected() {
        let error = Dataset::parse_jsonl(&record_json(&[("dataset_schema_version", "2")]))
            .expect_err("must be rejected");
        assert!(error.message.contains("unsupported dataset_schema_version"));
    }

    #[test]
    fn a_missing_schema_version_is_rejected() {
        let line = record_json(&[]).replace("\"dataset_schema_version\":1,", "");
        let error = Dataset::parse_jsonl(&line).expect_err("must be rejected");
        assert!(error.message.contains("missing dataset_schema_version"));
    }

    #[test]
    fn a_duplicate_id_is_rejected() {
        let line = record_json(&[]);
        let error = Dataset::parse_jsonl(&format!("{line}\n{line}")).expect_err("must be rejected");
        assert_eq!(error.message, "duplicate id");
        assert_eq!(error.line, 2);
    }

    #[test]
    fn unknown_enum_values_are_rejected() {
        for (field, value) in [
            ("split", "\"holdout\""),
            ("target_action", "\"insert\""),
            ("provenance", "\"guessed\""),
        ] {
            let error = Dataset::parse_jsonl(&record_json(&[(field, value)]))
                .expect_err("must be rejected");
            assert!(
                error.message.contains("does not match the record schema"),
                "{field}: {}",
                error.message
            );
        }
        let error = Dataset::parse_jsonl(&record_json(&[("target_domain", "\"biology\"")]))
            .expect_err("must be rejected");
        assert!(error.message.contains("unknown target_domain"));
    }

    #[test]
    fn action_and_ast_must_agree() {
        let error = Dataset::parse_jsonl(&record_json(&[("target_ast", "null")]))
            .expect_err("ast without an ast");
        assert!(error.message.contains("requires a target_ast"));

        let error = Dataset::parse_jsonl(&record_json(&[
            ("target_action", "\"raw\""),
            ("target_domain", "\"plain\""),
        ]))
        .expect_err("raw with an ast");
        assert!(error.message.contains("must not carry a target_ast"));

        let raw = Dataset::parse_jsonl(&record_json(&[
            ("target_action", "\"raw\""),
            ("target_domain", "\"plain\""),
            ("target_ast", "null"),
        ]))
        .expect("a raw record without an ast is valid");
        assert_eq!(raw.records[0].gold_domain(), None);
    }

    #[test]
    fn corrupt_jsonl_is_rejected_with_its_line_number() {
        let line = record_json(&[]);
        let error = Dataset::parse_jsonl(&format!("{line}\n{{not json")).expect_err("must fail");
        assert_eq!(error.line, 2);
        assert!(error.message.contains("not valid JSON"));
    }

    #[test]
    fn text_only_records_cannot_pretend_to_have_audio() {
        let error =
            Dataset::parse_jsonl(&record_json(&[("asr_hypotheses", "[{\"text\":\"один\"}]")]))
                .expect_err("text has no ASR");
        assert!(error.message.contains("cannot carry asr_hypotheses"));

        let error = Dataset::parse_jsonl(&record_json(&[("speaker_id", "\"spk-1\"")]))
            .expect_err("text has no speaker");
        assert!(error.message.contains("speaker_id would be invented"));
    }

    #[test]
    fn an_id_outside_its_family_is_rejected() {
        let error = Dataset::parse_jsonl(&record_json(&[("id", "\"other-001-a\"")]))
            .expect_err("must be rejected");
        assert!(error.message.contains("must start with its family_id"));
    }

    #[test]
    fn the_best_hypothesis_is_what_the_system_sees() {
        let mut record = Dataset::parse_jsonl(&record_json(&[]))
            .unwrap()
            .records
            .remove(0);
        assert_eq!(record.system_transcript(), "один");
        record.asr_hypotheses = vec![AsrHypothesis {
            text: "адин".into(),
            score: Some(-1.0),
        }];
        assert_eq!(record.system_transcript(), "адин");
    }
}

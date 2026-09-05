//! Oracle replacement, component isolation and first-blocking attribution.
//!
//! Two different questions are answered here and must not be confused:
//!
//! * **Oracle replacement** — replace one real component with a perfect one
//!   inside the full pipeline. This is the product question: which component
//!   is worth improving next?
//! * **Component isolation** — give one component perfect inputs from every
//!   earlier stage. This is the laboratory question: how good is that
//!   algorithm on its own?
//!
//! Oracle gains are reported separately and are never summed: the components
//! interact, and an additive decomposition would have to be proven, not
//! assumed.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::candidates::{Candidate, CandidateSource, DomainPolicy};
use crate::canonical::canonical_target_v1;
use crate::evaluate::{
    expected_payload, gold_target, oracle_hypotheses, run_pipeline, system_hypotheses, ErrorStage,
    EvalConfig, PipelineRun, Selector,
};
use crate::metrics::{proportion, Proportion};
use crate::schema::{Dataset, Record};

/// The gold answer packaged as a candidate. This is the **only** place in the
/// crate where the gold enters a candidate list, and the name says so.
pub fn oracle_candidates(record: &Record) -> Vec<Candidate> {
    let Ok(target) = gold_target(record) else {
        return Vec::new();
    };
    let Ok(canonical) = canonical_target_v1(&target) else {
        return Vec::new();
    };
    vec![Candidate {
        action: target,
        canonical,
        transcript: record.human_transcript.clone(),
        transcript_index: 0,
        domain: record.gold_domain(),
        resolved_domain: record.gold_domain(),
        source: CandidateSource::OracleGold,
        order: usize::MAX,
        warnings: Vec::new(),
        // A perfect generator hands over an answer the ranker may still fail
        // to pick, so it arrives with the confidence of a normal parse.
        structural_confidence: 1.0,
        structurally_valid: true,
    }]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// Everything as shipped.
    Real,
    OracleTranscript,
    OracleDomain,
    OracleCandidates,
    OracleRanker,
    OracleDecision,
}

impl Variant {
    pub fn as_str(self) -> &'static str {
        match self {
            Variant::Real => "real",
            Variant::OracleTranscript => "oracle_transcript",
            Variant::OracleDomain => "oracle_domain",
            Variant::OracleCandidates => "oracle_candidates",
            Variant::OracleRanker => "oracle_ranker",
            Variant::OracleDecision => "oracle_decision",
        }
    }
}

/// Runs one variant of the pipeline over one record.
pub fn run_variant(record: &Record, config: &EvalConfig, variant: Variant) -> Option<PipelineRun> {
    let target = gold_target(record).ok()?;
    let gold_key = canonical_target_v1(&target).ok()?;
    let mut config = config.clone();
    let mut transcripts = system_hypotheses(record);
    let mut extras: Vec<Candidate> = Vec::new();
    let mut selector = Selector::Deterministic;
    let mut threshold = None;

    match variant {
        Variant::Real => {}
        Variant::OracleTranscript => transcripts = oracle_hypotheses(record),
        Variant::OracleDomain => {
            if let Some(domain) = record.gold_domain() {
                config.domain_policy = DomainPolicy::Oracle(domain);
            }
        }
        Variant::OracleCandidates => extras = oracle_candidates(record),
        Variant::OracleRanker => selector = Selector::OracleGold,
        // A perfect decision inserts whenever the ranker offers a scientific
        // answer and abstains only when the ranker offers RAW.
        Variant::OracleDecision => threshold = Some(f32::NEG_INFINITY),
    }

    Some(run_pipeline(
        &transcripts,
        &gold_key,
        &config,
        &extras,
        selector,
        threshold,
    ))
}

#[derive(Clone, Debug, Serialize)]
pub struct OracleReplacement {
    pub variant: String,
    pub overall_exact_match: Proportion,
    /// Change against the real system, in absolute percentage points of the
    /// same metric. Gains from different variants must not be added together.
    pub delta_vs_real: Option<f64>,
    pub applicable_examples: usize,
}

pub fn oracle_replacement(dataset: &[&Record], config: &EvalConfig) -> Vec<OracleReplacement> {
    let baseline = variant_exact_match(dataset, config, Variant::Real);
    let mut out = vec![OracleReplacement {
        variant: Variant::Real.as_str().into(),
        overall_exact_match: baseline.clone(),
        delta_vs_real: Some(0.0),
        applicable_examples: dataset.len(),
    }];
    for variant in [
        Variant::OracleTranscript,
        Variant::OracleDomain,
        Variant::OracleCandidates,
        Variant::OracleRanker,
        Variant::OracleDecision,
    ] {
        let measured = variant_exact_match(dataset, config, variant);
        let applicable = match variant {
            // Replacing the transcript can only matter where ASR actually ran.
            Variant::OracleTranscript => dataset
                .iter()
                .filter(|record| !record.asr_hypotheses.is_empty())
                .count(),
            // Replacing the router can only matter where a domain was intended.
            Variant::OracleDomain => dataset
                .iter()
                .filter(|record| record.gold_domain().is_some())
                .count(),
            _ => dataset.len(),
        };
        let delta = match (measured.value, baseline.value) {
            (Some(a), Some(b)) => Some(a - b),
            _ => None,
        };
        out.push(OracleReplacement {
            variant: variant.as_str().into(),
            overall_exact_match: measured,
            delta_vs_real: delta,
            applicable_examples: applicable,
        });
    }
    out
}

fn variant_exact_match(dataset: &[&Record], config: &EvalConfig, variant: Variant) -> Proportion {
    let mut hits = 0usize;
    let mut total = 0usize;
    for record in dataset {
        let Ok(target) = gold_target(record) else {
            continue;
        };
        let Some(run) = run_variant(record, config, variant) else {
            continue;
        };
        total += 1;
        if run.emitted_payload == expected_payload(record, &target, config) {
            hits += 1;
        }
    }
    proportion(hits, total)
}

#[derive(Clone, Debug, Serialize)]
pub struct ComponentIsolation {
    pub router_isolated_accuracy: Proportion,
    pub candidate_isolated_recall_at_k: Proportion,
    pub rank_isolated_accuracy: Proportion,
    pub decision_isolated_accuracy: Proportion,
    pub note: String,
}

/// Each component is measured on perfect inputs from every earlier stage.
pub fn component_isolation(dataset: &[&Record], config: &EvalConfig) -> ComponentIsolation {
    let mut router_hits = 0usize;
    let mut router_total = 0usize;
    let mut candidate_hits = 0usize;
    let mut candidate_total = 0usize;
    let mut rank_hits = 0usize;
    let mut rank_total = 0usize;
    let mut decision_hits = 0usize;
    let mut decision_total = 0usize;

    for record in dataset {
        let Ok(target) = gold_target(record) else {
            continue;
        };
        let Ok(gold_key) = canonical_target_v1(&target) else {
            continue;
        };
        let expected = expected_payload(record, &target, config);
        let hypotheses = oracle_hypotheses(record);

        // router-isolated: perfect transcript, real routing.
        if let Some(gold_domain) = record.gold_domain() {
            let run = run_pipeline(
                &hypotheses,
                &gold_key,
                config,
                &[],
                Selector::Deterministic,
                None,
            );
            router_total += 1;
            if run.resolved_domain == Some(gold_domain) {
                router_hits += 1;
            }
        }

        // candidate-isolated: perfect transcript and perfect routing.
        let mut oracle_domain_config = config.clone();
        if let Some(domain) = record.gold_domain() {
            oracle_domain_config.domain_policy = DomainPolicy::Oracle(domain);
        }
        let run = run_pipeline(
            &hypotheses,
            &gold_key,
            &oracle_domain_config,
            &[],
            Selector::Deterministic,
            None,
        );
        candidate_total += 1;
        if run.gold_rank.is_some_and(|rank| rank <= config.k) {
            candidate_hits += 1;
        }

        // rank-isolated: perfect transcript, routing and candidate set.
        let extras = oracle_candidates(record);
        let run = run_pipeline(
            &hypotheses,
            &gold_key,
            &oracle_domain_config,
            &extras,
            Selector::Deterministic,
            None,
        );
        rank_total += 1;
        if run.selection_is_correct(&gold_key) {
            rank_hits += 1;
        }

        // decision-isolated: everything upstream perfect, including the ranker.
        let run = run_pipeline(
            &hypotheses,
            &gold_key,
            &oracle_domain_config,
            &extras,
            Selector::OracleGold,
            None,
        );
        decision_total += 1;
        if run.emitted_payload == expected {
            decision_hits += 1;
        }
    }

    ComponentIsolation {
        router_isolated_accuracy: proportion(router_hits, router_total),
        candidate_isolated_recall_at_k: proportion(candidate_hits, candidate_total),
        rank_isolated_accuracy: proportion(rank_hits, rank_total),
        decision_isolated_accuracy: proportion(decision_hits, decision_total),
        note: "Isolated numbers are laboratory component quality, not a product improvement ceiling; they are not additive with the oracle replacement deltas.".into(),
    }
}

/// The single first blocking stage for one record, or `None` when the record
/// is answered correctly.
///
/// The order of the stages is part of the protocol version. `ASR-first` is
/// simply unreachable for a text-only record: no recogniser ran, so a parser
/// failure may never be booked against ASR.
pub fn first_blocking(record: &Record, config: &EvalConfig) -> Option<ErrorStage> {
    let Ok(target) = gold_target(record) else {
        return Some(ErrorStage::DatasetInvalid);
    };
    let Ok(gold_key) = canonical_target_v1(&target) else {
        return Some(ErrorStage::DatasetInvalid);
    };
    let expected = expected_payload(record, &target, config);

    let real = run_variant(record, config, Variant::Real)?;
    if real.emitted_payload == expected {
        return None;
    }

    let has_asr = !record.asr_hypotheses.is_empty();
    let oracle_transcript = run_variant(record, config, Variant::OracleTranscript)?;
    if has_asr && oracle_transcript.emitted_payload == expected {
        return Some(ErrorStage::AsrFirst);
    }

    let mut upstream_config = config.clone();
    let run = match record.gold_domain() {
        Some(domain) => {
            upstream_config.domain_policy = DomainPolicy::Oracle(domain);
            let run = run_pipeline(
                &oracle_hypotheses(record),
                &gold_key,
                &upstream_config,
                &[],
                Selector::Deterministic,
                None,
            );
            if run.emitted_payload == expected {
                return Some(ErrorStage::RouterFirst);
            }
            run
        }
        // Ordinary text has no intended domain, so the router stage does not
        // apply and the chain continues with the perfect transcript.
        None => oracle_transcript,
    };

    if run.gold_rank.is_none() {
        return Some(ErrorStage::CandidateFirst);
    }
    if !run.selection_is_correct(&gold_key) {
        return Some(ErrorStage::RankFirst);
    }
    if run.emitted.is_ast() != target.is_ast() {
        return Some(ErrorStage::DecisionFirst);
    }
    if !target.is_ast() {
        // Both sides kept the words, yet the payload still differs: only a
        // recogniser could have changed them. Without audio that is a corpus
        // contradiction, not a render defect.
        return Some(if has_asr {
            ErrorStage::AsrFirst
        } else {
            ErrorStage::DatasetInvalid
        });
    }
    Some(ErrorStage::RenderFirst)
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct BottleneckTable {
    pub counts: BTreeMap<String, usize>,
    pub share_of_errors: BTreeMap<String, f64>,
    pub errors: usize,
    pub examples: usize,
    pub asr_first_applicable_examples: usize,
    pub notes: Vec<String>,
}

pub fn bottleneck_table(dataset: &Dataset, stages: &[Option<ErrorStage>]) -> BottleneckTable {
    let mut counts = BTreeMap::new();
    for stage in [
        ErrorStage::DatasetInvalid,
        ErrorStage::AsrFirst,
        ErrorStage::RouterFirst,
        ErrorStage::CandidateFirst,
        ErrorStage::RankFirst,
        ErrorStage::DecisionFirst,
        ErrorStage::RenderFirst,
    ] {
        counts.insert(stage.as_str().to_string(), 0usize);
    }
    let mut errors = 0usize;
    for stage in stages.iter().flatten() {
        errors += 1;
        *counts
            .get_mut(stage.as_str())
            .expect("every stage is pre-seeded") += 1;
    }
    let share_of_errors = counts
        .iter()
        .map(|(stage, count)| {
            let share = if errors == 0 {
                0.0
            } else {
                *count as f64 / errors as f64
            };
            (stage.clone(), share)
        })
        .collect();
    let with_audio = dataset
        .records
        .iter()
        .filter(|record| !record.asr_hypotheses.is_empty())
        .count();
    let mut notes = Vec::new();
    if with_audio == 0 {
        notes.push(
            "ASR-first is N/A: no record in this corpus carries an ASR hypothesis, so no failure can be attributed to recognition.".to_string(),
        );
    }
    BottleneckTable {
        counts,
        share_of_errors,
        errors,
        examples: stages.len(),
        asr_first_applicable_examples: with_audio,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Dataset;

    fn dataset(lines: &[&str]) -> Dataset {
        Dataset::parse_jsonl(&lines.join("\n")).expect("valid corpus")
    }

    fn record_line(id: &str, transcript: &str, action: &str, ast: &str, domain: &str) -> String {
        format!(
            r#"{{"dataset_schema_version":1,"id":"{id}-a","family_id":"{id}","provenance":"handcrafted_text","human_transcript":"{transcript}","asr_hypotheses":[],"target_domain":"{domain}","target_action":"{action}","target_ast":{ast},"split":"train","tags":[],"speaker_id":null}}"#
        )
    }

    #[test]
    fn a_correct_example_has_no_blocking_stage() {
        let corpus = dataset(&[&record_line(
            "chem-sulfuric-001",
            "серная кислота",
            "ast",
            r#"{"Chemical":{"Species":{"coefficient":1,"formula":{"parts":[{"Atom":{"symbol":"H","count":2}},{"Atom":{"symbol":"S","count":1}},{"Atom":{"symbol":"O","count":4}}]},"charge":null,"marker":null}}}"#,
            "chemistry",
        )]);
        let config = EvalConfig::default();
        assert_eq!(first_blocking(&corpus.records[0], &config), None);
    }

    #[test]
    fn a_name_the_router_does_not_recognise_is_attributed_to_routing() {
        // «вода» carries no domain keyword and no element name, so automatic
        // routing sends it to mathematics and the parse fails — while the very
        // same phrase parses under an explicit chemistry domain. That is a
        // router failure and the decomposition has to say so rather than
        // blaming the chemistry grammar.
        let corpus = dataset(&[&record_line(
            "chem-water-001",
            "вода",
            "ast",
            r#"{"Chemical":{"Species":{"coefficient":1,"formula":{"parts":[{"Atom":{"symbol":"H","count":2}},{"Atom":{"symbol":"O","count":1}}]},"charge":null,"marker":null}}}"#,
            "chemistry",
        )]);
        let config = EvalConfig::default();
        assert_eq!(
            first_blocking(&corpus.records[0], &config),
            Some(ErrorStage::RouterFirst)
        );
    }

    #[test]
    fn a_text_only_corpus_never_books_an_error_against_asr() {
        // «предел терпения» is ordinary speech that the parser refuses, so
        // RAW is both the gold answer and the system's answer.
        let corpus = dataset(&[
            &record_line("plain-001", "предел терпения", "raw", "null", "plain"),
            &record_line(
                "chem-broken-001",
                "феррит бария",
                "ast",
                r#"{"Chemical":{"Species":{"coefficient":1,"formula":{"parts":[{"Atom":{"symbol":"Ba","count":1}}]},"charge":null,"marker":null}}}"#,
                "chemistry",
            ),
        ]);
        let config = EvalConfig::default();
        let stages: Vec<_> = corpus
            .records
            .iter()
            .map(|record| first_blocking(record, &config))
            .collect();
        assert_eq!(stages[0], None, "ordinary speech is answered correctly");
        assert!(
            stages[1].is_some_and(|stage| stage != ErrorStage::AsrFirst),
            "a parser gap must not be booked as recognition: {stages:?}"
        );
        let table = bottleneck_table(&corpus, &stages);
        assert_eq!(table.counts["ASR-first"], 0);
        assert_eq!(table.asr_first_applicable_examples, 0);
        assert!(table.notes.iter().any(|note| note.contains("N/A")));
    }

    #[test]
    fn a_missing_candidate_is_attributed_to_generation() {
        // Nothing in the grammar builds barium ferrite, so with a perfect
        // transcript and a perfect domain the gold is still absent.
        let corpus = dataset(&[&record_line(
            "chem-ferrite-001",
            "феррит бария",
            "ast",
            r#"{"Chemical":{"Species":{"coefficient":1,"formula":{"parts":[{"Atom":{"symbol":"Ba","count":1}},{"Atom":{"symbol":"Fe","count":12}},{"Atom":{"symbol":"O","count":19}}]},"charge":null,"marker":null}}}"#,
            "chemistry",
        )]);
        let config = EvalConfig::default();
        assert_eq!(
            first_blocking(&corpus.records[0], &config),
            Some(ErrorStage::CandidateFirst)
        );
    }

    #[test]
    fn the_oracle_candidate_set_is_the_only_place_gold_appears() {
        let corpus = dataset(&[&record_line(
            "chem-ferrite-001",
            "феррит бария",
            "ast",
            r#"{"Chemical":{"Species":{"coefficient":1,"formula":{"parts":[{"Atom":{"symbol":"Ba","count":1}}]},"charge":null,"marker":null}}}"#,
            "chemistry",
        )]);
        let record = &corpus.records[0];
        let config = EvalConfig::default();
        let real = run_variant(record, &config, Variant::Real).unwrap();
        assert!(real
            .candidates
            .iter()
            .all(|candidate| candidate.source != CandidateSource::OracleGold));
        let oracle = run_variant(record, &config, Variant::OracleCandidates).unwrap();
        assert!(oracle
            .candidates
            .iter()
            .any(|candidate| candidate.source == CandidateSource::OracleGold));
        assert!(oracle.gold_rank.is_some());
        // The oracle generator guarantees availability, not first place.
        assert!(oracle.gold_rank.unwrap() > 1);
    }

    #[test]
    fn oracle_replacement_reports_the_real_system_first_and_never_sums_gains() {
        let corpus = dataset(&[&record_line(
            "chem-ferrite-001",
            "феррит бария",
            "ast",
            r#"{"Chemical":{"Species":{"coefficient":1,"formula":{"parts":[{"Atom":{"symbol":"Ba","count":1}}]},"charge":null,"marker":null}}}"#,
            "chemistry",
        )]);
        let records: Vec<&Record> = corpus.records.iter().collect();
        let table = oracle_replacement(&records, &EvalConfig::default());
        assert_eq!(table[0].variant, "real");
        assert_eq!(table[0].delta_vs_real, Some(0.0));
        // Perfect candidates plus a perfect ranker is a different experiment
        // from either alone, so each row stands on its own.
        assert!(table.iter().any(|row| row.variant == "oracle_candidates"));
        assert!(table.iter().any(|row| row.variant == "oracle_ranker"));
    }
}

//! One pass of the real system over one record, plus the per-example outcome
//! every metric is computed from.
//!
//! What is measured here ends *before* the operating system insertion step.
//! Nothing in this crate opens Word, so no metric may claim that the Word
//! path was verified.

use std::time::Instant;

use sciwhisper_core::{render, Domain, Renderer};

use crate::candidates::{generate_candidates, Candidate, DomainPolicy};
use crate::canonical::{canonical_target_v1, Target};
use crate::metrics::Severity;
use crate::schema::{AsrHypothesis, Record, TargetAction};

/// The largest `K` on the recall curve. Candidates are always generated up to
/// this depth so that `Recall@16` stays measurable while the system operates
/// at a smaller `K`.
pub const RECALL_K_GRID: [usize; 5] = [1, 2, 4, 8, 16];

pub fn recall_k_max() -> usize {
    RECALL_K_GRID[RECALL_K_GRID.len() - 1]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemAction {
    AutoInsert,
    KeepRaw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorStage {
    DatasetInvalid,
    AsrFirst,
    RouterFirst,
    CandidateFirst,
    RankFirst,
    DecisionFirst,
    RenderFirst,
}

impl ErrorStage {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorStage::DatasetInvalid => "dataset-invalid",
            ErrorStage::AsrFirst => "ASR-first",
            ErrorStage::RouterFirst => "router-first",
            ErrorStage::CandidateFirst => "candidate-first",
            ErrorStage::RankFirst => "rank-first",
            ErrorStage::DecisionFirst => "decision-first",
            ErrorStage::RenderFirst => "render-first",
        }
    }
}

#[derive(Clone, Debug)]
pub struct EvalConfig {
    /// Operating `K`: how many candidates the selector may look at.
    pub k: usize,
    /// Structural confidence needed to insert without asking. 0.9 mirrors the
    /// inline gate the shipped ASR pipeline uses.
    pub auto_insert_threshold: f32,
    pub domain_policy: DomainPolicy,
    pub renderer: Renderer,
    pub bootstrap_seed: u64,
    pub bootstrap_resamples: usize,
}

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            k: 4,
            auto_insert_threshold: 0.9,
            domain_policy: DomainPolicy::Auto,
            renderer: Renderer::Unicode,
            bootstrap_seed: 20_260_904,
            bootstrap_resamples: 2000,
        }
    }
}

/// One run of the pipeline over one utterance.
#[derive(Clone, Debug)]
pub struct PipelineRun {
    pub candidates: Vec<Candidate>,
    /// 1-based rank of the gold answer in the full generated list.
    pub gold_rank: Option<usize>,
    pub selected: Option<Candidate>,
    pub action: SystemAction,
    pub emitted: Target,
    pub emitted_payload: String,
    pub resolved_domain: Option<Domain>,
    pub latency_us: u64,
}

impl PipelineRun {
    pub fn selection_is_correct(&self, gold_key: &str) -> bool {
        self.selected
            .as_ref()
            .map(|candidate| candidate.canonical == gold_key)
            .unwrap_or(false)
    }
}

/// How the ranker picks from the candidate list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selector {
    /// The generator's own order — the deterministic baseline.
    Deterministic,
    /// Picks the gold answer whenever it is inside the operating window.
    /// Only oracle experiments may use this.
    OracleGold,
}

/// Runs candidate generation, selection and the insert/abstain decision for
/// one utterance.
///
/// `gold_key` locates the gold answer for the recall metric. It influences
/// selection only under `Selector::OracleGold`, which no ordinary evaluation
/// path uses.
pub fn run_pipeline(
    transcripts: &[AsrHypothesis],
    gold_key: &str,
    config: &EvalConfig,
    extra_candidates: &[Candidate],
    selector: Selector,
    threshold_override: Option<f32>,
) -> PipelineRun {
    let started = Instant::now();
    let mut candidates = generate_candidates(transcripts, config.domain_policy, recall_k_max());
    // Oracle experiments may hand in extra candidates. Ordinary evaluation
    // passes an empty slice, so the gold answer can never sneak in here.
    // Extras are appended, never prepended: an oracle generator guarantees
    // that the right answer is *available*, it does not hand the ranker the
    // answer already in first place.
    for extra in extra_candidates {
        if !candidates
            .iter()
            .any(|kept| kept.canonical == extra.canonical)
        {
            candidates.push(extra.clone());
        }
    }
    for (position, candidate) in candidates.iter_mut().enumerate() {
        candidate.order = position;
    }
    candidates.truncate(recall_k_max());

    let gold_rank = candidates
        .iter()
        .position(|candidate| candidate.canonical == gold_key)
        .map(|index| index + 1);

    let operating = &candidates[..candidates.len().min(config.k)];
    let selected = match selector {
        // The deterministic baseline is the generator's own order.
        Selector::Deterministic => operating.first().cloned(),
        Selector::OracleGold => operating
            .iter()
            .find(|candidate| candidate.canonical == gold_key)
            .or_else(|| operating.first())
            .cloned(),
    };
    let threshold = threshold_override.unwrap_or(config.auto_insert_threshold);
    let (action, emitted) = decide(selected.as_ref(), threshold);
    let fallback = transcripts
        .first()
        .map(|hypothesis| hypothesis.text.clone())
        .unwrap_or_default();
    let emitted_payload = payload(&emitted, config.renderer, &fallback);
    let resolved_domain = selected
        .as_ref()
        .and_then(|candidate| candidate.resolved_domain);
    PipelineRun {
        candidates,
        gold_rank,
        selected,
        action,
        emitted,
        emitted_payload,
        resolved_domain,
        latency_us: started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
    }
}

/// The insert/abstain rule. Keeping the words is the safe default: anything
/// that is not a confident scientific parse stays raw.
pub fn decide(selected: Option<&Candidate>, threshold: f32) -> (SystemAction, Target) {
    match selected {
        Some(candidate) if candidate.action.is_ast() => {
            if candidate.structural_confidence >= threshold {
                (SystemAction::AutoInsert, candidate.action.clone())
            } else {
                (SystemAction::KeepRaw, Target::Raw)
            }
        }
        _ => (SystemAction::KeepRaw, Target::Raw),
    }
}

/// What actually reaches the clipboard, before any OS insertion.
pub fn payload(target: &Target, renderer: Renderer, transcript: &str) -> String {
    match target {
        Target::Raw => transcript.to_string(),
        Target::Ast(node) => render(node, renderer),
    }
}

#[derive(Clone, Debug)]
pub struct ExampleOutcome {
    pub id: String,
    pub family_id: String,
    pub domain: String,
    pub tags: Vec<String>,
    pub target_action: TargetAction,
    pub selected_key: Option<String>,
    /// Where the selected answer came from, kept so a failure can be traced
    /// back to the pass that produced it.
    pub selected_source: Option<&'static str>,
    pub selected_warnings: Vec<String>,
    pub requested_domain: Option<String>,
    pub hypothesis_index: Option<usize>,
    pub transcript: String,
    pub emitted: Target,
    pub action: SystemAction,
    pub gold_rank: Option<usize>,
    pub candidate_count: usize,
    pub selection_correct: bool,
    pub payload_correct: bool,
    pub expected_payload: String,
    pub emitted_payload: String,
    pub routing_correct: Option<bool>,
    pub structurally_valid: bool,
    pub render_match: Option<bool>,
    pub latency_us: u64,
    pub severity: Option<Severity>,
    pub first_blocking: Option<ErrorStage>,
}

/// Evaluates one record with the real pipeline. `first_blocking` is filled in
/// later by the oracle module, which is what can tell the stages apart.
pub fn evaluate_record(record: &Record, config: &EvalConfig) -> Result<ExampleOutcome, String> {
    let gold_target = gold_target(record)?;
    let gold_key = canonical_target_v1(&gold_target).map_err(|error| error.to_string())?;
    let transcripts = system_hypotheses(record);
    let run = run_pipeline(
        &transcripts,
        &gold_key,
        config,
        &[],
        Selector::Deterministic,
        None,
    );

    let expected_payload = expected_payload(record, &gold_target, config);
    let selection_correct = run.selection_is_correct(&gold_key);
    let payload_correct = run.emitted_payload == expected_payload;

    // Routing is only defined where a scientific domain was intended.
    let routing_correct = record
        .gold_domain()
        .map(|gold| run.resolved_domain == Some(gold));

    // Render-first attribution needs an author-fixed expected string; it is
    // only checked where the corpus actually carries one. The question asked
    // is whether the renderer is faithful to the gold AST, independently of
    // whether the parser found that AST.
    let render_match = fixed_render(record, config.renderer).map(|expected| match &gold_target {
        Target::Ast(node) => render(node, config.renderer) == expected,
        Target::Raw => record.human_transcript == expected,
    });

    let severity = crate::metrics::classify_severity(&gold_target, &run.emitted, render_match);
    let structurally_valid = run
        .selected
        .as_ref()
        .map(|candidate| candidate.structurally_valid)
        .unwrap_or(true);

    Ok(ExampleOutcome {
        id: record.id.clone(),
        family_id: record.family_id.clone(),
        domain: record.target_domain.clone(),
        tags: record.tags.clone(),
        target_action: record.target_action,
        selected_key: run.selected.as_ref().map(|c| c.canonical.clone()),
        selected_source: run.selected.as_ref().map(|c| c.source.as_str()),
        selected_warnings: run
            .selected
            .as_ref()
            .map(|c| c.warnings.clone())
            .unwrap_or_default(),
        requested_domain: run
            .selected
            .as_ref()
            .and_then(|c| c.domain)
            .map(|domain| domain.as_str().to_string()),
        hypothesis_index: run.selected.as_ref().map(|c| c.transcript_index),
        // The transcript the selected answer actually came from, which is the
        // one to look at when a failure has to be explained.
        transcript: run
            .selected
            .as_ref()
            .map(|c| c.transcript.clone())
            .unwrap_or_else(|| record.system_transcript().to_string()),
        emitted: run.emitted.clone(),
        action: run.action,
        gold_rank: run.gold_rank,
        candidate_count: run.candidates.len(),
        selection_correct,
        payload_correct,
        expected_payload,
        emitted_payload: run.emitted_payload,
        routing_correct,
        structurally_valid,
        render_match,
        latency_us: run.latency_us,
        severity,
        first_blocking: None,
    })
}

/// The string the corpus author fixed for this renderer, if any.
pub fn fixed_render(record: &Record, renderer: Renderer) -> Option<&str> {
    match renderer {
        Renderer::Unicode => record.expected_render.unicode.as_deref(),
        Renderer::Latex => record.expected_render.latex.as_deref(),
        Renderer::Omml => record.expected_render.omml.as_deref(),
    }
}

/// What the system ought to emit before OS insertion.
///
/// An author-fixed render wins over the renderer's own output, so that a
/// renderer defect shows up as an error instead of defining itself as
/// correct.
pub fn expected_payload(record: &Record, gold: &Target, config: &EvalConfig) -> String {
    if let Some(fixed) = fixed_render(record, config.renderer) {
        return fixed.to_string();
    }
    match gold {
        Target::Raw => record.human_transcript.clone(),
        Target::Ast(node) => render(node, config.renderer),
    }
}

/// The gold answer as a `Target`, so `RAW` never has to masquerade as a node.
pub fn gold_target(record: &Record) -> Result<Target, String> {
    match (record.target_action, &record.target_ast) {
        (TargetAction::Raw, _) => Ok(Target::Raw),
        (TargetAction::Ast, Some(node)) => Ok(Target::Ast(node.clone())),
        (TargetAction::Ast, None) => Err(format!("record '{}' has no target_ast", record.id)),
    }
}

/// The hypotheses the real system would work from.
pub fn system_hypotheses(record: &Record) -> Vec<AsrHypothesis> {
    if record.asr_hypotheses.is_empty() {
        vec![AsrHypothesis {
            text: record.human_transcript.clone(),
            score: None,
        }]
    } else {
        record.asr_hypotheses.clone()
    }
}

/// The perfect-transcript variant: what the pipeline would see if ASR were
/// flawless.
pub fn oracle_hypotheses(record: &Record) -> Vec<AsrHypothesis> {
    vec![AsrHypothesis {
        text: record.human_transcript.clone(),
        score: None,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidates::CandidateSource;
    use sciwhisper_core::ast::Math;
    use sciwhisper_core::Node;

    fn candidate(confidence: f32, ast: bool) -> Candidate {
        let action = if ast {
            Target::Ast(Node::Math(Math::Number("1".into())))
        } else {
            Target::Raw
        };
        Candidate {
            canonical: canonical_target_v1(&action).unwrap(),
            action,
            transcript: "один".into(),
            transcript_index: 0,
            domain: None,
            resolved_domain: None,
            source: CandidateSource::PrimaryParse,
            order: 0,
            warnings: vec![],
            structural_confidence: confidence,
            structurally_valid: true,
        }
    }

    #[test]
    fn a_confident_scientific_parse_is_inserted() {
        let (action, emitted) = decide(Some(&candidate(0.95, true)), 0.9);
        assert_eq!(action, SystemAction::AutoInsert);
        assert!(emitted.is_ast());
    }

    #[test]
    fn an_ambiguous_parse_keeps_the_words() {
        let (action, emitted) = decide(Some(&candidate(0.7, true)), 0.9);
        assert_eq!(action, SystemAction::KeepRaw);
        assert_eq!(emitted, Target::Raw);
    }

    #[test]
    fn a_raw_candidate_or_no_candidate_keeps_the_words() {
        assert_eq!(
            decide(Some(&candidate(1.0, false)), 0.9).0,
            SystemAction::KeepRaw
        );
        assert_eq!(decide(None, 0.9).0, SystemAction::KeepRaw);
    }

    #[test]
    fn the_payload_is_the_transcript_when_the_system_abstains() {
        assert_eq!(
            payload(&Target::Raw, Renderer::Unicode, "предел терпения"),
            "предел терпения"
        );
    }

    #[test]
    fn the_gold_rank_is_read_off_the_full_list_not_the_operating_one() {
        let transcripts = vec![AsrHypothesis {
            text: "серная кислота".into(),
            score: None,
        }];
        let config = EvalConfig {
            k: 1,
            ..EvalConfig::default()
        };
        // RAW sits behind the parsed formula, so it is out of the operating
        // window but still visible on the recall curve.
        let run = run_pipeline(
            &transcripts,
            crate::canonical::RAW_KEY,
            &config,
            &[],
            Selector::Deterministic,
            None,
        );
        assert_eq!(run.gold_rank, Some(2));
        assert!(!run.selection_is_correct(crate::canonical::RAW_KEY));
    }
}

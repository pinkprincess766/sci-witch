//! What the candidate lattice actually offered, counted.
//!
//! The recall curve, the safety rates and the severity histogram are computed
//! elsewhere and are unchanged by this module; they ask "was the right answer
//! reachable, and did the system stay safe?". This one asks the question that
//! decides whether a ranker is worth training at all: **how often was there
//! more than one thing to choose between, and how often was the right one not
//! already first?**
//!
//! A lattice that always offers exactly one reading cannot be reranked. So can
//! a lattice that offers many but always has the gold at rank 1. Both look
//! like success on the recall curve and neither leaves a ranker anything to
//! learn, which is why those two numbers are reported next to each other.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::evaluate::ExampleOutcome;
use crate::metrics::{proportion, Proportion};

/// The smallest number of reranking opportunities worth fitting anything on.
///
/// A logistic baseline over the feature vector has on the order of ten free
/// parameters. Fewer examples than this and the fit is memorising the
/// corpus; the verdict below says so in words rather than producing
/// coefficients nobody should trust.
pub const MIN_RERANKABLE_FOR_BASELINE: usize = 50;

#[derive(Clone, Debug, Serialize)]
pub struct LatticeReport {
    pub examples: usize,
    /// Distinct structures offered per example, `RAW` excluded.
    pub distinct_asts: Distribution,
    /// Examples offering at least two distinct structures. Only these give a
    /// ranker anything to choose between.
    pub with_a_choice: Proportion,
    /// Examples where the gold answer was reachable but not at rank 1. These
    /// are the ones a ranker could actually win.
    pub gold_not_first: Proportion,
    /// Examples where the gold answer was not in the lattice at all. No
    /// ranker can recover these; they are generator work.
    pub gold_absent: Proportion,
    /// Candidates that exist only because the words were edited first.
    pub edited_candidates: usize,
    /// How many candidates each origin contributed, across the corpus.
    pub candidates_by_origin: BTreeMap<String, usize>,
    /// Whether a limit stopped generation on any example.
    pub truncated_examples: usize,
    pub verdict: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Distribution {
    pub min: usize,
    pub median: usize,
    pub max: usize,
    pub mean: Option<f64>,
    pub histogram: BTreeMap<String, usize>,
}

fn distribution(values: &[usize]) -> Distribution {
    let mut histogram = BTreeMap::new();
    for value in values {
        *histogram.entry(value.to_string()).or_insert(0) += 1;
    }
    if values.is_empty() {
        return Distribution {
            min: 0,
            median: 0,
            max: 0,
            mean: None,
            histogram,
        };
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let total: usize = sorted.iter().sum();
    Distribution {
        min: sorted[0],
        median: sorted[crate::metrics::percentile_index(sorted.len(), 50.0)],
        max: sorted[sorted.len() - 1],
        mean: Some(total as f64 / sorted.len() as f64),
        histogram,
    }
}

pub fn evaluate(outcomes: &[ExampleOutcome]) -> LatticeReport {
    let distinct: Vec<usize> = outcomes
        .iter()
        .map(|outcome| outcome.distinct_asts)
        .collect();
    let with_a_choice = proportion(
        distinct.iter().filter(|count| **count >= 2).count(),
        outcomes.len(),
    );
    let gold_not_first = proportion(
        outcomes
            .iter()
            .filter(|outcome| outcome.gold_rank.is_some_and(|rank| rank > 1))
            .count(),
        outcomes.len(),
    );
    let gold_absent = proportion(
        outcomes
            .iter()
            .filter(|outcome| outcome.gold_rank.is_none())
            .count(),
        outcomes.len(),
    );
    let mut candidates_by_origin: BTreeMap<String, usize> = BTreeMap::new();
    for outcome in outcomes {
        for origin in &outcome.candidate_origins {
            *candidates_by_origin.entry(origin.clone()).or_insert(0) += 1;
        }
    }
    let rerankable = gold_not_first.numerator;
    let verdict = verdict(with_a_choice.numerator, rerankable);
    LatticeReport {
        examples: outcomes.len(),
        distinct_asts: distribution(&distinct),
        with_a_choice,
        gold_not_first,
        gold_absent,
        edited_candidates: outcomes
            .iter()
            .map(|outcome| outcome.edited_candidates)
            .sum(),
        candidates_by_origin,
        truncated_examples: outcomes
            .iter()
            .filter(|outcome| outcome.lattice_truncated)
            .count(),
        verdict,
    }
}

/// The honest answer to "is there enough here for a logistic baseline?".
///
/// Two things have to hold, and they are different. There must be choices to
/// make at all, and the current order must be wrong often enough that
/// learning a better one is measurable rather than noise.
fn verdict(with_a_choice: usize, rerankable: usize) -> String {
    if rerankable == 0 {
        return format!(
            "логистический baseline обучать не на чем: примеров с выбором {with_a_choice}, \
             но ни в одном золотой ответ не стоит ниже первого места. \
             Ранжировщику нечего исправлять — оптимум уже достигается порядком генератора."
        );
    }
    if rerankable < MIN_RERANKABLE_FOR_BASELINE {
        return format!(
            "данных для логистического baseline недостаточно: {rerankable} примеров, где \
             золотой ответ не первый, нужно не менее {MIN_RERANKABLE_FOR_BASELINE}. \
             Это предварительный порог протокола, не теорема об обобщении."
        );
    }
    format!(
        "порог количества достигнут: {rerankable} примеров, где золотой \
         ответ не первый, при {with_a_choice} примерах с выбором. \
         Это не разрешение на обучение: проверьте reranker_readiness, разделение train/validation, \
         долю RAW и независимость семей."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::Target;
    use crate::evaluate::{ErrorStage, SystemAction};
    use crate::schema::TargetAction;

    fn outcome(distinct: usize, gold_rank: Option<usize>) -> ExampleOutcome {
        ExampleOutcome {
            id: "x".into(),
            family_id: "x".into(),
            domain: "plain".into(),
            tags: vec![],
            target_action: TargetAction::Raw,
            selected_key: None,
            selected_source: None,
            selected_warnings: vec![],
            requested_domain: None,
            hypothesis_index: None,
            transcript: String::new(),
            emitted: Target::Raw,
            action: SystemAction::KeepRaw,
            gold_rank,
            candidate_count: distinct + 1,
            selection_correct: false,
            payload_correct: false,
            expected_payload: String::new(),
            emitted_payload: String::new(),
            routing_correct: None,
            structurally_valid: true,
            render_match: None,
            latency_us: 0,
            severity: None,
            first_blocking: None::<ErrorStage>,
            selected_confidence: None,
            selected_is_ast: false,
            ast_distance: None,
            distinct_asts: distinct,
            candidate_origins: vec![],
            edited_candidates: 0,
            lattice_truncated: false,
        }
    }

    #[test]
    fn a_lattice_with_no_choices_says_so_instead_of_reporting_a_rate() {
        let report = evaluate(&[outcome(1, Some(1)), outcome(1, Some(1))]);
        assert_eq!(report.with_a_choice.numerator, 0);
        assert_eq!(report.gold_not_first.numerator, 0);
        assert!(
            report.verdict.contains("нечего исправлять"),
            "{}",
            report.verdict
        );
    }

    #[test]
    fn choices_without_mistakes_are_still_not_training_data() {
        // Four readings every time, and the right one always already first.
        let outcomes: Vec<_> = (0..40).map(|_| outcome(4, Some(1))).collect();
        let report = evaluate(&outcomes);
        assert_eq!(report.with_a_choice.numerator, 40);
        assert_eq!(report.gold_not_first.numerator, 0);
        assert!(
            report.verdict.contains("нечего исправлять"),
            "{}",
            report.verdict
        );
    }

    #[test]
    fn a_handful_of_reranking_opportunities_is_reported_as_too_few() {
        let mut outcomes: Vec<_> = (0..10).map(|_| outcome(3, Some(2))).collect();
        outcomes.extend((0..10).map(|_| outcome(3, Some(1))));
        let report = evaluate(&outcomes);
        assert_eq!(report.gold_not_first.numerator, 10);
        assert!(
            report.verdict.contains("недостаточно"),
            "{}",
            report.verdict
        );
        assert!(
            report
                .verdict
                .contains(&MIN_RERANKABLE_FOR_BASELINE.to_string()),
            "the verdict must name the bar it failed: {}",
            report.verdict
        );
    }

    #[test]
    fn enough_opportunities_do_not_authorize_training() {
        let outcomes: Vec<_> = (0..MIN_RERANKABLE_FOR_BASELINE)
            .map(|_| outcome(3, Some(2)))
            .collect();
        let report = evaluate(&outcomes);
        assert!(report.verdict.contains("порог количества достигнут"));
        assert!(report.verdict.contains("не разрешение на обучение"));
        assert!(report.verdict.contains("reranker_readiness"));
    }

    #[test]
    fn a_missing_gold_is_counted_apart_from_a_misranked_one() {
        let report = evaluate(&[outcome(2, None), outcome(2, Some(3)), outcome(2, Some(1))]);
        assert_eq!(report.gold_absent.numerator, 1);
        assert_eq!(report.gold_not_first.numerator, 1);
    }

    #[test]
    fn the_distribution_is_read_off_a_hand_table() {
        let report = evaluate(&[
            outcome(1, Some(1)),
            outcome(3, Some(1)),
            outcome(2, Some(1)),
            outcome(5, Some(1)),
        ]);
        assert_eq!(report.distinct_asts.min, 1);
        assert_eq!(report.distinct_asts.max, 5);
        // sorted 1,2,3,5 — nearest-rank median at 50% is the 2nd value.
        assert_eq!(report.distinct_asts.median, 2);
        assert_eq!(report.distinct_asts.mean, Some(2.75));
        assert_eq!(report.distinct_asts.histogram["3"], 1);
    }
}

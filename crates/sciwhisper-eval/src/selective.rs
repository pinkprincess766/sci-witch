//! When should the program answer, and when should it keep quiet?
//!
//! The threshold that decides is `0.9`, and it was chosen by hand. This
//! module is what makes that number answerable instead of assumed: it
//! replays the insert/abstain rule at every threshold and reports what each
//! one would have cost.
//!
//! # Two numbers, and they trade against each other
//!
//! * **coverage** — how often the program answers at all;
//! * **risk** — of the times it answered, how often it was wrong.
//!
//! Raising the threshold buys safety with silence. The point of a
//! risk–coverage curve is that the trade is visible rather than argued
//! about.
//!
//! # What this module refuses to do
//!
//! It does not fit anything. Calibration needs errors to learn from, and a
//! corpus with one error cannot support a fitted model — it can only
//! support the statement that it cannot. [`Calibration::verdict`] says so
//! in as many words, and says how many examples would be needed instead.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::evaluate::ExampleOutcome;
use crate::metrics::{proportion, Proportion};

/// Thresholds the curve is reported at.
///
/// The grid is the set of confidences the parser actually produces plus the
/// boundaries between them; a finer grid would print more rows that all say
/// the same thing, because the score takes four values and nothing in
/// between.
pub const THRESHOLD_GRID: [f32; 7] = [0.0, 0.5, 0.7, 0.8, 0.9, 0.95, 1.0];

/// How many examples must sit at a confidence level before its accuracy
/// means anything.
///
/// Below this a level's observed accuracy is noise: one error in five
/// examples and one in five hundred are the same number and completely
/// different evidence.
pub const MIN_EXAMPLES_PER_LEVEL: usize = 30;

/// Errors needed before a threshold can be chosen from data rather than by
/// hand. With fewer, every candidate threshold looks equally good.
pub const MIN_ERRORS_TO_FIT: usize = 20;

#[derive(Clone, Debug, Serialize)]
pub struct SelectivePrediction {
    /// What each threshold would have cost.
    pub curve: Vec<OperatingPoint>,
    /// The threshold the corpus actually ran at.
    pub configured_threshold: f32,
    /// Thresholds that give up nothing against the configured one on **all
    /// three** counts: they answer at least as often, are right at least as
    /// often, and are wrong-when-answering no more often.
    ///
    /// The third condition is not redundant. Accuracy treats a wrong
    /// insertion and a wrong silence as the same miss, and this project does
    /// not — the severity taxonomy puts a confident wrong answer above a
    /// safe abstention. Without it this list recommended a threshold of
    /// `0.0`, «answer always», because two extra answers, one right and one
    /// wrong, left accuracy unchanged.
    pub no_worse_than_configured: Vec<f32>,
    pub calibration: Calibration,
}

#[derive(Clone, Debug, Serialize)]
pub struct OperatingPoint {
    pub threshold: f32,
    /// Answered at all.
    pub coverage: Proportion,
    /// Wrong, among those answered. `None` when nothing was answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk: Option<Proportion>,
    /// Right, over the whole corpus — abstaining on ordinary speech counts
    /// as right, which is what makes this the number a user experiences.
    pub accuracy: Proportion,
}

/// How well the score means what it says.
#[derive(Clone, Debug, Serialize)]
pub struct Calibration {
    /// One row per distinct confidence the parser produced.
    pub levels: Vec<Level>,
    /// Mean squared difference between the score and the outcome. Lower is
    /// better; 0.25 is what a constant 0.5 would score.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brier: Option<f64>,
    /// Weighted mean gap between a level's score and its accuracy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_calibration_error: Option<f64>,
    pub scored_examples: usize,
    pub errors: usize,
    /// Whether the corpus can support choosing a threshold from data.
    pub verdict: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Level {
    pub confidence: f32,
    pub examples: usize,
    pub accuracy: Proportion,
    /// Whether this level has enough examples to be read as evidence.
    pub trustworthy: bool,
}

/// What one example does at a given threshold.
///
/// Extracted from the outcome rather than recomputed by re-running the
/// pipeline: the rule is `insert if the answer is a structure and its
/// confidence clears the bar`, and both facts are already recorded.
struct Decision {
    is_ast: bool,
    confidence: f32,
    /// Whether inserting this answer would match the corpus.
    insert_is_correct: bool,
    /// Whether keeping the words would match the corpus.
    abstain_is_correct: bool,
}

fn decisions(outcomes: &[ExampleOutcome]) -> Vec<Decision> {
    outcomes
        .iter()
        .map(|outcome| {
            let is_ast = outcome.selected_is_ast;
            Decision {
                is_ast,
                confidence: outcome.selected_confidence.unwrap_or(0.0),
                // `selection_correct` compares the chosen answer with the
                // gold one, whichever kind each is.
                insert_is_correct: is_ast && outcome.selection_correct,
                abstain_is_correct: outcome.target_action == crate::schema::TargetAction::Raw,
            }
        })
        .collect()
}

fn point(decisions: &[Decision], threshold: f32) -> OperatingPoint {
    let mut answered = 0usize;
    let mut answered_wrong = 0usize;
    let mut correct = 0usize;
    for decision in decisions {
        let inserted = decision.is_ast && decision.confidence >= threshold;
        if inserted {
            answered += 1;
            if decision.insert_is_correct {
                correct += 1;
            } else {
                answered_wrong += 1;
            }
        } else if decision.abstain_is_correct {
            correct += 1;
        }
    }
    OperatingPoint {
        threshold,
        coverage: proportion(answered, decisions.len()),
        risk: (answered > 0).then(|| proportion(answered_wrong, answered)),
        accuracy: proportion(correct, decisions.len()),
    }
}

pub fn evaluate(outcomes: &[ExampleOutcome], configured_threshold: f32) -> SelectivePrediction {
    let decisions = decisions(outcomes);
    let curve: Vec<OperatingPoint> = THRESHOLD_GRID
        .iter()
        .map(|threshold| point(&decisions, *threshold))
        .collect();

    let configured = point(&decisions, configured_threshold);
    let no_worse_than_configured = curve
        .iter()
        .filter(|candidate| no_worse(candidate, &configured))
        .map(|candidate| candidate.threshold)
        .collect();

    SelectivePrediction {
        curve,
        configured_threshold,
        no_worse_than_configured,
        calibration: calibrate(&decisions),
    }
}

/// Groups the scored answers by the confidence they were given.
fn calibrate(decisions: &[Decision]) -> Calibration {
    let scored: Vec<&Decision> = decisions.iter().filter(|d| d.is_ast).collect();
    let mut buckets: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for decision in &scored {
        let key = format!("{:.4}", decision.confidence);
        let entry = buckets.entry(key).or_insert((0, 0));
        entry.0 += 1;
        if decision.insert_is_correct {
            entry.1 += 1;
        }
    }
    let levels: Vec<Level> = buckets
        .iter()
        .map(|(key, (count, correct))| Level {
            confidence: key.parse().unwrap_or(0.0),
            examples: *count,
            accuracy: proportion(*correct, *count),
            trustworthy: *count >= MIN_EXAMPLES_PER_LEVEL,
        })
        .collect();

    let errors = scored.iter().filter(|d| !d.insert_is_correct).count();
    let (brier, ece) = if scored.is_empty() {
        (None, None)
    } else {
        let brier = scored
            .iter()
            .map(|d| {
                let outcome = f64::from(u8::from(d.insert_is_correct));
                let score = f64::from(d.confidence);
                (score - outcome).powi(2)
            })
            .sum::<f64>()
            / scored.len() as f64;
        let ece = levels
            .iter()
            .map(|level| {
                let weight = level.examples as f64 / scored.len() as f64;
                let observed = level.accuracy.value.unwrap_or(0.0);
                weight * (f64::from(level.confidence) - observed).abs()
            })
            .sum::<f64>();
        (Some(round(brier)), Some(round(ece)))
    };

    let distinct = levels.len();
    let verdict = if errors < MIN_ERRORS_TO_FIT {
        format!(
            "порог нельзя выбрать по данным: ошибок среди отвеченного {errors}, нужно не менее {MIN_ERRORS_TO_FIT}. \
             При таком числе ошибок любые два порога различаются в пределах шума, и «лучший» из них будет выбран случайно. \
             Оценка приняла {distinct} различных значений — калибровать шкалу почти без размаха тоже нечем."
        )
    } else if levels.iter().all(|level| !level.trustworthy) {
        format!(
            "калибровка не читается: ни на одном уровне уверенности нет {MIN_EXAMPLES_PER_LEVEL} примеров"
        )
    } else {
        "данных достаточно, чтобы выбирать порог по кривой risk–coverage".into()
    };

    Calibration {
        levels,
        brier,
        expected_calibration_error: ece,
        scored_examples: scored.len(),
        errors,
        verdict,
    }
}

/// Whether `candidate` gives up nothing against `reference`.
///
/// A trade is the owner's call, not a recommendation this file may make, so
/// only a threshold that is better-or-equal on every count is offered.
fn no_worse(candidate: &OperatingPoint, reference: &OperatingPoint) -> bool {
    let risk_of = |point: &OperatingPoint| point.risk.as_ref().and_then(|risk| risk.value);
    candidate.threshold != reference.threshold
        && at_least(candidate.coverage.value, reference.coverage.value)
        && at_least(candidate.accuracy.value, reference.accuracy.value)
        && at_most(risk_of(candidate), risk_of(reference))
}

/// Compares risk when one side may not have any.
///
/// «Nothing wrong out of nothing answered» is not a safety record, so a
/// threshold that starts answering where the configured one stayed silent
/// is only offered when it is **never** wrong. Anything else is a trade
/// between speaking up and being wrong, and this file does not make those.
fn at_most(candidate: Option<f64>, reference: Option<f64>) -> bool {
    match (candidate, reference) {
        (Some(candidate), Some(reference)) => candidate <= reference,
        (None, _) => true,
        (Some(candidate), None) => candidate == 0.0,
    }
}

/// Compares two measurements that may not exist. A measurement that was
/// never taken is not "at least" anything.
fn at_least(candidate: Option<f64>, reference: Option<f64>) -> bool {
    match (candidate, reference) {
        (Some(candidate), Some(reference)) => candidate >= reference,
        _ => false,
    }
}

fn round(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision(is_ast: bool, confidence: f32, insert_ok: bool, abstain_ok: bool) -> Decision {
        Decision {
            is_ast,
            confidence,
            insert_is_correct: insert_ok,
            abstain_is_correct: abstain_ok,
        }
    }

    /// Raising the threshold trades coverage for safety. The curve is what
    /// makes that trade visible instead of argued about.
    #[test]
    fn raising_the_threshold_buys_safety_with_silence() {
        let decisions = vec![
            // A confident, correct answer.
            decision(true, 0.95, true, false),
            // A hesitant answer that happens to be right.
            decision(true, 0.7, true, false),
            // A hesitant answer that is wrong.
            decision(true, 0.7, false, true),
        ];
        let low = point(&decisions, 0.5);
        let high = point(&decisions, 0.9);

        assert_eq!((low.coverage.numerator, low.coverage.denominator), (3, 3));
        assert_eq!(low.risk.as_ref().unwrap().numerator, 1);
        assert_eq!((high.coverage.numerator, high.coverage.denominator), (1, 3));
        assert_eq!(high.risk.as_ref().unwrap().numerator, 0);
        // Silence is right for the third example and wrong for the second.
        assert_eq!(high.accuracy.numerator, 2);
        assert_eq!(low.accuracy.numerator, 2);
    }

    /// Abstaining on ordinary speech is a correct answer, not a missing one.
    #[test]
    fn keeping_the_words_counts_as_right_where_that_is_what_was_wanted() {
        let decisions = vec![decision(false, 0.0, false, true)];
        let p = point(&decisions, 0.9);
        assert_eq!(p.coverage.numerator, 0);
        assert!(
            p.risk.is_none(),
            "nothing was answered, so nothing was risked"
        );
        assert_eq!(p.accuracy.numerator, 1);
    }

    /// The refusal that matters: a corpus with almost no errors cannot say
    /// which threshold is better, and must not pretend to.
    #[test]
    fn a_corpus_with_too_few_errors_refuses_to_choose_a_threshold() {
        let mut decisions: Vec<Decision> = (0..100)
            .map(|_| decision(true, 0.95, true, false))
            .collect();
        decisions.push(decision(true, 0.7, false, true));
        let report = evaluate_decisions(&decisions, 0.9);
        assert_eq!(report.calibration.errors, 1);
        assert!(
            report
                .calibration
                .verdict
                .contains("нельзя выбрать по данным"),
            "{}",
            report.calibration.verdict
        );
    }

    #[test]
    fn enough_errors_let_the_curve_be_read() {
        let mut decisions: Vec<Decision> =
            (0..60).map(|_| decision(true, 0.95, true, false)).collect();
        decisions.extend((0..40).map(|i| decision(true, 0.7, i % 2 == 0, i % 2 == 1)));
        let report = evaluate_decisions(&decisions, 0.9);
        assert!(report.calibration.errors >= MIN_ERRORS_TO_FIT);
        assert!(
            report.calibration.verdict.contains("достаточно"),
            "{}",
            report.calibration.verdict
        );
        // 0.7 is right half the time, so the score overstates it.
        let level = report
            .calibration
            .levels
            .iter()
            .find(|level| (level.confidence - 0.7).abs() < 1e-6)
            .expect("a 0.7 level");
        assert!(level.trustworthy);
        assert!((level.accuracy.value.unwrap() - 0.5).abs() < 0.01);
        assert!(report.calibration.expected_calibration_error.unwrap() > 0.05);
    }

    /// A level with a handful of examples is noise, and is marked as such.
    #[test]
    fn a_thin_level_is_not_read_as_evidence() {
        let decisions = vec![decision(true, 0.7, false, true)];
        let report = evaluate_decisions(&decisions, 0.9);
        assert!(!report.calibration.levels[0].trustworthy);
    }

    /// A threshold is only suggested when it is better on both counts;
    /// a trade is the owner's decision, not this file's.
    #[test]
    fn only_a_strictly_no_worse_threshold_is_offered() {
        // Everything is confident and correct: lowering the bar costs
        // nothing and answers more.
        let decisions: Vec<Decision> = (0..10).map(|_| decision(true, 0.7, true, false)).collect();
        let report = evaluate_decisions(&decisions, 0.9);
        assert!(report.no_worse_than_configured.contains(&0.7));

        // Now the hesitant answers are wrong: lowering the bar is a trade,
        // so nothing below the configured threshold is offered.
        let decisions: Vec<Decision> = (0..10).map(|_| decision(true, 0.7, false, true)).collect();
        let report = evaluate_decisions(&decisions, 0.9);
        assert!(!report.no_worse_than_configured.contains(&0.7));
    }

    /// The test-only entry point, so the curve can be exercised without
    /// building a whole corpus of outcomes.
    fn evaluate_decisions(decisions: &[Decision], configured: f32) -> SelectivePrediction {
        let curve: Vec<OperatingPoint> = THRESHOLD_GRID
            .iter()
            .map(|threshold| point(decisions, *threshold))
            .collect();
        let configured_point = point(decisions, configured);
        let no_worse_than_configured = curve
            .iter()
            .filter(|candidate| no_worse(candidate, &configured_point))
            .map(|candidate| candidate.threshold)
            .collect();
        SelectivePrediction {
            curve,
            configured_threshold: configured,
            no_worse_than_configured,
            calibration: calibrate(decisions),
        }
    }
}

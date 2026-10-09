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

use std::cmp::Ordering;
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

/// Area under the risk–coverage curve over every coverage level.
///
/// Sort the examples by score, highest first. With the top `k` answered
/// (`k = 1..=n`), the risk is the share of wrong answers among those `k`.
/// The result is the mean of the `n` risks (Geifman & El-Yaniv, 2017). Lower
/// is better: 0 when every answer is right, 1 when every answer is wrong, and
/// a ranking that puts the wrong answers first pushes it up.
///
/// This uses every coverage level, not [`THRESHOLD_GRID`]. The grid is a set
/// of reporting rows and says nothing about the area between them.
///
/// Ties: inside a block of equal scores the order is not known, so each
/// coverage level inside the block gets its expected risk over a random
/// order of the block: `j` answers into a block of `m` with `w` wrong add
/// `j · w / m` errors. The result does not depend on the input order. This
/// matters here: the manual parse levels take four values, so almost every
/// example is in a tie.
///
/// `None` for empty input, lengths that differ, or any NaN score.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "not yet wired into a report; tested below")
)]
pub fn aurc(scores: &[f64], correct: &[bool]) -> Option<f64> {
    if scores.is_empty() || scores.len() != correct.len() || scores.iter().any(|s| s.is_nan()) {
        return None;
    }
    let mut ranked: Vec<(f64, bool)> = scores
        .iter()
        .copied()
        .zip(correct.iter().copied())
        .collect();
    // Highest score first. NaN was rejected above, so the fallback is
    // unreachable; it only keeps this free of a panic path.
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));

    let mut answered = 0usize;
    let mut wrong_before = 0usize;
    let mut risk_sum = 0.0;
    for block in ranked.chunk_by(|a, b| a.0 == b.0) {
        let size = block.len();
        let wrong = block.iter().filter(|(_, right)| !right).count();
        for j in 1..=size {
            let expected_wrong = wrong_before as f64 + (j * wrong) as f64 / size as f64;
            risk_sum += expected_wrong / (answered + j) as f64;
        }
        answered += size;
        wrong_before += wrong;
    }
    Some(risk_sum / ranked.len() as f64)
}

/// Lowest probability [`log_loss`] sees. It keeps `-ln 0` out of the sum: one
/// certain prediction that is wrong would otherwise make the mean infinite.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "not yet wired into a report; tested below")
)]
pub const LOG_LOSS_EPS: f64 = 1e-15;

/// Log loss (cross-entropy) of a fitted model's probabilities.
///
/// For each example it takes `-ln p` if the answer was right and `-ln(1 - p)`
/// if it was wrong, then averages. Lower is better; a constant `p = 0.5`
/// scores `ln 2`.
///
/// Log loss is defined only for a probability. The compiler's `confidence` is
/// a parse level (1.0, 0.95, 0.7, 0.0), not a probability (see `AGENTS.md`),
/// so passing it here is a category error: the number would be read as a
/// probability it is not. This function is for the output of a fitted model,
/// as in question 2, model 2 (`research/protocol/question-2-calibration.md`).
///
/// Each `p` is clipped to `[LOG_LOSS_EPS, 1 - LOG_LOSS_EPS]` first, so a
/// certain prediction that turns out wrong costs about 34.5, not infinity.
///
/// `None` for empty input, lengths that differ, or any probability outside
/// `[0, 1]`, NaN included.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "not yet wired into a report; tested below")
)]
pub fn log_loss(probabilities: &[f64], correct: &[bool]) -> Option<f64> {
    if probabilities.is_empty() || probabilities.len() != correct.len() {
        return None;
    }
    // A NaN fails the range check too, so it is rejected here.
    if !probabilities.iter().all(|p| (0.0..=1.0).contains(p)) {
        return None;
    }
    let total: f64 = probabilities
        .iter()
        .zip(correct)
        .map(|(&p, &right)| {
            let p = p.clamp(LOG_LOSS_EPS, 1.0 - LOG_LOSS_EPS);
            -(if right { p } else { 1.0 - p }).ln()
        })
        .sum();
    Some(total / probabilities.len() as f64)
}

/// Brier score of a fitted model's probabilities: the mean of `(p - y)^2`,
/// with `y` 1 for a right answer and 0 for a wrong one. Lower is better; a
/// constant `p = 0.5` scores 0.25, and a perfect 0/1 model scores 0.
///
/// Like [`log_loss`], this is for a probability. The compiler's `confidence`
/// is a parse level, not a probability (see `AGENTS.md`), so it does not go
/// here.
///
/// `None` for empty input, lengths that differ, or any probability outside
/// `[0, 1]`, NaN included.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "not yet wired into a report; tested below")
)]
pub fn brier_score(probabilities: &[f64], correct: &[bool]) -> Option<f64> {
    if probabilities.is_empty() || probabilities.len() != correct.len() {
        return None;
    }
    // A NaN fails the range check too, so it is rejected here.
    if !probabilities.iter().all(|p| (0.0..=1.0).contains(p)) {
        return None;
    }
    let total: f64 = probabilities
        .iter()
        .zip(correct)
        .map(|(&p, &right)| {
            let outcome = f64::from(u8::from(right));
            (p - outcome).powi(2)
        })
        .sum();
    Some(total / probabilities.len() as f64)
}

/// Upper limit on the bin count of [`expected_calibration_error`]. The bins
/// are allocated up front, so an unbounded count would be an unbounded
/// allocation.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "not yet wired into a report; tested below")
)]
pub const MAX_ECE_BINS: usize = 100;

/// Expected calibration error of a fitted model's probabilities, over
/// `bins` equal-width bins on `[0, 1]`.
///
/// A probability `p` goes to bin `min(floor(p · bins), bins − 1)`, so `p = 1`
/// lands in the last bin, not in a bin of its own. Each non-empty bin adds
/// `(n_b / N) · |accuracy_b − mean p_b|`: the gap between how often the bin
/// was right and what it predicted, weighted by how many examples it holds.
/// Lower is better; 0 means every bin is calibrated.
///
/// The estimate is biased, and its value depends on `bins`: the same
/// predictions give different numbers with 10 bins and with 20. Report it
/// with [`brier_score`] and [`log_loss`], not alone.
///
/// Like [`log_loss`], this is for a fitted model's probability. The compiler's
/// `confidence` is a parse level, not a probability (see `AGENTS.md`).
///
/// `None` for empty input, lengths that differ, any probability outside
/// `[0, 1]` (NaN included), or `bins` outside `1..=MAX_ECE_BINS`.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "not yet wired into a report; tested below")
)]
pub fn expected_calibration_error(
    probabilities: &[f64],
    correct: &[bool],
    bins: usize,
) -> Option<f64> {
    if probabilities.is_empty() || probabilities.len() != correct.len() {
        return None;
    }
    // A NaN fails the range check too, so it is rejected here.
    if !probabilities.iter().all(|p| (0.0..=1.0).contains(p)) {
        return None;
    }
    if bins == 0 || bins > MAX_ECE_BINS {
        return None;
    }
    let mut count = vec![0usize; bins];
    let mut sum_p = vec![0.0f64; bins];
    let mut right = vec![0usize; bins];
    for (&p, &is_right) in probabilities.iter().zip(correct) {
        let bin = ((p * bins as f64).floor() as usize).min(bins - 1);
        count[bin] += 1;
        sum_p[bin] += p;
        right[bin] += usize::from(is_right);
    }
    let total = probabilities.len() as f64;
    let ece = (0..bins)
        .filter(|&bin| count[bin] > 0)
        .map(|bin| {
            let weight = count[bin] as f64 / total;
            let accuracy = right[bin] as f64 / count[bin] as f64;
            let mean_p = sum_p[bin] / count[bin] as f64;
            weight * (accuracy - mean_p).abs()
        })
        .sum();
    Some(ece)
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

    fn close(actual: Option<f64>, expected: f64, tolerance: f64) -> bool {
        actual.is_some_and(|value| (value - expected).abs() <= tolerance)
    }

    /// Hand values: scores 0.9, 0.8, 0.7, 0.6 with answers right, wrong,
    /// right, wrong. Risks at k = 1..4 are 0/1, 1/2, 1/3, 2/4, so
    /// AURC = (0 + 1/2 + 1/3 + 1/2) / 4 = 1/3.
    #[test]
    fn aurc_matches_hand_computed_risks() {
        let scores = [0.9, 0.8, 0.7, 0.6];
        let correct = [true, false, true, false];
        assert!(close(aurc(&scores, &correct), 1.0 / 3.0, 1e-12));
    }

    #[test]
    fn aurc_is_zero_when_every_answer_is_right() {
        let scores = [0.9, 0.5, 0.1];
        assert_eq!(aurc(&scores, &[true, true, true]), Some(0.0));
    }

    #[test]
    fn aurc_is_one_when_every_answer_is_wrong() {
        let scores = [0.9, 0.5, 0.1];
        assert!(close(aurc(&scores, &[false, false, false]), 1.0, 1e-12));
    }

    /// Perfect ranking (right answers scored higher): risks 0, 0, 1/3, 2/4,
    /// so AURC = (5/6) / 4 = 5/24.
    /// Reversed ranking (same answers, scores flipped): risks 1, 1, 2/3, 2/4,
    /// so AURC = (19/6) / 4 = 19/24.
    /// An implementation that ignores the scores gives 5/24 for both, and
    /// fails the second assertion.
    #[test]
    fn aurc_punishes_a_ranking_that_puts_wrong_answers_first() {
        let correct = [true, true, false, false];
        let perfect = [0.9, 0.8, 0.7, 0.6];
        let reversed = [0.6, 0.7, 0.8, 0.9];
        let good = aurc(&perfect, &correct).unwrap();
        let bad = aurc(&reversed, &correct).unwrap();
        assert!(close(Some(good), 5.0 / 24.0, 1e-12), "{good}");
        assert!(close(Some(bad), 19.0 / 24.0, 1e-12), "{bad}");
        assert!(bad > good);
    }

    /// Ties take the expected risk over a random order of the block. Four
    /// scores of 0.5 with two wrong: every risk inside the block is 1/2, so
    /// AURC = 1/2 in either input order. [0.5, 0.9, 0.5] with answers wrong,
    /// right, right: 0.9 first (risk 0), then the tied pair with one wrong:
    /// (0 + 1/2)/2 = 1/4 and 1/3, so AURC = (0 + 1/4 + 1/3)/3 = 7/36 — the
    /// mean of the two orders, 5/18 and 1/9. A rule that kept input order
    /// would give 5/18 for one order and 1/9 for the other.
    #[test]
    fn aurc_ties_do_not_depend_on_input_order() {
        let flat = [0.5, 0.5, 0.5, 0.5];
        assert!(close(aurc(&flat, &[true, false, true, false]), 0.5, 1e-12));
        assert!(close(aurc(&flat, &[false, true, false, true]), 0.5, 1e-12));
        let one = aurc(&[0.5, 0.9, 0.5], &[false, true, true]).unwrap();
        let other = aurc(&[0.5, 0.9, 0.5], &[true, true, false]).unwrap();
        assert!(close(Some(one), 7.0 / 36.0, 1e-12), "{one}");
        assert!(close(Some(other), 7.0 / 36.0, 1e-12), "{other}");
    }

    #[test]
    fn aurc_is_none_for_bad_input() {
        assert_eq!(aurc(&[], &[]), None);
        assert_eq!(aurc(&[0.5], &[]), None);
        assert_eq!(aurc(&[0.5, 0.4], &[true]), None);
        assert_eq!(aurc(&[f64::NAN, 0.4], &[true, false]), None);
    }

    /// p = 0.5 everywhere: every term is -ln 0.5 = ln 2, whatever the answer.
    #[test]
    fn log_loss_of_a_coin_is_ln_2() {
        let p = [0.5, 0.5, 0.5];
        let loss = log_loss(&p, &[true, false, true]).unwrap();
        assert!((loss - std::f64::consts::LN_2).abs() < 1e-12, "{loss}");
    }

    /// Right at p = 0.9 costs -ln 0.9. Wrong at p = 0.9 costs -ln 0.1.
    #[test]
    fn log_loss_takes_minus_ln_p_for_right_and_minus_ln_one_minus_p_for_wrong() {
        let right = log_loss(&[0.9], &[true]).unwrap();
        assert!((right - (-(0.9f64.ln()))).abs() < 1e-12, "{right}");
        let wrong = log_loss(&[0.9], &[false]).unwrap();
        assert!((wrong - (-(0.1f64.ln()))).abs() < 1e-12, "{wrong}");
        // Mean of the two: (-ln 0.9 - ln 0.1) / 2 = 1.2039728...
        let mean = log_loss(&[0.9, 0.9], &[true, false]).unwrap();
        assert!((mean - 1.203_972_804_325_936).abs() < 1e-12, "{mean}");
    }

    /// p = 0 for a right answer would be -ln 0 = infinity. The clip makes it
    /// -ln(1e-15) = 15 ln 10 = 34.5387...; it is finite and equal to that.
    /// p = 0 for a wrong answer is -ln(1 - 1e-15), about 1e-15.
    /// p = 1 with the roles swapped. There the complement is
    /// `1 - (1 - 1e-15)` in f64, which is 9.99e-16 and not exactly 1e-15
    /// (the nearest double to 1 - 1e-15 is off by one ulp of 1.1e-16). So
    /// that case is compared with the same expression, and with 15 ln 10 to
    /// within 0.1%.
    #[test]
    fn log_loss_clips_at_zero_and_one() {
        let big = 15.0 * 10.0f64.ln();
        let at_zero_right = log_loss(&[0.0], &[true]).unwrap();
        assert!(at_zero_right.is_finite());
        assert!((at_zero_right - big).abs() < 1e-9, "{at_zero_right}");

        let at_zero_wrong = log_loss(&[0.0], &[false]).unwrap();
        assert!(at_zero_wrong.abs() < 1e-14, "{at_zero_wrong}");

        let at_one_right = log_loss(&[1.0], &[true]).unwrap();
        assert!(at_one_right.abs() < 1e-14, "{at_one_right}");

        let at_one_wrong = log_loss(&[1.0], &[false]).unwrap();
        let clipped_complement = 1.0 - (1.0 - LOG_LOSS_EPS);
        assert!(at_one_wrong.is_finite());
        assert!(
            (at_one_wrong + clipped_complement.ln()).abs() < 1e-12,
            "{at_one_wrong}"
        );
        assert!((at_one_wrong - big).abs() / big < 1e-3, "{at_one_wrong}");
    }

    /// The clip does not touch values inside the range: p = 1e-12 is used as
    /// it is, and -ln(1e-12) = 27.631...
    #[test]
    fn log_loss_leaves_probabilities_inside_the_clip_alone() {
        let loss = log_loss(&[1e-12], &[true]).unwrap();
        assert!((loss - 27.631_021_115_928_547).abs() < 1e-9, "{loss}");
    }

    #[test]
    fn log_loss_is_none_for_invalid_input() {
        assert_eq!(log_loss(&[], &[]), None);
        assert_eq!(log_loss(&[0.5], &[]), None);
        assert_eq!(log_loss(&[0.5, 0.4], &[true]), None);
        assert_eq!(log_loss(&[-0.1], &[true]), None);
        assert_eq!(log_loss(&[1.1], &[false]), None);
        assert_eq!(log_loss(&[f64::NAN], &[true]), None);
    }

    /// p = 0.5 everywhere: every term is (0.5 - y)^2 = 0.25, right or wrong.
    #[test]
    fn brier_of_a_coin_is_one_quarter() {
        let brier = brier_score(&[0.5, 0.5, 0.5, 0.5], &[true, false, true, false]).unwrap();
        assert!((brier - 0.25).abs() < 1e-12, "{brier}");
    }

    /// p = 0.9 right costs (0.9 - 1)^2 = 0.01. p = 0.2 wrong costs
    /// (0.2 - 0)^2 = 0.04. Mean: (0.01 + 0.04) / 2 = 0.025.
    #[test]
    fn brier_takes_the_squared_gap_to_the_outcome() {
        let brier = brier_score(&[0.9, 0.2], &[true, false]).unwrap();
        assert!((brier - 0.025).abs() < 1e-12, "{brier}");
    }

    /// Certain and right, or certain and wrong: both at the ends of [0, 1]
    /// are accepted, and both score 0.
    #[test]
    fn brier_of_perfect_predictions_is_zero() {
        assert_eq!(brier_score(&[1.0, 0.0], &[true, false]), Some(0.0));
    }

    #[test]
    fn brier_is_none_for_invalid_input() {
        assert_eq!(brier_score(&[], &[]), None);
        assert_eq!(brier_score(&[0.5], &[]), None);
        assert_eq!(brier_score(&[0.5, 0.4], &[true]), None);
        assert_eq!(brier_score(&[-0.1], &[true]), None);
        assert_eq!(brier_score(&[1.1], &[false]), None);
        assert_eq!(brier_score(&[f64::NAN], &[true]), None);
    }

    /// Four predictions in two bins: [0, 0.5) and [0.5, 1].
    /// Bin 0 holds p = 0.25, right: n = 1, accuracy 1, mean p 0.25, gap 0.75.
    /// Bin 1 holds p = 0.75 wrong, 0.5 right, 1.0 right: n = 3, accuracy 2/3,
    /// mean p 0.75, gap 1/12.
    /// ECE = (1/4)(0.75) + (3/4)(1/12) = 0.1875 + 0.0625 = 0.25.
    /// An unweighted mean over the two bins would give (0.75 + 1/12) / 2 = 5/12.
    #[test]
    fn ece_weights_each_bin_by_its_share_of_the_examples() {
        let p = [0.25, 0.75, 0.5, 1.0];
        let right = [true, false, true, true];
        let ece = expected_calibration_error(&p, &right, 2).unwrap();
        assert!((ece - 0.25).abs() < 1e-12, "{ece}");
    }

    /// p = 1 belongs to the last bin. With two bins, 0.75 and 1.0 share bin 1:
    /// n = 2, accuracy 1/2, mean p 7/8, so ECE = 1 · |1/2 − 7/8| = 3/8.
    /// Without the clamp, p = 1 indexes bin 2 of 2 and panics.
    #[test]
    fn ece_puts_probability_one_in_the_last_bin() {
        let ece = expected_calibration_error(&[0.75, 1.0], &[true, false], 2).unwrap();
        assert!((ece - 0.375).abs() < 1e-12, "{ece}");
    }

    /// Empty bins are skipped. Of four bins only bin 0 is used: p = 0.1 and
    /// 0.2, both right, n = 2, accuracy 1, mean p 0.15, so ECE = 0.85. Without
    /// the skip, an empty bin divides 0 by 0 and the sum becomes NaN.
    #[test]
    fn ece_skips_empty_bins() {
        let ece = expected_calibration_error(&[0.1, 0.2], &[true, true], 4).unwrap();
        assert!((ece - 0.85).abs() < 1e-12, "{ece}");
    }

    /// The bin count is accepted from 1 to 100 and refused at 0 and 101.
    #[test]
    fn ece_bin_count_has_a_limit_on_both_sides() {
        let p = [0.5, 0.9];
        let right = [true, false];
        assert!(expected_calibration_error(&p, &right, 1).is_some());
        assert!(expected_calibration_error(&p, &right, 100).is_some());
        assert_eq!(expected_calibration_error(&p, &right, 101), None);
        assert_eq!(expected_calibration_error(&p, &right, 0), None);
    }

    #[test]
    fn ece_is_none_for_invalid_input() {
        assert_eq!(expected_calibration_error(&[], &[], 10), None);
        assert_eq!(expected_calibration_error(&[0.5], &[], 10), None);
        assert_eq!(expected_calibration_error(&[0.5, 0.4], &[true], 10), None);
        assert_eq!(expected_calibration_error(&[-0.1], &[true], 10), None);
        assert_eq!(expected_calibration_error(&[1.1], &[false], 10), None);
        assert_eq!(expected_calibration_error(&[f64::NAN], &[true], 10), None);
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

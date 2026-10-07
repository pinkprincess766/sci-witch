//! Metric primitives.
//!
//! Everything here is a pure function over small inputs so that each metric
//! can be checked against a table computed by hand, rather than against
//! another call of itself.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::canonical::Target;

/// 95% two-sided normal quantile.
const Z95: f64 = 1.959_963_984_540_054;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CiMethod {
    Wilson,
    Bootstrap,
    /// Percentile bootstrap that resamples whole clusters (speakers), not
    /// single examples; see [`cluster_bootstrap_proportion`].
    ClusterBootstrap,
    /// No interval is meaningful because the denominator is zero.
    Undefined,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Proportion {
    pub numerator: usize,
    pub denominator: usize,
    /// `None` when nothing was measured, so an empty slice never reads as 0%.
    pub value: Option<f64>,
    pub ci95_low: Option<f64>,
    pub ci95_high: Option<f64>,
    pub ci_method: CiMethod,
    /// Exact one-sided 95% upper bound when zero events were observed.
    /// An observed zero is not a proven zero probability.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zero_count_upper95: Option<f64>,
}

/// Wilson score interval: correct for binomial proportions at the small
/// counts this corpus produces, where a normal approximation would put the
/// bound outside `[0, 1]`.
pub fn proportion(numerator: usize, denominator: usize) -> Proportion {
    if denominator == 0 {
        return Proportion {
            numerator,
            denominator,
            value: None,
            ci95_low: None,
            ci95_high: None,
            ci_method: CiMethod::Undefined,
            zero_count_upper95: None,
        };
    }
    let n = denominator as f64;
    let p = numerator as f64 / n;
    let z2 = Z95 * Z95;
    let denominator_term = 1.0 + z2 / n;
    let centre = (p + z2 / (2.0 * n)) / denominator_term;
    let spread = (Z95 * ((p * (1.0 - p) / n) + z2 / (4.0 * n * n)).sqrt()) / denominator_term;
    // 1 − 0.05^(1/n): the exact Clopper–Pearson upper bound when no event was
    // seen at all.
    let zero_count_upper95 = (numerator == 0).then(|| 1.0 - 0.05f64.powf(1.0 / n));
    Proportion {
        numerator,
        denominator,
        value: Some(p),
        ci95_low: Some((centre - spread).max(0.0)),
        ci95_high: Some((centre + spread).min(1.0)),
        ci_method: CiMethod::Wilson,
        zero_count_upper95,
    }
}

/// Deterministic percentile bootstrap over per-example indicators. Used for
/// the headline rate, where resampling examples is the natural notion of
/// uncertainty; the seed is fixed so two runs agree exactly.
pub fn bootstrap_proportion(indicators: &[bool], seed: u64, resamples: usize) -> Proportion {
    let denominator = indicators.len();
    let numerator = indicators.iter().filter(|hit| **hit).count();
    if denominator == 0 || resamples == 0 {
        return proportion(numerator, denominator);
    }
    let mut rng = Pcg32::new(seed);
    let mut means = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let mut hits = 0usize;
        for _ in 0..denominator {
            let index = (rng.next_u32() as usize) % denominator;
            if indicators[index] {
                hits += 1;
            }
        }
        means.push(hits as f64 / denominator as f64);
    }
    means.sort_by(|a, b| a.partial_cmp(b).expect("bootstrap means are finite"));
    let low = means[percentile_index(means.len(), 2.5)];
    let high = means[percentile_index(means.len(), 97.5)];
    let n = denominator as f64;
    Proportion {
        numerator,
        denominator,
        value: Some(numerator as f64 / n),
        ci95_low: Some(low),
        ci95_high: Some(high),
        ci_method: CiMethod::Bootstrap,
        zero_count_upper95: (numerator == 0).then(|| 1.0 - 0.05f64.powf(1.0 / n)),
    }
}

/// Deterministic percentile bootstrap that resamples **clusters** (speakers)
/// instead of examples. Recordings of one speaker are correlated, so
/// resampling recordings treats them as independent and understates the
/// interval; the unit of independence here is the speaker.
///
/// Each inner `Vec` holds one speaker's indicators. A resample draws
/// `k` speakers with replacement, where `k` is the number of speakers with at
/// least one example, and takes the pooled proportion of the drawn speakers:
/// total hits over total examples. The point estimate is the pooled
/// proportion over all data. The draw is `next_u32() % k`, as in
/// [`bootstrap_proportion`], so with every cluster of size 1 the two
/// functions draw the same indices and give the same interval.
///
/// Edge cases, all fixed here rather than left to the caller:
///
/// - Speakers without examples are dropped up front. A drawn speaker
///   therefore always contributes at least one example, no resample has a
///   zero denominator, and no redraw loop exists.
/// - No clusters, or only empty ones: zero examples, `Undefined`, no value.
/// - `resamples == 0`: the Wilson interval on the pooled counts, exactly as
///   `bootstrap_proportion` does. That interval treats examples as
///   independent, so it is the one case that does not respect clusters.
/// - A single non-empty speaker: every resample is that speaker, the
///   interval collapses to the point estimate. One speaker carries no
///   information about between-speaker variation; the caller must not read
///   the zero width as certainty.
///
/// `zero_count_upper95` is always `None`: the exact binomial bound
/// `1 − 0.05^(1/n)` assumes independent examples, which is the assumption
/// this function exists to drop.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "not yet wired into a report; tested below")
)]
pub fn cluster_bootstrap_proportion(
    clusters: &[Vec<bool>],
    seed: u64,
    resamples: usize,
) -> Proportion {
    // (hits, examples) per non-empty speaker.
    let speakers: Vec<(usize, usize)> = clusters
        .iter()
        .filter(|cluster| !cluster.is_empty())
        .map(|cluster| (cluster.iter().filter(|hit| **hit).count(), cluster.len()))
        .collect();
    let numerator: usize = speakers.iter().map(|(hits, _)| hits).sum();
    let denominator: usize = speakers.iter().map(|(_, examples)| examples).sum();
    if denominator == 0 || resamples == 0 {
        return proportion(numerator, denominator);
    }
    let mut rng = Pcg32::new(seed);
    let mut means = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let mut hits = 0usize;
        let mut examples = 0usize;
        for _ in 0..speakers.len() {
            let (speaker_hits, speaker_examples) =
                speakers[(rng.next_u32() as usize) % speakers.len()];
            hits += speaker_hits;
            examples += speaker_examples;
        }
        means.push(hits as f64 / examples as f64);
    }
    means.sort_by(|a, b| a.partial_cmp(b).expect("bootstrap means are finite"));
    Proportion {
        numerator,
        denominator,
        value: Some(numerator as f64 / denominator as f64),
        ci95_low: Some(means[percentile_index(means.len(), 2.5)]),
        ci95_high: Some(means[percentile_index(means.len(), 97.5)]),
        ci_method: CiMethod::ClusterBootstrap,
        zero_count_upper95: None,
    }
}

/// Nearest-rank percentile on an already sorted slice.
pub fn percentile_index(len: usize, q: f64) -> usize {
    debug_assert!(len > 0);
    let rank = (q / 100.0 * len as f64).ceil() as usize;
    rank.clamp(1, len) - 1
}

pub fn percentile_u64(values: &mut [u64], q: f64) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    Some(values[percentile_index(values.len(), q)])
}

/// `CandidateRecall@K`: the share of examples whose gold answer appears in
/// the first `k` candidates. `ranks` holds the 1-based rank of the gold
/// answer, or `None` when it is absent from the whole generated list.
pub fn recall_at_k(ranks: &[Option<usize>], k: usize) -> Proportion {
    let hits = ranks
        .iter()
        .filter(|rank| matches!(rank, Some(rank) if *rank <= k))
        .count();
    proportion(hits, ranks.len())
}

/// Exact two-sided McNemar test for a paired comparison of two systems on
/// the same examples. `b` counts examples only the first system got right,
/// `c` those only the second got right; examples both got right or both got
/// wrong carry no information about which is better and are not passed in.
///
/// Under the null hypothesis each of the `n = b + c` discordant examples is
/// equally likely to fall either way, so `min(b, c)` is the lower tail of
/// Binomial(n, 1/2). The binomial is symmetric, so the two-sided p-value is
/// twice that tail, capped at 1:
///
/// `p = min(1, 2 · Σ_{k=0}^{min(b,c)} C(n, k) / 2^n)`.
///
/// With no discordant pairs (`b = c = 0`) the formula gives `min(1, 2) = 1`:
/// no evidence either way, never a significant result.
///
/// The terms are summed in log space. `C(n, k)` alone overflows `u64` at
/// `n = 68` and `f64` near `n = 1030`, and `2^-n` underflows soon after, so
/// a direct sum turns into `inf / inf = NaN` at sizes a real comparison
/// reaches. Each term follows from the previous one by
/// `C(n, k + 1) = C(n, k) · (n − k) / (k + 1)`; for `k ≤ n / 2` the terms
/// grow, so each new term is the larger one in the log-sum. The cost is
/// `min(b, c)` steps. A p-value below the smallest positive `f64` comes
/// back as `0.0`.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "not yet wired into a report; tested below")
)]
pub fn mcnemar_exact(b: u64, c: u64) -> f64 {
    let n = b as f64 + c as f64;
    let tail = b.min(c);
    let mut log_term = -n * std::f64::consts::LN_2;
    let mut log_sum = log_term;
    for k in 0..tail {
        let k = k as f64;
        log_term += ((n - k) / (k + 1.0)).ln();
        log_sum = log_term + (log_sum - log_term).exp().ln_1p();
    }
    (std::f64::consts::LN_2 + log_sum).exp().min(1.0)
}

// ------------------------------------------------------------------ severity

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Severity {
    /// Presentation only; the canonical AST is unchanged.
    S0,
    /// A safe structural failure or a safe abstention; nothing scientific was
    /// invented.
    S1,
    /// A scientific symbol, index, exponent, unit, function or grouping
    /// changed.
    S2,
    /// A coefficient, charge, reaction side, operator, derivative order,
    /// limit direction or unit power changed.
    S3,
    /// Ordinary text was rewritten into a scientific statement on its own.
    S4,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::S0 => "S0",
            Severity::S1 => "S1",
            Severity::S2 => "S2",
            Severity::S3 => "S3",
            Severity::S4 => "S4",
        }
    }
}

/// One difference between two ASTs: the field that changed, and the variant
/// it changed inside.
///
/// The variant matters. `kind` is a bracket style inside `Group` and the
/// difference between a sine and a cosine inside `Function`; a classifier
/// that saw only the field name had to price both the same.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Difference {
    pub variant: String,
    pub field: String,
}

impl Difference {
    fn new(variant: &str, field: &str) -> Self {
        Difference {
            variant: variant.to_string(),
            field: field.to_string(),
        }
    }
}

/// The variant name to use for the children of an object, so that a field is
/// priced by where it appears. An externally tagged enum is a single-key
/// object whose key is the variant; anything else keeps the context it had.
fn inner_variant<'a>(object: &'a serde_json::Map<String, Value>, current: &'a str) -> &'a str {
    match object.len() {
        1 => object.keys().next().map(String::as_str).unwrap_or(current),
        _ => current,
    }
}

/// Classifies one example's outcome. `None` means there was no error at all.
///
/// The rules are structural and deterministic: no annotator judgement is
/// involved, so the same corpus always yields the same severity histogram.
pub fn classify_severity(
    gold: &Target,
    produced: &Target,
    render_equal: Option<bool>,
) -> Option<Severity> {
    match (gold, produced) {
        (Target::Raw, Target::Raw) => None,
        // The one class that matters most: ordinary speech turned into a
        // formula nobody dictated.
        (Target::Raw, Target::Ast(_)) => Some(Severity::S4),
        // Keeping the words is always safe, even when a formula was wanted.
        (Target::Ast(_), Target::Raw) => Some(Severity::S1),
        (Target::Ast(gold), Target::Ast(produced)) => {
            let gold_value = serde_json::to_value(gold).ok()?;
            let produced_value = serde_json::to_value(produced).ok()?;
            if gold_value == produced_value {
                return match render_equal {
                    Some(false) => Some(Severity::S0),
                    _ => None,
                };
            }
            let mut fields = Vec::new();
            collect_differences(&gold_value, &produced_value, "", &mut fields, 0);
            // The class of a difference now comes from
            // `research/schema/ast-distance-v1.json`, the same file that
            // carries the distance weights, instead of two arrays of field
            // names in this file. The rule itself is unchanged: the worst
            // class among the differences wins.
            let weights = crate::distance::Weights::builtin();
            let worst = fields
                .iter()
                .map(|difference| weights.severity_of(&difference.variant, &difference.field))
                .max_by_key(|class| match *class {
                    "S4" => 4,
                    "S3" => 3,
                    "S2" => 2,
                    "S0" => 0,
                    _ => 1,
                });
            Some(match worst {
                Some("S3") => Severity::S3,
                Some("S2") => Severity::S2,
                _ => Severity::S1,
            })
        }
    }
}

/// Walks two ASTs in parallel and records the names of the fields at which
/// they diverge. Recursion stops as soon as the two sides stop agreeing on
/// their shape, so the reported names are the deepest ones that still
/// describe the same slot in both trees.
fn collect_differences(
    gold: &Value,
    produced: &Value,
    variant: &str,
    fields: &mut Vec<Difference>,
    depth: usize,
) {
    if depth > crate::canonical::MAX_CANONICAL_DEPTH {
        return;
    }
    if gold == produced {
        return;
    }
    match (gold, produced) {
        (Value::Object(a), Value::Object(b)) => {
            let mut shared = false;
            for (key, gold_child) in a {
                match b.get(key) {
                    Some(produced_child) => {
                        shared = true;
                        if gold_child != produced_child {
                            if is_leaf_pair(gold_child, produced_child) {
                                fields.push(Difference::new(variant, key));
                            } else {
                                let before = fields.len();
                                collect_differences(
                                    gold_child,
                                    produced_child,
                                    inner_variant(a, variant),
                                    fields,
                                    depth + 1,
                                );
                                if fields.len() == before {
                                    fields.push(Difference::new(variant, key));
                                }
                            }
                        }
                    }
                    None => fields.push(Difference::new(variant, key)),
                }
            }
            for key in b.keys() {
                if !a.contains_key(key) {
                    fields.push(Difference::new(variant, key));
                }
            }
            if !shared {
                // Different variant tags entirely: record both names.
                fields.extend(a.keys().map(|key| Difference::new(variant, key)));
                fields.extend(b.keys().map(|key| Difference::new(variant, key)));
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return;
            }
            for (gold_child, produced_child) in a.iter().zip(b.iter()) {
                collect_differences(gold_child, produced_child, variant, fields, depth + 1);
            }
        }
        _ => {}
    }
}

fn is_leaf_pair(a: &Value, b: &Value) -> bool {
    !matches!(a, Value::Object(_) | Value::Array(_))
        || !matches!(b, Value::Object(_) | Value::Array(_))
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SeverityReport {
    pub errors: usize,
    pub count_by_severity: BTreeMap<String, usize>,
    pub probability_given_error: BTreeMap<String, f64>,
    pub max_severity: Option<String>,
}

pub fn severity_report(severities: &[Option<Severity>]) -> SeverityReport {
    let observed: Vec<Severity> = severities.iter().flatten().copied().collect();
    let mut count_by_severity = BTreeMap::new();
    for class in [
        Severity::S0,
        Severity::S1,
        Severity::S2,
        Severity::S3,
        Severity::S4,
    ] {
        count_by_severity.insert(class.as_str().to_string(), 0usize);
    }
    for class in &observed {
        *count_by_severity
            .get_mut(class.as_str())
            .expect("every class is pre-seeded") += 1;
    }
    let errors = observed.len();
    let probability_given_error = count_by_severity
        .iter()
        .map(|(class, count)| {
            let share = if errors == 0 {
                0.0
            } else {
                *count as f64 / errors as f64
            };
            (class.clone(), share)
        })
        .collect();
    SeverityReport {
        errors,
        count_by_severity,
        probability_given_error,
        max_severity: observed
            .iter()
            .max()
            .map(|class| class.as_str().to_string()),
    }
}

// ------------------------------------------------------------------- rng

/// Small deterministic PRNG so that bootstrap intervals reproduce exactly
/// without pulling in a dependency.
struct Pcg32 {
    state: u64,
    increment: u64,
}

impl Pcg32 {
    fn new(seed: u64) -> Self {
        let mut rng = Pcg32 {
            state: 0,
            increment: (seed << 1) | 1,
        };
        rng.next_u32();
        rng.state = rng.state.wrapping_add(seed);
        rng.next_u32();
        rng
    }

    fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(self.increment);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciwhisper_core::ast::{Case, Chemical, Formula, Math, Part, Species, Symbol};
    use sciwhisper_core::Node;

    #[test]
    fn a_proportion_matches_a_hand_computed_wilson_interval() {
        // 7 of 10, z = 1.959964.
        //   centre = (0.7 + z²/20) / (1 + z²/10) = 0.892074 / 1.384146 = 0.644495
        //   spread = z·sqrt(0.7·0.3/10 + z²/400) / 1.384146 = 0.247714
        let p = proportion(7, 10);
        assert_eq!(p.value, Some(0.7));
        let low = p.ci95_low.unwrap();
        let high = p.ci95_high.unwrap();
        assert!((low - 0.396_781).abs() < 1e-5, "low was {low}");
        assert!((high - 0.892_209).abs() < 1e-5, "high was {high}");
        assert_eq!(p.zero_count_upper95, None);
    }

    #[test]
    fn an_empty_denominator_is_not_zero_percent() {
        let p = proportion(0, 0);
        assert_eq!(p.value, None);
        assert_eq!(p.ci_method, CiMethod::Undefined);
    }

    #[test]
    fn an_observed_zero_reports_an_upper_bound() {
        // 0 of 30 → exact one-sided bound 1 − 0.05^(1/30) ≈ 0.0951.
        let p = proportion(0, 30);
        assert_eq!(p.value, Some(0.0));
        let bound = p.zero_count_upper95.expect("a zero count needs a bound");
        assert!((bound - 0.095_1).abs() < 1e-3, "bound was {bound}");
        assert!(bound > 0.0, "an observed zero is not a proven zero");
    }

    #[test]
    fn recall_at_k_counts_ranks_not_presence() {
        // gold at ranks 1, 3, absent, 8, 2 out of five examples.
        let ranks = [Some(1), Some(3), None, Some(8), Some(2)];
        assert_eq!(recall_at_k(&ranks, 1).numerator, 1);
        assert_eq!(recall_at_k(&ranks, 2).numerator, 2);
        assert_eq!(recall_at_k(&ranks, 4).numerator, 3);
        assert_eq!(recall_at_k(&ranks, 8).numerator, 4);
        assert_eq!(recall_at_k(&ranks, 16).numerator, 4);
        assert_eq!(recall_at_k(&ranks, 4).value, Some(0.6));
    }

    #[test]
    fn percentiles_use_nearest_rank_on_a_hand_table() {
        let mut values = vec![10u64, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        assert_eq!(percentile_u64(&mut values, 50.0), Some(50));
        assert_eq!(percentile_u64(&mut values, 95.0), Some(100));
        let mut single = vec![7u64];
        assert_eq!(percentile_u64(&mut single, 50.0), Some(7));
        assert_eq!(percentile_u64(&mut Vec::new(), 50.0), None);
    }

    #[test]
    fn the_bootstrap_is_reproducible_and_brackets_the_point_estimate() {
        let indicators: Vec<bool> = (0..40).map(|index| index % 4 != 0).collect();
        let first = bootstrap_proportion(&indicators, 20260904, 500);
        let second = bootstrap_proportion(&indicators, 20260904, 500);
        assert_eq!(first, second);
        assert_eq!(first.value, Some(0.75));
        assert!(first.ci95_low.unwrap() <= 0.75 && first.ci95_high.unwrap() >= 0.75);
        assert_eq!(first.ci_method, CiMethod::Bootstrap);
    }

    fn flatten(clusters: &[Vec<bool>]) -> Vec<bool> {
        clusters.iter().flatten().copied().collect()
    }

    /// Ten speakers with four recordings each: five always right, five always
    /// wrong. Within a speaker the recordings are perfectly correlated.
    fn perfectly_correlated_speakers() -> Vec<Vec<bool>> {
        (0..10).map(|speaker| vec![speaker % 2 == 0; 4]).collect()
    }

    #[test]
    fn the_cluster_bootstrap_is_reproducible() {
        let clusters = perfectly_correlated_speakers();
        let first = cluster_bootstrap_proportion(&clusters, 20261007, 500);
        let second = cluster_bootstrap_proportion(&clusters, 20261007, 500);
        assert_eq!(first, second);
        assert_eq!(first.ci_method, CiMethod::ClusterBootstrap);
        // 20 hits of 40 examples.
        assert_eq!((first.numerator, first.denominator), (20, 40));
        assert_eq!(first.value, Some(0.5));
        assert_eq!(first.zero_count_upper95, None);
        // The all-or-nothing speakers above give a coarse interval that two
        // seeds can share; speakers of mixed accuracy and size do not.
        let mixed: Vec<Vec<bool>> = (1..=9)
            .map(|speaker| (0..speaker).map(|index| index % 3 != 0).collect())
            .collect();
        let intervals: Vec<(Option<f64>, Option<f64>)> = (0..8u64)
            .map(|seed| {
                let p = cluster_bootstrap_proportion(&mixed, seed, 300);
                (p.ci95_low, p.ci95_high)
            })
            .collect();
        assert!(
            intervals.iter().any(|interval| *interval != intervals[0]),
            "the seed must matter: {intervals:?}"
        );
    }

    #[test]
    fn with_every_cluster_of_size_one_it_draws_like_the_example_bootstrap() {
        // Same PRNG, same `next_u32() % n`, same sort and percentile: the
        // interval must be bit-identical to the example bootstrap.
        let indicators: Vec<bool> = (0..40).map(|index| index % 4 != 0).collect();
        let clusters: Vec<Vec<bool>> = indicators.iter().map(|hit| vec![*hit]).collect();
        let by_example = bootstrap_proportion(&indicators, 20260904, 500);
        let by_cluster = cluster_bootstrap_proportion(&clusters, 20260904, 500);
        assert_eq!(by_cluster.ci95_low, by_example.ci95_low);
        assert_eq!(by_cluster.ci95_high, by_example.ci95_high);
        assert_eq!(by_cluster.value, by_example.value);
        assert_eq!(by_cluster.numerator, by_example.numerator);
        assert_eq!(by_cluster.denominator, by_example.denominator);
        assert_eq!(by_cluster.ci_method, CiMethod::ClusterBootstrap);
    }

    #[test]
    fn two_opposite_speakers_give_hand_computable_bounds() {
        // Speaker A: 3 of 3 right. Speaker B: 1 of 1 wrong. Each resample
        // draws two speakers, so the pooled proportion is one of
        //   A,A: 6/6 = 1      (probability 1/4)
        //   B,B: 0/2 = 0      (probability 1/4)
        //   A,B or B,A: 3/4   (probability 1/2)
        // P(0) = 1/4 > 2.5% and P(1) = 1/4 > 2.5%, so the nearest-rank 2.5th
        // percentile is 0 and the 97.5th is 1. Point estimate: 3/4.
        let clusters = vec![vec![true; 3], vec![false]];
        let p = cluster_bootstrap_proportion(&clusters, 7, 2000);
        assert_eq!((p.numerator, p.denominator), (3, 4));
        assert_eq!(p.value, Some(0.75));
        assert_eq!(p.ci95_low, Some(0.0));
        assert_eq!(p.ci95_high, Some(1.0));
    }

    #[test]
    fn a_single_speaker_collapses_the_interval_to_the_point() {
        // Every resample is that one speaker: 3/4 each time.
        let clusters = vec![vec![true, true, true, false]];
        let p = cluster_bootstrap_proportion(&clusters, 7, 100);
        assert_eq!(p.value, Some(0.75));
        assert_eq!(p.ci95_low, Some(0.75));
        assert_eq!(p.ci95_high, Some(0.75));
    }

    #[test]
    fn correlated_speakers_widen_the_interval_beyond_the_example_bootstrap() {
        // The test an implementation resampling examples would pass wrongly:
        // every speaker is all-right or all-wrong, so 40 recordings carry
        // the information of about 10. By hand, the example bootstrap has
        // sd = sqrt(0.25 / 40) = 0.079, interval about 0.5 ± 0.155; the
        // cluster bootstrap has sd = sqrt(0.25 / 10) = 0.158, about
        // 0.5 ± 0.31. Wider by a factor near two.
        let clusters = perfectly_correlated_speakers();
        let by_example = bootstrap_proportion(&flatten(&clusters), 20261007, 2000);
        let by_cluster = cluster_bootstrap_proportion(&clusters, 20261007, 2000);
        assert_eq!(by_cluster.value, by_example.value);
        let example_width = by_example.ci95_high.unwrap() - by_example.ci95_low.unwrap();
        let cluster_width = by_cluster.ci95_high.unwrap() - by_cluster.ci95_low.unwrap();
        assert!(
            cluster_width > example_width,
            "cluster width {cluster_width} must exceed example width {example_width}"
        );
        assert!(
            cluster_width > 1.5 * example_width,
            "cluster width {cluster_width} should be near twice {example_width}"
        );
    }

    #[test]
    fn the_cluster_bootstrap_without_data_is_undefined_not_zero() {
        for clusters in [Vec::new(), vec![Vec::new()], vec![Vec::new(); 5]] {
            let p = cluster_bootstrap_proportion(&clusters, 1, 100);
            assert_eq!((p.numerator, p.denominator), (0, 0));
            assert_eq!(p.value, None);
            assert_eq!(p.ci95_low, None);
            assert_eq!(p.ci95_high, None);
            assert_eq!(p.ci_method, CiMethod::Undefined);
        }
    }

    #[test]
    fn empty_speakers_are_excluded_and_do_not_change_the_result() {
        // A zero-example speaker has no pooled contribution; drawing it
        // would give 0/0. It is dropped up front, so inserting it anywhere
        // leaves the result bit-identical (same number of draws, same
        // indices).
        let clusters = perfectly_correlated_speakers();
        let mut padded = clusters.clone();
        padded.insert(0, Vec::new());
        padded.insert(4, Vec::new());
        padded.push(Vec::new());
        let plain = cluster_bootstrap_proportion(&clusters, 20261007, 300);
        let with_empty = cluster_bootstrap_proportion(&padded, 20261007, 300);
        assert_eq!(plain, with_empty);
        assert!(with_empty.ci95_low.unwrap().is_finite());
        assert!(with_empty.ci95_high.unwrap().is_finite());
    }

    #[test]
    fn zero_cluster_resamples_fall_back_to_wilson_like_the_example_bootstrap() {
        let clusters = perfectly_correlated_speakers();
        let p = cluster_bootstrap_proportion(&clusters, 1, 0);
        assert_eq!(p, proportion(20, 40));
        assert_eq!(p.ci_method, CiMethod::Wilson);
        // Same fallback as `bootstrap_proportion` with zero resamples.
        assert_eq!(p, bootstrap_proportion(&flatten(&clusters), 1, 0));
    }

    #[test]
    fn the_cluster_bootstrap_method_serialises_as_cluster_bootstrap() {
        let name = |method: CiMethod| serde_json::to_string(&method).unwrap();
        assert_eq!(name(CiMethod::ClusterBootstrap), "\"cluster_bootstrap\"");
        // The existing names are unchanged.
        assert_eq!(name(CiMethod::Bootstrap), "\"bootstrap\"");
        assert_eq!(name(CiMethod::Wilson), "\"wilson\"");
        assert_eq!(name(CiMethod::Undefined), "\"undefined\"");
    }

    fn species(count: u32, coefficient: u32, charge: Option<i32>) -> Node {
        let mut s = Species::new(Formula {
            parts: vec![Part::Atom {
                symbol: "H".into(),
                count,
            }],
        });
        s.coefficient = coefficient;
        s.charge = charge;
        Node::Chemical(Chemical::Species(s))
    }

    #[test]
    fn severity_separates_a_coefficient_change_from_an_index_change() {
        let gold = species(2, 1, None);
        assert_eq!(
            classify_severity(
                &Target::Ast(gold.clone()),
                &Target::Ast(species(3, 1, None)),
                None
            ),
            Some(Severity::S2),
            "an index change is S2"
        );
        assert_eq!(
            classify_severity(
                &Target::Ast(gold.clone()),
                &Target::Ast(species(2, 2, None)),
                None
            ),
            Some(Severity::S3),
            "a coefficient change is S3"
        );
        assert_eq!(
            classify_severity(
                &Target::Ast(gold),
                &Target::Ast(species(2, 1, Some(1))),
                None
            ),
            Some(Severity::S3),
            "a charge change is S3"
        );
    }

    #[test]
    fn severity_flags_an_invented_formula_as_the_worst_class() {
        let invented = Target::Ast(Node::Math(Math::Number("1".into())));
        assert_eq!(
            classify_severity(&Target::Raw, &invented, None),
            Some(Severity::S4)
        );
        assert_eq!(classify_severity(&Target::Raw, &Target::Raw, None), None);
    }

    #[test]
    fn keeping_the_words_is_the_safe_class_and_presentation_is_the_lightest() {
        let gold = Target::Ast(Node::Math(Math::Symbol(Symbol::latin('x', Case::Lower))));
        assert_eq!(
            classify_severity(&gold, &Target::Raw, None),
            Some(Severity::S1)
        );
        assert_eq!(classify_severity(&gold, &gold, Some(true)), None);
        assert_eq!(
            classify_severity(&gold, &gold, Some(false)),
            Some(Severity::S0),
            "same AST, different rendering, is presentation only"
        );
    }

    #[test]
    fn severity_report_matches_a_hand_counted_table() {
        let severities = [
            Some(Severity::S1),
            Some(Severity::S3),
            None,
            Some(Severity::S1),
            Some(Severity::S4),
            None,
        ];
        let report = severity_report(&severities);
        assert_eq!(report.errors, 4);
        assert_eq!(report.count_by_severity["S1"], 2);
        assert_eq!(report.count_by_severity["S3"], 1);
        assert_eq!(report.count_by_severity["S4"], 1);
        assert_eq!(report.count_by_severity["S0"], 0);
        assert_eq!(report.probability_given_error["S1"], 0.5);
        assert_eq!(report.max_severity.as_deref(), Some("S4"));
        let empty = severity_report(&[None, None]);
        assert_eq!(empty.errors, 0);
        assert_eq!(empty.max_severity, None);
        assert_eq!(empty.probability_given_error["S4"], 0.0);
    }

    /// The published taxonomy, the distance weights and the classifier must
    /// all agree about which change is which class. They used to be three
    /// places: a document, two arrays in this file, and the classifier. Now
    /// `ast-distance-v1.json` is the one that decides, and this checks the
    /// document still matches it.
    #[test]
    fn the_published_taxonomy_matches_the_file_the_classifier_reads() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../research/schema/severity-v1.json"
        );
        let text = std::fs::read_to_string(path).expect("severity-v1.json must exist");
        let published: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        let weights = crate::distance::Weights::builtin();
        for class in ["S2", "S3"] {
            let listed = published["ast_field_classes"][class]
                .as_array()
                .unwrap_or_else(|| panic!("missing ast_field_classes.{class}"));
            for field in listed {
                let field = field.as_str().expect("string");
                assert_eq!(
                    weights.severity_of("", field),
                    class,
                    "{field} is published as {class} but the weight file disagrees"
                );
            }
        }
    }

    /// The migration off the two Rust arrays must not have moved any class
    /// by accident. Every field those arrays named keeps the class it had —
    /// with one deliberate exception, listed here so it cannot be silent.
    #[test]
    fn moving_the_classes_into_data_changed_exactly_one_of_them() {
        const PREVIOUS_S3: [&str; 9] = [
            "coefficient",
            "charge",
            "arrow",
            "op",
            "order",
            "direction",
            "condition",
            "power",
            "divide",
        ];
        const PREVIOUS_S2: [&str; 15] = [
            "symbol",
            "count",
            "letter",
            "case",
            "alphabet",
            "kind",
            "exp",
            "sub",
            "index",
            "Number",
            "factors",
            "marker",
            "variables",
            "left",
            "right",
        ];
        let weights = crate::distance::Weights::builtin();
        for field in PREVIOUS_S3 {
            assert_eq!(weights.severity_of("", field), "S3", "{field}");
        }
        for field in PREVIOUS_S2 {
            assert_eq!(weights.severity_of("", field), "S2", "{field}");
        }

        // The exception: a round bracket becoming a square one is
        // presentation, and used to be classed with a changed element
        // because both were called `kind`. Naming the variant is what makes
        // the two separable at all.
        assert_eq!(weights.severity_of("Group", "kind"), "S1");
        assert_eq!(weights.severity_of("Function", "kind"), "S2");
    }
    #[test]
    fn mcnemar_exact_matches_hand_computed_binomial_tails() {
        // n = 5, tail k = 0: 2 · 1/32.
        assert!((mcnemar_exact(0, 5) - 0.0625).abs() < 1e-15);
        // n = 6, tail k ≤ 1: 2 · (1 + 6)/64 = 7/32.
        assert!((mcnemar_exact(1, 5) - 0.21875).abs() < 1e-15);
        // n = 10, tail k ≤ 2: 2 · (1 + 10 + 45)/1024 = 7/64.
        assert!((mcnemar_exact(2, 8) - 0.109375).abs() < 1e-15);
        // n = 9, tail k ≤ 4 is exactly half of 2^9: 2 · 256/512 = 1.
        assert!((mcnemar_exact(4, 5) - 1.0).abs() < 1e-15);
    }

    #[test]
    fn mcnemar_exact_is_one_without_a_difference() {
        assert_eq!(mcnemar_exact(0, 0), 1.0);
        for b in [1, 2, 7, 40, 1000] {
            assert_eq!(mcnemar_exact(b, b), 1.0, "b = c = {b}");
        }
    }

    #[test]
    fn mcnemar_exact_does_not_depend_on_which_system_is_first() {
        for (b, c) in [(0, 5), (1, 5), (3, 11), (10, 30), (250, 300)] {
            assert_eq!(mcnemar_exact(b, c), mcnemar_exact(c, b), "b = {b}, c = {c}");
        }
    }

    #[test]
    fn mcnemar_exact_stays_finite_where_a_direct_sum_overflows() {
        // References from exact rational arithmetic (Python `fractions`,
        // sum of `math.comb`) for the first two, and from `math.lgamma`
        // terms for the third, which is too large to sum exactly quickly.
        let cases = [
            (10, 30, 0.002_221_433_773_229_364_3),
            // C(2000, 500) ≈ 10^486 is past f64::MAX.
            (500, 1500, 1.474_397_522_976_895e-115),
            (1_000_000, 1_010_000, 1.753_968_557_555_584_7e-12),
        ];
        for (b, c, expected) in cases {
            let p = mcnemar_exact(b, c);
            assert!(p.is_finite(), "b = {b}, c = {c}: {p}");
            let relative = (p - expected).abs() / expected;
            assert!(relative < 1e-6, "b = {b}, c = {c}: {p} vs {expected}");
        }
        // 2 · 2^-2000 is below the smallest f64: zero, not NaN.
        assert_eq!(mcnemar_exact(0, 2000), 0.0);
        assert_eq!(mcnemar_exact(1_000_000, 1_000_000), 1.0);
    }
}

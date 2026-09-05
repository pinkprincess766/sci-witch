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

/// Field names whose change alters a quantity, a direction or an operator.
pub const S3_FIELDS: [&str; 9] = [
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

/// Field names whose change alters a symbol, an index, an exponent, a unit or
/// the grouping of an expression.
pub const S2_FIELDS: [&str; 15] = [
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
            collect_differences(&gold_value, &produced_value, &mut fields, 0);
            if fields
                .iter()
                .any(|field| S3_FIELDS.contains(&field.as_str()))
            {
                Some(Severity::S3)
            } else if fields
                .iter()
                .any(|field| S2_FIELDS.contains(&field.as_str()))
            {
                Some(Severity::S2)
            } else {
                Some(Severity::S1)
            }
        }
    }
}

/// Walks two ASTs in parallel and records the names of the fields at which
/// they diverge. Recursion stops as soon as the two sides stop agreeing on
/// their shape, so the reported names are the deepest ones that still
/// describe the same slot in both trees.
fn collect_differences(gold: &Value, produced: &Value, fields: &mut Vec<String>, depth: usize) {
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
                                fields.push(key.clone());
                            } else {
                                let before = fields.len();
                                collect_differences(gold_child, produced_child, fields, depth + 1);
                                if fields.len() == before {
                                    fields.push(key.clone());
                                }
                            }
                        }
                    }
                    None => fields.push(key.clone()),
                }
            }
            for key in b.keys() {
                if !a.contains_key(key) {
                    fields.push(key.clone());
                }
            }
            if !shared {
                // Different variant tags entirely: record both names.
                fields.extend(a.keys().cloned());
                fields.extend(b.keys().cloned());
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return;
            }
            for (gold_child, produced_child) in a.iter().zip(b.iter()) {
                collect_differences(gold_child, produced_child, fields, depth + 1);
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

    #[test]
    fn the_severity_field_lists_match_the_published_taxonomy() {
        // The taxonomy file is the machine-readable contract; the code must
        // not drift away from it.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../research/schema/severity-v1.json"
        );
        let text = std::fs::read_to_string(path).expect("severity-v1.json must exist");
        let published: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        let list = |key: &str| -> Vec<String> {
            published["ast_field_classes"][key]
                .as_array()
                .unwrap_or_else(|| panic!("missing ast_field_classes.{key}"))
                .iter()
                .map(|value| value.as_str().expect("string").to_string())
                .collect()
        };
        let mut published_s3 = list("S3");
        let mut published_s2 = list("S2");
        published_s3.sort();
        published_s2.sort();
        let mut code_s3: Vec<String> = S3_FIELDS.iter().map(|f| f.to_string()).collect();
        let mut code_s2: Vec<String> = S2_FIELDS.iter().map(|f| f.to_string()).collect();
        code_s3.sort();
        code_s2.sort();
        assert_eq!(published_s3, code_s3);
        assert_eq!(published_s2, code_s2);
    }
}

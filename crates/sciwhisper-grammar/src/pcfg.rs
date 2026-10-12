//! Probabilistic CFG over the productions of `docs/grammar/math.ebnf`.
//!
//! The probability of a production is its relative frequency among the uses
//! of its left-hand side in training derivations. A sentence with several
//! derivations splits its count evenly between them; this is a shortcut, not
//! EM. Add-one smoothing keeps a production unseen in train above zero.
//!
//! [`em_estimate`] is the real EM: it weights the trees of a sentence by their
//! probability under the current model instead of splitting evenly. The
//! derivations are already enumerated, so it runs over the tree lists and
//! needs no inside-outside chart.

use std::collections::HashMap;

use crate::earley::Production;
use crate::likelihood::{sentence_log_prob, tree_log_prob};

/// Pseudo-count added to every production of a left-hand side before
/// normalising (add-one / Laplace smoothing). Without it a production
/// unseen in train has probability 0 and any validation sentence that
/// needs it gets ln P = -inf.
pub const PSEUDOCOUNT: f64 = 1.0;

/// Expected number of uses of each production over a training set.
/// A sentence with N derivations adds 1/N for every use of a production
/// in each of its derivations (ambiguity is split evenly between trees —
/// a simplification, not EM). Err if a sentence has no derivations or an
/// id is >= n_productions.
pub fn expected_counts(
    n_productions: usize,
    sentences: &[Vec<Vec<usize>>],
) -> Result<Vec<f64>, String> {
    let mut counts = vec![0.0; n_productions];
    for (index, trees) in sentences.iter().enumerate() {
        if trees.is_empty() {
            return Err(format!("sentence {index} has no derivations"));
        }
        let share = 1.0 / trees.len() as f64;
        for tree in trees {
            for &prod in tree {
                if prod >= n_productions {
                    return Err(format!(
                        "sentence {index}: production id {prod} is not below {n_productions}"
                    ));
                }
                counts[prod] += share;
            }
        }
    }
    Ok(counts)
}

/// Probability of each production: `(count + pseudocount)` divided by the
/// same sum over the productions with the same left-hand side. If that sum
/// is 0 (pseudocount 0 and no uses), the alternatives get the uniform
/// distribution 1/k. `estimate(prods, &vec![0.0; n], 1.0)` is the uniform
/// PCFG baseline: every alternative of a left-hand side is equally likely.
///
/// Panics if `counts.len() != productions.len()`, if `pseudocount` is
/// negative or NaN, or if a count is negative or NaN.
pub fn estimate(productions: &[Production], counts: &[f64], pseudocount: f64) -> Vec<f64> {
    assert_eq!(
        counts.len(),
        productions.len(),
        "one count per production, got {} counts for {} productions",
        counts.len(),
        productions.len()
    );
    assert!(
        pseudocount >= 0.0,
        "pseudocount must not be negative or NaN, got {pseudocount}"
    );
    for (id, &count) in counts.iter().enumerate() {
        assert!(count >= 0.0, "count {id} is negative or NaN: {count}");
    }
    let mut total: HashMap<&str, f64> = HashMap::new();
    let mut arity: HashMap<&str, usize> = HashMap::new();
    for (prod, &count) in productions.iter().zip(counts) {
        *total.entry(prod.lhs.as_str()).or_insert(0.0) += count + pseudocount;
        *arity.entry(prod.lhs.as_str()).or_insert(0) += 1;
    }
    productions
        .iter()
        .zip(counts)
        .map(|(prod, &count)| {
            let lhs = prod.lhs.as_str();
            if total[lhs] == 0.0 {
                1.0 / arity[lhs] as f64
            } else {
                (count + pseudocount) / total[lhs]
            }
        })
        .collect()
}

/// Iterations `em_estimate` runs. Each is one E-step and one M-step; a fixed
/// count keeps the report reproducible. In the convergence test the gap to
/// the known maximum is 1.3e-8 nats after 50 iterations (hand check in
/// Python); 100 doubles that with a margin.
pub const EM_ITERATIONS: usize = 100;

/// Result of [`em_estimate`].
pub struct EmFit {
    pub prob: Vec<f64>,
    /// Objective after each iteration, starting with the initial model
    /// (len = iterations + 1): Σ_s ln P(s) + pseudocount · Σ_p ln prob[p].
    pub objective: Vec<f64>,
}

/// Expected production counts when each tree of a sentence has weight
/// P(tree) / Σ P(trees of that sentence) under `prob`. Weights are computed in
/// log space: the largest tree log-probability of the sentence is subtracted
/// before `exp`, so long derivations do not underflow. Err if all trees of a
/// sentence have probability 0. Ids must be below `prob.len()`.
fn weighted_counts(prob: &[f64], sentences: &[Vec<Vec<usize>>]) -> Result<Vec<f64>, String> {
    let mut counts = vec![0.0; prob.len()];
    for (index, trees) in sentences.iter().enumerate() {
        let log_probs: Vec<f64> = trees.iter().map(|tree| tree_log_prob(prob, tree)).collect();
        let max_log_prob = log_probs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if max_log_prob == f64::NEG_INFINITY {
            return Err(format!(
                "sentence {index}: every derivation has probability 0"
            ));
        }
        let shifted: Vec<f64> = log_probs
            .iter()
            .map(|&log_prob| (log_prob - max_log_prob).exp())
            .collect();
        let total_shifted: f64 = shifted.iter().sum();
        for (tree, weight) in trees.iter().zip(&shifted) {
            for &prod in tree {
                counts[prod] += weight / total_shifted;
            }
        }
    }
    Ok(counts)
}

/// Σ_s ln P(s) + pseudocount · Σ_p ln prob[p]. The second term is the
/// Dirichlet(1 + pseudocount) log-prior; it is skipped for pseudocount 0
/// because 0 · ln 0 is NaN in floating point.
fn objective(prob: &[f64], sentences: &[Vec<Vec<usize>>], pseudocount: f64) -> f64 {
    let log_likelihood: f64 = sentences
        .iter()
        .map(|trees| sentence_log_prob(prob, trees))
        .sum();
    if pseudocount == 0.0 {
        return log_likelihood;
    }
    let log_prior: f64 = prob.iter().map(|p| p.ln()).sum();
    log_likelihood + pseudocount * log_prior
}

/// EM over enumerated derivations. Start: `estimate(expected_counts(..),
/// pseudocount)` (the even split). E-step: each tree of a sentence gets weight
/// P(tree) / Σ P(trees of that sentence). M-step: `estimate(weighted counts,
/// pseudocount)`. With pseudocount > 0 this is MAP-EM under a
/// Dirichlet(1 + pseudocount) prior, and the objective includes the prior
/// term; it never decreases. Errors from `expected_counts` propagate; a
/// sentence whose trees all have probability 0 is an Err (cannot happen with
/// pseudocount > 0).
pub fn em_estimate(
    productions: &[Production],
    sentences: &[Vec<Vec<usize>>],
    pseudocount: f64,
    iterations: usize,
) -> Result<EmFit, String> {
    let start_counts = expected_counts(productions.len(), sentences)?;
    let mut prob = estimate(productions, &start_counts, pseudocount);
    let mut objective_after = vec![objective(&prob, sentences, pseudocount)];
    for _ in 0..iterations {
        let counts = weighted_counts(&prob, sentences)?;
        prob = estimate(productions, &counts, pseudocount);
        objective_after.push(objective(&prob, sentences, pseudocount));
    }
    Ok(EmFit {
        prob,
        objective: objective_after,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use crate::earley::{derivations, productions, DerivationOutcome, Production, Symbol};
    use crate::ebnf::{parse, Grammar};

    use super::{
        em_estimate, estimate, expected_counts, weighted_counts, EM_ITERATIONS, PSEUDOCOUNT,
    };

    const TOL: f64 = 1e-12;

    fn g(src: &str) -> Grammar {
        parse(src).unwrap_or_else(|e| panic!("{e} in {src}"))
    }

    fn prods(src: &str) -> Vec<Production> {
        productions(&g(src))
    }

    /// Every derivation tree of `tokens` from start symbol `s`.
    fn trees(src: &str, tokens: &[&str]) -> Vec<Vec<usize>> {
        match derivations(&g(src), "s", tokens) {
            DerivationOutcome::Derivations(trees) => trees,
            other => panic!("{tokens:?}: {other:?}"),
        }
    }

    fn t(name: &str) -> Symbol {
        Symbol::Terminal(name.to_string())
    }

    fn n(name: &str) -> Symbol {
        Symbol::Nonterminal(name.to_string())
    }

    fn prod(lhs: &str, rhs: &[Symbol]) -> Production {
        Production {
            lhs: lhs.to_string(),
            rhs: rhs.to_vec(),
        }
    }

    fn assert_close(actual: &[f64], expected: &[f64]) {
        assert_eq!(actual.len(), expected.len(), "{actual:?}");
        for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert!((a - e).abs() <= TOL, "entry {i}: {a} != {e} in {actual:?}");
        }
    }

    #[test]
    fn counts_and_probabilities_of_a_right_recursive_rule() {
        // Ids: 0 is s → "a" s, 1 is s → "b". "b" has the tree [1]; "a b"
        // has the tree [0, 1]. Id 0 is used once, id 1 twice.
        let src = r#"s = "a" s | "b" ;"#;
        let ps = prods(src);
        assert_eq!(ps, vec![prod("s", &[t("a"), n("s")]), prod("s", &[t("b")])]);
        let sentences = vec![trees(src, &["b"]), trees(src, &["a", "b"])];
        assert_eq!(sentences, vec![vec![vec![1]], vec![vec![0, 1]]]);
        let counts = expected_counts(ps.len(), &sentences).unwrap();
        assert_close(&counts, &[1.0, 2.0]);
        // Without smoothing: 1/3 and 2/3. With add-one: 2/5 and 3/5.
        assert_close(&estimate(&ps, &counts, 0.0), &[1.0 / 3.0, 2.0 / 3.0]);
        assert_close(
            &estimate(&ps, &counts, PSEUDOCOUNT),
            &[2.0 / 5.0, 3.0 / 5.0],
        );
    }

    #[test]
    fn an_ambiguous_sentence_splits_its_count_between_its_trees() {
        // Ids: 0 is s → x, 1 is s → y, 2 is x → "t", 3 is y → "t". "t" has
        // the trees [0, 2] and [1, 3]; each tree counts 1/2, so every
        // production in them gets 1/2.
        let src = r#"s = x | y ; x = "t" ; y = "t" ;"#;
        let ps = prods(src);
        assert_eq!(
            ps,
            vec![
                prod("s", &[n("x")]),
                prod("s", &[n("y")]),
                prod("x", &[t("t")]),
                prod("y", &[t("t")]),
            ]
        );
        let sentences = vec![trees(src, &["t"])];
        assert_eq!(sentences, vec![vec![vec![0, 2], vec![1, 3]]]);
        let counts = expected_counts(ps.len(), &sentences).unwrap();
        assert_close(&counts, &[0.5, 0.5, 0.5, 0.5]);
        // s → x: (1/2 + 1) / (1/2 + 1 + 1/2 + 1) = 1/2. x → t: (1/2 + 1) / (1/2 + 1) = 1.
        assert_close(&estimate(&ps, &counts, PSEUDOCOUNT), &[0.5, 0.5, 1.0, 1.0]);
    }

    #[test]
    fn a_left_hand_side_with_no_uses_gets_the_uniform_distribution() {
        // z is never used: its two alternatives get (0 + 1) / (0 + 1 + 0 + 1)
        // = 1/2 each with add-one, and 1/2 each from the uniform fallback
        // without smoothing.
        let src = r#"s = x | y ; x = "t" ; y = "t" ; z = "p" | "q" ;"#;
        let ps = prods(src);
        let counts = expected_counts(ps.len(), &[trees(src, &["t"])]).unwrap();
        assert_close(&counts, &[0.5, 0.5, 0.5, 0.5, 0.0, 0.0]);
        assert_close(
            &estimate(&ps, &counts, PSEUDOCOUNT),
            &[0.5, 0.5, 1.0, 1.0, 0.5, 0.5],
        );
        assert_close(
            &estimate(&ps, &counts, 0.0),
            &[0.5, 0.5, 1.0, 1.0, 0.5, 0.5],
        );
    }

    #[test]
    fn zero_counts_with_pseudocount_one_are_the_uniform_baseline() {
        let ps = prods(r#"s = "a" | "b" | "c" ;"#);
        assert_close(&estimate(&ps, &[0.0; 3], PSEUDOCOUNT), &[1.0 / 3.0; 3]);
    }

    #[test]
    fn probabilities_of_math_ebnf_sum_to_one_per_lhs() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/grammar/math.ebnf");
        let src =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let ps = productions(&parse(&src).unwrap_or_else(|e| panic!("{e}")));
        let counts: Vec<f64> = (0..ps.len()).map(|id| (id % 7) as f64).collect();
        let probs = estimate(&ps, &counts, PSEUDOCOUNT);
        let mut sums: HashMap<&str, f64> = HashMap::new();
        for (prod, &p) in ps.iter().zip(&probs) {
            assert!(p > 0.0 && p <= 1.0, "{} has probability {p}", prod.lhs);
            *sums.entry(prod.lhs.as_str()).or_insert(0.0) += p;
        }
        for (lhs, sum) in &sums {
            assert!((sum - 1.0).abs() <= TOL, "{lhs}: sum {sum}");
        }
    }

    #[test]
    fn a_sentence_without_derivations_is_an_error() {
        let err = expected_counts(2, &[Vec::new()]).unwrap_err();
        assert!(err.contains("no derivations"), "{err}");
    }

    #[test]
    fn a_production_id_out_of_range_is_an_error() {
        assert!(expected_counts(2, &[vec![vec![1]]]).is_ok());
        let err = expected_counts(2, &[vec![vec![2]]]).unwrap_err();
        assert!(err.contains("production id 2"), "{err}");
    }

    #[test]
    #[should_panic(expected = "one count per production")]
    fn counts_must_match_the_productions() {
        estimate(&prods(r#"s = "a" ;"#), &[1.0, 2.0], PSEUDOCOUNT);
    }

    #[test]
    #[should_panic(expected = "pseudocount must not be negative")]
    fn a_negative_pseudocount_panics() {
        estimate(&prods(r#"s = "a" ;"#), &[1.0], -1.0);
    }

    #[test]
    #[should_panic(expected = "count 0 is negative or NaN")]
    fn a_negative_count_panics() {
        estimate(&prods(r#"s = "a" ;"#), &[-1.0], PSEUDOCOUNT);
    }

    #[test]
    #[should_panic(expected = "count 0 is negative or NaN")]
    fn a_nan_count_panics() {
        estimate(&prods(r#"s = "a" ;"#), &[f64::NAN], PSEUDOCOUNT);
    }

    #[test]
    fn em_on_unambiguous_data_equals_the_closed_form_estimate() {
        // Ids: 0 is s → "a" s, 1 is s → "b". Counts are [1, 2] as above and
        // every sentence has one tree, so every E-step gives the same counts:
        // the fit is [2/5, 3/5] and the objective does not move.
        let src = r#"s = "a" s | "b" ;"#;
        let ps = prods(src);
        let sentences = vec![trees(src, &["b"]), trees(src, &["a", "b"])];
        let fit = em_estimate(&ps, &sentences, 1.0, 5).unwrap();
        let counts = expected_counts(ps.len(), &sentences).unwrap();
        assert_close(&fit.prob, &estimate(&ps, &counts, 1.0));
        assert_eq!(fit.objective.len(), 6);
        for value in &fit.objective {
            assert!(
                (value - fit.objective[0]).abs() <= TOL,
                "{:?}",
                fit.objective
            );
        }
    }

    #[test]
    fn em_objective_never_decreases() {
        // Ids: 0 is s → x, 1 is s → y, 2 is x → "t", 3 is x → "u", 4 is y → "t".
        // "t" is ambiguous ([0, 2] and [1, 4]); "u" has one tree ([0, 3]).
        // Slack 1e-12: the objective is a sum of a few terms of order 1 to 10,
        // so rounding is near 1e-15; a real decrease would be far larger.
        let src = r#"s = x | y ; x = "t" | "u" ; y = "t" ;"#;
        let ps = prods(src);
        let sentences = vec![trees(src, &["t"]), trees(src, &["t"]), trees(src, &["u"])];
        for pseudocount in [0.0, 1.0] {
            let fit = em_estimate(&ps, &sentences, pseudocount, 30).unwrap();
            assert_eq!(fit.objective.len(), 31);
            for pair in fit.objective.windows(2) {
                assert!(
                    pair[1] >= pair[0] - 1e-12,
                    "pseudocount {pseudocount}: {:?}",
                    fit.objective
                );
            }
        }
        // Here the even split is already a maximum, so the objective is flat.
        // The grammar of the maximum-likelihood test below moves: it gains
        // 0.25 nats (see there), so the check above is not vacuous.
        let src = r#"s = x y ; x = "a" | "a" "a" ; y = "a" | "a" "a" | "a" "a" "a" ;"#;
        let sentences: Vec<Vec<Vec<usize>>> =
            (2..=4).map(|len| trees(src, &vec!["a"; len])).collect();
        for pseudocount in [0.0, 1.0] {
            let fit = em_estimate(&prods(src), &sentences, pseudocount, 30).unwrap();
            for pair in fit.objective.windows(2) {
                assert!(
                    pair[1] >= pair[0] - 1e-12,
                    "pseudocount {pseudocount}: {:?}",
                    fit.objective
                );
            }
            // Gains measured: 0.25 nats without smoothing, 0.0068 with it.
            let gain = fit.objective[30] - fit.objective[0];
            let min_gain = if pseudocount == 0.0 { 0.2 } else { 0.005 };
            assert!(gain > min_gain, "pseudocount {pseudocount}: gain {gain}");
        }
    }

    #[test]
    fn em_reaches_the_known_maximum_likelihood() {
        // Ids: 0 is s → x y, 1 is x → a, 2 is x → a a, 3 is y → a,
        // 4 is y → a a, 5 is y → a a a. Sentences "a a" ([0, 1, 3]),
        // "a a a" ([0, 1, 4] and [0, 2, 3]) and "a a a a" ([0, 1, 5] and
        // [0, 2, 4]), one each. The three sentence probabilities sum to at
        // most 1, so ln L <= 3 ln(1/3) = -3.295836866004329; it is reached
        // by x → a = 1 and y → a = y → a a = y → a a a = 1/3, so this is the
        // maximum. (Parameters are not unique in general; the value is.)
        // Even-split start: counts [3, 2, 1, 1.5, 1, 0.5] give x → a = 2/3,
        // y = [1/2, 1/3, 1/6], so P = [1/3, 2/3·1/3 + 1/3·1/2 = 7/18,
        // 2/3·1/6 + 1/3·1/3 = 2/9] and ln L = -3.54715...: 0.2513
        // below the maximum.
        let src = r#"s = x y ; x = "a" | "a" "a" ; y = "a" | "a" "a" | "a" "a" "a" ;"#;
        let ps = prods(src);
        let one = trees(src, &["a", "a"]);
        let two = trees(src, &["a", "a", "a"]);
        let three = trees(src, &["a", "a", "a", "a"]);
        assert_eq!(one, vec![vec![0, 1, 3]]);
        assert_eq!(two, vec![vec![0, 1, 4], vec![0, 2, 3]]);
        assert_eq!(three, vec![vec![0, 1, 5], vec![0, 2, 4]]);
        let sentences = vec![one, two, three];
        let max_log_likelihood = 3.0 * (1.0_f64 / 3.0).ln();
        let fit = em_estimate(&ps, &sentences, 0.0, EM_ITERATIONS).unwrap();
        assert_eq!(fit.objective.len(), EM_ITERATIONS + 1);
        let start = fit.objective[0];
        let expected_start = (1.0_f64 / 3.0).ln() + (7.0_f64 / 18.0).ln() + (2.0_f64 / 9.0).ln();
        assert!((start - expected_start).abs() <= TOL, "{start}");
        assert!(start < max_log_likelihood - 0.25, "{start}");
        // 1e-6 nats: far above rounding (1e-15) and far below the 0.25 gap
        // of the start, which is what a missing E-step weighting leaves.
        let last = fit.objective[EM_ITERATIONS];
        assert!(
            (last - max_log_likelihood).abs() <= 1e-6,
            "{last} vs {max_log_likelihood}"
        );
    }

    #[test]
    fn em_keeps_a_start_that_is_already_a_maximum() {
        // Ids: 0 is s → a, 1 is s → b, 2 is a → "t", 3 is b → "t", 4 is b → "u".
        // Data t, t, u. P(u) = P(s → b) · P(b → u), P(t) = 1 - P(u), so only
        // P(t) matters and the maximum is 2/3 (relative frequency):
        // max ln L = 2 ln(2/3) + ln(1/3) = -1.9095425048844386. The even
        // split already has P(t) = 1/3 + 2/3 · 1/2 = 2/3, so EM must not
        // move away from it.
        let src = r#"s = a | b ; a = "t" ; b = "t" | "u" ;"#;
        let ps = prods(src);
        let sentences = vec![trees(src, &["t"]), trees(src, &["t"]), trees(src, &["u"])];
        let fit = em_estimate(&ps, &sentences, 0.0, EM_ITERATIONS).unwrap();
        for value in &fit.objective {
            assert!((value + 1.9095425048844386).abs() <= TOL, "{value}");
        }
    }

    #[test]
    fn em_weights_do_not_underflow_on_long_derivations() {
        // Ids 0 and 1 have probability 1e-200 and 1e-201. Trees [0, 0, 0] and
        // [1, 1, 1] have probabilities 1e-600 and 1e-603, both 0 in f64, but
        // their log-space weights are 1000/1001 and 1/1001. Production 0 is
        // used 3 times by the first tree, production 1 by the second.
        let prob = [1e-200, 1e-201];
        let sentences = vec![vec![vec![0, 0, 0], vec![1, 1, 1]]];
        let counts = weighted_counts(&prob, &sentences).unwrap();
        assert_close(&counts, &[3.0 * 1000.0 / 1001.0, 3.0 / 1001.0]);
    }

    #[test]
    fn em_rejects_a_sentence_whose_trees_all_have_probability_zero() {
        let err = weighted_counts(&[0.0, 0.5], &[vec![vec![0], vec![0, 1]]]).unwrap_err();
        assert!(err.contains("probability 0"), "{err}");
        // One tree with positive probability is enough.
        assert!(weighted_counts(&[0.0, 0.5], &[vec![vec![0], vec![1]]]).is_ok());
    }

    #[test]
    fn em_propagates_errors_from_expected_counts() {
        let ps = prods(r#"s = "a" ;"#);
        let err = em_estimate(&ps, &[Vec::new()], 1.0, 3).err().unwrap();
        assert!(err.contains("no derivations"), "{err}");
        let err = em_estimate(&ps, &[vec![vec![1]]], 1.0, 3).err().unwrap();
        assert!(err.contains("production id 1"), "{err}");
    }

    #[test]
    fn em_with_zero_iterations_is_the_even_split() {
        let src = r#"s = x | y ; x = "t" ; y = "t" ;"#;
        let ps = prods(src);
        let sentences = vec![trees(src, &["t"])];
        let fit = em_estimate(&ps, &sentences, 1.0, 0).unwrap();
        assert_eq!(fit.objective.len(), 1);
        assert_close(&fit.prob, &[0.5, 0.5, 1.0, 1.0]);
    }
}

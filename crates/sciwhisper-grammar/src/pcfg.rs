//! Probabilistic CFG over the productions of `docs/grammar/math.ebnf`.
//!
//! The probability of a production is its relative frequency among the uses
//! of its left-hand side in training derivations. A sentence with several
//! derivations splits its count evenly between them; this is a shortcut, not
//! EM. Add-one smoothing keeps a production unseen in train above zero.

use std::collections::HashMap;

use crate::earley::Production;

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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use crate::earley::{derivations, productions, DerivationOutcome, Production, Symbol};
    use crate::ebnf::{parse, Grammar};

    use super::{estimate, expected_counts, PSEUDOCOUNT};

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
}

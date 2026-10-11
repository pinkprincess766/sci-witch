//! Log-probabilities of derivations and sentences under a PCFG, and perplexity.
//!
//! A derivation is a list of production ids (pre-order). Its probability is
//! the product of its production probabilities. A sentence can have several
//! derivations, so its probability is the sum over all of them (ambiguity is
//! summed over trees). All log values are natural logs, in nats.

/// `ln P(tree) = Σ ln prob[p]` over the production ids of one derivation.
///
/// A production with probability 0 makes it -inf. Panics if an id is out of range.
pub fn tree_log_prob(prob: &[f64], derivation: &[usize]) -> f64 {
    derivation.iter().map(|&p| prob[p].ln()).sum()
}

/// ln P(sentence) = ln Σ_trees P(tree), by log-sum-exp.
///
/// No derivations, or all of them -inf, give -inf.
pub fn sentence_log_prob(prob: &[f64], derivations: &[Vec<usize>]) -> f64 {
    let tree_log_probs: Vec<f64> = derivations
        .iter()
        .map(|derivation| tree_log_prob(prob, derivation))
        .collect();
    log_sum_exp(&tree_log_probs)
}

/// ln Σ exp(x) without underflow: the largest term is factored out first.
fn log_sum_exp(xs: &[f64]) -> f64 {
    let max = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if max == f64::NEG_INFINITY {
        return f64::NEG_INFINITY;
    }
    let sum_shifted: f64 = xs.iter().map(|&x| (x - max).exp()).sum();
    max + sum_shifted.ln()
}

/// Per-token perplexity exp(-total_log_prob / total_tokens); `total_log_prob` is in nats.
///
/// `None` when `total_tokens` is 0.
pub fn perplexity(total_log_prob: f64, total_tokens: usize) -> Option<f64> {
    if total_tokens == 0 {
        return None;
    }
    let nats_per_token = -total_log_prob / total_tokens as f64;
    Some(nats_per_token.exp())
}

#[cfg(test)]
mod tests {
    use super::{perplexity, sentence_log_prob, tree_log_prob};

    /// Relative tolerance 1e-12. A few `ln`/`exp`/add operations on values of
    /// order 1 to 1000 give errors near 1e-15, so 1e-12 is generous. A real
    /// bug (underflow to -inf, a wrong sign) is off by far more.
    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= 1e-12 * expected.abs(),
            "{actual} != {expected}"
        );
    }

    #[test]
    fn tree_is_sum_of_production_logs() {
        // 3 * ln 0.5
        assert_close(tree_log_prob(&[0.5, 0.5], &[0, 0, 1]), -2.0794415416798357);
    }

    #[test]
    #[should_panic]
    fn tree_panics_on_unknown_production() {
        tree_log_prob(&[0.5], &[1]);
    }

    #[test]
    fn two_derivations_add_probabilities() {
        // ln(0.1 + 0.3) = ln 0.4
        assert_close(
            sentence_log_prob(&[0.1, 0.3], &[vec![0], vec![1]]),
            -0.916290731874155,
        );
    }

    #[test]
    fn underflowing_trees_do_not_become_minus_inf() {
        // Each tree has P = e^-1000, which underflows in f64. The sum is
        // 2 * e^-1000, so ln = -1000 + ln 2. A naive ln(Σ exp) gives -inf.
        let prob = [(-500f64).exp()];
        let derivations = [vec![0, 0], vec![0, 0]];
        assert_close(sentence_log_prob(&prob, &derivations), -999.3068528194401);
    }

    #[test]
    fn no_derivations_is_minus_inf() {
        assert_eq!(sentence_log_prob(&[0.5], &[]), f64::NEG_INFINITY);
    }

    #[test]
    fn zero_probability_tree_is_minus_inf_not_nan() {
        let log_p = sentence_log_prob(&[0.0], &[vec![0]]);
        assert!(log_p == f64::NEG_INFINITY, "got {log_p}");
    }

    #[test]
    fn zero_probability_tree_adds_nothing() {
        // Only the tree with prob 0.5 counts: ln 0.5.
        assert_close(
            sentence_log_prob(&[0.0, 0.5], &[vec![0], vec![1]]),
            0.5f64.ln(),
        );
    }

    #[test]
    fn perplexity_of_two_tokens_at_half() {
        // exp(-(2 ln 0.5) / 2) = 2
        assert_close(perplexity(2.0 * 0.5f64.ln(), 2).unwrap(), 2.0);
    }

    #[test]
    fn perplexity_of_zero_tokens_is_none() {
        assert_eq!(perplexity(-3.0, 0), None);
    }

    #[test]
    fn perplexity_of_certain_sentence_is_one() {
        assert_close(perplexity(0.0, 5).unwrap(), 1.0);
    }
}

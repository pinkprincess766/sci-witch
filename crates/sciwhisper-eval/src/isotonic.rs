//! Isotonic regression (pool adjacent violators) for calibration.
//!
//! Fits a non-decreasing step function from a score to the chance that an
//! answer is right. Its output is a probability of the outcome, unlike the
//! parse levels, which are ranks. Tied scores are pooled into one point, and
//! between thresholds the calibrator is a step: a score takes the value of
//! the largest threshold at or below it.

/// A run of adjacent points that share one fitted value.
#[derive(Clone, Copy)]
struct Block {
    weight: f64,
    value: f64,
    len: usize,
}

/// Weighted least-squares non-decreasing fit of `y` (pool adjacent violators).
///
/// Panics if lengths differ, a weight is not `> 0`, or a value is not finite.
pub fn pool_adjacent_violators(y: &[f64], w: &[f64]) -> Vec<f64> {
    assert_eq!(y.len(), w.len(), "y and w must have the same length");
    assert!(y.iter().all(|v| v.is_finite()), "y must be finite");
    assert!(w.iter().all(|&v| v > 0.0), "weights must be > 0");

    let mut blocks: Vec<Block> = Vec::with_capacity(y.len());
    for (&value, &weight) in y.iter().zip(w) {
        blocks.push(Block {
            weight,
            value,
            len: 1,
        });
        // A merge can push the new block below the one before it, so keep
        // merging until the last two blocks are in order.
        loop {
            let k = blocks.len();
            if k < 2 || blocks[k - 2].value <= blocks[k - 1].value {
                break;
            }
            let last = blocks[k - 1];
            blocks.truncate(k - 1);
            let before = &mut blocks[k - 2];
            let weight_sum = before.weight + last.weight;
            before.value += (last.value - before.value) * (last.weight / weight_sum);
            before.weight = weight_sum;
            before.len += last.len;
        }
    }
    blocks
        .iter()
        .flat_map(|b| std::iter::repeat_n(b.value, b.len))
        .collect()
}

/// Step-function calibrator fitted on (score, outcome) pairs.
pub struct IsotonicCalibrator {
    /// Distinct scores, ascending.
    thresholds: Vec<f64>,
    /// Fitted value from each threshold up to the next one. Non-decreasing.
    values: Vec<f64>,
}

impl IsotonicCalibrator {
    /// Sorts by score, pools equal scores into one point (weight = their
    /// count, value = mean outcome), then runs [`pool_adjacent_violators`].
    ///
    /// Err on empty input, on a length mismatch, or on a non-finite score.
    pub fn fit(scores: &[f64], outcomes: &[bool]) -> Result<Self, String> {
        if scores.len() != outcomes.len() {
            return Err(format!(
                "{} scores but {} outcomes",
                scores.len(),
                outcomes.len()
            ));
        }
        if scores.is_empty() {
            return Err("no examples to fit".to_string());
        }
        if scores.iter().any(|s| !s.is_finite()) {
            return Err("score is not finite".to_string());
        }

        let mut pairs: Vec<(f64, bool)> = scores
            .iter()
            .copied()
            .zip(outcomes.iter().copied())
            .collect();
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

        let mut thresholds = Vec::new();
        let mut weights = Vec::new();
        let mut means = Vec::new();
        for group in pairs.chunk_by(|a, b| a.0 == b.0) {
            let right = group.iter().filter(|p| p.1).count();
            thresholds.push(group[0].0);
            weights.push(group.len() as f64);
            means.push(right as f64 / group.len() as f64);
        }
        Ok(Self {
            thresholds,
            values: pool_adjacent_violators(&means, &weights),
        })
    }

    /// Value of the largest threshold at or below `score`. Below the smallest
    /// threshold, the first value. Always in `[0, 1]`, because outcomes are 0/1.
    pub fn predict(&self, score: f64) -> f64 {
        let passed = self.thresholds.partition_point(|&t| t <= score);
        self.values[passed.saturating_sub(1)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed-seed generator for the randomized tests. It only has to repeat
    /// exactly on every run; it is not a statistical source.
    struct Lcg(u64);

    impl Lcg {
        fn draw(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % n
        }
    }

    fn assert_close(actual: &[f64], expected: &[f64]) {
        assert_eq!(actual.len(), expected.len(), "{actual:?} vs {expected:?}");
        for (a, e) in actual.iter().zip(expected) {
            assert!((a - e).abs() <= 1e-12, "{actual:?} vs {expected:?}");
        }
    }

    fn weighted_sse(y: &[f64], w: &[f64], fit: &[f64]) -> f64 {
        y.iter()
            .zip(w)
            .zip(fit)
            .map(|((y, w), f)| w * (y - f).powi(2))
            .sum()
    }

    /// Best weighted SSE over every split of `0..n` into contiguous blocks
    /// whose means are non-decreasing. Bit `i` of `cuts` ends a block after
    /// index `i`. Exact for integer `y` and `w`, so the means compare exactly.
    fn brute_force_sse(y: &[f64], w: &[f64]) -> f64 {
        let n = y.len();
        let mut best = f64::INFINITY;
        for cuts in 0..(1u32 << (n - 1)) {
            let mut sse = 0.0;
            let mut prev_mean = f64::NEG_INFINITY;
            let mut monotone = true;
            let mut start = 0;
            for end in 1..=n {
                if end == n || (cuts >> (end - 1)) & 1 == 1 {
                    let block_w: f64 = w[start..end].iter().sum();
                    let block_wy: f64 = (start..end).map(|i| w[i] * y[i]).sum();
                    let mean = block_wy / block_w;
                    if mean < prev_mean {
                        monotone = false;
                        break;
                    }
                    prev_mean = mean;
                    sse += (start..end)
                        .map(|i| w[i] * (y[i] - mean).powi(2))
                        .sum::<f64>();
                    start = end;
                }
            }
            if monotone && sse < best {
                best = sse;
            }
        }
        best
    }

    #[test]
    fn pav_of_a_single_violating_pair_pools_it() {
        // 1, 3, 2, 4: the pair (3, 2) is pooled to 2.5.
        assert_close(
            &pool_adjacent_violators(&[1.0, 3.0, 2.0, 4.0], &[1.0; 4]),
            &[1.0, 2.5, 2.5, 4.0],
        );
    }

    #[test]
    fn pav_of_a_decreasing_run_pools_all_of_it() {
        // 3, 2, 1: the whole run pools to its mean, 2.
        assert_close(
            &pool_adjacent_violators(&[3.0, 2.0, 1.0], &[1.0; 3]),
            &[2.0, 2.0, 2.0],
        );
    }

    #[test]
    fn pav_weights_the_mean() {
        // (3 * 2 + 1 * 0) / (3 + 1) = 1.5.
        assert_close(
            &pool_adjacent_violators(&[2.0, 0.0], &[3.0, 1.0]),
            &[1.5, 1.5],
        );
    }

    /// The pair (5, 1) pools to 3, which is below 4. That merge has to be
    /// checked again: (4 + 5 + 1) / 3 = 10/3. A pass that merges once and
    /// stops prints [4, 3, 3], which is not non-decreasing.
    #[test]
    fn pav_merges_again_when_a_merge_drops_below_the_block_before() {
        assert_close(
            &pool_adjacent_violators(&[4.0, 5.0, 1.0], &[1.0; 3]),
            &[10.0 / 3.0; 3],
        );
    }

    #[test]
    fn pav_matches_brute_force_on_small_inputs() {
        let mut rng = Lcg(20_261_012);
        for case in 0..500 {
            let n = 1 + rng.draw(8) as usize;
            let y: Vec<f64> = (0..n).map(|_| rng.draw(6) as f64).collect();
            let w: Vec<f64> = (0..n).map(|_| (1 + rng.draw(3)) as f64).collect();
            let fit = pool_adjacent_violators(&y, &w);
            assert!(
                fit.windows(2).all(|p| p[0] <= p[1]),
                "case {case}: {fit:?} is not non-decreasing"
            );
            let best = brute_force_sse(&y, &w);
            let got = weighted_sse(&y, &w, &fit);
            assert!(
                (got - best).abs() <= 1e-9,
                "case {case}: pav sse {got} vs brute force {best} for y={y:?} w={w:?}"
            );
        }
    }

    #[test]
    fn pav_output_is_monotone_and_keeps_the_weighted_sum() {
        let mut rng = Lcg(7_777);
        for _ in 0..200 {
            let n = 1 + rng.draw(60) as usize;
            let y: Vec<f64> = (0..n).map(|_| rng.draw(1000) as f64 / 1000.0).collect();
            let w: Vec<f64> = (0..n).map(|_| (1 + rng.draw(5)) as f64).collect();
            let fit = pool_adjacent_violators(&y, &w);
            assert!(fit.windows(2).all(|p| p[0] <= p[1]), "{fit:?}");
            // Pooling keeps each block's weighted sum, so the total does too.
            let sum_y: f64 = y.iter().zip(&w).map(|(y, w)| y * w).sum();
            let sum_fit: f64 = fit.iter().zip(&w).map(|(f, w)| f * w).sum();
            assert!((sum_y - sum_fit).abs() <= 1e-9, "{sum_y} vs {sum_fit}");
        }
    }

    #[test]
    #[should_panic(expected = "same length")]
    fn pav_panics_on_length_mismatch() {
        pool_adjacent_violators(&[1.0, 2.0], &[1.0]);
    }

    #[test]
    #[should_panic(expected = "weights must be > 0")]
    fn pav_panics_on_a_zero_weight() {
        pool_adjacent_violators(&[1.0, 2.0], &[1.0, 0.0]);
    }

    #[test]
    #[should_panic(expected = "y must be finite")]
    fn pav_panics_on_a_nan_value() {
        pool_adjacent_violators(&[1.0, f64::NAN], &[1.0, 1.0]);
    }

    #[test]
    fn calibrator_fits_a_step_function_on_a_small_example() {
        // Scores 0.2 holds one right and one wrong answer, so its value is 0.5.
        let cal =
            IsotonicCalibrator::fit(&[0.1, 0.2, 0.2, 0.9], &[false, true, false, true]).unwrap();
        assert_eq!(cal.thresholds, vec![0.1, 0.2, 0.9]);
        assert_close(&cal.values, &[0.0, 0.5, 1.0]);
        assert_eq!(cal.predict(0.05), 0.0);
        assert_eq!(cal.predict(0.2), 0.5);
        assert_eq!(cal.predict(0.5), 0.5);
        assert_eq!(cal.predict(1.0), 1.0);
    }

    #[test]
    fn calibrator_pools_a_violation_between_thresholds() {
        // Outcomes 1, 0, 0 are out of order, so all three pool to 1/3.
        let cal = IsotonicCalibrator::fit(&[0.1, 0.2, 0.3], &[true, false, false]).unwrap();
        assert_close(&cal.values, &[1.0 / 3.0; 3]);
    }

    #[test]
    fn calibrator_takes_the_first_value_below_the_smallest_threshold() {
        let cal = IsotonicCalibrator::fit(&[0.5, 0.5], &[true, false]).unwrap();
        assert_eq!(cal.thresholds, vec![0.5]);
        assert_eq!(cal.predict(-3.0), 0.5);
        assert_eq!(cal.predict(0.5), 0.5);
        assert_eq!(cal.predict(9.0), 0.5);
    }

    #[test]
    fn calibrator_predictions_are_probabilities_and_non_decreasing() {
        let mut rng = Lcg(31_337);
        for _ in 0..100 {
            let n = 1 + rng.draw(40) as usize;
            let scores: Vec<f64> = (0..n).map(|_| rng.draw(100) as f64 / 100.0).collect();
            let outcomes: Vec<bool> = (0..n).map(|_| rng.draw(2) == 1).collect();
            let cal = IsotonicCalibrator::fit(&scores, &outcomes).unwrap();
            let mut prev = f64::NEG_INFINITY;
            for step in 0..=120 {
                let p = cal.predict(f64::from(step) / 100.0 - 0.1);
                assert!((0.0..=1.0).contains(&p), "{p}");
                assert!(p >= prev, "{p} after {prev}");
                prev = p;
            }
        }
    }

    #[test]
    fn calibrator_rejects_empty_input() {
        assert!(IsotonicCalibrator::fit(&[], &[]).is_err());
    }

    #[test]
    fn calibrator_rejects_a_nan_score() {
        assert!(IsotonicCalibrator::fit(&[0.1, f64::NAN], &[true, false]).is_err());
    }

    #[test]
    fn calibrator_rejects_an_infinite_score() {
        assert!(IsotonicCalibrator::fit(&[f64::INFINITY], &[true]).is_err());
    }

    #[test]
    fn calibrator_rejects_a_length_mismatch() {
        assert!(IsotonicCalibrator::fit(&[0.1, 0.2], &[true]).is_err());
    }
}

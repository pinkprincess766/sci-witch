//! Question 1 rescoring of a Whisper N-best list (S6), by the sealed formula of
//! `research/protocol/question-1-nbest.md`:
//!
//! `score = λ · ASR + (1 − λ) · grammar`
//!
//! - ASR: the hypothesis's `sum_logprob` (S5, `research/asr/whisper_nbest.py`);
//! - grammar: `ln P(text)` under a PCFG over `docs/grammar/math.ebnf`, see
//!   [`GrammarScorer::log_prob`]; `None` when the text does not tokenize or
//!   does not parse (the protocol has no rule for that; it counts as −∞);
//! - λ = 1 is ASR only, λ = 0 is grammar only; a term with weight 0 is not
//!   evaluated, so λ = 1 never computes `0 · (−∞)`;
//! - ties go to the hypothesis earlier in the recogniser's list.
//!
//! No λ selection, no cross-validation and no AST exact match here (that is
//! S7). Nothing here reads the test voice.

use sciwhisper_core::research::math_token_classes;
use sciwhisper_grammar::{derivations, likelihood::sentence_log_prob, DerivationOutcome, Grammar};

/// The sealed λ grid: 0.0, 0.1, …, 1.0.
pub const LAMBDA_GRID: [f64; 11] = [0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0];

/// `ln P(text)` under a fixed PCFG: the grammar, its start rule, production
/// probabilities indexed like `sciwhisper_grammar::productions(grammar)`, and
/// the tokenizer mode.
pub struct GrammarScorer<'a> {
    pub grammar: &'a Grammar,
    pub start: &'a str,
    pub prob: &'a [f64],
    pub physics: bool,
}

impl GrammarScorer<'_> {
    /// `ln P(text)` summed over all derivation trees; `None` if the text does
    /// not tokenize, is rejected, or is too long or too ambiguous to enumerate.
    pub fn log_prob(&self, text: &str) -> Option<f64> {
        let tokens = math_token_classes(text, self.physics).ok()?;
        let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        match derivations(self.grammar, self.start, &refs) {
            DerivationOutcome::Derivations(trees) => Some(sentence_log_prob(self.prob, &trees)),
            _ => None,
        }
    }
}

/// The combined score of one hypothesis. A missing grammar score is −∞; the
/// term whose weight is 0 is skipped.
pub fn combined(lambda: f64, asr: f64, grammar: Option<f64>) -> f64 {
    assert!((0.0..=1.0).contains(&lambda), "λ out of [0, 1]: {lambda}");
    let g = || grammar.unwrap_or(f64::NEG_INFINITY);
    if lambda == 1.0 {
        asr
    } else if lambda == 0.0 {
        g()
    } else {
        lambda * asr + (1.0 - lambda) * g()
    }
}

/// Index of the chosen hypothesis among `(asr, grammar)` pairs in list order:
/// the highest combined score, the earliest on a tie. `None` for an empty list.
pub fn choose(lambda: f64, hypotheses: &[(f64, Option<f64>)]) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (i, &(asr, grammar)) in hypotheses.iter().enumerate() {
        let s = combined(lambda, asr, grammar);
        assert!(!s.is_nan(), "NaN score at hypothesis {i}");
        if best.is_none_or(|(_, b)| s > b) {
            best = Some((i, s));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciwhisper_grammar::{ebnf::parse, productions};

    /// Synthetic N-best, K = 4: hypothesis 1 (greedy) is not the ASR argmax.
    const NBEST: [(f64, Option<f64>); 4] = [
        (-5.0, Some(-9.0)),
        (-4.0, None),
        (-6.0, Some(-2.0)),
        (-4.5, Some(-3.0)),
    ];

    #[test]
    fn lambda_one_picks_the_asr_argmax_not_the_first_hypothesis() {
        assert_eq!(choose(1.0, &NBEST), Some(1));
    }

    #[test]
    fn lambda_one_never_evaluates_an_unparsed_grammar_term() {
        let s = combined(1.0, -4.0, None);
        assert_eq!(s, -4.0);
        assert!(!s.is_nan());
    }

    #[test]
    fn lambda_zero_picks_the_grammar_argmax_and_unparsed_loses() {
        assert_eq!(choose(0.0, &NBEST), Some(2));
        assert_eq!(combined(0.0, -1.0, None), f64::NEG_INFINITY);
    }

    #[test]
    fn a_middle_lambda_mixes_both() {
        // λ = 0.5: −7, −∞, −4, −3.75.
        assert_eq!(choose(0.5, &NBEST), Some(3));
    }

    #[test]
    fn ties_go_to_the_earlier_hypothesis() {
        let tied = [(-3.0, Some(-1.0)), (-3.0, Some(-1.0)), (-3.0, Some(-1.0))];
        for &l in &LAMBDA_GRID {
            assert_eq!(choose(l, &tied), Some(0), "λ = {l}");
        }
        assert_eq!(choose(1.0, &[(-2.0, None), (-2.0, Some(0.0))]), Some(0));
    }

    #[test]
    fn all_unparsed_at_lambda_below_one_keeps_the_first() {
        let none = [(-1.0, None), (-0.5, None)];
        assert_eq!(choose(0.3, &none), Some(0));
        assert_eq!(choose(1.0, &none), Some(1));
        assert_eq!(choose(0.5, &[]), None);
    }

    #[test]
    fn the_grid_is_the_sealed_one() {
        assert_eq!(LAMBDA_GRID.len(), 11);
        assert_eq!((LAMBDA_GRID[0], LAMBDA_GRID[10]), (0.0, 1.0));
    }

    fn math_grammar() -> Grammar {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/grammar/math.ebnf");
        parse(&std::fs::read_to_string(path).expect("math.ebnf")).expect("parses")
    }

    #[test]
    fn grammar_score_is_finite_for_a_formula_and_none_for_prose() {
        let grammar = math_grammar();
        let start = grammar.start().expect("start").to_string();
        let n = productions(&grammar).len();
        let prob = vec![0.5; n];
        let scorer = GrammarScorer {
            grammar: &grammar,
            start: &start,
            prob: &prob,
            physics: false,
        };
        let lp = scorer.log_prob("икс в кубе");
        assert!(lp.is_some_and(|x| x.is_finite() && x < 0.0), "{lp:?}");
        assert_eq!(
            scorer.log_prob("Поправка вышла в следующем номере журнала."),
            None
        );
    }
}

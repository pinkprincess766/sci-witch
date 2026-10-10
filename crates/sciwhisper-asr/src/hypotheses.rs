//! Reader for the JSON written by `research/asr/whisper_hypotheses.py`:
//! Whisper hypotheses with per-token log-probabilities.
//!
//! The aggregates in the file (`sum_logprob`, `mean_logprob`, `min_logprob`) are
//! recomputed here from the token list, and a file where they disagree is refused.
//! `window_avg_logprob` and `no_speech_prob` come from Whisper itself and are
//! passed through unchecked.

use serde::Deserialize;

use crate::error::{Error, Result};

/// The only file layout this module reads.
pub const SCHEMA_VERSION: u32 = 1;

/// Largest allowed `|stored - recomputed| / max(1, |recomputed|)` for an aggregate.
/// The stored numbers are f64 sums of the same f64 token values, so only summation
/// order and the last digit of float parsing differ: at most 224 tokens (Whisper's
/// sample limit) times 2.2e-16 gives about 5e-14. 1e-9 leaves four orders of margin
/// and is still far below the 1e-6 change of one token that must be caught.
pub const AGGREGATE_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Clone, Deserialize)]
pub struct Hypotheses {
    pub schema_version: u32,
    pub manifest: Manifest,
    pub language: String,
    pub hypotheses: Vec<Hypothesis>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub script: FileRef,
    pub python_version: String,
    pub torch_version: String,
    pub whisper_version: String,
    pub model: ModelRef,
    /// `greedy` or `sample`.
    pub mode: String,
    pub temperature: f64,
    pub best_of: Option<u32>,
    pub beam_size: Option<u32>,
    pub seed: Option<i64>,
    pub device: String,
    pub fp16: bool,
    pub language_requested: String,
    pub audio: FileRef,
    pub sample_rate_hz: u32,
    pub audio_duration_s: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileRef {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelRef {
    pub name: String,
    pub file: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Hypothesis {
    pub text: String,
    /// Text tokens only; the end-of-text token is `eot_logprob`.
    pub tokens: Vec<Token>,
    pub eot_logprob: f64,
    /// Over text tokens plus end-of-text.
    pub sum_logprob: f64,
    /// Over text tokens only; `None` when there are none.
    pub mean_logprob: Option<f64>,
    /// Over text tokens only; `None` when there are none.
    pub min_logprob: Option<f64>,
    /// Whisper's own `avg_logprob` for the window.
    pub window_avg_logprob: f64,
    pub no_speech_prob: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Token {
    pub id: u32,
    pub text: String,
    pub logprob: f64,
}

fn unreadable(detail: String) -> Error {
    Error::OutputUnreadable { detail }
}

fn close(stored: f64, recomputed: f64) -> bool {
    (stored - recomputed).abs() <= AGGREGATE_TOLERANCE * recomputed.abs().max(1.0)
}

fn check_aggregate(
    index: usize,
    name: &str,
    stored: Option<f64>,
    recomputed: Option<f64>,
) -> Result<()> {
    let agrees = match (stored, recomputed) {
        (Some(s), Some(r)) => close(s, r),
        (None, None) => true,
        _ => false,
    };
    if agrees {
        Ok(())
    } else {
        Err(unreadable(format!(
            "hypothesis {index}: {name} is {stored:?} in the file but {recomputed:?} from its tokens"
        )))
    }
}

fn check_hypothesis(index: usize, hyp: &Hypothesis) -> Result<()> {
    let token_sum: f64 = hyp.tokens.iter().map(|t| t.logprob).sum();
    let n_tokens = hyp.tokens.len();
    let mean = (n_tokens > 0).then(|| token_sum / n_tokens as f64);
    let min = hyp.tokens.iter().map(|t| t.logprob).reduce(f64::min);
    check_aggregate(
        index,
        "sum_logprob",
        Some(hyp.sum_logprob),
        Some(token_sum + hyp.eot_logprob),
    )?;
    check_aggregate(index, "mean_logprob", hyp.mean_logprob, mean)?;
    check_aggregate(index, "min_logprob", hyp.min_logprob, min)
}

/// Parse the JSON and refuse a file whose aggregates do not match its tokens.
pub fn parse_hypotheses(json: &str) -> Result<Hypotheses> {
    let parsed: Hypotheses = serde_json::from_str(json)?;
    if parsed.schema_version != SCHEMA_VERSION {
        return Err(unreadable(format!(
            "schema_version {} is not supported (expected {SCHEMA_VERSION})",
            parsed.schema_version
        )));
    }
    if parsed.hypotheses.is_empty() {
        return Err(unreadable("the file has no hypotheses".to_string()));
    }
    for (index, hyp) in parsed.hypotheses.iter().enumerate() {
        check_hypothesis(index, hyp)?;
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hypothesis(token_logprobs: &[f64], eot_logprob: f64) -> Hypothesis {
        let tokens: Vec<Token> = token_logprobs
            .iter()
            .map(|&logprob| Token {
                id: 1,
                text: "a".into(),
                logprob,
            })
            .collect();
        let sum: f64 = token_logprobs.iter().sum();
        Hypothesis {
            text: "a".into(),
            eot_logprob,
            sum_logprob: sum + eot_logprob,
            mean_logprob: (!tokens.is_empty()).then(|| sum / tokens.len() as f64),
            min_logprob: token_logprobs.iter().copied().reduce(f64::min),
            tokens,
            window_avg_logprob: 0.0,
            no_speech_prob: 0.0,
        }
    }

    #[test]
    fn consistent_hypothesis_passes() {
        check_hypothesis(0, &hypothesis(&[-0.5, -1.5, -0.25], -0.125)).unwrap();
    }

    #[test]
    fn sum_tolerance_holds_on_both_sides() {
        let mut inside = hypothesis(&[-0.5, -1.5], -0.25);
        inside.sum_logprob += 0.5 * AGGREGATE_TOLERANCE;
        check_hypothesis(0, &inside).unwrap();

        let mut outside = hypothesis(&[-0.5, -1.5], -0.25);
        // |sum| is 2.25, so the allowed gap is 2.25 * AGGREGATE_TOLERANCE.
        outside.sum_logprob += 4.0 * AGGREGATE_TOLERANCE;
        assert!(check_hypothesis(0, &outside).is_err());
    }

    #[test]
    fn sum_without_eot_is_refused() {
        // The obvious wrong sum: text tokens only, end-of-text left out.
        let mut hyp = hypothesis(&[-0.5, -1.5], -0.25);
        hyp.sum_logprob -= hyp.eot_logprob;
        assert!(check_hypothesis(0, &hyp).is_err());
    }

    #[test]
    fn empty_token_list_needs_empty_mean_and_min() {
        check_hypothesis(0, &hypothesis(&[], -0.25)).unwrap();

        let mut hyp = hypothesis(&[], -0.25);
        hyp.mean_logprob = Some(0.0);
        assert!(check_hypothesis(0, &hyp).is_err());

        let mut hyp = hypothesis(&[-0.5], -0.25);
        hyp.min_logprob = None;
        assert!(check_hypothesis(0, &hyp).is_err());
    }
}

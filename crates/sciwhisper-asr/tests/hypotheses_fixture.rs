//! `parse_hypotheses` against a real file from `research/asr/whisper_hypotheses.py`.
//!
//! The fixture is the greedy run on `research/voice-samples/synthetic/quadratic.caf`
//! with model `base`, openai-whisper 20250625, CPU. The "wrong" tests edit one
//! number in it and expect the file to be refused.

use sciwhisper_asr::hypotheses::{parse_hypotheses, SCHEMA_VERSION};

const FIXTURE: &str = include_str!("fixtures/whisper-hypotheses-greedy-quadratic.json");

/// The fixture after `edit` has changed it.
fn edited(edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut value: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    edit(&mut value);
    value.to_string()
}

fn error_text(json: &str) -> String {
    parse_hypotheses(json).unwrap_err().to_string()
}

#[test]
fn real_file_parses_with_its_manifest() {
    let parsed = parse_hypotheses(FIXTURE).unwrap();
    assert_eq!(parsed.language, "ru");
    assert_eq!(parsed.manifest.mode, "greedy");
    assert_eq!(parsed.manifest.temperature, 0.0);
    assert_eq!(parsed.manifest.seed, None);
    assert_eq!(parsed.manifest.best_of, None);
    assert_eq!(parsed.manifest.sample_rate_hz, 16_000);
    assert_eq!(parsed.manifest.model.name, "base");
    assert_eq!(parsed.manifest.audio.sha256.len(), 64);
    assert!(!parsed.manifest.audio.path.starts_with('/'));
    assert_eq!(parsed.hypotheses.len(), 1);
}

#[test]
fn real_file_values_are_the_values_in_the_file() {
    // Copied from the fixture; a swapped or mis-named field breaks this.
    let hyp = &parse_hypotheses(FIXTURE).unwrap().hypotheses[0];
    assert_eq!(hyp.text, "x² плюс 2x-3 равно нулю.");
    assert_eq!(hyp.tokens.len(), 11);
    assert_eq!(hyp.tokens[0].id, 2031);
    assert_eq!(hyp.tokens[0].text, " x");
    assert!(
        (hyp.sum_logprob - (-5.091_591)).abs() < 1e-6,
        "{}",
        hyp.sum_logprob
    );
    assert!((hyp.min_logprob.unwrap() - (-0.933_342_8)).abs() < 1e-6);
    assert!((hyp.no_speech_prob - 0.036_095_54).abs() < 1e-6);
}

#[test]
fn sum_matches_whisper_window_avg_times_tokens_plus_one() {
    // ASR_SCORES_RU.md: avg_logprob = sum / (n_tokens + 1), EOT included.
    // Whisper computes in float32, so the gap is about 1e-6; 1e-4 is the bound the doc found.
    let hyp = &parse_hypotheses(FIXTURE).unwrap().hypotheses[0];
    let from_window = hyp.window_avg_logprob * (hyp.tokens.len() as f64 + 1.0);
    let relative_gap = (hyp.sum_logprob - from_window).abs() / hyp.sum_logprob.abs();
    assert!(relative_gap < 1e-4, "{relative_gap}");
}

#[test]
fn edited_token_logprob_is_refused() {
    // Obvious wrong way: one token changed, aggregates left as they were.
    let json = edited(|v| v["hypotheses"][0]["tokens"][3]["logprob"] = (-0.5).into());
    let message = error_text(&json);
    assert!(message.contains("sum_logprob"), "{message}");
}

#[test]
fn edited_lowest_token_is_refused_by_min_and_mean() {
    // Raising the minimum token keeps min wrong even if someone fixes the sum.
    let json = edited(|v| {
        let hyp = &mut v["hypotheses"][0];
        let old = hyp["tokens"][1]["logprob"].as_f64().unwrap();
        hyp["tokens"][1]["logprob"] = (-0.1).into();
        let sum = hyp["sum_logprob"].as_f64().unwrap();
        hyp["sum_logprob"] = (sum - old - 0.1).into();
    });
    let message = error_text(&json);
    assert!(message.contains("mean_logprob"), "{message}");
}

#[test]
fn edited_min_logprob_is_refused() {
    let json = edited(|v| v["hypotheses"][0]["min_logprob"] = (-0.5).into());
    assert!(error_text(&json).contains("min_logprob"));
}

#[test]
fn edited_eot_logprob_is_refused() {
    let json = edited(|v| v["hypotheses"][0]["eot_logprob"] = (-1.0).into());
    assert!(error_text(&json).contains("sum_logprob"));
}

#[test]
fn removed_token_is_refused() {
    let json = edited(|v| {
        v["hypotheses"][0]["tokens"].as_array_mut().unwrap().pop();
    });
    assert!(parse_hypotheses(&json).is_err());
}

#[test]
fn zero_hypotheses_is_refused() {
    // 0 of 0 must not pass.
    let json = edited(|v| v["hypotheses"] = serde_json::json!([]));
    assert!(error_text(&json).contains("no hypotheses"));
}

#[test]
fn other_schema_version_is_refused() {
    let json = edited(|v| v["schema_version"] = (SCHEMA_VERSION + 1).into());
    assert!(error_text(&json).contains("schema_version"));
}

#[test]
fn sampling_manifest_fields_are_read() {
    let json = edited(|v| {
        v["manifest"]["mode"] = "sample".into();
        v["manifest"]["temperature"] = 0.6.into();
        v["manifest"]["best_of"] = 5.into();
        v["manifest"]["seed"] = 7.into();
    });
    let manifest = parse_hypotheses(&json).unwrap().manifest;
    assert_eq!(manifest.mode, "sample");
    assert_eq!(manifest.best_of, Some(5));
    assert_eq!(manifest.seed, Some(7));
}

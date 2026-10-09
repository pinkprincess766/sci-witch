//! `parse_openai_json` against a real `openai-whisper` output.
//!
//! The fixture is the temperature-0 JSON that `whisper` 20250625 wrote for
//! `research/voice-samples/synthetic/quadratic.caf`
//! (`--model base --language ru --output_format json --fp16 False --temperature 0`).
//! Every other parser test feeds hand-written JSON; this one fails if the real
//! field names or types drift away from what the parser reads.

use sciwhisper_asr::engine::Transcript;
use sciwhisper_asr::whisper_cli::parse_openai_json;

const FIXTURE: &str = include_str!("fixtures/openai-whisper-base-quadratic.json");

/// What is wrong with the scores of `t`, one line per problem. Empty means fine.
/// An empty segment list is a problem: zero segments must not count as "all
/// segments have scores".
fn score_problems(t: &Transcript) -> Vec<String> {
    let mut problems = Vec::new();
    if t.text.trim().is_empty() {
        problems.push("text is empty".to_string());
    }
    if t.segments.is_empty() {
        problems.push("no segments".to_string());
    }
    for (i, seg) in t.segments.iter().enumerate() {
        if seg.avg_logprob.is_none() {
            problems.push(format!("segment {i}: avg_logprob is None"));
        }
        if seg.no_speech_prob.is_none() {
            problems.push(format!("segment {i}: no_speech_prob is None"));
        }
    }
    problems
}

/// The fixture with the listed keys removed from every segment.
fn fixture_without(keys: &[&str]) -> String {
    let mut value: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    for seg in value["segments"].as_array_mut().unwrap() {
        for key in keys {
            seg.as_object_mut().unwrap().remove(*key);
        }
    }
    value.to_string()
}

#[test]
fn real_openai_json_has_text_and_scores_on_every_segment() {
    let t = parse_openai_json(FIXTURE).unwrap();
    assert_eq!(score_problems(&t), Vec::<String>::new());
    assert_eq!(t.language.as_deref(), Some("ru"));
    assert_eq!(t.segments.len(), 1);
}

#[test]
fn real_openai_json_scores_are_the_values_in_the_file() {
    // Values copied from the fixture; a swapped or mis-named field breaks this.
    let seg = &parse_openai_json(FIXTURE).unwrap().segments[0];
    let avg_logprob = seg.avg_logprob.unwrap();
    let no_speech_prob = seg.no_speech_prob.unwrap();
    assert!(
        (avg_logprob - (-0.456_792_83)).abs() < 1e-6,
        "{avg_logprob}"
    );
    assert!(
        (no_speech_prob - 0.042_765_316).abs() < 1e-6,
        "{no_speech_prob}"
    );
}

#[test]
fn check_rejects_json_without_scores() {
    // The obvious wrong way: scores missing, parser still succeeds, text is fine.
    let t = parse_openai_json(&fixture_without(&["avg_logprob", "no_speech_prob"])).unwrap();
    assert!(!t.text.trim().is_empty());
    assert_eq!(
        score_problems(&t),
        vec![
            "segment 0: avg_logprob is None".to_string(),
            "segment 0: no_speech_prob is None".to_string(),
        ]
    );
}

#[test]
fn check_rejects_json_without_segments() {
    // 0 of 0 segments must not pass.
    let t = parse_openai_json(r#"{"text": " x", "segments": [], "language": "ru"}"#).unwrap();
    assert_eq!(score_problems(&t), vec!["no segments".to_string()]);
}

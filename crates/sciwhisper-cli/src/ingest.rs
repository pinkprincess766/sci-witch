//! Turning recordings into corpus records.
//!
//! The research corpus can hold audio from `dataset_schema_version: 2`, but
//! nothing filled those fields in: the numbers a record needs — checksum,
//! duration, format, SNR — come from the file, and the ASR hypothesis comes
//! from a real recogniser run. Typing them by hand is how a corpus acquires
//! numbers nobody measured.
//!
//! What this does **not** do is invent the parts only a person can supply.
//! `human_transcript`, `speaker_id`, `target_*` and, above all, consent must
//! already be in the input manifest. A record missing them is refused, not
//! completed with a plausible guess.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sciwhisper_asr::corpus::describe_wav;
use serde_json::{Map, Value};

/// Schema version in which the audio block exists. Records written here
/// declare it, because they now carry one.
const AUDIO_SCHEMA_VERSION: u64 = 2;

pub struct IngestOptions {
    pub manifest: PathBuf,
    pub output: PathBuf,
    /// Measure the audio but do not run the recogniser. Useful for checking
    /// a freshly collected batch before spending an hour of CPU on it.
    pub describe_only: bool,
}

/// One line of the manifest, after checking.
struct Prepared {
    record: Map<String, Value>,
    audio_path: PathBuf,
}

pub fn run(
    options: IngestOptions,
    transcribe: &mut dyn FnMut(&Path) -> Result<String, String>,
) -> Result<(), String> {
    let text = std::fs::read_to_string(&options.manifest)
        .map_err(|e| format!("{}: {e}", options.manifest.display()))?;
    let root = options
        .manifest
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut prepared = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        prepared.push(prepare(line, index + 1, &root)?);
    }
    if prepared.is_empty() {
        return Err(format!("{}: no records", options.manifest.display()));
    }

    let mut out = String::new();
    let mut spoken = 0usize;
    let mut silent = Vec::new();
    for mut item in prepared {
        let id = item
            .record
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        let facts = describe_wav(&item.audio_path).map_err(|e| format!("{id}: {e}"))?;
        if !facts.has_speech {
            silent.push(id.clone());
        }

        let audio = item
            .record
            .get_mut("audio")
            .and_then(Value::as_object_mut)
            .expect("checked in prepare");
        audio.insert("sha256".into(), Value::String(facts.sha256));
        audio.insert(
            "duration_secs".into(),
            serde_json::json!(round(facts.duration_secs, 3)),
        );
        audio.insert(
            "sample_rate_hz".into(),
            serde_json::json!(facts.sample_rate_hz),
        );
        audio.insert("channels".into(), serde_json::json!(facts.channels));
        match facts.snr_db {
            Some(snr) => {
                audio.insert("snr_db".into(), serde_json::json!(round(snr, 1)));
            }
            // A recording with no speech gets no SNR rather than a number
            // computed from a floor that measured nothing.
            None => {
                audio.remove("snr_db");
            }
        }

        if !options.describe_only {
            let hypothesis = transcribe(&item.audio_path).map_err(|e| format!("{id}: {e}"))?;
            if hypothesis.trim().is_empty() {
                item.record
                    .insert("asr_hypotheses".into(), Value::Array(Vec::new()));
            } else {
                spoken += 1;
                item.record.insert(
                    "asr_hypotheses".into(),
                    serde_json::json!([{ "text": hypothesis.trim() }]),
                );
            }
        }

        item.record.insert(
            "dataset_schema_version".into(),
            Value::Number(AUDIO_SCHEMA_VERSION.into()),
        );
        out.push_str(&serde_json::to_string(&item.record).map_err(|e| e.to_string())?);
        out.push('\n');
        println!("{id}: {}", summary(&item.record));
    }

    std::fs::write(&options.output, out)
        .map_err(|e| format!("{}: {e}", options.output.display()))?;
    println!("written to {}", options.output.display());
    if !options.describe_only {
        println!("transcribed: {spoken}");
    }
    if !silent.is_empty() {
        // Not an error: a silent take is a fact about the session, and the
        // person who collected it decides whether to re-record or drop it.
        println!("no speech detected in: {}", silent.join(", "));
    }
    Ok(())
}

fn summary(record: &Map<String, Value>) -> String {
    let audio = record.get("audio").and_then(Value::as_object);
    let seconds = audio
        .and_then(|a| a.get("duration_secs"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let snr = audio
        .and_then(|a| a.get("snr_db"))
        .and_then(Value::as_f64)
        .map(|value| format!("{value:.0} дБ"))
        .unwrap_or_else(|| "нет речи".into());
    let heard = record
        .get("asr_hypotheses")
        .and_then(Value::as_array)
        .and_then(|list| list.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("—");
    format!("{seconds:.1} с, SNR {snr} → {heard}")
}

fn round(value: f64, places: u32) -> f64 {
    let factor = 10f64.powi(places as i32);
    (value * factor).round() / factor
}

/// Everything checked before a single second of audio is read, so a manifest
/// with a missing consent line fails immediately rather than after an hour.
fn prepare(line: &str, number: usize, root: &Path) -> Result<Prepared, String> {
    let value: Value =
        serde_json::from_str(line).map_err(|e| format!("line {number}: not valid JSON: {e}"))?;
    let mut record = match value {
        Value::Object(map) => map,
        _ => return Err(format!("line {number}: a record must be a JSON object")),
    };
    let id = record
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("line {number}: missing id"))?;

    for field in [
        "human_transcript",
        "target_domain",
        "target_action",
        "split",
    ] {
        if record.get(field).and_then(Value::as_str).is_none() {
            return Err(format!("{id}: missing '{field}'"));
        }
    }
    let provenance = record
        .get("provenance")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{id}: missing 'provenance'"))?
        .to_string();
    if !matches!(provenance.as_str(), "tts" | "real_audio") {
        return Err(format!(
            "{id}: provenance '{provenance}' has no audio; this command only fills records that do"
        ));
    }
    if provenance == "real_audio"
        && record
            .get("speaker_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        return Err(format!(
            "{id}: a recording of a person needs a speaker_id, so speaker leakage between splits can be checked"
        ));
    }

    let audio = record
        .get("audio")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{id}: missing 'audio' block naming the recording"))?;
    let file = audio
        .get("file")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{id}: audio.file is required"))?
        .to_string();
    let relative = Path::new(&file);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(format!(
            "{id}: audio.file '{file}' must be relative to the manifest and stay inside its directory"
        ));
    }
    if provenance == "real_audio" {
        let consent = audio
            .get("consent")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{id}: a recording of a person needs a consent block"))?;
        let granted = consent.get("granted").and_then(Value::as_bool) == Some(true);
        let named = consent
            .get("statement_id")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        let dated = consent
            .get("date")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        if !(granted && named && dated) {
            return Err(format!(
                "{id}: consent must be granted, name the statement the speaker agreed to, and carry a date"
            ));
        }
    }

    let audio_path = root.join(relative);
    if !audio_path.is_file() {
        return Err(format!("{id}: {} not found", audio_path.display()));
    }
    // Rebuilt so the written record keeps a deterministic key order.
    let ordered: Map<String, Value> = record
        .clone()
        .into_iter()
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .collect();
    record = ordered;
    Ok(Prepared { record, audio_path })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(path: &Path, speech: bool) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        let mut state: u32 = 5;
        for i in 0..32_000 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let room = ((state >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0) * 0.0004;
            let t = i as f32 / 16_000.0;
            let value = if speech && (8_000..24_000).contains(&i) {
                room + (t * 2.0 * std::f32::consts::PI * 150.0).sin() * 0.05
            } else {
                room
            };
            writer
                .write_sample((value.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .unwrap();
        }
        writer.finalize().unwrap();
    }

    const CONSENT: &str = r#"{"granted":true,"statement_id":"consent-ru-v1","date":"2026-09-05"}"#;

    fn manifest_line(overrides: &str, audio: &str) -> String {
        format!(
            r#"{{"id":"voice-001-a","family_id":"voice-001","provenance":"real_audio","human_transcript":"вода","target_domain":"chemistry","target_action":"ast","target_ast":{{"Chemical":{{"Species":{{"coefficient":1,"formula":{{"parts":[{{"Atom":{{"symbol":"H","count":2}}}},{{"Atom":{{"symbol":"O","count":1}}}}]}},"charge":null,"marker":null}}}}}},"split":"train","tags":["voice"],"speaker_id":"spk01","audio":{audio}{overrides}}}"#
        )
    }

    fn setup(
        speech: bool,
        audio_json: &str,
        overrides: &str,
    ) -> (tempfile::TempDir, IngestOptions) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("audio")).unwrap();
        wav(&dir.path().join("audio/spk01-0001.wav"), speech);
        let manifest = dir.path().join("in.jsonl");
        std::fs::write(&manifest, manifest_line(overrides, audio_json)).unwrap();
        let output = dir.path().join("out.jsonl");
        let options = IngestOptions {
            manifest,
            output,
            describe_only: false,
        };
        (dir, options)
    }

    fn full_audio() -> String {
        format!(r#"{{"file":"audio/spk01-0001.wav","consent":{CONSENT}}}"#)
    }

    fn read_out(options: &IngestOptions) -> Map<String, Value> {
        let text = std::fs::read_to_string(&options.output).unwrap();
        match serde_json::from_str(text.lines().next().unwrap()).unwrap() {
            Value::Object(map) => map,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_recording_becomes_a_schema_two_record() {
        let (_dir, options) = setup(true, &full_audio(), "");
        run(options_ref(&options), &mut |_| Ok("вода".into())).unwrap();
        let record = read_out(&options);
        assert_eq!(record["dataset_schema_version"], serde_json::json!(2));
        let audio = record["audio"].as_object().unwrap();
        assert_eq!(audio["sample_rate_hz"], serde_json::json!(16_000));
        assert_eq!(audio["channels"], serde_json::json!(1));
        assert_eq!(audio["sha256"].as_str().unwrap().len(), 64);
        assert!((audio["duration_secs"].as_f64().unwrap() - 2.0).abs() < 0.01);
        assert!(audio["snr_db"].as_f64().unwrap() > 20.0);
        assert_eq!(
            record["asr_hypotheses"],
            serde_json::json!([{"text": "вода"}])
        );
        // Untouched fields survive verbatim.
        assert_eq!(record["human_transcript"], serde_json::json!("вода"));
        assert_eq!(record["speaker_id"], serde_json::json!("spk01"));
    }

    /// The measured facts must come from the file, so a manifest that
    /// pre-declares them cannot smuggle in numbers nobody measured.
    #[test]
    fn declared_audio_facts_are_overwritten_by_the_measured_ones() {
        let lying = format!(
            r#"{{"file":"audio/spk01-0001.wav","sha256":"{}","duration_secs":99.0,"sample_rate_hz":48000,"channels":2,"snr_db":80.0,"consent":{CONSENT}}}"#,
            "f".repeat(64)
        );
        let (_dir, options) = setup(true, &lying, "");
        run(options_ref(&options), &mut |_| Ok("вода".into())).unwrap();
        let audio = read_out(&options)["audio"].as_object().unwrap().clone();
        assert_ne!(audio["sha256"].as_str().unwrap(), "f".repeat(64));
        assert_eq!(audio["sample_rate_hz"], serde_json::json!(16_000));
        assert_eq!(audio["channels"], serde_json::json!(1));
        assert!(audio["duration_secs"].as_f64().unwrap() < 3.0);
        assert!(audio["snr_db"].as_f64().unwrap() < 80.0);
    }

    #[test]
    fn a_silent_take_gets_no_signal_to_noise_number() {
        let (_dir, options) = setup(false, &full_audio(), "");
        run(options_ref(&options), &mut |_| Ok(String::new())).unwrap();
        let record = read_out(&options);
        assert!(!record["audio"].as_object().unwrap().contains_key("snr_db"));
        assert_eq!(record["asr_hypotheses"], serde_json::json!([]));
    }

    #[test]
    fn a_recording_without_consent_is_refused_before_any_audio_is_read() {
        let no_consent = r#"{"file":"audio/spk01-0001.wav"}"#;
        let (_dir, options) = setup(true, no_consent, "");
        let mut called = false;
        let error = run(options_ref(&options), &mut |_| {
            called = true;
            Ok("вода".into())
        })
        .unwrap_err();
        assert!(error.contains("consent"), "{error}");
        assert!(
            !called,
            "nothing may be transcribed before consent is checked"
        );
        assert!(!options.output.exists(), "no output on a refused manifest");
    }

    #[test]
    fn a_manifest_pointing_outside_its_directory_is_refused() {
        let escaping = format!(r#"{{"file":"../../etc/passwd","consent":{CONSENT}}}"#);
        let (_dir, options) = setup(true, &escaping, "");
        let error = run(options_ref(&options), &mut |_| Ok(String::new())).unwrap_err();
        assert!(error.contains("stay inside"), "{error}");
    }

    #[test]
    fn text_provenance_is_refused() {
        let (_dir, options) = setup(true, &full_audio(), r#","x":1"#);
        let manifest = std::fs::read_to_string(&options.manifest)
            .unwrap()
            .replace("real_audio", "handcrafted_text");
        std::fs::write(&options.manifest, manifest).unwrap();
        let error = run(options_ref(&options), &mut |_| Ok(String::new())).unwrap_err();
        assert!(error.contains("has no audio"), "{error}");
    }

    #[test]
    fn describe_only_measures_without_transcribing() {
        let (_dir, mut options) = setup(true, &full_audio(), "");
        options.describe_only = true;
        let mut called = false;
        run(options_ref(&options), &mut |_| {
            called = true;
            Ok("вода".into())
        })
        .unwrap();
        assert!(!called);
        let record = read_out(&options);
        assert!(
            record["audio"].as_object().unwrap()["snr_db"]
                .as_f64()
                .unwrap()
                > 20.0
        );
        assert!(record.get("asr_hypotheses").is_none());
    }

    fn options_ref(options: &IngestOptions) -> IngestOptions {
        IngestOptions {
            manifest: options.manifest.clone(),
            output: options.output.clone(),
            describe_only: options.describe_only,
        }
    }
}

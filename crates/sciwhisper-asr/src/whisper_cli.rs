//! Local Whisper CLI adapter.
//!
//! Prefers the installed `whisper` (openai-whisper). If `whisper-cli` /
//! whisper.cpp is on PATH, uses that instead. The scientific parser never
//! reads Whisper's string as a formula.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::engine::{AsrEngine, EngineInfo, EngineKind, Segment, TranscribeOptions, Transcript};
use crate::error::{Error, Result};

pub struct WhisperCliEngine {
    pub info: EngineInfo,
}

impl WhisperCliEngine {
    pub fn discover(model: Option<&str>) -> Result<Self> {
        let binary = find_whisper_binary()?;
        let kind = detect_kind(&binary);
        let model = model.map(|s| s.to_string()).unwrap_or_else(default_model);
        Ok(Self {
            info: EngineInfo {
                kind,
                binary,
                model,
            },
        })
    }

    pub fn with_binary(binary: PathBuf, kind: EngineKind, model: String) -> Self {
        Self {
            info: EngineInfo {
                kind,
                binary,
                model,
            },
        }
    }
}

impl AsrEngine for WhisperCliEngine {
    fn transcribe(&mut self, audio: &Path, opts: &TranscribeOptions) -> Result<Transcript> {
        match self.info.kind {
            EngineKind::OpenaiWhisper => transcribe_openai(self, audio, opts),
            EngineKind::WhisperCpp => transcribe_cpp(self, audio, opts),
        }
    }
}

fn transcribe_openai(
    eng: &WhisperCliEngine,
    audio: &Path,
    opts: &TranscribeOptions,
) -> Result<Transcript> {
    ensure_openai_model_cached(&eng.info.model)?;
    let tmp = tempfile::tempdir().map_err(|e| Error::Message(e.to_string()))?;
    let stem = audio
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("input");
    let mut cmd = Command::new(&eng.info.binary);
    cmd.arg(audio)
        .arg("--model")
        .arg(&eng.info.model)
        .arg("--language")
        .arg(&opts.language)
        .arg("--task")
        .arg("transcribe")
        .arg("--output_format")
        .arg("json")
        .arg("--output_dir")
        .arg(tmp.path())
        .arg("--verbose")
        .arg("False")
        .arg("--temperature")
        .arg(opts.temperature.to_string())
        .arg("--condition_on_previous_text")
        .arg("False")
        .arg("--fp16")
        .arg("False");
    if !opts.initial_prompt.is_empty() {
        cmd.arg("--initial_prompt").arg(&opts.initial_prompt);
    }
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(Error::Whisper(String::from_utf8_lossy(&out.stderr).into()));
    }
    let json_path = tmp.path().join(format!("{stem}.json"));
    if !json_path.exists() {
        // openai-whisper names the json after the input file stem
        let fallback = fs::read_dir(tmp.path())?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
            .ok_or_else(|| {
                Error::Whisper("whisper produced no json (is the model cached?)".into())
            })?;
        return parse_openai_json(&fs::read_to_string(fallback)?);
    }
    parse_openai_json(&fs::read_to_string(json_path)?)
}

fn transcribe_cpp(
    eng: &WhisperCliEngine,
    audio: &Path,
    opts: &TranscribeOptions,
) -> Result<Transcript> {
    let tmp = tempfile::tempdir().map_err(|e| Error::Message(e.to_string()))?;
    let out_base = tmp.path().join("out");
    let model = resolve_cpp_model(&eng.info.model)?;
    let mut cmd = Command::new(&eng.info.binary);
    cmd.arg("-m")
        .arg(model)
        .arg("-f")
        .arg(audio)
        .arg("-l")
        .arg(&opts.language)
        .arg("-otxt")
        .arg("-of")
        .arg(&out_base)
        .arg("-nt")
        .arg("-np");
    if !opts.initial_prompt.is_empty() {
        cmd.arg("--prompt").arg(&opts.initial_prompt);
    }
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(Error::Whisper(String::from_utf8_lossy(&out.stderr).into()));
    }
    let txt = out_base.with_extension("txt");
    let text = if txt.exists() {
        fs::read_to_string(txt)?
    } else {
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let text = text.trim().to_string();
    Ok(Transcript {
        no_speech: text.is_empty(),
        text,
        language: Some(opts.language.clone()),
        segments: vec![],
    })
}

#[derive(Deserialize)]
struct OpenaiJson {
    #[serde(default)]
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    segments: Vec<OpenaiSegment>,
}

#[derive(Deserialize, Default)]
struct OpenaiSegment {
    #[serde(default)]
    text: String,
    start: Option<f32>,
    end: Option<f32>,
    no_speech_prob: Option<f32>,
    avg_logprob: Option<f32>,
}

pub fn parse_openai_json(s: &str) -> Result<Transcript> {
    let parsed: OpenaiJson = serde_json::from_str(s)?;
    let segments: Vec<Segment> = parsed
        .segments
        .iter()
        .map(|g| Segment {
            text: g.text.clone(),
            start: g.start,
            end: g.end,
            no_speech_prob: g.no_speech_prob,
            avg_logprob: g.avg_logprob,
        })
        .collect();
    let text = parsed.text.trim().to_string();
    let no_speech = text.is_empty()
        || (!segments.is_empty()
            && segments
                .iter()
                .all(|g| g.no_speech_prob.unwrap_or(0.0) > 0.6)
            && text.chars().count() < 3);
    Ok(Transcript {
        text,
        language: parsed.language,
        segments,
        no_speech,
    })
}

pub fn find_whisper_binary() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("SCIWHISPER_WHISPER") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Ok(pb);
        }
    }
    for name in ["whisper-cli", "whisper-cpp"] {
        if let Some(p) = search_path(name) {
            return Ok(p);
        }
    }
    if let Some(p) = search_path("whisper") {
        return Ok(p);
    }
    Err(Error::WhisperNotFound)
}

fn detect_kind(bin: &Path) -> EngineKind {
    let name = bin
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.contains("cli") || name.contains("cpp") || looks_like_whisper_cpp(bin) {
        EngineKind::WhisperCpp
    } else {
        EngineKind::OpenaiWhisper
    }
}

fn looks_like_whisper_cpp(bin: &Path) -> bool {
    let out = Command::new(bin).arg("-h").output();
    match out {
        Ok(o) => {
            let s = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            s.contains("whisper.cpp") || s.contains("-m MODEL") || s.contains("--model FNAME")
        }
        Err(_) => false,
    }
}

fn search_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn default_model() -> String {
    let cache = whisper_cache();
    default_model_in(&cache)
}

fn default_model_in(cache: &Path) -> String {
    if cache.join("large-v3-turbo.pt").exists() || cache.join("turbo.pt").exists() {
        return "turbo".into();
    }
    if cache.join("small.pt").exists() {
        return "small".into();
    }
    if cache.join("base.pt").exists() {
        return "base".into();
    }
    "base".into()
}

fn ensure_openai_model_cached(model: &str) -> Result<PathBuf> {
    let cache = whisper_cache();
    let file_names: Vec<String> = match model {
        "turbo" => vec!["large-v3-turbo.pt".into(), "turbo.pt".into()],
        "large" => vec!["large-v3.pt".into(), "large.pt".into()],
        name => vec![format!("{name}.pt")],
    };
    for name in file_names {
        let path = cache.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(Error::LocalModelMissing {
        model: model.to_string(),
        cache: cache.display().to_string(),
    })
}

pub fn whisper_cache() -> PathBuf {
    dirs_home()
        .map(|h| h.join(".cache/whisper"))
        .unwrap_or_else(|| PathBuf::from(".cache/whisper"))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn resolve_cpp_model(name: &str) -> Result<PathBuf> {
    let p = PathBuf::from(name);
    if p.exists() {
        return Ok(p);
    }
    let cache = whisper_cache();
    for cand in [
        cache.join(name),
        cache.join(format!("ggml-{name}.bin")),
        cache.join(format!("ggml-{name}-q5_1.bin")),
        PathBuf::from("models").join(format!("ggml-{name}.bin")),
    ] {
        if cand.exists() {
            return Ok(cand);
        }
    }
    Err(Error::Message(format!(
        "whisper.cpp model '{name}' not found; pass a ggml .bin path via --model"
    )))
}

pub fn doctor() -> String {
    let mut lines = Vec::new();
    let mut detected_kind = None;
    match find_whisper_binary() {
        Ok(p) => {
            let kind = detect_kind(&p);
            lines.push(format!("whisper binary: {}", p.display()));
            lines.push(format!("backend: {kind:?}"));
            detected_kind = Some(kind);
        }
        Err(_) => lines.push("whisper binary: NOT FOUND".into()),
    }
    let cache = whisper_cache();
    lines.push(format!("model cache: {}", cache.display()));
    if cache.is_dir() {
        if let Ok(rd) = fs::read_dir(&cache) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.ends_with(".pt") || n.ends_with(".bin") || n.ends_with(".ggml") {
                    let sz = e.metadata().map(|m| m.len()).unwrap_or(0);
                    lines.push(format!("  {n} ({:.1} MiB)", sz as f64 / 1048576.0));
                }
            }
        }
    }
    let default = default_model();
    lines.push(format!("default model: {default}"));
    let model_ready = match detected_kind {
        Some(EngineKind::WhisperCpp) => resolve_cpp_model(&default).is_ok(),
        Some(EngineKind::OpenaiWhisper) => ensure_openai_model_cached(&default).is_ok(),
        None => false,
    };
    lines.push(format!(
        "default model cached: {}",
        if model_ready { "yes" } else { "no" }
    ));
    lines.push("runtime network policy: disabled (missing models are rejected)".into());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_json_transcript() {
        let raw = r#"{
            "text": " гидроксид меди два",
            "language": "ru",
            "segments": [
                {
                    "text": " гидроксид меди два",
                    "start": 0.0,
                    "end": 1.8,
                    "no_speech_prob": 0.01,
                    "avg_logprob": -0.2
                }
            ]
        }"#;
        let t = parse_openai_json(raw).unwrap();
        assert_eq!(t.text, "гидроксид меди два");
        assert_eq!(t.language.as_deref(), Some("ru"));
        assert!(!t.no_speech);
    }

    #[test]
    fn best_cached_default_prefers_turbo_over_base() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("base.pt"), b"").unwrap();
        fs::write(dir.path().join("large-v3-turbo.pt"), b"").unwrap();
        assert_eq!(default_model_in(dir.path()), "turbo");
    }

    #[test]
    fn silence_json_is_no_speech() {
        let raw = r#"{
            "text": "",
            "segments": [{"text": "", "no_speech_prob": 0.95}]
        }"#;
        let t = parse_openai_json(raw).unwrap();
        assert!(t.no_speech);
    }

    #[test]
    fn missing_openai_model_is_rejected_before_launch() {
        let err = ensure_openai_model_cached("sciwhisper-model-that-does-not-exist")
            .expect_err("unknown model must not trigger a download");
        assert!(matches!(err, Error::LocalModelMissing { .. }));
    }
}

//! Local Whisper CLI adapter.
//!
//! The backend is chosen by [`crate::backend`]: an explicitly configured path,
//! then the copy shipped inside the bundle, then a local `whisper.cpp`, and
//! only outside a bundle a Python `openai-whisper`. The scientific parser
//! never reads Whisper's string as a formula.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::backend::{display_name, Backend, BackendOrigin, Layout};
use crate::engine::{AsrEngine, EngineInfo, EngineKind, Segment, TranscribeOptions, Transcript};
use crate::error::{Error, Result};
use crate::model;
use crate::process::{self, Limits};

pub struct WhisperCliEngine {
    pub info: EngineInfo,
    pub origin: BackendOrigin,
    layout: Option<Layout>,
    limits: Limits,
}

impl WhisperCliEngine {
    pub fn discover(model: Option<&str>) -> Result<Self> {
        Self::discover_with(None, model)
    }

    /// `explicit` is a backend path the user configured.
    pub fn discover_with(explicit: Option<&Path>, model: Option<&str>) -> Result<Self> {
        let backend = crate::backend::discover(explicit)?;
        Ok(Self::from_backend(backend, model))
    }

    pub fn from_backend(backend: Backend, model: Option<&str>) -> Self {
        let model = model
            .map(|s| s.to_string())
            .unwrap_or_else(|| match backend.kind {
                // whisper.cpp loads a file, and the bundle names it in its
                // manifest; there is no notion of a "default model name" to guess.
                EngineKind::WhisperCpp => String::new(),
                EngineKind::OpenaiWhisper => default_model(),
            });
        Self {
            info: EngineInfo {
                kind: backend.kind,
                binary: backend.binary,
                model,
            },
            origin: backend.origin,
            layout: Layout::detect(),
            limits: Limits::default(),
        }
    }

    pub fn with_binary(binary: PathBuf, kind: EngineKind, model: String) -> Self {
        Self {
            info: EngineInfo {
                kind,
                binary,
                model,
            },
            origin: BackendOrigin::Configured,
            layout: Layout::detect(),
            limits: Limits::default(),
        }
    }

    /// Overrides where the model is looked for. Used by tests and by callers
    /// that know the layout better than `current_exe` does.
    pub fn with_layout(mut self, layout: Option<Layout>) -> Self {
        self.layout = layout;
        self
    }

    /// Overrides the deadline and output ceiling for this engine. The default
    /// comes from [`Limits::default`], which honours `SCIWHISPER_TIMEOUT_SECS`.
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
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
    let out = process::run(cmd, eng.limits)?;
    if !out.success {
        return Err(Error::BackendFailed {
            code: out.code,
            detail: out.tail(3),
        });
    }
    let json_path = tmp.path().join(format!("{stem}.json"));
    if !json_path.exists() {
        // openai-whisper names the json after the input file stem
        let fallback = fs::read_dir(tmp.path())?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
            .ok_or_else(|| Error::OutputUnreadable {
                detail: "движок не создал файл с расшифровкой".into(),
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
    let model = resolve_cpp_model(eng, &eng.info.model)?;
    let mut cmd = Command::new(&eng.info.binary);
    // Every value goes in as its own argument, so spaces and Cyrillic in the
    // model or audio path survive untouched.
    cmd.arg("-m")
        .arg(&model)
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
    let out = process::run(cmd, eng.limits)?;
    if !out.success {
        // The backend ran and refused. Its own last words are more useful to
        // the user than anything this layer could invent.
        return Err(Error::BackendFailed {
            code: out.code,
            detail: out.tail(3),
        });
    }
    let txt = out_base.with_extension("txt");
    let text = if txt.exists() {
        fs::read_to_string(&txt).map_err(|error| Error::OutputUnreadable {
            detail: error.to_string(),
        })?
    } else if !out.stdout.trim().is_empty() {
        out.stdout.clone()
    } else {
        return Err(Error::OutputUnreadable {
            detail: "движок завершился успешно, но не выдал текста".into(),
        });
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

/// Kept for callers that only need the path. New code should use
/// [`crate::backend::discover`], which also reports where the backend came
/// from.
pub fn find_whisper_binary() -> Result<PathBuf> {
    crate::backend::discover(configured_backend().as_deref()).map(|backend| backend.binary)
}

/// A backend path the user set explicitly.
pub fn configured_backend() -> Option<PathBuf> {
    std::env::var_os("SCIWHISPER_WHISPER").map(PathBuf::from)
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

/// Finds the ggml model whisper.cpp should load.
///
/// A name the caller supplied is honoured or refused; it is never quietly
/// replaced by the bundled model, because a run against a different model than
/// the one that was asked for is worse than a clear failure.
fn resolve_cpp_model(eng: &WhisperCliEngine, name: &str) -> Result<PathBuf> {
    let requirements = model::Requirements::for_layout(eng.layout.as_ref(), false);
    if name.is_empty() {
        return model::resolve(eng.layout.as_ref(), None, requirements);
    }
    let asked = PathBuf::from(name);
    if asked.is_file() {
        return Ok(asked);
    }
    Err(Error::ModelUnusable {
        detail: format!(
            "модель «{name}» не найдена. Для whisper.cpp нужен путь к файлу .bin; \
             уберите эту настройку, чтобы использовать модель из комплекта."
        ),
    })
}

/// A readiness report a user can paste into a bug report.
///
/// It deliberately carries no absolute paths: a file name says everything
/// needed to diagnose a broken bundle, while a full path would publish
/// somebody's user name and folder layout.
#[derive(Clone, Debug)]
pub struct DoctorReport {
    /// File name, where it came from, and what kind of engine it actually is.
    pub backend: std::result::Result<(String, BackendOrigin, EngineKind), String>,
    pub model: String,
    pub model_ready: bool,
    pub microphones: Vec<String>,
    pub word_integration: String,
    pub bundle: String,
}

impl DoctorReport {
    pub fn collect(verify_model_checksum: bool) -> Self {
        let layout = Layout::detect();
        let bundle = match &layout {
            Some(layout) if layout.is_packaged_bundle() => {
                "официальный комплект sci-witch".to_string()
            }
            Some(_) => "рабочее дерево разработчика".to_string(),
            None => "не удалось определить папку программы".to_string(),
        };
        let backend = match crate::backend::discover(configured_backend().as_deref()) {
            Ok(found) => Ok((display_name(&found.binary), found.origin, found.kind)),
            Err(error) => Err(error.to_string()),
        };
        let uses_cpp = matches!(&backend, Ok((_, _, EngineKind::WhisperCpp)));
        let (model, model_ready) = if uses_cpp {
            // Exactly the call the recogniser makes, so a green report cannot
            // disagree with the next recording.
            let status = model::inspect(
                layout.as_ref(),
                None,
                model::Requirements::for_layout(layout.as_ref(), verify_model_checksum),
            );
            (status.message(), status.is_ready())
        } else {
            let name = default_model();
            let ready = ensure_openai_model_cached(&name).is_ok();
            (
                format!(
                    "режим разработчика: модель Python Whisper «{name}» {}",
                    if ready {
                        "найдена"
                    } else {
                        "не найдена"
                    }
                ),
                ready,
            )
        };
        DoctorReport {
            backend,
            model,
            model_ready,
            microphones: crate::capture::input_devices(),
            word_integration: word_integration_status(),
            bundle,
        }
    }

    pub fn render(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("комплект: {}", self.bundle));
        match &self.backend {
            Ok((name, origin, kind)) => {
                lines.push(format!("движок распознавания: {name}"));
                lines.push(format!("источник движка: {}", origin.as_str()));
                lines.push(format!(
                    "тип движка: {}",
                    match kind {
                        EngineKind::WhisperCpp => "whisper.cpp",
                        EngineKind::OpenaiWhisper => "Python openai-whisper",
                    }
                ));
            }
            Err(error) => {
                lines.push("движок распознавания: НЕ НАЙДЕН".into());
                lines.push(format!("  причина: {error}"));
            }
        }
        lines.push(format!("модель: {}", self.model));
        lines.push(format!(
            "модель готова: {}",
            if self.model_ready { "да" } else { "нет" }
        ));
        if self.microphones.is_empty() {
            lines.push("микрофон: не найден".into());
        } else {
            lines.push("микрофон: готов".into());
            for name in &self.microphones {
                lines.push(format!("  {name}"));
            }
        }
        lines.push(format!("интеграция с Word: {}", self.word_integration));
        lines.push("сеть: не используется — ни модели, ни движки не скачиваются".into());
        lines.join("\n")
    }
}

fn word_integration_status() -> String {
    if cfg!(windows) {
        // Whether Word is actually installed and answers COM is something only
        // a real insertion can show; claiming more here would be a guess.
        "Windows: вставка уравнения проверяется командой SciWhisper-Test.cmd".into()
    } else {
        "недоступна: нативные уравнения Word вставляются только в Windows".into()
    }
}

pub fn doctor() -> String {
    DoctorReport::collect(false).render()
}

/// The same report, with the model file hashed in full.
pub fn doctor_verified() -> String {
    DoctorReport::collect(true).render()
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

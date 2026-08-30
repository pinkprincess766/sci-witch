//! Warm Whisper process: the model is loaded once and kept in memory.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::engine::{AsrEngine, Segment, TranscribeOptions, Transcript};
use crate::error::{Error, Result};
use crate::whisper_cli::{default_model, find_whisper_binary, whisper_cache};

const SCRIPT: &str = include_str!("../../../scripts/whisperd.py");

pub struct WarmWhisper {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    pub model: String,
}

impl WarmWhisper {
    pub fn spawn(model: Option<&str>) -> Result<Self> {
        let model = model.unwrap_or(&default_model()).to_string();
        let python = python_with_whisper()?;
        let script_path = write_script()?;
        let mut child = Command::new(&python)
            .arg(&script_path)
            .env("SCIWHISPER_MODEL", &model)
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::Message(format!("failed to start whisperd: {e}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Message("whisperd stdin missing".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Message("whisperd stdout missing".into()))?;
        let mut stdout = BufReader::new(stdout);
        // Wait until ready (or fail).
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        loop {
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                return Err(Error::Message(
                    "whisperd timed out loading the model".into(),
                ));
            }
            let mut line = String::new();
            stdout.read_line(&mut line)?;
            if line.is_empty() {
                return Err(Error::Message("whisperd exited while loading".into()));
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                if v.get("event").and_then(|e| e.as_str()) == Some("ready") {
                    break;
                }
                if v.get("ok").and_then(|o| o.as_bool()) == Some(false) {
                    let msg = v
                        .get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or("whisperd failed");
                    return Err(Error::Message(msg.into()));
                }
            }
        }
        Ok(Self {
            child,
            stdin,
            stdout,
            model,
        })
    }

    fn request(&mut self, req: serde_json::Value) -> Result<DaemonMsg> {
        writeln!(self.stdin, "{req}")?;
        self.stdin.flush()?;
        let mut line = String::new();
        self.stdout.read_line(&mut line)?;
        if line.is_empty() {
            return Err(Error::Message("whisperd closed the pipe".into()));
        }
        let msg: DaemonMsg = serde_json::from_str(line.trim())?;
        if !msg.ok {
            return Err(Error::Whisper(
                msg.error.unwrap_or_else(|| "whisperd error".into()),
            ));
        }
        Ok(msg)
    }
}

impl AsrEngine for WarmWhisper {
    fn transcribe(&mut self, audio: &Path, opts: &TranscribeOptions) -> Result<Transcript> {
        let msg = self.request(json!({
            "cmd": "transcribe",
            "path": audio.to_string_lossy(),
            "language": opts.language,
            "prompt": opts.initial_prompt,
        }))?;
        let text = msg.text.unwrap_or_default().trim().to_string();
        Ok(Transcript {
            no_speech: msg.no_speech.unwrap_or(text.is_empty()),
            language: msg.language,
            segments: msg
                .segments
                .into_iter()
                .map(|s| Segment {
                    text: s.text,
                    start: s.start,
                    end: s.end,
                    no_speech_prob: s.no_speech_prob,
                    avg_logprob: s.avg_logprob,
                })
                .collect(),
            text,
        })
    }
}

impl Drop for WarmWhisper {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, r#"{{"cmd":"quit"}}"#);
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Mutex wrapper so the tray can share one warm engine.
pub struct SharedEngine {
    inner: Mutex<WarmWhisper>,
}

impl SharedEngine {
    pub fn spawn(model: Option<&str>) -> Result<Self> {
        Ok(Self {
            inner: Mutex::new(WarmWhisper::spawn(model)?),
        })
    }

    pub fn transcribe(&self, audio: &Path, opts: &TranscribeOptions) -> Result<Transcript> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| Error::Message("whisper engine poisoned".into()))?;
        g.transcribe(audio, opts)
    }
}

#[derive(Deserialize)]
struct DaemonMsg {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    no_speech: Option<bool>,
    #[serde(default)]
    segments: Vec<DaemonSeg>,
}

#[derive(Deserialize, Default)]
struct DaemonSeg {
    #[serde(default)]
    text: String,
    start: Option<f32>,
    end: Option<f32>,
    no_speech_prob: Option<f32>,
    avg_logprob: Option<f32>,
}

fn write_script() -> Result<PathBuf> {
    let dir = tempfile::Builder::new()
        .prefix("sciwhisper-daemon-")
        .tempdir()
        .map_err(|e| Error::Message(e.to_string()))?;
    let path = dir.path().join("whisperd.py");
    std::fs::write(&path, SCRIPT)?;
    // Keep dir alive for process lifetime by leaking it.
    std::mem::forget(dir);
    Ok(path)
}

fn python_with_whisper() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("SCIWHISPER_PYTHON") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Ok(pb);
        }
    }
    if let Ok(bin) = find_whisper_binary() {
        if let Ok(s) = std::fs::read_to_string(&bin) {
            if let Some(line) = s.lines().next() {
                if let Some(rest) = line.strip_prefix("#!") {
                    let path = rest.split_whitespace().next().unwrap_or("");
                    let pb = PathBuf::from(path);
                    if pb.exists() {
                        return Ok(pb);
                    }
                }
            }
        }
    }
    for name in ["python3", "python"] {
        if let Some(p) = search_path(name) {
            let ok = Command::new(&p)
                .args(["-c", "import whisper"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                return Ok(p);
            }
        }
    }
    let _ = whisper_cache();
    Err(Error::Message(
        "no Python with the whisper package; set SCIWHISPER_PYTHON".into(),
    ))
}

fn search_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|c| c.is_file())
}

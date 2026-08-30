//! ASR is a replaceable adapter. Whisper is never the source of scientific truth.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Clone, Debug)]
pub struct TranscribeOptions {
    pub language: String,
    pub model: String,
    pub initial_prompt: String,
    pub temperature: f32,
}

impl Default for TranscribeOptions {
    fn default() -> Self {
        Self {
            language: "ru".into(),
            model: "base".into(),
            initial_prompt: crate::prompt::combined(),
            temperature: 0.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Transcript {
    pub text: String,
    pub language: Option<String>,
    pub segments: Vec<Segment>,
    pub no_speech: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub text: String,
    pub start: Option<f32>,
    pub end: Option<f32>,
    pub no_speech_prob: Option<f32>,
    pub avg_logprob: Option<f32>,
}

pub trait AsrEngine {
    fn transcribe(&mut self, audio: &Path, opts: &TranscribeOptions) -> Result<Transcript>;
}

#[derive(Clone, Debug)]
pub struct EngineInfo {
    pub kind: EngineKind,
    pub binary: PathBuf,
    pub model: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineKind {
    OpenaiWhisper,
    WhisperCpp,
}

/// In-memory engine for tests. Never calls Whisper.
pub struct FakeEngine {
    pub transcript: Transcript,
}

impl AsrEngine for FakeEngine {
    fn transcribe(&mut self, _audio: &Path, _opts: &TranscribeOptions) -> Result<Transcript> {
        Ok(self.transcript.clone())
    }
}

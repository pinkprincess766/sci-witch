//! Replaceable Whisper ASR layer. SciWhisper is an overlay: Whisper transcribes,
//! the core crate compiles scientific notation.

pub mod capture;
pub mod engine;
pub mod error;
pub mod pipeline;
pub mod prompt;
pub mod whisper_cli;
pub mod whisperd;

pub use capture::PttSession;
pub use engine::{AsrEngine, EngineKind, FakeEngine, TranscribeOptions, Transcript};
pub use error::{Error, Result};
pub use pipeline::{
    compile_transcript, from_audio, from_microphone, PipelineOptions, PipelineResult,
};
pub use whisper_cli::{doctor, WhisperCliEngine};
pub use whisperd::{SharedEngine, WarmWhisper};

//! Replaceable Whisper ASR layer. SciWhisper is an overlay: Whisper transcribes,
//! the core crate compiles scientific notation.

pub mod backend;
pub mod capture;
pub mod corpus;
pub mod engine;
pub mod error;
pub mod model;
pub mod pipeline;
pub mod process;
pub mod prompt;
pub mod vad;
pub mod whisper_cli;
pub mod whisperd;

pub use backend::{Backend, BackendOrigin, Layout};
pub use capture::PttSession;
pub use engine::{AsrEngine, EngineKind, FakeEngine, TranscribeOptions, Transcript};
pub use error::{Error, Result};
pub use model::{ModelManifest, ModelStatus};
pub use pipeline::{
    compile_transcript, from_audio, from_microphone, PipelineOptions, PipelineResult,
};
pub use whisper_cli::{doctor, WhisperCliEngine};
pub use whisperd::{SharedEngine, WarmWhisper};

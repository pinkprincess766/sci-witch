use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error("whisper binary not found (install openai-whisper or whisper.cpp)")]
    WhisperNotFound,
    #[error("whisper failed: {0}")]
    Whisper(String),
    #[error("local Whisper model '{model}' not found in {cache}; SciWhisper will not download it automatically")]
    LocalModelMissing { model: String, cache: String },
    #[error("no microphone input device")]
    NoMicrophone,
    #[error("audio error: {0}")]
    Audio(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Message(s)
    }
}

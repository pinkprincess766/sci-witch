use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("asr: {0}")]
    Asr(#[from] sciwhisper_asr::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

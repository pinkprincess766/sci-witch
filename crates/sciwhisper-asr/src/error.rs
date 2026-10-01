use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error("whisper binary not found (install openai-whisper or whisper.cpp)")]
    WhisperNotFound,
    /// Nothing usable was found anywhere in the discovery order.
    #[error("не найден движок распознавания ({looked_for}): {detail}. Переустановите комплект sci-witch целиком.")]
    BackendMissing { looked_for: String, detail: String },
    /// A shipped bundle is missing one of its own files.
    #[error("комплект sci-witch неполный: отсутствует {missing}. Распакуйте официальный архив целиком, не выборочно.")]
    BundleIncomplete { missing: String },
    /// The model is absent, truncated or does not match its manifest.
    #[error("{detail}")]
    ModelUnusable { detail: String },
    #[error("файл MODEL_MANIFEST.json повреждён: {reason}")]
    ModelManifestInvalid { reason: String },
    /// The recogniser started and exited with a failure.
    #[error("движок распознавания завершился с ошибкой{}: {detail}", .code.map(|c| format!(" (код {c})")).unwrap_or_default())]
    BackendFailed { code: Option<i32>, detail: String },
    /// The recogniser was still running when the deadline passed.
    #[error("движок распознавания не ответил за {seconds} с и был остановлен. Попробуйте более короткую запись или более лёгкую модель.")]
    BackendTimedOut { seconds: u64 },
    /// The process finished, but its output could not be read as a transcript.
    #[error("не удалось разобрать ответ движка распознавания: {detail}")]
    OutputUnreadable { detail: String },
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

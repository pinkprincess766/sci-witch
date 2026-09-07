use std::fmt;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// The archive asked for something an archive is not allowed to ask for.
    /// Carries the entry name exactly as it appeared, so a refusal can be
    /// reported without paraphrasing what was in the file.
    Rejected {
        entry: String,
        reason: String,
    },
    /// A limit was exceeded. Separate from `Rejected` because this is about
    /// size, not intent, and the two need different messages.
    TooLarge(String),
    /// What came out of the archive is not a SciWhisper installation.
    NotABundle(String),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Message(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Rejected { entry, reason } => {
                write!(f, "архив отклонён: запись {entry:?} — {reason}")
            }
            Error::TooLarge(what) => write!(f, "архив превышает предел: {what}"),
            Error::NotABundle(what) => {
                write!(f, "распакованное не похоже на комплект SciWhisper: {what}")
            }
            Error::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Error::Message(text) => f.write_str(text),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub(crate) fn io(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.into();
    move |source| Error::Io { path, source }
}

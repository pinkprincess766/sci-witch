use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("lexicon schema {found} is newer than supported {supported}")]
    UnsupportedSchema { found: u32, supported: u32 },
    #[error("failed to load lexicon {name}: {source}")]
    Lexicon {
        name: &'static str,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("invalid chemical formula '{0}'")]
    InvalidFormula(String),
    #[error("cannot parse spoken input as {domain}: {reason}")]
    Parse {
        domain: &'static str,
        reason: String,
    },
    #[error("unresolved spoken span: {0}")]
    Unresolved(String),
}

pub type Result<T> = std::result::Result<T, Error>;

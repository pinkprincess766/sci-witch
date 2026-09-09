//! A local record of the times the user disagreed with the answer.
//!
//! Every corrected utterance is a labelled example that nobody had to
//! collect on purpose: the words that were heard, what the system inserted,
//! and what the person actually wanted. That is exactly the shape the
//! research corpus needs, and it is the only source of it that does not
//! require organising a recording session.
//!
//! # Three properties this file is built around
//!
//! **Off by default.** This writes what the user dictated to disk. That is
//! their speech, and the project's whole promise is that it stays theirs.
//! `remember_corrections` starts `false`, exactly like `persist_history`,
//! and the tray says what turning it on means.
//!
//! **Local and inert.** An append-only JSONL file beside the configuration.
//! Nothing reads it at runtime, nothing uploads it, and no behaviour depends
//! on it. It is material for a later, deliberate step — not a feedback loop
//! that quietly changes what the program does.
//!
//! **Bounded.** A dictation tool left running for months must not fill a
//! disk. The log stops accepting entries at [`MAX_ENTRIES`] rather than
//! rotating: silently discarding the oldest examples would bias whatever is
//! eventually learned from them towards recent use.

use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many corrections one file will hold.
///
/// A person dictating all day corrects perhaps tens of times. The independent
/// byte limit below is the hard disk bound; the entry limit also prevents a
/// huge number of tiny records from making review unwieldy.
pub const MAX_ENTRIES: usize = 10_000;

/// Hard cap for the correction log, regardless of record count.
///
/// A character limit alone is not a byte limit under UTF-8, and three fields
/// of 4,000 characters across 10,000 records could otherwise occupy hundreds
/// of megabytes. The user previously had to clean gigabytes of build debris;
/// an opt-in product log must be bounded by bytes, not by a hopeful estimate.
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Longest single field kept. A misrecognised utterance can be long; a
/// megabyte of it is a bug somewhere else, not a correction.
pub const MAX_FIELD_CHARS: usize = 4_000;

/// Which button produced the correction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The user picked a competing reading the parser had offered.
    Alternative,
    /// The user asked for the words Whisper heard, verbatim.
    RawTranscript,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Alternative => "alternative",
            Kind::RawTranscript => "raw_transcript",
        })
    }
}

/// One disagreement, in the shape the research corpus can consume.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Correction {
    pub schema_version: u32,
    /// What Whisper heard.
    pub transcript: String,
    /// What the application had already inserted.
    pub inserted: String,
    /// What the user replaced it with.
    pub chosen: String,
    pub kind: Kind,
    /// The domain the answer had been routed to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

pub const SCHEMA_VERSION: u32 = 1;

impl Correction {
    pub fn new(
        transcript: &str,
        inserted: &str,
        chosen: &str,
        kind: Kind,
        domain: Option<&str>,
    ) -> Self {
        Correction {
            schema_version: SCHEMA_VERSION,
            transcript: clamp(transcript),
            inserted: clamp(inserted),
            chosen: clamp(chosen),
            kind,
            domain: domain.map(clamp),
        }
    }

    /// Whether this correction says anything. A "correction" that changed
    /// nothing is noise, and a corpus full of it would suggest disagreement
    /// where there was none.
    pub fn is_informative(&self) -> bool {
        !self.transcript.trim().is_empty()
            && !self.chosen.trim().is_empty()
            && self.chosen.trim() != self.inserted.trim()
    }
}

fn clamp(text: &str) -> String {
    text.chars().take(MAX_FIELD_CHARS).collect()
}

/// Where corrections are kept: beside the configuration, so a user who knows
/// where their settings live knows where this is too.
pub fn path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("corrections.jsonl")
}

#[derive(Debug)]
pub enum Refusal {
    /// The user has not turned this on.
    NotEnabled,
    /// Nothing changed, so there is nothing to learn.
    Uninformative,
    /// The file is full.
    Full {
        entries: usize,
    },
    Io(std::io::Error),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NotEnabled => f.write_str("запись исправлений выключена"),
            Refusal::Uninformative => f.write_str("исправление ничего не изменило"),
            Refusal::Full { entries } => write!(
                f,
                "файл исправлений заполнен ({entries} записей); перенесите его, чтобы продолжить"
            ),
            Refusal::Io(error) => write!(f, "{error}"),
        }
    }
}

/// Appends one correction. `enabled` is the user's setting, passed in rather
/// than read here so the decision is visible at the call site.
pub fn record(
    file: &Path,
    correction: &Correction,
    enabled: bool,
) -> std::result::Result<(), Refusal> {
    if !enabled {
        return Err(Refusal::NotEnabled);
    }
    if !correction.is_informative() {
        return Err(Refusal::Uninformative);
    }
    let existing_bytes = std::fs::metadata(file).map(|meta| meta.len()).unwrap_or(0);
    let entries = count(file);
    if entries >= MAX_ENTRIES {
        return Err(Refusal::Full { entries });
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(Refusal::Io)?;
    }
    let line = serde_json::to_string(correction)
        .map_err(|error| Refusal::Io(std::io::Error::other(error)))?;
    let appended = u64::try_from(line.len())
        .ok()
        .and_then(|length| length.checked_add(1))
        .and_then(|length| existing_bytes.checked_add(length));
    if appended.is_none_or(|bytes| bytes > MAX_FILE_BYTES) {
        return Err(Refusal::Full { entries });
    }
    let mut handle = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .map_err(Refusal::Io)?;
    writeln!(handle, "{line}").map_err(Refusal::Io)
}

/// How many corrections the file holds. A missing file holds none.
pub fn count(file: &Path) -> usize {
    let Ok(handle) = std::fs::File::open(file) else {
        return 0;
    };
    BufReader::new(handle)
        .lines()
        .take(MAX_ENTRIES)
        .filter_map(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .count()
}

/// Reads the log back, skipping lines that no longer parse rather than
/// failing: a truncated last line must not cost the user everything before
/// it.
pub fn read(file: &Path) -> Vec<Correction> {
    let Ok(handle) = std::fs::File::open(file) else {
        return Vec::new();
    };
    BufReader::new(handle)
        .lines()
        .take(MAX_ENTRIES)
        .filter_map(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Correction {
        Correction::new(
            "корень из икс плюс один",
            "√x + 1",
            "√(x + 1)",
            Kind::Alternative,
            Some("mathematics"),
        )
    }

    #[test]
    fn nothing_is_written_until_the_user_turns_it_on() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("corrections.jsonl");
        assert!(matches!(
            record(&file, &sample(), false),
            Err(Refusal::NotEnabled)
        ));
        assert!(!file.exists(), "an off switch must leave no file behind");
    }

    #[test]
    fn a_correction_is_appended_in_the_shape_the_corpus_reads() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("corrections.jsonl");
        record(&file, &sample(), true).unwrap();
        record(&file, &sample(), true).unwrap();
        assert_eq!(count(&file), 2);

        let back = read(&file);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0], sample());
        assert_eq!(back[0].schema_version, SCHEMA_VERSION);
        assert_eq!(back[0].kind, Kind::Alternative);
    }

    /// A "correction" that chose what was already there is not a
    /// disagreement, and a corpus full of them would claim disagreement
    /// where there was none.
    #[test]
    fn choosing_what_was_already_inserted_is_not_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("corrections.jsonl");
        let same = Correction::new("вода", "H₂O", "H₂O", Kind::Alternative, None);
        assert!(matches!(
            record(&file, &same, true),
            Err(Refusal::Uninformative)
        ));
        assert_eq!(count(&file), 0);
    }

    #[test]
    fn an_empty_utterance_is_not_a_correction() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("corrections.jsonl");
        let blank = Correction::new("   ", "x", "y", Kind::RawTranscript, None);
        assert!(matches!(
            record(&file, &blank, true),
            Err(Refusal::Uninformative)
        ));
    }

    /// The log stops rather than rotating: dropping the oldest examples
    /// would tilt whatever is eventually learned towards recent use.
    #[test]
    fn a_full_log_refuses_instead_of_forgetting_the_earliest_examples() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("corrections.jsonl");
        let line = serde_json::to_string(&sample()).unwrap();
        let body: String = std::iter::repeat_n(line.as_str(), MAX_ENTRIES)
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&file, format!("{body}\n")).unwrap();

        let first = read(&file)[0].clone();
        assert!(matches!(
            record(&file, &sample(), true),
            Err(Refusal::Full { .. })
        ));
        assert_eq!(count(&file), MAX_ENTRIES);
        assert_eq!(read(&file)[0], first, "the earliest example must survive");
    }

    #[test]
    fn a_very_long_utterance_is_clamped_rather_than_stored_whole() {
        let huge = "а".repeat(MAX_FIELD_CHARS * 3);
        let correction = Correction::new(&huge, "x", "y", Kind::RawTranscript, None);
        assert_eq!(correction.transcript.chars().count(), MAX_FIELD_CHARS);
    }

    #[test]
    fn the_log_has_a_hard_byte_limit_not_just_an_entry_limit() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("corrections.jsonl");
        std::fs::write(&file, vec![b'x'; MAX_FILE_BYTES as usize]).unwrap();

        assert!(matches!(
            record(&file, &sample(), true),
            Err(Refusal::Full { .. })
        ));
        assert_eq!(std::fs::metadata(&file).unwrap().len(), MAX_FILE_BYTES);
    }

    /// A half-written last line must not cost the user everything before it.
    #[test]
    fn a_truncated_line_does_not_lose_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("corrections.jsonl");
        record(&file, &sample(), true).unwrap();
        let mut handle = std::fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap();
        write!(handle, "{{\"schema_version\":1,\"transc").unwrap();
        drop(handle);
        assert_eq!(read(&file).len(), 1);
    }

    #[test]
    fn corrections_live_beside_the_configuration() {
        let config = Path::new("/home/user/.config/sciwhisper/config.yaml");
        assert_eq!(
            path(config),
            Path::new("/home/user/.config/sciwhisper/corrections.jsonl")
        );
    }
}

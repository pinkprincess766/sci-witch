//! Split hygiene.
//!
//! Paraphrases of one construction share a `family_id` and must stay inside a
//! single split. A family that straddles train and validation turns the
//! benchmark into a memory test, so the audit is a hard failure, not a note.
//!
//! A speaker is the same problem one level up. Once the corpus holds real
//! voices, a person who appears in both train and dev_holdout makes the
//! holdout measure how well the system knows *that voice*, not how well it
//! handles a new one. Speakers are audited exactly like families.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::schema::Dataset;

#[derive(Clone, Debug, Serialize)]
pub struct LeakingFamily {
    pub family_id: String,
    pub splits: Vec<String>,
    pub ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LeakingSpeaker {
    pub speaker_id: String,
    pub splits: Vec<String>,
    pub ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SplitAudit {
    pub families: usize,
    pub records: usize,
    /// Distinct `speaker_id`s. Zero for a text-only corpus, which is not a
    /// failure — it is the reason `ASR-first` reads `N/A`.
    pub speakers: usize,
    pub counts_by_split: BTreeMap<String, usize>,
    pub families_by_split: BTreeMap<String, usize>,
    pub speakers_by_split: BTreeMap<String, usize>,
    pub leaking_families: Vec<LeakingFamily>,
    pub leaking_speakers: Vec<LeakingSpeaker>,
    pub clean: bool,
}

pub fn audit_splits(dataset: &Dataset) -> SplitAudit {
    let mut splits_by_family: BTreeMap<&str, BTreeSet<&'static str>> = BTreeMap::new();
    let mut ids_by_family: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut counts_by_split: BTreeMap<String, usize> = BTreeMap::new();
    let mut family_sets: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    let mut splits_by_speaker: BTreeMap<&str, BTreeSet<&'static str>> = BTreeMap::new();
    let mut ids_by_speaker: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut speaker_sets: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();

    for record in &dataset.records {
        splits_by_family
            .entry(record.family_id.as_str())
            .or_default()
            .insert(record.split.as_str());
        ids_by_family
            .entry(record.family_id.as_str())
            .or_default()
            .push(record.id.clone());
        *counts_by_split
            .entry(record.split.as_str().to_string())
            .or_insert(0) += 1;
        family_sets
            .entry(record.split.as_str().to_string())
            .or_default()
            .insert(record.family_id.as_str());
        if let Some(speaker) = record.speaker_id.as_deref() {
            splits_by_speaker
                .entry(speaker)
                .or_default()
                .insert(record.split.as_str());
            ids_by_speaker
                .entry(speaker)
                .or_default()
                .push(record.id.clone());
            speaker_sets
                .entry(record.split.as_str().to_string())
                .or_default()
                .insert(speaker);
        }
    }

    let leaking_families: Vec<LeakingFamily> = splits_by_family
        .iter()
        .filter(|(_, splits)| splits.len() > 1)
        .map(|(family, splits)| LeakingFamily {
            family_id: (*family).to_string(),
            splits: splits.iter().map(|split| (*split).to_string()).collect(),
            ids: ids_by_family.get(family).cloned().unwrap_or_default(),
        })
        .collect();

    let leaking_speakers: Vec<LeakingSpeaker> = splits_by_speaker
        .iter()
        .filter(|(_, splits)| splits.len() > 1)
        .map(|(speaker, splits)| LeakingSpeaker {
            speaker_id: (*speaker).to_string(),
            splits: splits.iter().map(|split| (*split).to_string()).collect(),
            ids: ids_by_speaker.get(speaker).cloned().unwrap_or_default(),
        })
        .collect();

    SplitAudit {
        families: splits_by_family.len(),
        records: dataset.records.len(),
        speakers: splits_by_speaker.len(),
        counts_by_split,
        families_by_split: family_sets
            .into_iter()
            .map(|(split, families)| (split, families.len()))
            .collect(),
        speakers_by_split: speaker_sets
            .into_iter()
            .map(|(split, speakers)| (split, speakers.len()))
            .collect(),
        clean: leaking_families.is_empty() && leaking_speakers.is_empty(),
        leaking_families,
        leaking_speakers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn line(id: &str, family: &str, split: &str) -> String {
        format!(
            r#"{{"dataset_schema_version":1,"id":"{id}","family_id":"{family}","provenance":"handcrafted_text","human_transcript":"вода","asr_hypotheses":[],"target_domain":"plain","target_action":"raw","target_ast":null,"split":"{split}","tags":[],"speaker_id":null}}"#
        )
    }

    #[test]
    fn a_family_inside_one_split_is_clean() {
        let corpus = Dataset::parse_jsonl(&format!(
            "{}\n{}\n{}",
            line("fam-1-a", "fam-1", "train"),
            line("fam-1-b", "fam-1", "train"),
            line("fam-2-a", "fam-2", "validation"),
        ))
        .unwrap();
        let audit = audit_splits(&corpus);
        assert!(audit.clean);
        assert_eq!(audit.families, 2);
        assert_eq!(audit.counts_by_split["train"], 2);
        assert_eq!(audit.families_by_split["validation"], 1);
    }

    #[test]
    fn a_family_split_across_two_splits_is_reported() {
        let corpus = Dataset::parse_jsonl(&format!(
            "{}\n{}",
            line("fam-1-a", "fam-1", "train"),
            line("fam-1-b", "fam-1", "validation"),
        ))
        .unwrap();
        let audit = audit_splits(&corpus);
        assert!(!audit.clean);
        assert_eq!(audit.leaking_families.len(), 1);
        assert_eq!(audit.leaking_families[0].family_id, "fam-1");
        assert_eq!(
            audit.leaking_families[0].splits,
            vec!["train".to_string(), "validation".to_string()]
        );
    }
}

#[cfg(test)]
mod speaker_tests {
    use super::*;

    fn voice(id: &str, family: &str, split: &str, speaker: &str) -> String {
        format!(
            r#"{{"dataset_schema_version":2,"id":"{id}","family_id":"{family}","provenance":"real_audio","human_transcript":"вода","asr_hypotheses":[{{"text":"вода"}}],"audio":{{"file":"audio/{id}.wav","sha256":"0000000000000000000000000000000000000000000000000000000000000000","duration_secs":1.0,"sample_rate_hz":16000,"channels":1,"consent":{{"granted":true,"statement_id":"consent-ru-v1","date":"2026-09-05"}}}},"target_domain":"plain","target_action":"raw","target_ast":null,"split":"{split}","tags":[],"speaker_id":"{speaker}"}}"#
        )
    }

    #[test]
    fn one_speaker_per_split_is_clean() {
        let corpus = Dataset::parse_jsonl(&format!(
            "{}\n{}\n{}",
            voice("fam-1-a", "fam-1", "train", "spk01"),
            voice("fam-2-a", "fam-2", "train", "spk01"),
            voice("fam-3-a", "fam-3", "dev_holdout", "spk02"),
        ))
        .unwrap();
        let audit = audit_splits(&corpus);
        assert!(audit.clean);
        assert_eq!(audit.speakers, 2);
        assert_eq!(audit.speakers_by_split["train"], 1);
        assert_eq!(audit.speakers_by_split["dev_holdout"], 1);
    }

    /// The same voice in train and dev_holdout makes the holdout measure how
    /// well the system knows that speaker.
    #[test]
    fn a_speaker_in_two_splits_is_a_leak() {
        let corpus = Dataset::parse_jsonl(&format!(
            "{}\n{}",
            voice("fam-1-a", "fam-1", "train", "spk01"),
            voice("fam-2-a", "fam-2", "dev_holdout", "spk01"),
        ))
        .unwrap();
        let audit = audit_splits(&corpus);
        assert!(!audit.clean);
        assert_eq!(audit.leaking_speakers.len(), 1);
        assert_eq!(audit.leaking_speakers[0].speaker_id, "spk01");
        assert_eq!(
            audit.leaking_speakers[0].splits,
            vec!["dev_holdout".to_string(), "train".to_string()]
        );
        assert!(audit.leaking_families.is_empty());
    }

    /// A text corpus has no speakers, which is a fact about the corpus, not a
    /// fault in it.
    #[test]
    fn a_text_corpus_reports_no_speakers_and_stays_clean() {
        let corpus =
            Dataset::parse_jsonl(&super::tests::line("fam-1-a", "fam-1", "train")).unwrap();
        let audit = audit_splits(&corpus);
        assert!(audit.clean);
        assert_eq!(audit.speakers, 0);
        assert!(audit.speakers_by_split.is_empty());
    }
}

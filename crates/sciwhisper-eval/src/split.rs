//! Split hygiene.
//!
//! Paraphrases of one construction share a `family_id` and must stay inside a
//! single split. A family that straddles train and validation turns the
//! benchmark into a memory test, so the audit is a hard failure, not a note.

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
pub struct SplitAudit {
    pub families: usize,
    pub records: usize,
    pub counts_by_split: BTreeMap<String, usize>,
    pub families_by_split: BTreeMap<String, usize>,
    pub leaking_families: Vec<LeakingFamily>,
    pub clean: bool,
}

pub fn audit_splits(dataset: &Dataset) -> SplitAudit {
    let mut splits_by_family: BTreeMap<&str, BTreeSet<&'static str>> = BTreeMap::new();
    let mut ids_by_family: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut counts_by_split: BTreeMap<String, usize> = BTreeMap::new();
    let mut family_sets: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();

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

    SplitAudit {
        families: splits_by_family.len(),
        records: dataset.records.len(),
        counts_by_split,
        families_by_split: family_sets
            .into_iter()
            .map(|(split, families)| (split, families.len()))
            .collect(),
        clean: leaking_families.is_empty(),
        leaking_families,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, family: &str, split: &str) -> String {
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

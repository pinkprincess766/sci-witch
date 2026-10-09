//! The voice-v1 reading list is the output of the rule in
//! `research/protocol/voice-v1.md`, section «Что читают».
//!
//! The protocol says `voice-v1-families.json` is what the five steps of that
//! rule produce from the three corpora. This test runs the steps and compares
//! the result with the JSON, field by field and in order. The JSON is not
//! regenerated here: if it drifts from the rule or the corpora, this test
//! fails.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde_json::{json, Value};

/// Step 1: the only corpora the rule reads, in the protocol's order.
const SOURCES: [&str; 3] = [
    "dev-seed-v2.jsonl",
    "ambiguous-v1.jsonl",
    "nomenclature-v1.jsonl",
];

/// Step 3: a phrase with more space-separated words than this is dropped.
const MAX_WORDS: usize = 25;

/// Step 4: prefix length per domain, in the protocol's order. Plain speech
/// is the domain `plain`; `plain_is_the_raw_action` checks that this domain
/// and action `raw` select the same records.
const QUOTAS: [(&str, usize); 4] = [
    ("chemistry", 32),
    ("mathematics", 24),
    ("physics", 12),
    ("plain", 12),
];

/// Step 5: records per block and domain, as the protocol states them.
const BLOCK_COUNTS: [(&str, usize); 4] = [
    ("chemistry", 16),
    ("mathematics", 12),
    ("physics", 6),
    ("plain", 6),
];

#[derive(Clone, Debug)]
struct Phrase {
    family_id: String,
    id: String,
    domain: String,
    action: String,
    human_transcript: String,
    source: String,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} is not a string in {value}"))
}

fn read_jsonl(name: &str) -> Vec<Value> {
    let path = root().join("research/data").join(name);
    let body = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    body.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{name}: {e}")))
        .collect()
}

fn listed() -> Value {
    let path = root().join("research/protocol/voice-v1-families.json");
    let body = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn block<'a>(file: &'a Value, name: &str) -> &'a [Value] {
    file["blocks"][name]
        .as_array()
        .unwrap_or_else(|| panic!("blocks.{name} is not an array"))
}

/// Steps 1 to 3: one record per family, the smallest `id` wins, then the
/// length limit. The only record over the limit is a family of its own, so
/// dropping before or after the choice gives the same list.
fn candidates() -> Vec<Phrase> {
    let mut kept: BTreeMap<String, Phrase> = BTreeMap::new();
    for source in SOURCES {
        for row in read_jsonl(source) {
            let phrase = Phrase {
                family_id: text(&row, "family_id").to_owned(),
                id: text(&row, "id").to_owned(),
                domain: text(&row, "target_domain").to_owned(),
                action: text(&row, "target_action").to_owned(),
                human_transcript: text(&row, "human_transcript").to_owned(),
                source: source.to_owned(),
            };
            match kept.get(&phrase.family_id) {
                Some(current) if current.id <= phrase.id => {}
                _ => {
                    kept.insert(phrase.family_id.clone(), phrase);
                }
            }
        }
    }
    kept.into_values()
        .filter(|phrase| phrase.human_transcript.split(' ').count() <= MAX_WORDS)
        .collect()
}

/// Steps 4 and 5. Blocks list domains in `QUOTAS` order, each domain in
/// `family_id` order, which is the order of the JSON.
fn blocks(candidates: &[Phrase]) -> (Vec<Phrase>, Vec<Phrase>) {
    let mut block_i = Vec::new();
    let mut block_ii = Vec::new();
    for (domain, quota) in QUOTAS {
        let mut pool: Vec<&Phrase> = candidates.iter().filter(|p| p.domain == domain).collect();
        pool.sort_by(|a, b| a.family_id.cmp(&b.family_id));
        assert!(
            pool.len() >= quota,
            "{domain}: {} families, quota {quota}",
            pool.len()
        );
        for (position, phrase) in pool.into_iter().take(quota).enumerate() {
            // Even positions of the prefix go to block I, odd ones to block II.
            if position % 2 == 0 {
                block_i.push(phrase.clone());
            } else {
                block_ii.push(phrase.clone());
            }
        }
    }
    (block_i, block_ii)
}

/// Compares field by field and in order. The first differing entry is the
/// one the assertion prints.
fn assert_same_block(name: &str, listed: &[Value], computed: &[Phrase]) {
    assert_eq!(listed.len(), computed.len(), "block {name}: length");
    for (position, (entry, phrase)) in listed.iter().zip(computed).enumerate() {
        let got = [
            text(entry, "family_id"),
            text(entry, "id"),
            text(entry, "domain"),
            text(entry, "action"),
            text(entry, "human_transcript"),
            text(entry, "source"),
        ];
        let want = [
            phrase.family_id.as_str(),
            phrase.id.as_str(),
            phrase.domain.as_str(),
            phrase.action.as_str(),
            phrase.human_transcript.as_str(),
            phrase.source.as_str(),
        ];
        assert_eq!(got, want, "block {name}, entry {position}");
    }
}

#[test]
fn plain_is_the_raw_action() {
    // Plain speech is identified by domain `plain` in `QUOTAS`. That is only
    // correct if the same records have action `raw`.
    for source in SOURCES {
        for row in read_jsonl(source) {
            let plain = text(&row, "target_domain") == "plain";
            let raw = text(&row, "target_action") == "raw";
            assert_eq!(plain, raw, "{source}: {}", text(&row, "id"));
        }
    }
}

#[test]
fn quotas_and_sources_match_protocol() {
    let file = listed();
    assert_eq!(
        file["quotas_per_domain_total"],
        json!({"chemistry": 32, "mathematics": 24, "physics": 12, "plain": 12})
    );
    assert_eq!(file["sources"], json!(SOURCES));
    let quotas: BTreeMap<&str, usize> = QUOTAS.into_iter().collect();
    assert_eq!(json!(quotas), file["quotas_per_domain_total"]);
}

#[test]
fn blocks_match_the_rule_in_order() {
    let file = listed();
    let (block_i, block_ii) = blocks(&candidates());
    assert_same_block("I", block(&file, "I"), &block_i);
    assert_same_block("II", block(&file, "II"), &block_ii);
}

#[test]
fn blocks_have_protocol_counts_per_domain() {
    let file = listed();
    let expected: BTreeMap<&str, usize> = BLOCK_COUNTS.into_iter().collect();
    for name in ["I", "II"] {
        let entries = block(&file, name);
        assert_eq!(entries.len(), 40, "block {name}: families");
        let mut counted: BTreeMap<&str, usize> = BTreeMap::new();
        for entry in entries {
            *counted.entry(text(entry, "domain")).or_insert(0) += 1;
        }
        assert_eq!(counted, expected, "block {name}");
    }
}

#[test]
fn blocks_are_disjoint_by_family() {
    let file = listed();
    let family_ids = |name: &str| -> BTreeSet<&str> {
        block(&file, name)
            .iter()
            .map(|entry| text(entry, "family_id"))
            .collect()
    };
    let shared: Vec<&str> = family_ids("I")
        .intersection(&family_ids("II"))
        .copied()
        .collect();
    assert!(shared.is_empty(), "families in both blocks: {shared:?}");
}

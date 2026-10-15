//! Size of the shared packed parse forest on every corpus transcript.
//!
//! Every record of the four corpora is used, whatever its split: the report
//! describes the grammar, nothing is fitted. Tokenisation is the one of
//! `differential.rs`. Phrases the grammar rejects have no forest and are not
//! counted.
//!
//! Set `GRAMMAR_FOREST_WRITE=1` to record `research/results/grammar-forest-v1.json`.
//! Without it the test rebuilds the report and compares it to that file.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sciwhisper_core::research::math_token_classes;
use sciwhisper_grammar::{
    count_parses,
    ebnf::{parse, Grammar},
    parse_forest, CountOutcome, ForestOutcome, ParseCount, MAX_PARSES,
};
use serde_json::{json, Value};

const CORPORA: &[(&str, &str)] = &[
    ("dev-seed-v3", "research/data/dev-seed-v3.jsonl"),
    ("ambiguous-v2", "research/data/ambiguous-v2.jsonl"),
    ("nomenclature-v1", "research/data/nomenclature-v1.jsonl"),
    (
        "prose-negatives-v2",
        "research/data/prose-negatives-v2.jsonl",
    ),
];

const MODES: &[(&str, bool)] = &[("math", false), ("physics", true)];

/// How many of the phrases with the most trees the report keeps.
const LARGEST: usize = 20;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load_math_ebnf() -> String {
    let path = repo_root().join("docs/grammar/math.ebnf");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn write_json(path: &Path, value: &Value) {
    let pretty = serde_json::to_string_pretty(value).expect("report JSON");
    fs::write(path, pretty).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!("wrote {}", path.display());
}

fn transcripts(path: &Path) -> Vec<String> {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("{}:{i}: {e}", path.display()));
        let t = value
            .get("human_transcript")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("{}:{i}: missing human_transcript", path.display()));
        out.push(t.to_string());
    }
    out
}

/// One phrase with a forest.
struct Row {
    text: String,
    corpus: String,
    mode: String,
    tokens: usize,
    trees: u64,
    nodes: usize,
    packed_entries: usize,
    /// What `count_parses` says about the same tokens.
    counted: ParseCount,
}

/// Rows keyed by corpus, then mode.
type Runs = BTreeMap<String, BTreeMap<String, Vec<Row>>>;

#[derive(Default)]
struct Stats {
    phrases: u64,
    max_tokens: usize,
    max_trees: u64,
    max_nodes: usize,
    max_packed_entries: usize,
    sum_trees: u64,
    sum_nodes: u64,
    sum_packed_entries: u64,
}

fn start_symbol(grammar: &Grammar) -> String {
    let start = grammar
        .start()
        .expect("math.ebnf has a syntactic start rule")
        .to_string();
    assert_eq!(start, "math_input");
    start
}

/// The forest of one tokenised phrase, or `None` when there is none.
fn forest_row(
    grammar: &Grammar,
    start: &str,
    corpus: &str,
    mode: &str,
    physics: bool,
    text: &str,
) -> Option<Row> {
    let tokens = math_token_classes(text, physics).ok()?;
    let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
    let ForestOutcome::Forest(forest) = parse_forest(grammar, start, &refs) else {
        return None;
    };
    let CountOutcome::Counted(counted) = count_parses(grammar, start, &refs) else {
        panic!("{corpus}/{mode}: a forest for {text:?} but count_parses refuses it");
    };
    Some(Row {
        text: text.to_string(),
        corpus: corpus.to_string(),
        mode: mode.to_string(),
        tokens: tokens.len(),
        trees: forest.tree_count(),
        nodes: forest.nodes().len(),
        packed_entries: forest.nodes().iter().map(|node| node.packed.len()).sum(),
        counted,
    })
}

/// Rows of every corpus and mode, keyed by corpus then mode (empty cells
/// included).
fn run(grammar: &Grammar, start: &str) -> Runs {
    let root = repo_root();
    let mut out = BTreeMap::new();
    for (corpus, rel) in CORPORA {
        let records = transcripts(&root.join(rel));
        assert!(
            !records.is_empty(),
            "{corpus} produced 0 records; the report would describe nothing"
        );
        let mut by_mode = BTreeMap::new();
        for (mode, physics) in MODES {
            let rows: Vec<Row> = records
                .iter()
                .filter_map(|text| forest_row(grammar, start, corpus, mode, *physics, text))
                .collect();
            by_mode.insert((*mode).to_string(), rows);
        }
        out.insert((*corpus).to_string(), by_mode);
    }
    out
}

fn stats(rows: &[Row]) -> Stats {
    let mut s = Stats::default();
    for row in rows {
        s.phrases += 1;
        s.max_tokens = s.max_tokens.max(row.tokens);
        s.max_trees = s.max_trees.max(row.trees);
        s.max_nodes = s.max_nodes.max(row.nodes);
        s.max_packed_entries = s.max_packed_entries.max(row.packed_entries);
        s.sum_trees = s.sum_trees.saturating_add(row.trees);
        s.sum_nodes += row.nodes as u64;
        s.sum_packed_entries += row.packed_entries as u64;
    }
    s
}

fn stats_json(s: &Stats) -> Value {
    json!({
        "phrases": s.phrases,
        "max_tokens": s.max_tokens,
        "max_trees": s.max_trees,
        "max_nodes": s.max_nodes,
        "max_packed_entries": s.max_packed_entries,
        "sum_trees": s.sum_trees,
        "sum_nodes": s.sum_nodes,
        "sum_packed_entries": s.sum_packed_entries,
    })
}

fn row_json(row: &Row) -> Value {
    json!({
        "text": row.text,
        "corpus": row.corpus,
        "mode": row.mode,
        "tokens": row.tokens,
        "trees": row.trees,
        "nodes": row.nodes,
        "packed_entries": row.packed_entries,
    })
}

/// Text, then corpus, then mode: the tie-break of both rankings.
fn by_name(a: &Row, b: &Row) -> std::cmp::Ordering {
    a.text
        .cmp(&b.text)
        .then(a.corpus.cmp(&b.corpus))
        .then(a.mode.cmp(&b.mode))
}

fn report(start: &str, runs: &Runs) -> Value {
    let all: Vec<&Row> = runs
        .values()
        .flat_map(|modes| modes.values())
        .flatten()
        .collect();
    let mut corpora = serde_json::Map::new();
    for (corpus, modes) in runs {
        let mut by_mode = serde_json::Map::new();
        for (mode, rows) in modes {
            by_mode.insert(mode.clone(), stats_json(&stats(rows)));
        }
        corpora.insert(corpus.clone(), Value::Object(by_mode));
    }
    let mut largest = all.clone();
    largest.sort_by(|a, b| b.trees.cmp(&a.trees).then_with(|| by_name(a, b)));
    let largest: Vec<Value> = largest.into_iter().take(LARGEST).map(row_json).collect();
    let mut longest = all;
    longest.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| by_name(a, b)));
    json!({
        "version": "grammar-forest-v1",
        "start": start,
        "largest_cap": LARGEST,
        "note": "The shared packed parse forest has one node per (nonterminal, span); it is not Scott's binarised SPPF, so a packed entry lists all children of a production at once. On these phrases nodes outnumber trees, because the phrases are short and the grammar chains many single-child nonterminals, so sharing saves nothing here; it pays only when the tree count grows faster than the span count, as in long sums. Every record of the four corpora is used, whatever its split, because the numbers describe the grammar and nothing is fitted; phrases the grammar rejects have no forest, so a corpus with none of them has zero rows. The ambiguous-addition family is not in math.ebnf, so longest_phrase reports the corpus phrase with the most tokens instead.",
        "corpora": Value::Object(corpora),
        "largest": largest,
        "longest_phrase": row_json(longest[0]),
    })
}

fn live() -> (Grammar, String, Runs) {
    let grammar = parse(&load_math_ebnf()).unwrap_or_else(|e| panic!("math.ebnf: {e}"));
    let start = start_symbol(&grammar);
    let runs = run(&grammar, &start);
    (grammar, start, runs)
}

#[test]
fn forest_report_matches_the_committed_file() {
    let (_, start, runs) = live();
    let total: u64 = runs
        .values()
        .flat_map(|modes| modes.values())
        .map(|rows| rows.len() as u64)
        .sum();
    assert!(total > 0, "no phrase has a forest; the report is empty");
    let live = report(&start, &runs);
    let path = repo_root().join("research/results/grammar-forest-v1.json");
    if std::env::var_os("GRAMMAR_FOREST_WRITE").is_some() {
        write_json(&path, &live);
        return;
    }
    let stored_text = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; record it with GRAMMAR_FOREST_WRITE=1 cargo test -p sciwhisper-grammar \
             --test forest_report -- --nocapture",
            path.display()
        )
    });
    let stored: Value = serde_json::from_str(&stored_text).expect("stored forest report parses");
    assert_eq!(
        live,
        stored,
        "forest sizes diverged from {}; re-record with GRAMMAR_FOREST_WRITE=1 if intended",
        path.display()
    );
}

#[test]
fn forest_tree_counts_equal_grammar_differential_v2() {
    let (grammar, start, runs) = live();
    let path = repo_root().join("research/results/grammar-differential-v2.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let v2: Value = serde_json::from_str(&text).expect("v2 report parses");
    let listed = v2["most_parses"].as_array().expect("most_parses is a list");
    assert!(!listed.is_empty(), "v2 lists no phrase; nothing to compare");
    for entry in listed {
        let text = entry["text"].as_str().expect("text");
        let corpus = entry["corpus"].as_str().expect("corpus");
        let mode = entry["mode"].as_str().expect("mode");
        let physics = MODES
            .iter()
            .find(|(name, _)| *name == mode)
            .unwrap_or_else(|| panic!("unknown mode {mode}"))
            .1;
        let row = forest_row(&grammar, &start, corpus, mode, physics, text)
            .unwrap_or_else(|| panic!("{corpus}/{mode}: no forest for {text:?}"));
        match entry["count"].as_u64() {
            Some(recorded) => assert_eq!(
                row.trees, recorded,
                "{corpus}/{mode}: {text:?} has {} forest trees, v2 records {recorded}",
                row.trees
            ),
            None => assert!(
                row.trees >= u64::from(MAX_PARSES),
                "{corpus}/{mode}: v2 records the cap for {text:?}, forest has {}",
                row.trees
            ),
        }
    }
    // The report must not drop phrases: its phrase counts are v2's accepted counts.
    for (corpus, modes) in &runs {
        for (mode, rows) in modes {
            let accepted = v2["parse_counts"][corpus][mode]["accepted"]
                .as_u64()
                .unwrap_or_else(|| panic!("{corpus}/{mode}: v2 has no accepted count"));
            assert_eq!(
                rows.len() as u64,
                accepted,
                "{corpus}/{mode}: forests versus v2 accepted phrases"
            );
        }
    }
}

#[test]
fn every_forest_tree_count_equals_count_parses() {
    let (_, _, runs) = live();
    let mut checked = 0u64;
    for rows in runs.values().flat_map(|modes| modes.values()) {
        for row in rows {
            match row.counted {
                ParseCount::Exact(n) => assert_eq!(
                    row.trees,
                    u64::from(n),
                    "{}/{}: {:?}: forest versus count_parses",
                    row.corpus,
                    row.mode,
                    row.text
                ),
                ParseCount::AtLeastMax => assert!(
                    row.trees >= u64::from(MAX_PARSES),
                    "{}/{}: {:?}: count_parses hit the cap, forest has {}",
                    row.corpus,
                    row.mode,
                    row.text,
                    row.trees
                ),
            }
            checked += 1;
        }
    }
    assert!(checked > 0, "nothing was compared");
}

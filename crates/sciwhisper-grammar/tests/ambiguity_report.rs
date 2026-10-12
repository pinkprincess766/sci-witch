//! Bounded search for ambiguous sentences of `docs/grammar/math.ebnf`.
//!
//! Every token-class string of length 1..=`MAX_LEN` that the grammar derives
//! is counted with `count_parses`. The result is
//! `research/results/grammar-ambiguity-v1.json`.
//!
//! Set `GRAMMAR_AMBIGUITY_WRITE=1` to record that file. Without it the test
//! compares the live report to the committed one.
//!
//! Length `MAX_LEN` alone is 167368 strings and about three minutes in a debug
//! build, so an ordinary run searches only up to `QUICK_LEN` and compares
//! that part of the file. `GRAMMAR_AMBIGUITY_FULL=1` (or the write variable)
//! searches up to `MAX_LEN` and compares the whole file; a release build does
//! that in about 15 s.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use sciwhisper_core::research::math_token_classes;
use sciwhisper_grammar::{
    count_parses, ebnf::parse, generate::sentences_up_to, generate::MAX_LEN, CountOutcome,
    ParseCount, MAX_PARSES,
};

/// How many ambiguous strings the report lists.
const SHORTEST_LISTED: usize = 60;

/// Longest string an ordinary test run searches: 6420 strings in all.
const QUICK_LEN: usize = 3;

/// Whether this run searches up to `MAX_LEN` and checks the whole file.
fn full_run() -> bool {
    std::env::var_os("GRAMMAR_AMBIGUITY_FULL").is_some()
        || std::env::var_os("GRAMMAR_AMBIGUITY_WRITE").is_some()
}

fn searched_len() -> usize {
    if full_run() {
        MAX_LEN
    } else {
        QUICK_LEN
    }
}

#[derive(Clone, Copy, Default)]
struct Row {
    strings: u64,
    ambiguous: u64,
    max_parses: u32,
    exact_2: u64,
    exact_3_to_10: u64,
    above_10: u64,
    at_least_max: u64,
}

struct Search {
    start: String,
    by_len: BTreeMap<usize, Row>,
    /// Ambiguous strings only, with their counts.
    ambiguous: Vec<(Vec<String>, ParseCount)>,
    /// All counted strings, keyed by their space-joined tokens.
    counts: HashMap<String, ParseCount>,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rank(count: ParseCount) -> u32 {
    match count {
        ParseCount::Exact(n) => n,
        ParseCount::AtLeastMax => u32::MAX,
    }
}

fn count_json(count: ParseCount) -> serde_json::Value {
    match count {
        ParseCount::Exact(n) => serde_json::json!(n),
        ParseCount::AtLeastMax => serde_json::json!(format!(">={MAX_PARSES}")),
    }
}

fn run_search() -> Search {
    let path = repo_root().join("docs/grammar/math.ebnf");
    let src = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let grammar = parse(&src).unwrap_or_else(|e| panic!("math.ebnf: {e}"));
    let start = grammar
        .start()
        .expect("math.ebnf has a syntactic start rule")
        .to_string();
    let strings = sentences_up_to(&grammar, &start, searched_len())
        .unwrap_or_else(|e| panic!("sentences_up_to: {e:?}"));
    assert!(!strings.is_empty(), "the generator returned no strings");

    let mut by_len: BTreeMap<usize, Row> = BTreeMap::new();
    let mut ambiguous = Vec::new();
    let mut counts = HashMap::with_capacity(strings.len());
    for tokens in strings {
        let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        let count = match count_parses(&grammar, &start, &refs) {
            CountOutcome::Counted(c) => c,
            other => panic!("generated string {:?} was not counted: {other:?}", tokens),
        };
        if count == ParseCount::Exact(0) {
            panic!("generated string {tokens:?} has 0 parses");
        }
        let row = by_len.entry(tokens.len()).or_default();
        row.strings += 1;
        match count {
            ParseCount::Exact(1) => {}
            ParseCount::Exact(2) => row.exact_2 += 1,
            ParseCount::Exact(n) if n <= 10 => row.exact_3_to_10 += 1,
            ParseCount::Exact(_) => row.above_10 += 1,
            ParseCount::AtLeastMax => {
                row.above_10 += 1;
                row.at_least_max += 1;
            }
        }
        if count != ParseCount::Exact(1) {
            row.ambiguous += 1;
            row.max_parses = row.max_parses.max(match count {
                ParseCount::Exact(n) => n,
                ParseCount::AtLeastMax => MAX_PARSES,
            });
            ambiguous.push((tokens.clone(), count));
        }
        counts.insert(tokens.join(" "), count);
    }
    Search {
        start,
        by_len,
        ambiguous,
        counts,
    }
}

fn search() -> &'static Search {
    static SEARCH: OnceLock<Search> = OnceLock::new();
    SEARCH.get_or_init(run_search)
}

fn report_json(s: &Search) -> serde_json::Value {
    let mut per_length = serde_json::Map::new();
    for (len, r) in &s.by_len {
        per_length.insert(
            len.to_string(),
            serde_json::json!({
                "strings": r.strings,
                "ambiguous": r.ambiguous,
                "max_parses": r.max_parses,
                "exact_2": r.exact_2,
                "exact_3_to_10": r.exact_3_to_10,
                "above_10": r.above_10,
                "at_least_max": r.at_least_max,
            }),
        );
    }
    let mut sorted: Vec<_> = s.ambiguous.iter().collect();
    sorted.sort_by(|a, b| {
        a.0.len()
            .cmp(&b.0.len())
            .then(rank(b.1).cmp(&rank(a.1)))
            .then(a.0.cmp(&b.0))
    });
    let shortest: Vec<serde_json::Value> = sorted
        .into_iter()
        .take(SHORTEST_LISTED)
        .map(|(tokens, count)| {
            serde_json::json!({"tokens": tokens.join(" "), "parses": count_json(*count)})
        })
        .collect();
    serde_json::json!({
        "version": "grammar-ambiguity-v1",
        "grammar": "docs/grammar/math.ebnf",
        "start": s.start,
        "max_len": MAX_LEN,
        "max_parses": MAX_PARSES,
        "shortest_listed_cap": SHORTEST_LISTED,
        "note": "This is the ambiguity of the context-free grammar over token classes: every string of length 1 to max_len that the grammar derives, with its number of derivation trees. The handwritten parser picks one tree by ordered choice, so an ambiguous string is not necessarily mis-parsed. Semantic constraints written in prose in math.ebnf are not applied, so some of these strings are rejected by the parser. Length is counted in token classes, not in spoken words. above_10 includes the at_least_max strings; max_parses is capped at max_parses.",
        "per_length": per_length,
        "shortest_ambiguous": shortest,
    })
}

fn write_json(path: &Path, value: &serde_json::Value) {
    let pretty = serde_json::to_string_pretty(value).expect("report JSON");
    fs::write(path, pretty).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!("wrote {}", path.display());
}

#[test]
fn ambiguity_report_matches_the_committed_file() {
    let live = report_json(search());
    let path = repo_root().join("research/results/grammar-ambiguity-v1.json");
    if std::env::var_os("GRAMMAR_AMBIGUITY_WRITE").is_some() {
        write_json(&path, &live);
        return;
    }
    assert!(
        path.exists(),
        "{} is missing; record it with GRAMMAR_AMBIGUITY_WRITE=1 cargo test -p sciwhisper-grammar \
         --test ambiguity_report -- --nocapture",
        path.display()
    );
    let stored_text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut stored: serde_json::Value =
        serde_json::from_str(&stored_text).expect("stored ambiguity report must parse");
    if !full_run() {
        // Only lengths up to QUICK_LEN were searched. The listed strings are
        // sorted by length and all have length <= QUICK_LEN in the full
        // report too, so only the longer per-length rows are left out.
        let per_length = stored["per_length"]
            .as_object_mut()
            .expect("per_length is an object");
        per_length.retain(|len, _| len.parse::<usize>().expect("length key") <= QUICK_LEN);
        assert_eq!(per_length.len(), QUICK_LEN, "stored lengths 1..=QUICK_LEN");
        assert!(
            stored["shortest_ambiguous"]
                .as_array()
                .expect("shortest_ambiguous")
                .iter()
                .all(|row| row["tokens"].as_str().unwrap().split(' ').count() <= QUICK_LEN),
            "a listed string is longer than QUICK_LEN; run with GRAMMAR_AMBIGUITY_FULL=1"
        );
    }
    assert_eq!(
        live,
        stored,
        "ambiguity report diverged from {}; re-record with GRAMMAR_AMBIGUITY_WRITE=1 if the change is intended",
        path.display()
    );
}

#[test]
fn every_generated_string_is_accepted() {
    // run_search panics on the first string that is not Counted.
    let s = search();
    let total: u64 = s.by_len.values().map(|r| r.strings).sum();
    assert_eq!(
        total as usize,
        s.counts.len(),
        "duplicate strings in the search"
    );
    for len in 1..=searched_len() {
        assert!(
            s.by_len.get(&len).is_some_and(|r| r.strings > 0),
            "no strings of length {len}"
        );
    }
    for (len, r) in &s.by_len {
        assert_eq!(
            r.ambiguous,
            r.exact_2 + r.exact_3_to_10 + r.above_10,
            "length {len}: buckets do not add up to ambiguous"
        );
    }
}

#[test]
fn corpus_ambiguity_is_found_by_the_search() {
    let path = repo_root().join("research/results/grammar-differential-v2.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let v2: serde_json::Value = serde_json::from_str(&text).expect("differential v2 must parse");
    let rows = v2["most_parses"].as_array().expect("most_parses");
    let s = search();
    let mut checked = 0;
    for row in rows {
        let phrase = row["text"].as_str().expect("text");
        let physics = match row["mode"].as_str().expect("mode") {
            "physics" => true,
            "math" => false,
            other => panic!("unknown mode {other:?}"),
        };
        let tokens = math_token_classes(phrase, physics)
            .unwrap_or_else(|e| panic!("{phrase:?}: tokenisation failed: {e:?}"));
        if tokens.len() > searched_len() {
            continue;
        }
        let key = tokens.join(" ");
        let found = s.counts.get(&key).unwrap_or_else(|| {
            panic!("corpus phrase {phrase:?} ({key}) is missing from the search")
        });
        assert_eq!(
            count_json(*found),
            row["count"],
            "corpus phrase {phrase:?} ({key}): search count differs from grammar-differential-v2"
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "no most_parses phrase has length <= {}; the cross-check compared nothing",
        searched_len()
    );
    println!("cross-checked {checked} corpus phrases");
}

//! Differential test: Earley on `docs/grammar/chem.ebnf` versus the
//! handwritten chemistry parser, on every corpus transcript.
//!
//! Same cells as `differential.rs` (both accept, both reject, only Earley,
//! only the parser, tokenizer failure), plus how many derivation trees each
//! Earley-accepted phrase has. Discrepancies are measured, not repaired.
//!
//! The grammar's terminals are word classes, produced by
//! `research::chem_token_classes`. The semantic section of `chem.ebnf` (Х1–Х11)
//! and the special sequences `? … ?` are not checked by the recogniser, so
//! "only Earley" is the expected superset; "only the parser" is a phrase the
//! code reads as chemistry that the class sequence does not derive.
//!
//! Set `GRAMMAR_CHEM_WRITE=1` to record
//! `research/results/grammar-differential-chem-v1.json`. Without it the test
//! compares the live run to that file.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sciwhisper_core::{interpret, research::chem_token_classes, Domain, InterpretOptions, Node};
use sciwhisper_grammar::{
    count_parses,
    ebnf::{parse, Grammar},
    CountOutcome, ParseCount, MAX_PARSES,
};

const CORPORA: &[(&str, &str)] = &[
    ("dev-seed-v3", "research/data/dev-seed-v3.jsonl"),
    ("ambiguous-v2", "research/data/ambiguous-v2.jsonl"),
    ("nomenclature-v1", "research/data/nomenclature-v1.jsonl"),
    (
        "prose-negatives-v2",
        "research/data/prose-negatives-v2.jsonl",
    ),
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load_chem_ebnf() -> String {
    let path = repo_root().join("docs/grammar/chem.ebnf");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The parser accepts a phrase when `interpret` in `Domain::Chemistry` gives
/// a chemical node (`Node::Chemical`, a species or an equation) with
/// confidence above zero. That is the chemistry counterpart of the math rule
/// in `differential.rs`. A failed parse comes back as `Node::Text` with
/// confidence 0, so either condition alone would also accept refusals that
/// carry text or a zero confidence.
fn parser_accepts(text: &str) -> bool {
    let result = interpret(
        text,
        InterpretOptions {
            domain: Domain::Chemistry,
            allow_shortcuts: false,
        },
    );
    result.confidence > 0.0 && matches!(result.ast, Node::Chemical(_))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Cell {
    both_accept: u64,
    both_reject: u64,
    earley_only: u64,
    parser_only: u64,
    tokenize_fail: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Buckets {
    accepted: u64,
    exact_1: u64,
    exact_2: u64,
    exact_3_to_10: u64,
    above_10: u64,
    at_least_max: u64,
}

#[derive(Clone, Debug)]
struct Discrepancy {
    text: String,
    corpus: String,
    kind: &'static str,
    tokens: Vec<String>,
    furthest: Option<usize>,
}

#[derive(Default)]
struct Run {
    cells: BTreeMap<String, Cell>,
    buckets: BTreeMap<String, Buckets>,
    earley_only: Vec<Discrepancy>,
    parser_only: Vec<Discrepancy>,
    /// Phrases `chem_token_classes` refuses but the parser reads as chemistry.
    /// The grammar cannot be asked about them, so they sit outside the cells.
    tokenize_fail_parser_accepts: Vec<Discrepancy>,
}

fn earley_judge(
    grammar: &Grammar,
    start: &str,
    tokens: &[String],
) -> (bool, Option<usize>, Option<ParseCount>) {
    let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
    match count_parses(grammar, start, &refs) {
        CountOutcome::Counted(count) => (true, None, Some(count)),
        CountOutcome::Rejected { furthest } => (false, Some(furthest), None),
        CountOutcome::TooLong | CountOutcome::TooManyItems => (false, None, None),
    }
}

fn add_bucket(buckets: &mut Buckets, count: ParseCount) {
    buckets.accepted += 1;
    match count {
        ParseCount::Exact(1) => buckets.exact_1 += 1,
        ParseCount::Exact(2) => buckets.exact_2 += 1,
        ParseCount::Exact(n) if (3..=10).contains(&n) => buckets.exact_3_to_10 += 1,
        ParseCount::Exact(_) => buckets.above_10 += 1,
        ParseCount::AtLeastMax => {
            buckets.above_10 += 1;
            buckets.at_least_max += 1;
        }
    }
}

fn compare_one(
    grammar: &Grammar,
    start: &str,
    corpus: &str,
    text: &str,
    accepts: &dyn Fn(&str) -> bool,
    run: &mut Run,
) {
    let cell = run.cells.entry(corpus.to_string()).or_default();
    let tokens = match chem_token_classes(text) {
        Ok(t) => t,
        Err(_) => {
            cell.tokenize_fail += 1;
            if accepts(text) {
                run.tokenize_fail_parser_accepts.push(Discrepancy {
                    text: text.to_string(),
                    corpus: corpus.to_string(),
                    kind: "tokenize_fail_parser_accepts",
                    tokens: Vec::new(),
                    furthest: None,
                });
            }
            return;
        }
    };
    let (earley, furthest, parses) = earley_judge(grammar, start, &tokens);
    let parsed = accepts(text);
    let kind = match (earley, parsed) {
        (true, true) => {
            cell.both_accept += 1;
            None
        }
        (false, false) => {
            cell.both_reject += 1;
            None
        }
        (true, false) => {
            cell.earley_only += 1;
            Some("earley_only")
        }
        (false, true) => {
            cell.parser_only += 1;
            Some("parser_only")
        }
    };
    if let Some(kind) = kind {
        let row = Discrepancy {
            text: text.to_string(),
            corpus: corpus.to_string(),
            kind,
            tokens,
            furthest,
        };
        if kind == "earley_only" {
            run.earley_only.push(row);
        } else {
            run.parser_only.push(row);
        }
    }
    if let Some(count) = parses {
        if count == ParseCount::Exact(0) {
            panic!(
                "{corpus}: Earley accepted {text:?} but the derivation count is 0; \
                 the counter missed a completed production"
            );
        }
        add_bucket(run.buckets.entry(corpus.to_string()).or_default(), count);
    }
}

fn transcripts(path: &Path) -> Vec<String> {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("{}:{i}: {e}", path.display()));
        let t = value
            .get("human_transcript")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("{}:{i}: missing human_transcript", path.display()));
        out.push(t.to_string());
    }
    out
}

fn run_corpora(grammar: &Grammar, start: &str, accepts: &dyn Fn(&str) -> bool) -> Run {
    let mut run = Run::default();
    let root = repo_root();
    for (corpus, rel) in CORPORA {
        let records = transcripts(&root.join(rel));
        assert!(
            !records.is_empty(),
            "{corpus} produced 0 records; the gate would pass on nothing"
        );
        run.cells.entry((*corpus).to_string()).or_default();
        run.buckets.entry((*corpus).to_string()).or_default();
        for text in &records {
            compare_one(grammar, start, corpus, text, accepts, &mut run);
        }
    }
    run
}

fn discrepancy_json(d: &Discrepancy) -> serde_json::Value {
    serde_json::json!({
        "text": d.text,
        "corpus": d.corpus,
        "kind": d.kind,
        "tokens": d.tokens,
        "furthest": d.furthest,
    })
}

fn cell_json(c: &Cell) -> serde_json::Value {
    serde_json::json!({
        "both_accept": c.both_accept,
        "both_reject": c.both_reject,
        "earley_only": c.earley_only,
        "parser_only": c.parser_only,
        "tokenize_fail": c.tokenize_fail,
    })
}

fn buckets_json(b: &Buckets) -> serde_json::Value {
    serde_json::json!({
        "accepted": b.accepted,
        "exact_1": b.exact_1,
        "exact_2": b.exact_2,
        "exact_3_to_10": b.exact_3_to_10,
        "above_10": b.above_10,
        "at_least_max": b.at_least_max,
    })
}

fn report_json(grammar: &Grammar, start: &str, run: &Run) -> serde_json::Value {
    let cells: serde_json::Map<String, serde_json::Value> = run
        .cells
        .iter()
        .map(|(corpus, cell)| (corpus.clone(), cell_json(cell)))
        .collect();
    let parse_counts: serde_json::Map<String, serde_json::Value> = run
        .buckets
        .iter()
        .map(|(corpus, b)| (corpus.clone(), buckets_json(b)))
        .collect();
    serde_json::json!({
        "version": "grammar-differential-chem-v1",
        "max_parses": MAX_PARSES,
        "start": start,
        "syntactic_literals": grammar.syntactic_literals(),
        "note": "Grammar: docs/grammar/chem.ebnf, terminals from sciwhisper_core::research::chem_token_classes. Parser accepts when interpret(Domain::Chemistry) gives a Node::Chemical with confidence above 0. The recogniser does not enforce the semantic section (Х1-Х11) or special sequences; it accepts a superset. parse_counts: derivation trees of Earley-accepted phrases (both_accept + earley_only); above_10 includes every count that hit the cap, at_least_max is the subset that hit it.",
        "cells": cells,
        "parse_counts": parse_counts,
        "earley_only": run.earley_only.iter().map(discrepancy_json).collect::<Vec<_>>(),
        "parser_only": run.parser_only.iter().map(discrepancy_json).collect::<Vec<_>>(),
        "tokenize_fail_parser_accepts": run
            .tokenize_fail_parser_accepts
            .iter()
            .map(discrepancy_json)
            .collect::<Vec<_>>(),
    })
}

fn check_run(run: &Run) {
    for (corpus, cell) in &run.cells {
        let buckets = &run.buckets[corpus];
        println!(
            "{corpus}: both_accept={} both_reject={} earley_only={} parser_only={} \
             tokenize_fail={} parses 1={} 2={} 3-10={} >10={} of which >={MAX_PARSES}={}",
            cell.both_accept,
            cell.both_reject,
            cell.earley_only,
            cell.parser_only,
            cell.tokenize_fail,
            buckets.exact_1,
            buckets.exact_2,
            buckets.exact_3_to_10,
            buckets.above_10,
            buckets.at_least_max
        );
        let summed = buckets.exact_1 + buckets.exact_2 + buckets.exact_3_to_10 + buckets.above_10;
        assert_eq!(
            summed, buckets.accepted,
            "{corpus}: buckets do not add up to the accepted denominator"
        );
        assert_eq!(
            buckets.accepted,
            cell.both_accept + cell.earley_only,
            "{corpus}: accepted counts are not the Earley accepts"
        );
        assert!(
            buckets.at_least_max <= buckets.above_10,
            "{corpus}: at_least_max sits outside above_10"
        );
        let compared = cell.both_accept + cell.both_reject + cell.earley_only + cell.parser_only;
        assert!(
            cell.tokenize_fail > 0 || compared > 0,
            "{corpus}: nothing was counted"
        );
    }
    let both_accept: u64 = run.cells.values().map(|c| c.both_accept).sum();
    assert!(
        both_accept > 0,
        "no phrase is accepted by both sides; the comparison would pass on nothing"
    );
    assert_eq!(run.cells.len(), CORPORA.len(), "a corpus is missing");
}

fn print_examples(title: &str, rows: &[Discrepancy]) {
    println!("{title}: {} total", rows.len());
    for d in rows {
        println!(
            "  [{}] {:?} tokens={:?} furthest={:?}",
            d.corpus, d.text, d.tokens, d.furthest
        );
    }
}

fn write_json(path: &Path, value: &serde_json::Value) {
    let pretty = serde_json::to_string_pretty(value).expect("report JSON");
    fs::write(path, pretty).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!("wrote {}", path.display());
}

#[test]
fn chem_grammar_versus_the_handwritten_parser() {
    let src = load_chem_ebnf();
    let grammar = parse(&src).unwrap_or_else(|e| panic!("chem.ebnf: {e}"));
    let start = grammar
        .start()
        .expect("chem.ebnf has a syntactic start rule")
        .to_string();
    assert_eq!(start, "chem_input");
    assert!(
        grammar.syntactic_literals().is_empty(),
        "chem.ebnf syntactic rules have no quoted literals; lexical phrases are not the recogniser's alphabet"
    );

    let run = run_corpora(&grammar, &start, &parser_accepts);
    print_examples("earley_only", &run.earley_only);
    print_examples("parser_only", &run.parser_only);
    print_examples(
        "tokenize_fail_parser_accepts",
        &run.tokenize_fail_parser_accepts,
    );
    check_run(&run);

    let live = report_json(&grammar, &start, &run);
    let path = repo_root().join("research/results/grammar-differential-chem-v1.json");
    if std::env::var_os("GRAMMAR_CHEM_WRITE").is_some() {
        write_json(&path, &live);
    } else if !path.exists() {
        panic!(
            "{} is missing; record it with \
             GRAMMAR_CHEM_WRITE=1 cargo test -p sciwhisper-grammar \
             --test differential_chem -- --nocapture",
            path.display()
        );
    } else {
        let stored_text =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let stored: serde_json::Value =
            serde_json::from_str(&stored_text).expect("stored chem report must parse");
        assert_eq!(
            &live,
            &stored,
            "chem differential diverged from {}; re-record with GRAMMAR_CHEM_WRITE=1 if the change is intended",
            path.display()
        );
    }
}

#[test]
fn a_grammar_with_plus_removed_is_not_a_trivial_match() {
    let src = load_chem_ebnf();
    let broken = src.replace(
        "side_joint = PLUS | CONJUNCTION | JOIN_REAGENT ;",
        "side_joint = CONJUNCTION | JOIN_REAGENT ;",
    );
    assert_ne!(
        src, broken,
        "the PLUS alternative was not found in chem.ebnf; the sabotage did nothing"
    );
    let original = parse(&src).unwrap();
    let sabotaged = parse(&broken).unwrap();
    let start = original.start().unwrap().to_string();
    let phrases = [
        "водород плюс кислород превращается в вода",
        "натрий плюс хлор образует хлорид натрия",
        "вода",
    ];
    let mut before = Run::default();
    let mut after = Run::default();
    for text in phrases {
        compare_one(
            &original,
            &start,
            "hand",
            text,
            &parser_accepts,
            &mut before,
        );
        compare_one(
            &sabotaged,
            &start,
            "hand",
            text,
            &parser_accepts,
            &mut after,
        );
    }
    assert!(
        before.cells["hand"].both_accept > 0,
        "the small set should contain a phrase both sides accept, got {:?}",
        before.cells["hand"]
    );
    assert!(
        after.cells["hand"].parser_only > before.cells["hand"].parser_only,
        "removing PLUS from side_joint must grow parser_only (before {:?}, after {:?})",
        before.cells["hand"],
        after.cells["hand"]
    );
}

#[test]
fn an_always_accepting_parser_cannot_match_the_recorded_cells() {
    // A parser that accepts everything turns every Earley rejection into a
    // `parser_only` row, so the live report must differ from an honest run.
    let src = load_chem_ebnf();
    let grammar = parse(&src).unwrap();
    let start = grammar.start().unwrap().to_string();
    let honest = run_corpora(&grammar, &start, &parser_accepts);
    let always = run_corpora(&grammar, &start, &|_| true);
    assert_ne!(
        report_json(&grammar, &start, &honest),
        report_json(&grammar, &start, &always),
        "an always-true parser_accepts produced the same report as the real one"
    );
    let always_parser_only: u64 = always.cells.values().map(|c| c.parser_only).sum();
    let always_both_reject: u64 = always.cells.values().map(|c| c.both_reject).sum();
    assert_eq!(always_both_reject, 0, "both_reject must vanish");
    assert!(always_parser_only > 0);
}

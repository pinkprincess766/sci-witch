//! Differential test: Earley on `docs/grammar/math.ebnf` versus the
//! handwritten mathematics parser, on every corpus transcript.
//!
//! Four cells: both accept, both reject, only Earley, only the parser.
//! Discrepancies are measured, not repaired. Semantic constraints that the
//! EBNF describes in prose, and special sequences `? … ?`, are not checked
//! by the recogniser, so "only Earley" is the expected superset. "Only the
//! parser" is a phrase the code accepts that the grammar does not describe
//! (comma skipping, reparse, and the rest of the semantic section).
//!
//! Set `GRAMMAR_DIFF_WRITE=1` to record `research/results/grammar-differential-v1.json`.
//! Without it the test compares the live counts to that file, if present.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sciwhisper_core::{interpret, research::math_token_classes, Domain, InterpretOptions, Node};
use sciwhisper_grammar::{
    ebnf::{parse, Grammar},
    recognise, Recognition,
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

const MODES: &[(&str, bool)] = &[("math", false), ("physics", true)];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load_math_ebnf() -> String {
    let path = repo_root().join("docs/grammar/math.ebnf");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn parser_accepts(text: &str, physics: bool) -> bool {
    let result = interpret(
        text,
        InterpretOptions {
            domain: if physics {
                Domain::Physics
            } else {
                Domain::Mathematics
            },
            allow_shortcuts: false,
        },
    );
    result.confidence > 0.0 && matches!(result.ast, Node::Math(_))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Cell {
    both_accept: u64,
    both_reject: u64,
    earley_only: u64,
    parser_only: u64,
    tokenize_fail: u64,
}

#[derive(Clone, Debug)]
struct Discrepancy {
    text: String,
    corpus: String,
    mode: String,
    kind: &'static str,
    tokens: Vec<String>,
    furthest: Option<usize>,
}

struct Run {
    cells: BTreeMap<String, BTreeMap<String, Cell>>,
    earley_only: Vec<Discrepancy>,
    parser_only: Vec<Discrepancy>,
}

fn earley_accepts(grammar: &Grammar, start: &str, tokens: &[String]) -> (bool, Option<usize>) {
    let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
    match recognise(grammar, start, &refs) {
        Recognition::Accepted => (true, None),
        Recognition::Rejected { furthest } => (false, Some(furthest)),
        Recognition::TooLong | Recognition::TooManyItems => (false, None),
    }
}

#[allow(clippy::too_many_arguments)]
fn compare_one(
    grammar: &Grammar,
    start: &str,
    corpus: &str,
    text: &str,
    mode: &str,
    physics: bool,
    cell: &mut Cell,
    earley_only: &mut Vec<Discrepancy>,
    parser_only: &mut Vec<Discrepancy>,
) {
    let tokens = match math_token_classes(text, physics) {
        Ok(t) => t,
        Err(_) => {
            cell.tokenize_fail += 1;
            return;
        }
    };
    let (earley, furthest) = earley_accepts(grammar, start, &tokens);
    let parsed = parser_accepts(text, physics);
    match (earley, parsed) {
        (true, true) => cell.both_accept += 1,
        (false, false) => cell.both_reject += 1,
        (true, false) => {
            cell.earley_only += 1;
            earley_only.push(Discrepancy {
                text: text.to_string(),
                corpus: corpus.to_string(),
                mode: mode.to_string(),
                kind: "earley_only",
                tokens,
                furthest,
            });
        }
        (false, true) => {
            cell.parser_only += 1;
            parser_only.push(Discrepancy {
                text: text.to_string(),
                corpus: corpus.to_string(),
                mode: mode.to_string(),
                kind: "parser_only",
                tokens,
                furthest,
            });
        }
    }
}

fn run_on_texts(
    grammar: &Grammar,
    start: &str,
    labelled: &[(&str, &str)],
) -> (Cell, Vec<Discrepancy>, Vec<Discrepancy>) {
    let mut cell = Cell::default();
    let mut earley_only = Vec::new();
    let mut parser_only = Vec::new();
    for (corpus, text) in labelled {
        compare_one(
            grammar,
            start,
            corpus,
            text,
            "math",
            false,
            &mut cell,
            &mut earley_only,
            &mut parser_only,
        );
    }
    (cell, earley_only, parser_only)
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

fn run_corpora(grammar: &Grammar, start: &str) -> Run {
    let mut cells: BTreeMap<String, BTreeMap<String, Cell>> = BTreeMap::new();
    let mut earley_only = Vec::new();
    let mut parser_only = Vec::new();
    let root = repo_root();
    for (corpus, rel) in CORPORA {
        let records = transcripts(&root.join(rel));
        assert!(
            !records.is_empty(),
            "{corpus} produced 0 records; the gate would pass on nothing"
        );
        let mut by_mode = BTreeMap::new();
        for (mode, physics) in MODES {
            let mut cell = Cell::default();
            for text in &records {
                compare_one(
                    grammar,
                    start,
                    corpus,
                    text,
                    mode,
                    *physics,
                    &mut cell,
                    &mut earley_only,
                    &mut parser_only,
                );
            }
            by_mode.insert((*mode).to_string(), cell);
        }
        cells.insert((*corpus).to_string(), by_mode);
    }
    Run {
        cells,
        earley_only,
        parser_only,
    }
}

fn discrepancy_json(d: &Discrepancy) -> serde_json::Value {
    serde_json::json!({
        "text": d.text,
        "corpus": d.corpus,
        "mode": d.mode,
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

fn report_json(grammar: &Grammar, start: &str, run: &Run) -> serde_json::Value {
    let mut cells = serde_json::Map::new();
    for (corpus, modes) in &run.cells {
        let mut mode_map = serde_json::Map::new();
        for (mode, cell) in modes {
            mode_map.insert(mode.clone(), cell_json(cell));
        }
        cells.insert(corpus.clone(), serde_json::Value::Object(mode_map));
    }
    serde_json::json!({
        "version": "grammar-differential-v1",
        "start": start,
        "syntactic_literals": grammar.syntactic_literals(),
        "note": "The recogniser does not enforce semantic constraints or special sequences; it accepts a superset. Syntactic literals, if any, match a token only when the literal text equals the token-class name.",
        "cells": cells,
        "earley_only": run.earley_only.iter().map(discrepancy_json).collect::<Vec<_>>(),
        "parser_only": run.parser_only.iter().map(discrepancy_json).collect::<Vec<_>>(),
    })
}

fn print_examples(title: &str, rows: &[Discrepancy]) {
    println!("{title}: {} total, showing up to 20", rows.len());
    for d in rows.iter().take(20) {
        println!(
            "  [{} {}] {:?} tokens={:?} furthest={:?}",
            d.corpus, d.mode, d.text, d.tokens, d.furthest
        );
    }
}

fn cells_only(value: &serde_json::Value) -> &serde_json::Value {
    value
        .get("cells")
        .unwrap_or_else(|| panic!("report is missing `cells`"))
}

#[test]
fn math_grammar_versus_the_handwritten_parser() {
    let src = load_math_ebnf();
    let grammar = parse(&src).unwrap_or_else(|e| panic!("math.ebnf: {e}"));
    let start = grammar
        .start()
        .expect("math.ebnf has a syntactic start rule")
        .to_string();
    assert_eq!(start, "math_input");
    assert!(
        grammar.syntactic_literals().is_empty(),
        "math.ebnf syntactic rules have no quoted literals; lexical phrases are not the recogniser's alphabet"
    );

    let run = run_corpora(&grammar, &start);
    print_examples("earley_only", &run.earley_only);
    print_examples("parser_only", &run.parser_only);
    for (corpus, modes) in &run.cells {
        for (mode, cell) in modes {
            println!(
                "{corpus}/{mode}: both_accept={} both_reject={} \
                 earley_only={} parser_only={} tokenize_fail={}",
                cell.both_accept,
                cell.both_reject,
                cell.earley_only,
                cell.parser_only,
                cell.tokenize_fail
            );
            let compared =
                cell.both_accept + cell.both_reject + cell.earley_only + cell.parser_only;
            assert!(
                cell.tokenize_fail > 0 || compared > 0,
                "{corpus}/{mode}: nothing was counted"
            );
        }
    }

    let live = report_json(&grammar, &start, &run);
    let path = repo_root().join("research/results/grammar-differential-v1.json");
    if std::env::var_os("GRAMMAR_DIFF_WRITE").is_some() {
        let pretty = serde_json::to_string_pretty(&live).expect("report JSON");
        fs::write(&path, pretty).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        println!("wrote {}", path.display());
        return;
    }
    if !path.exists() {
        panic!(
            "{} is missing; record it with \
             GRAMMAR_DIFF_WRITE=1 cargo test -p sciwhisper-grammar \
             --test differential -- --nocapture",
            path.display()
        );
    }
    let stored_text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let stored: serde_json::Value =
        serde_json::from_str(&stored_text).expect("stored differential report must parse");
    assert_eq!(
        cells_only(&live),
        cells_only(&stored),
        "cell counts diverged from {}; re-record with GRAMMAR_DIFF_WRITE=1 if the change is intended",
        path.display()
    );
}

#[test]
fn a_grammar_with_plus_removed_is_not_a_trivial_match() {
    let src = load_math_ebnf();
    let broken = src.replace(
        "add_op = PLUS | MINUS | PLUS_MINUS ;",
        "add_op = MINUS | PLUS_MINUS ;",
    );
    assert_ne!(
        src, broken,
        "the PLUS alternative was not found in math.ebnf; the sabotage did nothing"
    );
    let original = parse(&src).unwrap();
    let sabotaged = parse(&broken).unwrap();
    let start = original.start().unwrap();
    let phrases: &[(&str, &str)] = &[
        ("hand", "два плюс два"),
        ("hand", "икс плюс игрек"),
        ("hand", "два минус три"),
        ("hand", "икс"),
    ];
    let (before, _, before_parser_only) = run_on_texts(&original, start, phrases);
    let (after, _, after_parser_only) = run_on_texts(&sabotaged, start, phrases);
    assert!(
        before.both_accept > 0,
        "the small set should contain a phrase both sides accept, got {before:?}"
    );
    assert!(
        after.parser_only > before.parser_only,
        "removing PLUS from add_op must grow parser_only (before parser_only={} after={}); examples before={:?} after={:?}",
        before.parser_only,
        after.parser_only,
        before_parser_only
            .iter()
            .map(|d| d.text.as_str())
            .collect::<Vec<_>>(),
        after_parser_only
            .iter()
            .map(|d| d.text.as_str())
            .collect::<Vec<_>>()
    );
}

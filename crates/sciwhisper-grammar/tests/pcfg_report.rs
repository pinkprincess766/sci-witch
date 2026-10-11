//! PCFG over `docs/grammar/math.ebnf`: rule probabilities from the train
//! split of the four corpora, perplexity on the validation split.
//!
//! Records with split `dev_holdout` are skipped before tokenising. Only the
//! number of them is kept. Any split other than `train`, `validation` and
//! `dev_holdout` is a panic.
//!
//! Set `PCFG_WRITE=1` to record `research/results/pcfg-v1.json`. Without it
//! the test rebuilds the report and compares it to that file.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sciwhisper_core::research::math_token_classes;
use sciwhisper_grammar::{
    consistency::{check_consistency, Consistency},
    derivations,
    ebnf::{parse, Grammar},
    likelihood::{perplexity, sentence_log_prob},
    pcfg::{estimate, expected_counts, PSEUDOCOUNT},
    productions, DerivationOutcome, Production, Symbol,
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

/// Floats in the report are rounded to this many decimal places.
const REPORT_DECIMALS: i32 = 6;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Split {
    Train,
    Validation,
}

struct Record {
    split: Split,
    text: String,
}

/// Train and validation records of one corpus, and how many `dev_holdout`
/// records were skipped. A skipped record is not looked at beyond its split.
fn read_records(path: &Path) -> (Vec<Record>, u64) {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut records = Vec::new();
    let mut skipped = 0;
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("{}:{i}: {e}", path.display()));
        let split = match value.get("split").and_then(Value::as_str) {
            Some("dev_holdout") => {
                skipped += 1;
                continue;
            }
            Some("train") => Split::Train,
            Some("validation") => Split::Validation,
            other => panic!("{}:{i}: unknown split {other:?}", path.display()),
        };
        let transcript = value
            .get("human_transcript")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{}:{i}: missing human_transcript", path.display()));
        records.push(Record {
            split,
            text: transcript.to_string(),
        });
    }
    (records, skipped)
}

/// Sentences of one split that went through the model.
#[derive(Default)]
struct Sentences {
    total: u64,
    tokenize_fail: u64,
    rejected: u64,
    too_many: u64,
    too_long: u64,
    too_many_items: u64,
    /// Derivations of each used sentence.
    trees: Vec<Vec<Vec<usize>>>,
    /// Token count of each used sentence.
    tokens: Vec<usize>,
}

impl Sentences {
    fn add(&mut self, grammar: &Grammar, start: &str, text: &str, physics: bool) {
        self.total += 1;
        let Ok(tokens) = math_token_classes(text, physics) else {
            self.tokenize_fail += 1;
            return;
        };
        let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        match derivations(grammar, start, &refs) {
            DerivationOutcome::Derivations(trees) => {
                self.trees.push(trees);
                self.tokens.push(tokens.len());
            }
            DerivationOutcome::Rejected { .. } => self.rejected += 1,
            DerivationOutcome::TooMany => self.too_many += 1,
            DerivationOutcome::TooLong => self.too_long += 1,
            DerivationOutcome::TooManyItems => self.too_many_items += 1,
        }
    }

    fn total_tokens(&self) -> usize {
        self.tokens.iter().sum()
    }

    fn json(&self) -> Value {
        json!({
            "total": self.total,
            "tokenize_fail": self.tokenize_fail,
            "rejected": self.rejected,
            "too_many": self.too_many,
            "too_long": self.too_long,
            "too_many_items": self.too_many_items,
            "used": self.trees.len(),
            "tokens": self.total_tokens(),
        })
    }
}

/// Round to `REPORT_DECIMALS` places. Very large values (the uniform
/// baseline's norm is near 1e197) are kept as they are.
fn round_float(x: f64) -> Value {
    assert!(x.is_finite(), "report float is not finite: {x}");
    let scale = 10f64.powi(REPORT_DECIMALS);
    let rounded = if x.abs() >= 1e15 {
        x
    } else {
        (x * scale).round() / scale
    };
    json!(rounded)
}

fn consistency_json(result: &Consistency) -> Value {
    match result {
        Consistency::Proven { power, norm } => {
            json!({"proven": true, "power": power, "norm": round_float(*norm)})
        }
        Consistency::NotProven { norm } => {
            json!({"proven": false, "power": null, "norm": round_float(*norm)})
        }
    }
}

fn log_prob_total(prob: &[f64], sentences: &Sentences) -> f64 {
    sentences
        .trees
        .iter()
        .map(|trees| sentence_log_prob(prob, trees))
        .sum()
}

fn perplexity_json(total_log_prob: f64, sentences: &Sentences) -> Value {
    match perplexity(total_log_prob, sentences.total_tokens()) {
        Some(p) => round_float(p),
        None => Value::Null,
    }
}

fn rhs_text(prod: &Production) -> String {
    let names: Vec<&str> = prod
        .rhs
        .iter()
        .map(|sym| match sym {
            Symbol::Terminal(name) | Symbol::Nonterminal(name) => name.as_str(),
        })
        .collect();
    names.join(" ")
}

/// The trained probabilities of one mode, kept for the sum-to-one test.
struct ModeModel {
    mode: &'static str,
    trained: Vec<f64>,
}

struct Run {
    json: Value,
    models: Vec<ModeModel>,
}

const NOTE: &str = "The data are a few dozen sentences per mode, so this is a demonstration \
of the method, not a language model. A sentence with several derivation trees splits its count \
evenly between them; this is not EM. Probabilities are add-one smoothed, and perplexity is per \
token in token classes, natural log. The uniform baseline gives every alternative of a left-hand \
side the same probability; its consistency check comes out not proven with a huge norm, so it is \
probably not a proper distribution, and its perplexity is a score under a deficient model, not a \
fair baseline.";

fn run() -> Run {
    let src = load_math_ebnf();
    let grammar = parse(&src).unwrap_or_else(|e| panic!("math.ebnf: {e}"));
    let start = grammar
        .start()
        .expect("math.ebnf has a syntactic start rule")
        .to_string();
    let prods = productions(&grammar);
    let root = repo_root();

    let mut records: Vec<Record> = Vec::new();
    let mut skipped: BTreeMap<String, u64> = BTreeMap::new();
    for (name, rel) in CORPORA {
        let (corpus_records, corpus_skipped) = read_records(&root.join(rel));
        records.extend(corpus_records);
        skipped.insert(name.to_string(), corpus_skipped);
    }

    let mut modes = serde_json::Map::new();
    let mut models = Vec::new();
    for &(mode, physics) in MODES {
        let mut train = Sentences::default();
        let mut validation = Sentences::default();
        for record in &records {
            let target = match record.split {
                Split::Train => &mut train,
                Split::Validation => &mut validation,
            };
            target.add(&grammar, &start, &record.text, physics);
        }

        let counts = expected_counts(prods.len(), &train.trees)
            .unwrap_or_else(|e| panic!("{mode}: expected counts: {e}"));
        let trained = estimate(&prods, &counts, PSEUDOCOUNT);
        let uniform = estimate(&prods, &vec![0.0; prods.len()], 1.0);

        let trained_validation = log_prob_total(&trained, &validation);
        let uniform_validation = log_prob_total(&uniform, &validation);
        let trained_train = log_prob_total(&trained, &train);

        let unseen = validation
            .trees
            .iter()
            .filter(|trees| {
                trees
                    .iter()
                    .all(|tree| tree.iter().any(|&id| counts[id] == 0.0))
            })
            .count();

        let used_productions: Vec<Value> = prods
            .iter()
            .enumerate()
            .filter(|(id, _)| counts[*id] > 0.0)
            .map(|(id, prod)| {
                json!({
                    "id": id,
                    "lhs": prod.lhs,
                    "rhs": rhs_text(prod),
                    "train_count": round_float(counts[id]),
                    "prob": round_float(trained[id]),
                })
            })
            .collect();

        modes.insert(
            mode.to_string(),
            json!({
                "train": train.json(),
                "validation": validation.json(),
                "sentences_with_unseen_productions": unseen,
                "trained": {
                    "validation_log_prob": round_float(trained_validation),
                    "validation_perplexity": perplexity_json(trained_validation, &validation),
                    "train_log_prob": round_float(trained_train),
                    "train_perplexity": perplexity_json(trained_train, &train),
                    "consistency": consistency_json(&check_consistency(&prods, &trained, &start)),
                },
                "uniform": {
                    "validation_log_prob": round_float(uniform_validation),
                    "validation_perplexity": perplexity_json(uniform_validation, &validation),
                    "consistency": consistency_json(&check_consistency(&prods, &uniform, &start)),
                },
                "productions": used_productions,
            }),
        );
        models.push(ModeModel { mode, trained });
    }

    let corpora: Vec<&str> = CORPORA.iter().map(|(name, _)| *name).collect();
    let json = json!({
        "version": "pcfg-v1",
        "grammar": "docs/grammar/math.ebnf",
        "start": start,
        "n_productions": prods.len(),
        "pseudocount": PSEUDOCOUNT,
        "corpora": corpora,
        "splits": {"train": "train", "evaluation": "validation"},
        "dev_holdout_skipped": skipped,
        "modes": modes,
        "note": NOTE,
    });
    Run { json, models }
}

#[test]
fn pcfg_report_matches_the_committed_file() {
    let live = run().json;
    let path = repo_root().join("research/results/pcfg-v1.json");
    if std::env::var_os("PCFG_WRITE").is_some() {
        write_json(&path, &live);
    } else if !path.exists() {
        panic!(
            "{} is missing; record it with \
             PCFG_WRITE=1 cargo test -p sciwhisper-grammar --test pcfg_report -- --nocapture",
            path.display()
        );
    } else {
        let stored_text =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let stored: Value = serde_json::from_str(&stored_text).expect("stored report must parse");
        // serde_json without `float_roundtrip` can parse a float one ulp off
        // the printed value. Send the live report through the same text and
        // parser, so both sides carry the same parsing error.
        let live_text = serde_json::to_string_pretty(&live).expect("report JSON");
        let live_parsed: Value = serde_json::from_str(&live_text).expect("live report must parse");
        assert_eq!(
            &live_parsed,
            &stored,
            "PCFG report diverged from {}; re-record with PCFG_WRITE=1 if the change is intended",
            path.display()
        );
    }
}

/// Hand-counted `dev_holdout` lines: dev-seed-v3, ambiguous-v2,
/// nomenclature-v1, prose-negatives-v2.
const HAND_DEV_HOLDOUT: &[(&str, u64)] = &[
    ("dev-seed-v3", 21),
    ("ambiguous-v2", 9),
    ("nomenclature-v1", 0),
    ("prose-negatives-v2", 34),
];

/// Lines whose split is `dev_holdout`, counted without parsing JSON.
fn count_dev_holdout_lines(path: &Path) -> u64 {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let lines = text.lines().filter(|line| {
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        compact.contains("\"split\":\"dev_holdout\"")
    });
    lines.count() as u64
}

#[test]
fn dev_holdout_is_never_tokenised() {
    let live = run().json;
    let root = repo_root();
    for &(name, expected) in HAND_DEV_HOLDOUT {
        let (_, rel) = CORPORA
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap_or_else(|| panic!("{name} is not in CORPORA"));
        let path = root.join(rel);
        let lines = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count() as u64;
        assert_eq!(
            count_dev_holdout_lines(&path),
            expected,
            "{name}: hand number"
        );
        let (records, skipped) = read_records(&path);
        assert_eq!(skipped, expected, "{name}: skipped by the reader");
        assert_eq!(
            records.len() as u64,
            lines - expected,
            "{name}: the reader returned a dev_holdout record"
        );
        assert!(
            records
                .iter()
                .all(|r| matches!(r.split, Split::Train | Split::Validation)),
            "{name}: a record outside train/validation"
        );
        assert_eq!(
            live["dev_holdout_skipped"][name],
            json!(expected),
            "{name}: dev_holdout_skipped in the report"
        );
    }
    // Every record that was read is tokenised or counted in some bucket, so
    // the totals per mode are the train and validation records, nothing more.
    for &(mode, _) in MODES {
        let mode_json = &live["modes"][mode];
        let total = mode_json["train"]["total"].as_u64().expect("train total")
            + mode_json["validation"]["total"]
                .as_u64()
                .expect("validation total");
        let read: u64 = CORPORA
            .iter()
            .map(|(_, rel)| read_records(&root.join(rel)).0.len() as u64)
            .sum();
        assert_eq!(total, read, "{mode}: sentences seen by the model");
    }
}

#[test]
fn trained_probabilities_sum_to_one_per_lhs() {
    let src = load_math_ebnf();
    let grammar = parse(&src).unwrap_or_else(|e| panic!("math.ebnf: {e}"));
    let prods = productions(&grammar);
    for model in run().models {
        assert_eq!(model.trained.len(), prods.len(), "{}", model.mode);
        let mut sums: BTreeMap<&str, f64> = BTreeMap::new();
        for (prod, &p) in prods.iter().zip(&model.trained) {
            assert!(p > 0.0 && p <= 1.0, "{}: {} has {p}", model.mode, prod.lhs);
            *sums.entry(prod.lhs.as_str()).or_insert(0.0) += p;
        }
        for (lhs, sum) in &sums {
            assert!(
                (sum - 1.0).abs() <= 1e-9,
                "{}: {lhs} sums to {sum}",
                model.mode
            );
        }
    }
}

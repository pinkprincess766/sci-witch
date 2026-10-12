//! PCFG report v2 over `docs/grammar/math.ebnf`: the v1 setup (same corpora,
//! splits, modes and `dev_holdout` handling) with three models per mode: the
//! even-split model of v1, EM over the enumerated derivations, and the uniform
//! PCFG. Each carries the upper consistency check and the lower bound on the
//! spectral radius.
//!
//! Set `PCFG_V2_WRITE=1` to record `research/results/pcfg-v2.json`. Without
//! it the test rebuilds the report and compares it to that file.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sciwhisper_core::research::math_token_classes;
use sciwhisper_grammar::{
    consistency::{check_consistency, rho_lower_bound, unproductive_nonterminals, Consistency},
    derivations,
    ebnf::{parse, Grammar},
    likelihood::{perplexity, sentence_log_prob},
    pcfg::{em_estimate, estimate, expected_counts, EM_ITERATIONS, PSEUDOCOUNT},
    productions, DerivationOutcome, Production, Symbol,
};
use serde_json::{json, Map, Value};

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

fn rho_json(prob: &[f64], prods: &[Production], start: &str) -> Value {
    let rho = rho_lower_bound(prods, prob, start);
    json!({"power": rho.power, "bound": round_float(rho.bound)})
}

/// The numbers every model carries. Train and validation are scored under
/// the same probabilities.
fn model_json(
    prob: &[f64],
    prods: &[Production],
    start: &str,
    train: &Sentences,
    validation: &Sentences,
) -> Map<String, Value> {
    let train_log_prob = log_prob_total(prob, train);
    let validation_log_prob = log_prob_total(prob, validation);
    let mut model = Map::new();
    model.insert("train_log_prob".into(), round_float(train_log_prob));
    model.insert(
        "train_perplexity".into(),
        perplexity_json(train_log_prob, train),
    );
    model.insert(
        "validation_log_prob".into(),
        round_float(validation_log_prob),
    );
    model.insert(
        "validation_perplexity".into(),
        perplexity_json(validation_log_prob, validation),
    );
    model.insert(
        "consistency".into(),
        consistency_json(&check_consistency(prods, prob, start)),
    );
    model.insert("rho_lower_bound".into(), rho_json(prob, prods, start));
    model
}

/// What the tests need from one mode besides the JSON.
struct ModeModel {
    mode: &'static str,
    /// EM objective after each iteration, starting with the initial model.
    em_objective: Vec<f64>,
}

struct Run {
    json: Value,
    models: Vec<ModeModel>,
}

const NOTE: &str = "EM runs over the enumerated derivation trees of each sentence: a tree is \
weighted by its probability under the current model, and the M-step is add-one smoothing, so the \
fit is MAP under a Dirichlet prior and its objective, which includes that prior, never decreases. \
The even-split model is the start of EM and the model of v1. The uniform PCFG is improper by \
Booth and Thompson: the lower bound on the spectral radius of its mean-count matrix is above 1 \
and every nonterminal reachable from the start symbol is productive, so its perplexity is a score \
under a deficient model and not a baseline at all. The bounds are computed in f64 without \
interval arithmetic; the uniform bound is far above 1 compared with the rounding error. This is \
still a demonstration on a few dozen sentences per mode, not a language model.";

fn run() -> Run {
    let src = load_math_ebnf();
    let grammar = parse(&src).unwrap_or_else(|e| panic!("math.ebnf: {e}"));
    let start = grammar
        .start()
        .expect("math.ebnf has a syntactic start rule")
        .to_string();
    let prods = productions(&grammar);
    let root = repo_root();
    let unproductive = unproductive_nonterminals(&prods, &start);

    let mut records: Vec<Record> = Vec::new();
    let mut skipped: BTreeMap<String, u64> = BTreeMap::new();
    for (name, rel) in CORPORA {
        let (corpus_records, corpus_skipped) = read_records(&root.join(rel));
        records.extend(corpus_records);
        skipped.insert(name.to_string(), corpus_skipped);
    }

    let mut modes = Map::new();
    let mut models = Vec::new();
    let mut uniform_bounds = Vec::new();
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
        let even_split = estimate(&prods, &counts, PSEUDOCOUNT);
        let fit = em_estimate(&prods, &train.trees, PSEUDOCOUNT, EM_ITERATIONS)
            .unwrap_or_else(|e| panic!("{mode}: EM: {e}"));
        let uniform = estimate(&prods, &vec![0.0; prods.len()], 1.0);
        uniform_bounds.push(rho_lower_bound(&prods, &uniform, &start).bound);

        let unseen = validation
            .trees
            .iter()
            .filter(|trees| {
                trees
                    .iter()
                    .all(|tree| tree.iter().any(|&id| counts[id] == 0.0))
            })
            .count();

        let em_productions: Vec<Value> = prods
            .iter()
            .enumerate()
            .filter(|(id, _)| counts[*id] > 0.0)
            .map(|(id, prod)| {
                json!({
                    "id": id,
                    "lhs": prod.lhs,
                    "rhs": rhs_text(prod),
                    "prob": round_float(fit.prob[id]),
                })
            })
            .collect();

        let mut em_model = model_json(&fit.prob, &prods, &start, &train, &validation);
        em_model.insert(
            "objective_first".into(),
            round_float(*fit.objective.first().expect("objective is never empty")),
        );
        em_model.insert(
            "objective_last".into(),
            round_float(*fit.objective.last().expect("objective is never empty")),
        );
        em_model.insert("iterations".into(), json!(fit.objective.len() - 1));

        modes.insert(
            mode.to_string(),
            json!({
                "train": train.json(),
                "validation": validation.json(),
                "sentences_with_unseen_productions": unseen,
                "models": {
                    "even_split": model_json(&even_split, &prods, &start, &train, &validation),
                    "em": em_model,
                    "uniform": model_json(&uniform, &prods, &start, &train, &validation),
                },
                "em_productions": em_productions,
            }),
        );
        models.push(ModeModel {
            mode,
            em_objective: fit.objective,
        });
    }

    let uniform_improper = unproductive.is_empty() && uniform_bounds.iter().all(|&b| b > 1.0);
    let corpora: Vec<&str> = CORPORA.iter().map(|(name, _)| *name).collect();
    let json = json!({
        "version": "pcfg-v2",
        "grammar": "docs/grammar/math.ebnf",
        "start": start,
        "n_productions": prods.len(),
        "pseudocount": PSEUDOCOUNT,
        "em_iterations": EM_ITERATIONS,
        "corpora": corpora,
        "splits": {"train": "train", "evaluation": "validation"},
        "dev_holdout_skipped": skipped,
        "unproductive_nonterminals": unproductive,
        "uniform_improper": uniform_improper,
        "modes": modes,
        "note": NOTE,
    });
    Run { json, models }
}

/// serde_json without `float_roundtrip` can parse a float one ulp off the
/// printed value. Send a live report through the same text and parser as the
/// stored one, so both sides carry the same parsing error.
fn reparsed(live: &Value) -> Value {
    let text = serde_json::to_string_pretty(live).expect("report JSON");
    serde_json::from_str(&text).expect("live report must parse")
}

fn read_stored(rel: &str) -> Value {
    let path = repo_root().join(rel);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).expect("stored report must parse")
}

#[test]
fn pcfg_v2_report_matches_the_committed_file() {
    let live = run().json;
    let path = repo_root().join("research/results/pcfg-v2.json");
    if std::env::var_os("PCFG_V2_WRITE").is_some() {
        write_json(&path, &live);
    } else if !path.exists() {
        panic!(
            "{} is missing; record it with PCFG_V2_WRITE=1 cargo test -p sciwhisper-grammar \
             --test pcfg_report_v2 -- --nocapture",
            path.display()
        );
    } else {
        let stored = read_stored("research/results/pcfg-v2.json");
        assert_eq!(
            &reparsed(&live),
            &stored,
            "PCFG v2 report diverged from {}; re-record with PCFG_V2_WRITE=1 if the change is \
             intended",
            path.display()
        );
    }
}

#[test]
fn even_split_numbers_equal_pcfg_v1() {
    let live = reparsed(&run().json);
    let v1 = read_stored("research/results/pcfg-v1.json");
    let uniform_keys = [
        "validation_log_prob",
        "validation_perplexity",
        "consistency",
    ];
    let trained_keys = [
        "train_log_prob",
        "train_perplexity",
        "validation_log_prob",
        "validation_perplexity",
        "consistency",
    ];
    for &(mode, _) in MODES {
        for key in trained_keys {
            let old = &v1["modes"][mode]["trained"][key];
            // A missing key reads as null on both sides and would compare equal.
            assert!(!old.is_null(), "{mode}: v1 has no trained.{key}");
            assert_eq!(
                &live["modes"][mode]["models"]["even_split"][key], old,
                "{mode}: even_split.{key} differs from pcfg-v1"
            );
        }
        for key in uniform_keys {
            let old = &v1["modes"][mode]["uniform"][key];
            assert!(!old.is_null(), "{mode}: v1 has no uniform.{key}");
            assert_eq!(
                &live["modes"][mode]["models"]["uniform"][key], old,
                "{mode}: uniform.{key} differs from pcfg-v1"
            );
        }
    }
}

/// Slack for float noise in the objective; the real steps are far larger.
const OBJECTIVE_SLACK: f64 = 1e-9;

#[test]
fn em_objective_does_not_decrease_on_the_corpus() {
    for model in run().models {
        let objective = &model.em_objective;
        // One value for the start and one after each iteration. A run that
        // never iterated would be flat and pass the monotone check.
        assert_eq!(objective.len(), EM_ITERATIONS + 1, "{}", model.mode);
        for (i, pair) in objective.windows(2).enumerate() {
            assert!(
                pair[1] >= pair[0] - OBJECTIVE_SLACK,
                "{}: objective fell at iteration {}: {} -> {}",
                model.mode,
                i + 1,
                pair[0],
                pair[1]
            );
        }
        let first = objective[0];
        let last = objective[objective.len() - 1];
        assert!(
            last > first + OBJECTIVE_SLACK,
            "{}: EM did not improve on the even split: {first} -> {last}",
            model.mode
        );
    }
}

//! `sciwhisper-eval` — the research harness.
//!
//! It reads a versioned corpus, drives the real `sciwhisper-core`, and reports
//! where the quality is actually lost. It never modifies the application and
//! never ships a model into it.

mod candidates;
mod canonical;
mod evaluate;
mod metrics;
mod oracle;
mod report;
mod schema;
mod split;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use sciwhisper_core::Renderer;

use crate::candidates::DomainPolicy;
use crate::evaluate::{EvalConfig, RECALL_K_GRID};
use crate::metrics::recall_at_k;
use crate::report::{build_report, human_table, strip_timing, Inputs};
use crate::schema::Dataset;
use crate::split::audit_splits;

#[derive(Parser)]
#[command(
    name = "sciwhisper-eval",
    version,
    about = "Reproducible evaluation of SciWhisper on a versioned corpus"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SplitArg {
    All,
    Train,
    Validation,
    DevHoldout,
}

impl SplitArg {
    fn as_str(self) -> &'static str {
        match self {
            SplitArg::All => "all",
            SplitArg::Train => "train",
            SplitArg::Validation => "validation",
            SplitArg::DevHoldout => "dev_holdout",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum PolicyArg {
    Auto,
    AutoThenExplicit,
}

impl From<PolicyArg> for DomainPolicy {
    fn from(value: PolicyArg) -> Self {
        match value {
            PolicyArg::Auto => DomainPolicy::Auto,
            PolicyArg::AutoThenExplicit => DomainPolicy::AutoThenExplicit,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum RendererArg {
    Unicode,
    Latex,
    Omml,
}

impl From<RendererArg> for Renderer {
    fn from(value: RendererArg) -> Self {
        match value {
            RendererArg::Unicode => Renderer::Unicode,
            RendererArg::Latex => Renderer::Latex,
            RendererArg::Omml => Renderer::Omml,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Check the corpus against the schema and the split hygiene rules.
    ValidateDataset {
        #[arg(long)]
        dataset: PathBuf,
    },
    /// Run the full evaluation and write a machine-readable report.
    Evaluate {
        #[arg(long)]
        dataset: PathBuf,
        #[arg(long, value_enum, default_value = "all")]
        split: SplitArg,
        #[arg(long, default_value_t = 4)]
        k: usize,
        #[arg(long, default_value_t = 0.9)]
        threshold: f32,
        #[arg(long, value_enum, default_value = "unicode")]
        renderer: RendererArg,
        #[arg(long, value_enum, default_value = "auto")]
        policy: PolicyArg,
        #[arg(long, default_value_t = 2000)]
        bootstrap_resamples: usize,
        #[arg(long, default_value_t = 20_260_904)]
        bootstrap_seed: u64,
        #[arg(long)]
        output: Option<PathBuf>,
        /// Print only the JSON, without the human table.
        #[arg(long)]
        quiet: bool,
    },
    /// Print CandidateRecall@K for K = 1, 2, 4, 8, 16.
    RecallCurve {
        #[arg(long)]
        dataset: PathBuf,
        #[arg(long, value_enum, default_value = "all")]
        split: SplitArg,
        #[arg(long, value_enum, default_value = "auto")]
        policy: PolicyArg,
    },
    /// List the failures recorded in a report.
    InspectErrors {
        #[arg(long)]
        report: PathBuf,
        /// Only failures whose first blocking stage matches, e.g. `candidate-first`.
        #[arg(long)]
        stage: Option<String>,
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },
    /// Diff two reports, ignoring timing.
    Compare {
        #[arg(long)]
        baseline: PathBuf,
        #[arg(long)]
        candidate: PathBuf,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let cli = Cli::parse();
    match cli.command {
        Command::ValidateDataset { dataset } => {
            let (corpus, _) = load(&dataset)?;
            let audit = audit_splits(&corpus);
            println!(
                "{} records, {} families, schema version {}",
                corpus.records.len(),
                corpus.family_count(),
                schema::DATASET_SCHEMA_VERSION
            );
            for (split, count) in &audit.counts_by_split {
                println!("  split {split:<12} {count} records");
            }
            for (domain, count) in corpus.domain_counts() {
                println!("  domain {domain:<11} {count} records");
            }
            for (action, count) in corpus.action_counts() {
                println!("  action {action:<11} {count} records");
            }
            for (provenance, count) in corpus.provenance_counts() {
                println!("  provenance {provenance:<7} {count} records");
            }
            if audit.clean {
                println!("family/split leakage: none");
                Ok(ExitCode::SUCCESS)
            } else {
                for family in &audit.leaking_families {
                    eprintln!(
                        "family '{}' spans splits {:?} ({:?})",
                        family.family_id, family.splits, family.ids
                    );
                }
                Err("family leakage between splits".into())
            }
        }
        Command::Evaluate {
            dataset,
            split,
            k,
            threshold,
            renderer,
            policy,
            bootstrap_resamples,
            bootstrap_seed,
            output,
            quiet,
        } => {
            let (corpus, bytes) = load(&dataset)?;
            let selected = filter(&corpus, split)?;
            let config = EvalConfig {
                k,
                auto_insert_threshold: threshold,
                domain_policy: policy.into(),
                renderer: renderer.into(),
                bootstrap_seed,
                bootstrap_resamples,
            };
            let report = build_report(&Inputs {
                dataset: &corpus,
                dataset_path: &dataset,
                dataset_bytes: &bytes,
                selected,
                split_filter: split.as_str().to_string(),
                config,
            })?;
            let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            match &output {
                Some(path) => {
                    std::fs::write(path, format!("{json}\n")).map_err(|e| e.to_string())?;
                    if !quiet {
                        println!("{}", human_table(&report));
                        println!("report written to {}", display(path));
                    }
                }
                None => println!("{json}"),
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::RecallCurve {
            dataset,
            split,
            policy,
        } => {
            let (corpus, _) = load(&dataset)?;
            let selected = filter(&corpus, split)?;
            let config = EvalConfig {
                domain_policy: policy.into(),
                ..EvalConfig::default()
            };
            let mut ranks = Vec::new();
            for record in &selected {
                let target = evaluate::gold_target(record)?;
                let key =
                    canonical::canonical_target_v1(&target).map_err(|error| error.to_string())?;
                let run = evaluate::run_pipeline(
                    &evaluate::system_hypotheses(record),
                    &key,
                    &config,
                    &[],
                    evaluate::Selector::Deterministic,
                    None,
                );
                ranks.push(run.gold_rank);
            }
            println!("CandidateRecall@K over {} examples", ranks.len());
            for k in RECALL_K_GRID {
                let p = recall_at_k(&ranks, k);
                match p.value {
                    Some(value) => println!(
                        "  @{k:<3} {:6.1}%  [{}/{}]  95% CI [{:.3}, {:.3}]",
                        value * 100.0,
                        p.numerator,
                        p.denominator,
                        p.ci95_low.unwrap_or(0.0),
                        p.ci95_high.unwrap_or(1.0)
                    ),
                    None => println!("  @{k:<3} n/a"),
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::InspectErrors {
            report,
            stage,
            limit,
        } => {
            let text = std::fs::read_to_string(&report)
                .map_err(|error| format!("{}: {error}", display(&report)))?;
            let value: serde_json::Value =
                serde_json::from_str(&text).map_err(|error| error.to_string())?;
            let empty = Vec::new();
            let errors = value["errors"].as_array().unwrap_or(&empty);
            let mut shown = 0usize;
            for entry in errors {
                let entry_stage = entry["first_blocking"].as_str().unwrap_or("none");
                if let Some(wanted) = &stage {
                    if entry_stage != wanted {
                        continue;
                    }
                }
                if shown >= limit {
                    println!("… {} more", errors.len() - shown);
                    break;
                }
                shown += 1;
                println!(
                    "{:<28} {:<16} {:<4} expected {:?}\n{:<28} {:<16} {:<4} produced {:?}",
                    entry["id"].as_str().unwrap_or("?"),
                    entry_stage,
                    entry["severity"].as_str().unwrap_or("-"),
                    entry["expected"].as_str().unwrap_or(""),
                    "",
                    "",
                    "",
                    entry["produced"].as_str().unwrap_or("")
                );
            }
            if shown == 0 {
                println!("no matching failures");
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Compare {
            baseline,
            candidate,
        } => {
            let read = |path: &Path| -> Result<serde_json::Value, String> {
                let text = std::fs::read_to_string(path)
                    .map_err(|error| format!("{}: {error}", display(path)))?;
                serde_json::from_str(&text).map_err(|error| error.to_string())
            };
            let mut a = read(&baseline)?;
            let mut b = read(&candidate)?;
            for value in [&mut a, &mut b] {
                strip_timing(value);
            }
            let headline = [
                "ast_exact_match",
                "overall_exact_match",
                "auto_insert_exact_match",
                "coverage",
                "routing_accuracy",
                "raw_accuracy",
                "false_scientific_rewrite_rate",
                "false_abstention_rate",
                "ast_validity",
            ];
            println!(
                "{:<34}{:>12}{:>12}{:>10}",
                "metric", "baseline", "candidate", "Δ pp"
            );
            for key in headline {
                let left = a["metrics"][key]["value"].as_f64();
                let right = b["metrics"][key]["value"].as_f64();
                let delta = match (left, right) {
                    (Some(left), Some(right)) => format!("{:+.1}", (right - left) * 100.0),
                    _ => "n/a".into(),
                };
                println!(
                    "{key:<34}{:>12}{:>12}{:>10}",
                    left.map(|v| format!("{:.3}", v))
                        .unwrap_or_else(|| "n/a".into()),
                    right
                        .map(|v| format!("{:.3}", v))
                        .unwrap_or_else(|| "n/a".into()),
                    delta
                );
            }
            if a == b {
                println!("\nreports are identical once timing is ignored");
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn load(path: &Path) -> Result<(Dataset, Vec<u8>), String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", display(path)))?;
    let text = String::from_utf8(bytes.clone())
        .map_err(|error| format!("{}: not UTF-8: {error}", display(path)))?;
    let corpus = Dataset::parse_jsonl(&text).map_err(|error| error.to_string())?;
    Ok((corpus, bytes))
}

fn filter(corpus: &Dataset, split: SplitArg) -> Result<Vec<&schema::Record>, String> {
    let selected: Vec<&schema::Record> = corpus
        .records
        .iter()
        .filter(|record| match split {
            SplitArg::All => true,
            other => record.split.as_str() == other.as_str(),
        })
        .collect();
    if selected.is_empty() {
        return Err(format!("split '{}' selects no records", split.as_str()));
    }
    Ok(selected)
}

/// Never prints an absolute path into a report; this is for terminal messages
/// only.
fn display(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_carries_no_absolute_path() {
        let corpus_text = r#"{"dataset_schema_version":1,"id":"plain-001-a","family_id":"plain-001","provenance":"handcrafted_text","human_transcript":"предел терпения","asr_hypotheses":[],"target_domain":"plain","target_action":"raw","target_ast":null,"split":"train","tags":["raw"],"speaker_id":null}"#
            .to_string();
        let corpus = Dataset::parse_jsonl(&corpus_text).unwrap();
        let path = PathBuf::from("/somewhere/absolute/dev-seed-v1.jsonl");
        let report = build_report(&Inputs {
            dataset: &corpus,
            dataset_path: &path,
            dataset_bytes: corpus_text.as_bytes(),
            selected: corpus.records.iter().collect(),
            split_filter: "all".into(),
            config: EvalConfig::default(),
        })
        .expect("report builds");
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("/somewhere/absolute"), "{json}");
        assert!(json.contains("dev-seed-v1.jsonl"));
        assert!(!json.contains(env!("CARGO_MANIFEST_DIR")));
    }

    #[test]
    fn two_runs_of_the_same_configuration_agree_apart_from_timing() {
        let corpus_text = [
            r#"{"dataset_schema_version":1,"id":"plain-001-a","family_id":"plain-001","provenance":"handcrafted_text","human_transcript":"предел терпения","asr_hypotheses":[],"target_domain":"plain","target_action":"raw","target_ast":null,"split":"train","tags":["raw"],"speaker_id":null}"#,
            r#"{"dataset_schema_version":1,"id":"chem-water-001-a","family_id":"chem-water-001","provenance":"handcrafted_text","human_transcript":"вода","asr_hypotheses":[],"target_domain":"chemistry","target_action":"ast","target_ast":{"Chemical":{"Species":{"coefficient":1,"formula":{"parts":[{"Atom":{"symbol":"H","count":2}},{"Atom":{"symbol":"O","count":1}}]},"charge":null,"marker":null}}},"split":"train","tags":["formula"],"speaker_id":null}"#,
        ]
        .join("\n");
        let corpus = Dataset::parse_jsonl(&corpus_text).unwrap();
        let path = PathBuf::from("dev-seed-v1.jsonl");
        let build = || {
            build_report(&Inputs {
                dataset: &corpus,
                dataset_path: &path,
                dataset_bytes: corpus_text.as_bytes(),
                selected: corpus.records.iter().collect(),
                split_filter: "all".into(),
                config: EvalConfig::default(),
            })
            .expect("report builds")
        };
        let first = crate::report::without_timing(&build()).unwrap();
        let second = crate::report::without_timing(&build()).unwrap();
        assert_eq!(first, second);
    }
}

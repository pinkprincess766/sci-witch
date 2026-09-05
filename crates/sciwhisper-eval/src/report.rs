//! Machine-readable report plus a compact human table.
//!
//! The report records the dataset digest, the schema versions, the program
//! version and the commit it ran from, so that a number can always be traced
//! back to the exact inputs that produced it. Paths are stored as bare file
//! names: an absolute path from one machine is noise everywhere else.

use std::collections::BTreeMap;
use std::path::Path;

use sciwhisper_core::Renderer;
use serde::Serialize;

use crate::canonical::CANONICAL_SCHEMA_VERSION;
use crate::evaluate::{evaluate_record, ErrorStage, EvalConfig, ExampleOutcome, RECALL_K_GRID};
use crate::metrics::{
    bootstrap_proportion, percentile_u64, proportion, recall_at_k, severity_report, Proportion,
    SeverityReport,
};
use crate::oracle::{
    bottleneck_table, component_isolation, first_blocking, oracle_replacement, BottleneckTable,
    ComponentIsolation, OracleReplacement,
};
use crate::schema::{Dataset, Record, TargetAction, DATASET_SCHEMA_VERSION};
use crate::split::{audit_splits, SplitAudit};

pub const REPORT_SCHEMA_VERSION: u32 = 1;
pub const SEVERITY_SCHEMA_VERSION: u32 = 1;
pub const BASELINE_ID: &str = "deterministic-v1";

#[derive(Clone, Debug, Serialize)]
pub struct ProgramInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub git_commit: String,
    pub git_dirty: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct DatasetInfo {
    /// Bare file name only; an absolute path would not reproduce elsewhere.
    pub file: String,
    pub sha256: String,
    pub dataset_schema_version: u32,
    pub records: usize,
    pub families: usize,
    pub counts_by_split: BTreeMap<String, usize>,
    pub counts_by_domain: BTreeMap<String, usize>,
    pub counts_by_action: BTreeMap<String, usize>,
    pub counts_by_provenance: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConfigInfo {
    pub baseline_id: &'static str,
    pub k: usize,
    pub recall_k_grid: Vec<usize>,
    pub auto_insert_threshold: f32,
    pub domain_policy: &'static str,
    pub primary_renderer: &'static str,
    pub bootstrap_seed: u64,
    pub bootstrap_resamples: usize,
    pub evaluated_split: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CandidateStats {
    pub p50: Option<u64>,
    pub p95: Option<u64>,
    pub max: Option<u64>,
    pub examples_with_more_than_one_candidate: usize,
    pub examples_with_more_than_one_ast_candidate: usize,
}

/// Whether the corpus can support a *learned* reranker at all.
///
/// A ranker can only earn its keep where more than one distinct scientific
/// answer is on offer and the right one is not already first. A model trained
/// where the choice is always "the single AST or RAW" is not a reranker: it is
/// an abstention gate wearing a reranker's name, and it would report a
/// flattering accuracy for solving a different problem.
///
/// The admission rule is fixed here, before the numbers are looked at.
#[derive(Clone, Debug, Serialize)]
pub struct RerankerReadiness {
    pub scientific_examples: usize,
    pub examples_with_at_least_two_distinct_ast_candidates: usize,
    pub examples_where_gold_is_present_but_not_first: usize,
    pub ambiguous_by_split: BTreeMap<String, usize>,
    pub required_ambiguous_examples: usize,
    pub required_gold_not_first: usize,
    pub required_per_split: usize,
    pub verdict: &'static str,
    pub note: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Metrics {
    pub examples: usize,
    /// Selection correctness before the insert/abstain decision.
    pub ast_exact_match: Proportion,
    /// The payload the user would receive, per renderer, before any OS
    /// insertion. The protocol's EndToEndExactMatch also covers the Word
    /// insertion step, which this harness does not exercise.
    pub pre_insertion_end_to_end_exact_match: BTreeMap<String, Proportion>,
    pub auto_insert_exact_match: Proportion,
    pub coverage: Proportion,
    pub overall_exact_match: Proportion,
    pub overall_exact_match_bootstrap: Proportion,
    pub candidate_recall_at_k: BTreeMap<String, Proportion>,
    pub routing_accuracy: Proportion,
    pub raw_accuracy: Proportion,
    pub false_scientific_rewrite_rate: Proportion,
    pub false_abstention_rate: Proportion,
    pub ast_validity: Proportion,
    pub candidates: CandidateStats,
    pub reranker_readiness: RerankerReadiness,
}

#[derive(Clone, Debug, Serialize)]
pub struct Breakdown {
    pub by_domain: BTreeMap<String, Proportion>,
    pub by_tag: BTreeMap<String, Proportion>,
    /// Compact `[correct, total]` per family, so the file stays small.
    pub by_family: BTreeMap<String, [usize; 2]>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ErrorEntry {
    pub id: String,
    pub domain: String,
    pub target_action: &'static str,
    pub first_blocking: String,
    pub severity: Option<String>,
    pub transcript: String,
    pub expected: String,
    pub produced: String,
    pub candidates: usize,
    pub gold_rank: Option<usize>,
    pub selected_key: Option<String>,
    pub selected_source: Option<&'static str>,
    pub selected_warnings: Vec<String>,
    pub requested_domain: Option<String>,
    pub hypothesis_index: Option<usize>,
    /// `None` when the corpus fixes no expected rendering for this record.
    pub renderer_faithful_to_gold: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Timing {
    pub latency_us_p50: Option<u64>,
    pub latency_us_p95: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub report_schema_version: u32,
    pub canonical_schema_version: u32,
    pub severity_schema_version: u32,
    pub program: ProgramInfo,
    pub dataset: DatasetInfo,
    pub config: ConfigInfo,
    pub split_audit: SplitAudit,
    pub metrics: Metrics,
    pub breakdown: Breakdown,
    pub error_decomposition: BottleneckTable,
    pub oracle_replacement: Vec<OracleReplacement>,
    pub component_isolation: ComponentIsolation,
    pub severity: SeverityReport,
    pub errors: Vec<ErrorEntry>,
    pub notes: Vec<String>,
    /// Everything that legitimately differs between two runs of the same
    /// configuration lives here, and nowhere else.
    pub timing: Timing,
}

pub struct Inputs<'a> {
    pub dataset: &'a Dataset,
    pub dataset_path: &'a Path,
    pub dataset_bytes: &'a [u8],
    pub selected: Vec<&'a Record>,
    pub split_filter: String,
    pub config: EvalConfig,
}

pub fn build_report(inputs: &Inputs<'_>) -> Result<Report, String> {
    let config = &inputs.config;
    let mut outcomes: Vec<ExampleOutcome> = Vec::new();
    for record in &inputs.selected {
        let mut outcome = evaluate_record(record, config)?;
        outcome.first_blocking = first_blocking(record, config);
        outcomes.push(outcome);
    }

    let metrics = compute_metrics(&inputs.selected, &outcomes, config)?;
    let breakdown = compute_breakdown(&outcomes);
    let stages: Vec<Option<ErrorStage>> = outcomes
        .iter()
        .map(|outcome| outcome.first_blocking)
        .collect();
    let severities: Vec<_> = outcomes.iter().map(|outcome| outcome.severity).collect();

    let errors = outcomes
        .iter()
        .filter(|outcome| !outcome.payload_correct || outcome.severity.is_some())
        .map(|outcome| ErrorEntry {
            id: outcome.id.clone(),
            domain: outcome.domain.clone(),
            target_action: outcome.target_action.as_str(),
            first_blocking: outcome
                .first_blocking
                .map(ErrorStage::as_str)
                .unwrap_or("none")
                .to_string(),
            severity: outcome
                .severity
                .map(|severity| severity.as_str().to_string()),
            transcript: outcome.transcript.clone(),
            expected: outcome.expected_payload.clone(),
            produced: outcome.emitted_payload.clone(),
            candidates: outcome.candidate_count,
            gold_rank: outcome.gold_rank,
            selected_key: outcome.selected_key.clone(),
            selected_source: outcome.selected_source,
            selected_warnings: outcome.selected_warnings.clone(),
            requested_domain: outcome.requested_domain.clone(),
            hypothesis_index: outcome.hypothesis_index,
            renderer_faithful_to_gold: outcome.render_match,
        })
        .collect();

    let mut latencies: Vec<u64> = outcomes.iter().map(|outcome| outcome.latency_us).collect();

    Ok(Report {
        report_schema_version: REPORT_SCHEMA_VERSION,
        canonical_schema_version: CANONICAL_SCHEMA_VERSION,
        severity_schema_version: SEVERITY_SCHEMA_VERSION,
        program: program_info(),
        dataset: DatasetInfo {
            file: inputs
                .dataset_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".into()),
            sha256: sha256_hex(inputs.dataset_bytes),
            dataset_schema_version: DATASET_SCHEMA_VERSION,
            records: inputs.dataset.records.len(),
            families: inputs.dataset.family_count(),
            counts_by_split: to_string_map(inputs.dataset.split_counts()),
            counts_by_domain: inputs.dataset.domain_counts(),
            counts_by_action: to_string_map(inputs.dataset.action_counts()),
            counts_by_provenance: to_string_map(inputs.dataset.provenance_counts()),
        },
        config: ConfigInfo {
            baseline_id: BASELINE_ID,
            k: config.k,
            recall_k_grid: RECALL_K_GRID.to_vec(),
            auto_insert_threshold: config.auto_insert_threshold,
            domain_policy: config.domain_policy.as_str(),
            primary_renderer: renderer_name(config.renderer),
            bootstrap_seed: config.bootstrap_seed,
            bootstrap_resamples: config.bootstrap_resamples,
            evaluated_split: inputs.split_filter.clone(),
        },
        split_audit: audit_splits(inputs.dataset),
        metrics,
        breakdown,
        error_decomposition: bottleneck_table(inputs.dataset, &stages),
        oracle_replacement: oracle_replacement(&inputs.selected, config),
        component_isolation: component_isolation(&inputs.selected, config),
        severity: severity_report(&severities),
        errors,
        notes: notes(),
        timing: Timing {
            latency_us_p50: percentile_u64(&mut latencies, 50.0),
            latency_us_p95: percentile_u64(&mut latencies, 95.0),
        },
    })
}

fn compute_metrics(
    records: &[&Record],
    outcomes: &[ExampleOutcome],
    config: &EvalConfig,
) -> Result<Metrics, String> {
    let examples = outcomes.len();
    let selection_hits = outcomes
        .iter()
        .filter(|outcome| outcome.selection_correct)
        .count();
    let payload_hits = outcomes
        .iter()
        .filter(|outcome| outcome.payload_correct)
        .count();
    let indicators: Vec<bool> = outcomes
        .iter()
        .map(|outcome| outcome.payload_correct)
        .collect();

    let auto_inserted: Vec<&ExampleOutcome> = outcomes
        .iter()
        .filter(|outcome| outcome.action == crate::evaluate::SystemAction::AutoInsert)
        .collect();
    let auto_insert_hits = auto_inserted
        .iter()
        .filter(|outcome| outcome.payload_correct)
        .count();

    let ranks: Vec<Option<usize>> = outcomes.iter().map(|outcome| outcome.gold_rank).collect();
    let mut candidate_recall_at_k = BTreeMap::new();
    for k in RECALL_K_GRID {
        candidate_recall_at_k.insert(format!("{k}"), recall_at_k(&ranks, k));
    }

    let routing: Vec<bool> = outcomes
        .iter()
        .filter_map(|outcome| outcome.routing_correct)
        .collect();

    let raw_records: Vec<&ExampleOutcome> = outcomes
        .iter()
        .filter(|outcome| outcome.target_action == TargetAction::Raw)
        .collect();
    let raw_kept = raw_records
        .iter()
        .filter(|outcome| !outcome.emitted.is_ast())
        .count();
    let false_rewrites = raw_records.len() - raw_kept;

    let ast_records: Vec<&ExampleOutcome> = outcomes
        .iter()
        .filter(|outcome| outcome.target_action == TargetAction::Ast)
        .collect();
    let false_abstentions = ast_records
        .iter()
        .filter(|outcome| !outcome.emitted.is_ast())
        .count();

    let emitted_asts: Vec<&ExampleOutcome> = outcomes
        .iter()
        .filter(|outcome| outcome.emitted.is_ast())
        .collect();
    let valid_asts = emitted_asts
        .iter()
        .filter(|outcome| outcome.structurally_valid)
        .count();

    let counts: Vec<u64> = outcomes
        .iter()
        .map(|outcome| outcome.candidate_count as u64)
        .collect();
    let ambiguous = outcomes
        .iter()
        .filter(|outcome| outcome.candidate_count > 1)
        .count();

    // How many examples the ranker could ever have a say about: more than one
    // *scientific* answer on offer, not merely "an AST and RAW".
    let mut ambiguous_ast = 0usize;
    let mut ambiguous_by_split: BTreeMap<String, usize> = BTreeMap::new();
    for record in records {
        let transcripts = crate::evaluate::system_hypotheses(record);
        let candidates = crate::candidates::generate_candidates(
            &transcripts,
            config.domain_policy,
            crate::evaluate::recall_k_max(),
        );
        if candidates.iter().filter(|c| !c.is_raw()).count() > 1 {
            ambiguous_ast += 1;
            *ambiguous_by_split
                .entry(record.split.as_str().to_string())
                .or_insert(0) += 1;
        }
    }
    let gold_not_first = outcomes
        .iter()
        .filter(|outcome| outcome.gold_rank.is_some_and(|rank| rank > 1))
        .count();
    let scientific_examples = ast_records.len();
    const REQUIRED_AMBIGUOUS: usize = 50;
    const REQUIRED_GOLD_NOT_FIRST: usize = 20;
    const REQUIRED_PER_SPLIT: usize = 10;
    let enough_per_split = ["train", "validation"]
        .iter()
        .all(|split| ambiguous_by_split.get(*split).copied().unwrap_or(0) >= REQUIRED_PER_SPLIT);
    let sufficient = ambiguous_ast >= REQUIRED_AMBIGUOUS
        && gold_not_first >= REQUIRED_GOLD_NOT_FIRST
        && enough_per_split;
    let readiness = RerankerReadiness {
        scientific_examples,
        examples_with_at_least_two_distinct_ast_candidates: ambiguous_ast,
        examples_where_gold_is_present_but_not_first: gold_not_first,
        ambiguous_by_split,
        required_ambiguous_examples: REQUIRED_AMBIGUOUS,
        required_gold_not_first: REQUIRED_GOLD_NOT_FIRST,
        required_per_split: REQUIRED_PER_SPLIT,
        verdict: if sufficient {
            "sufficient"
        } else {
            "insufficient"
        },
        note: if sufficient {
            "The corpus offers enough genuinely ambiguous examples to train and validate a ranking model.".into()
        } else {
            format!(
                "A logistic reranker is statistically meaningless on this corpus: {ambiguous_ast} examples offer more than one distinct scientific AST and {gold_not_first} have the right answer outside first place, against a pre-registered requirement of {REQUIRED_AMBIGUOUS} and {REQUIRED_GOLD_NOT_FIRST}. Training here would only learn to tell a single AST from RAW."
            )
        },
    };

    // The payload metric per renderer: the same decisions, rendered three ways.
    let mut pre_insertion = BTreeMap::new();
    for renderer in [Renderer::Unicode, Renderer::Latex, Renderer::Omml] {
        let mut renderer_config = config.clone();
        renderer_config.renderer = renderer;
        let mut hits = 0usize;
        for record in records {
            let outcome = evaluate_record(record, &renderer_config)?;
            if outcome.payload_correct {
                hits += 1;
            }
        }
        pre_insertion.insert(
            renderer_name(renderer).to_string(),
            proportion(hits, records.len()),
        );
    }

    Ok(Metrics {
        examples,
        ast_exact_match: proportion(selection_hits, examples),
        pre_insertion_end_to_end_exact_match: pre_insertion,
        auto_insert_exact_match: proportion(auto_insert_hits, auto_inserted.len()),
        coverage: proportion(auto_inserted.len(), examples),
        overall_exact_match: proportion(payload_hits, examples),
        overall_exact_match_bootstrap: bootstrap_proportion(
            &indicators,
            config.bootstrap_seed,
            config.bootstrap_resamples,
        ),
        candidate_recall_at_k,
        routing_accuracy: proportion(routing.iter().filter(|hit| **hit).count(), routing.len()),
        raw_accuracy: proportion(raw_kept, raw_records.len()),
        false_scientific_rewrite_rate: proportion(false_rewrites, raw_records.len()),
        false_abstention_rate: proportion(false_abstentions, ast_records.len()),
        ast_validity: proportion(valid_asts, emitted_asts.len()),
        reranker_readiness: readiness,
        candidates: CandidateStats {
            p50: percentile_u64(&mut counts.clone(), 50.0),
            p95: percentile_u64(&mut counts.clone(), 95.0),
            max: counts.iter().copied().max(),
            examples_with_more_than_one_candidate: ambiguous,
            examples_with_more_than_one_ast_candidate: ambiguous_ast,
        },
    })
}

fn compute_breakdown(outcomes: &[ExampleOutcome]) -> Breakdown {
    let mut by_domain: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut by_tag: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut by_family: BTreeMap<String, [usize; 2]> = BTreeMap::new();
    for outcome in outcomes {
        let entry = by_domain.entry(outcome.domain.clone()).or_insert((0, 0));
        entry.1 += 1;
        if outcome.payload_correct {
            entry.0 += 1;
        }
        for tag in &outcome.tags {
            let entry = by_tag.entry(tag.clone()).or_insert((0, 0));
            entry.1 += 1;
            if outcome.payload_correct {
                entry.0 += 1;
            }
        }
        let entry = by_family.entry(outcome.family_id.clone()).or_insert([0, 0]);
        entry[1] += 1;
        if outcome.payload_correct {
            entry[0] += 1;
        }
    }
    Breakdown {
        by_domain: by_domain
            .into_iter()
            .map(|(key, (hits, total))| (key, proportion(hits, total)))
            .collect(),
        by_tag: by_tag
            .into_iter()
            .map(|(key, (hits, total))| (key, proportion(hits, total)))
            .collect(),
        by_family,
    }
}

fn notes() -> Vec<String> {
    vec![
        "This is a text-level development corpus. There are no real voices in it, and ASR accuracy is not measured here.".into(),
        "pre_insertion_end_to_end_exact_match stops before the operating-system insertion step. The Word COM path is not exercised by this harness and must not be called verified.".into(),
        "The confidence produced by sciwhisper-core is a deterministic parse level, not a calibrated probability. No metric here treats it as one.".into(),
        "Oracle replacement deltas answer the product question and are not additive; component isolation answers the laboratory question and is a different quantity.".into(),
        "An observed count of zero for a rare safety error is reported with its exact one-sided 95% upper bound, never as a proven zero.".into(),
    ]
}

pub fn renderer_name(renderer: Renderer) -> &'static str {
    match renderer {
        Renderer::Unicode => "unicode",
        Renderer::Latex => "latex",
        Renderer::Omml => "omml",
    }
}

fn to_string_map(map: BTreeMap<&'static str, usize>) -> BTreeMap<String, usize> {
    map.into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn program_info() -> ProgramInfo {
    let commit = run_git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let dirty = run_git(&["status", "--porcelain"])
        .map(|out| !out.trim().is_empty())
        .unwrap_or(false);
    ProgramInfo {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        git_commit: commit,
        git_dirty: dirty,
    }
}

fn run_git(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// --------------------------------------------------------------- sha-256

/// Minimal SHA-256, so the corpus digest needs no dependency.
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in message.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (index, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[index] = u32::from_be_bytes(*word);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}

/// Compact table for a human reading the terminal.
pub fn human_table(report: &Report) -> String {
    let show = |p: &Proportion| match p.value {
        None => "     n/a".to_string(),
        Some(value) => format!(
            "{:6.1}%  [{}/{}]",
            value * 100.0,
            p.numerator,
            p.denominator
        ),
    };
    let mut out = String::new();
    out.push_str(&format!(
        "dataset {} ({} records, {} families, sha256 {}…)\n",
        report.dataset.file,
        report.dataset.records,
        report.dataset.families,
        &report.dataset.sha256[..12]
    ));
    out.push_str(&format!(
        "baseline {}  split {}  K={}  threshold {}  policy {}\n\n",
        report.config.baseline_id,
        report.config.evaluated_split,
        report.config.k,
        report.config.auto_insert_threshold,
        report.config.domain_policy
    ));
    out.push_str(&format!(
        "{:<38}{}\n",
        "AST exact match",
        show(&report.metrics.ast_exact_match)
    ));
    out.push_str(&format!(
        "{:<38}{}\n",
        "overall exact match (pre-insertion)",
        show(&report.metrics.overall_exact_match)
    ));
    out.push_str(&format!(
        "{:<38}{}\n",
        "auto-insert exact match",
        show(&report.metrics.auto_insert_exact_match)
    ));
    out.push_str(&format!(
        "{:<38}{}\n",
        "coverage",
        show(&report.metrics.coverage)
    ));
    out.push_str(&format!(
        "{:<38}{}\n",
        "routing accuracy",
        show(&report.metrics.routing_accuracy)
    ));
    out.push_str(&format!(
        "{:<38}{}\n",
        "RAW accuracy",
        show(&report.metrics.raw_accuracy)
    ));
    out.push_str(&format!(
        "{:<38}{}",
        "false scientific rewrite rate",
        show(&report.metrics.false_scientific_rewrite_rate)
    ));
    if let Some(bound) = report
        .metrics
        .false_scientific_rewrite_rate
        .zero_count_upper95
    {
        out.push_str(&format!("  (95% upper bound {:.3})", bound));
    }
    out.push('\n');
    out.push_str(&format!(
        "{:<38}{}\n",
        "false abstention rate",
        show(&report.metrics.false_abstention_rate)
    ));
    out.push_str(&format!(
        "{:<38}{}\n\n",
        "AST validity",
        show(&report.metrics.ast_validity)
    ));

    out.push_str("CandidateRecall@K\n");
    for k in RECALL_K_GRID {
        if let Some(p) = report.metrics.candidate_recall_at_k.get(&format!("{k}")) {
            out.push_str(&format!("  @{k:<35}{}\n", show(p)));
        }
    }
    out.push('\n');

    out.push_str(&format!(
        "reranker readiness: {} ({} examples with >1 distinct AST, {} with gold not first)\n\n",
        report.metrics.reranker_readiness.verdict,
        report
            .metrics
            .reranker_readiness
            .examples_with_at_least_two_distinct_ast_candidates,
        report
            .metrics
            .reranker_readiness
            .examples_where_gold_is_present_but_not_first
    ));

    out.push_str("first blocking stage\n");
    for (stage, count) in &report.error_decomposition.counts {
        out.push_str(&format!("  {stage:<35}{count}\n"));
    }
    out.push('\n');

    out.push_str("severity of errors\n");
    for (class, count) in &report.severity.count_by_severity {
        out.push_str(&format!("  {class:<35}{count}\n"));
    }
    out.push('\n');

    out.push_str("oracle replacement (Δ overall exact match, not additive)\n");
    for row in &report.oracle_replacement {
        let delta = row
            .delta_vs_real
            .map(|d| format!("{:+.1} pp", d * 100.0))
            .unwrap_or_else(|| "n/a".into());
        out.push_str(&format!(
            "  {:<35}{:>10}   {}\n",
            row.variant,
            delta,
            show(&row.overall_exact_match)
        ));
    }
    out.push('\n');
    out.push_str("component isolation\n");
    out.push_str(&format!(
        "  {:<35}{}\n",
        "router-isolated",
        show(&report.component_isolation.router_isolated_accuracy)
    ));
    out.push_str(&format!(
        "  {:<35}{}\n",
        "candidate-isolated recall@K",
        show(&report.component_isolation.candidate_isolated_recall_at_k)
    ));
    out.push_str(&format!(
        "  {:<35}{}\n",
        "rank-isolated",
        show(&report.component_isolation.rank_isolated_accuracy)
    ));
    out.push_str(&format!(
        "  {:<35}{}\n",
        "decision-isolated",
        show(&report.component_isolation.decision_isolated_accuracy)
    ));
    out
}

/// The report with every timing field removed, so that two runs of the same
/// configuration can be compared for exact equality.
#[cfg(test)]
pub fn without_timing(report: &Report) -> Result<serde_json::Value, String> {
    let mut value = serde_json::to_value(report).map_err(|error| error.to_string())?;
    strip_timing(&mut value);
    Ok(value)
}

/// Timing is the one part of a report that legitimately differs between runs.
pub fn strip_timing(value: &mut serde_json::Value) {
    if let Some(object) = value.as_object_mut() {
        object.remove("timing");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A four-record corpus whose every metric was worked out by hand before
    /// the harness was pointed at it.
    ///
    /// | record | gold | what the system does | correct? |
    /// |---|---|---|---|
    /// | sulfuric | AST H₂SO₄ | routes to chemistry, parses, inserts | yes |
    /// | water | AST H₂O | «вода» carries no routing cue, so auto sends it to mathematics, the parse fails and the words are kept | no |
    /// | patience | RAW | no parse, words kept | yes |
    /// | boiled | RAW | no parse, words kept | yes |
    fn hand_table_corpus() -> String {
        [
            r#"{"dataset_schema_version":1,"id":"chem-sulfuric-001-a","family_id":"chem-sulfuric-001","provenance":"handcrafted_text","human_transcript":"серная кислота","asr_hypotheses":[],"target_domain":"chemistry","target_action":"ast","target_ast":{"Chemical":{"Species":{"coefficient":1,"formula":{"parts":[{"Atom":{"symbol":"H","count":2}},{"Atom":{"symbol":"S","count":1}},{"Atom":{"symbol":"O","count":4}}]},"charge":null,"marker":null}}},"split":"train","tags":["formula"],"speaker_id":null}"#,
            r#"{"dataset_schema_version":1,"id":"chem-water-001-a","family_id":"chem-water-001","provenance":"handcrafted_text","human_transcript":"вода","asr_hypotheses":[],"target_domain":"chemistry","target_action":"ast","target_ast":{"Chemical":{"Species":{"coefficient":1,"formula":{"parts":[{"Atom":{"symbol":"H","count":2}},{"Atom":{"symbol":"O","count":1}}]},"charge":null,"marker":null}}},"split":"train","tags":["formula"],"speaker_id":null}"#,
            r#"{"dataset_schema_version":1,"id":"raw-patience-001-a","family_id":"raw-patience-001","provenance":"handcrafted_text","human_transcript":"предел терпения","asr_hypotheses":[],"target_domain":"plain","target_action":"raw","target_ast":null,"split":"train","tags":["raw"],"speaker_id":null}"#,
            r#"{"dataset_schema_version":1,"id":"raw-boiled-001-a","family_id":"raw-boiled-001","provenance":"handcrafted_text","human_transcript":"вода закипела в чайнике","asr_hypotheses":[],"target_domain":"plain","target_action":"raw","target_ast":null,"split":"train","tags":["raw"],"speaker_id":null}"#,
        ]
        .join("\n")
    }

    fn hand_table_report() -> Report {
        let text = hand_table_corpus();
        let corpus = Dataset::parse_jsonl(&text).expect("valid corpus");
        let path = PathBuf::from("hand-table.jsonl");
        let selected: Vec<&Record> = corpus.records.iter().collect();
        build_report(&Inputs {
            dataset: &corpus,
            dataset_path: &path,
            dataset_bytes: text.as_bytes(),
            selected,
            split_filter: "all".into(),
            config: EvalConfig::default(),
        })
        .expect("report builds")
    }

    fn ratio(p: &Proportion) -> (usize, usize) {
        (p.numerator, p.denominator)
    }

    #[test]
    fn every_headline_metric_matches_the_hand_table() {
        let report = hand_table_report();
        let m = &report.metrics;
        assert_eq!(m.examples, 4);
        // Three of four answers match what the corpus asked for.
        assert_eq!(ratio(&m.overall_exact_match), (3, 4));
        assert_eq!(ratio(&m.ast_exact_match), (3, 4));
        // Only the sulfuric acid record is confident enough to be inserted.
        assert_eq!(ratio(&m.coverage), (1, 4));
        assert_eq!(ratio(&m.auto_insert_exact_match), (1, 1));
        // Routing is scored only where a scientific domain was intended.
        assert_eq!(ratio(&m.routing_accuracy), (1, 2));
        // Both ordinary sentences keep their words, and neither becomes a formula.
        assert_eq!(ratio(&m.raw_accuracy), (2, 2));
        assert_eq!(ratio(&m.false_scientific_rewrite_rate), (0, 2));
        assert!(
            m.false_scientific_rewrite_rate
                .zero_count_upper95
                .is_some_and(|bound| bound > 0.0),
            "an observed zero must still carry an upper bound"
        );
        // One of the two scientific records is answered with an abstention.
        assert_eq!(ratio(&m.false_abstention_rate), (1, 2));
        assert_eq!(ratio(&m.ast_validity), (1, 1));
        for k in RECALL_K_GRID {
            assert_eq!(
                ratio(&m.candidate_recall_at_k[&format!("{k}")]),
                (3, 4),
                "recall@{k}"
            );
        }
        // Every renderer sees the same decisions, so the payload rate is the same.
        for renderer in ["unicode", "latex", "omml"] {
            assert_eq!(
                ratio(&m.pre_insertion_end_to_end_exact_match[renderer]),
                (3, 4),
                "{renderer}"
            );
        }
    }

    #[test]
    fn the_hand_table_decomposition_and_severity_agree_with_the_walkthrough() {
        let report = hand_table_report();
        assert_eq!(report.error_decomposition.errors, 1);
        assert_eq!(report.error_decomposition.counts["router-first"], 1);
        assert_eq!(report.error_decomposition.counts["ASR-first"], 0);
        assert_eq!(report.error_decomposition.counts["candidate-first"], 0);
        assert_eq!(report.error_decomposition.asr_first_applicable_examples, 0);
        // The single failure is an abstention: safe, and nothing was invented.
        assert_eq!(report.severity.errors, 1);
        assert_eq!(report.severity.count_by_severity["S1"], 1);
        assert_eq!(report.severity.count_by_severity["S4"], 0);
        assert_eq!(report.severity.max_severity.as_deref(), Some("S1"));
        // Perfect routing is the only replacement that recovers the failure.
        let by_variant = |name: &str| {
            report
                .oracle_replacement
                .iter()
                .find(|row| row.variant == name)
                .unwrap_or_else(|| panic!("missing variant {name}"))
                .overall_exact_match
                .numerator
        };
        assert_eq!(by_variant("real"), 3);
        assert_eq!(by_variant("oracle_domain"), 4);
        assert_eq!(by_variant("oracle_transcript"), 3);
    }

    #[test]
    fn the_reranker_verdict_is_computed_not_asserted() {
        let report = hand_table_report();
        let readiness = &report.metrics.reranker_readiness;
        assert_eq!(readiness.scientific_examples, 2);
        // Neither record offers a second distinct scientific answer, so there
        // is nothing for a ranking model to learn.
        assert_eq!(
            readiness.examples_with_at_least_two_distinct_ast_candidates,
            0
        );
        assert_eq!(readiness.verdict, "insufficient");
        assert!(readiness.note.contains("statistically meaningless"));
    }

    #[test]
    fn sha256_matches_the_published_test_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }
}

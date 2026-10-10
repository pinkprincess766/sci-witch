//! Admission gates, evaluated rather than read.
//!
//! A release checklist with a line like «точность не ниже 90%» is a line a
//! person ticks. This module makes it a program that reads the report and
//! refuses.
//!
//! Three rules give the gates their teeth, and each one exists because the
//! obvious alternative would let a release through on evidence that does not
//! support it:
//!
//! 1. **Bounds, not point estimates.** 90% of twenty examples is not 90%.
//!    Every proportional gate is judged on the end of the confidence
//!    interval that could embarrass us — the lower bound for an accuracy,
//!    the upper bound for a failure rate.
//! 2. **An unmet precondition is not a pass.** A gate about speech,
//!    evaluated on a corpus of written text, reports *not measurable*, and
//!    that blocks the release exactly like a failure. Otherwise 98% on
//!    handcrafted sentences would quietly become a claim about microphones.
//! 3. **A zero is judged on its upper bound.** Zero false rewrites in 28
//!    sentences is compatible with a true rate of 10%. Shipping on that
//!    would be claiming something nobody measured.
//!
//! The gates themselves live in versioned JSON rather than in a constant
//! somebody can move. `research/schema/compiler-gates-v2.json` is the text
//! compiler (v1 is kept unchanged for the reports it judged). `research/schema/release-gates-v1.json` is the voice
//! application and still refuses a written corpus. Changing a threshold is
//! an edit to one of those files.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const GATES_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize)]
pub struct GateFile {
    pub gates_schema_version: u32,
    pub profile: String,
    /// The corpora this profile is judged on. `None` means the profile
    /// judges the one report it is handed, and pins that report some other
    /// way — the voice profile does it with the frozen seal.
    #[serde(default)]
    pub benchmark: Option<Benchmark>,
    #[serde(default)]
    pub gates: Vec<Gate>,
}

/// A fixed set of corpora, named by digest, judged as one pooled sample.
///
/// Without this a text profile can be passed on any file with enough rows:
/// twenty easy formulas and no ordinary speech at all passed every gate of
/// the first compiler profile, because "no formula was invented" is
/// trivially true of a corpus that contains nothing to invent one from.
/// Pinning the corpora by SHA-256 makes the benchmark a reviewed edit to
/// this file rather than a choice made at the command line.
#[derive(Clone, Debug, Deserialize)]
pub struct Benchmark {
    pub corpora: Vec<BenchmarkCorpus>,
    /// Report `config` entries every corpus must have been evaluated with.
    /// An insert threshold nobody ships, or a single split instead of the
    /// whole file, changes what the numbers mean.
    #[serde(default)]
    pub config: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BenchmarkCorpus {
    pub corpus_id: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Gate {
    pub id: String,
    pub title: String,
    /// Dotted path into the report JSON, e.g.
    /// `metrics.pre_insertion_end_to_end_exact_match.unicode`.
    pub metric: String,
    pub judge: Judge,
    pub comparison: Comparison,
    #[serde(default)]
    pub threshold: f64,
    #[serde(default)]
    pub requires: Requirements,
    #[serde(default)]
    pub rationale: String,
}

/// Which number of a measurement the gate reads.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Judge {
    /// The optimistic end. Used for accuracies, where the question is "can
    /// the data support at least this?"
    Ci95Low,
    /// The pessimistic end. Used for failure rates, where the question is
    /// "could the true rate still be worse than this?"
    Ci95High,
    /// The value itself. For counts and flags, which have no interval.
    Value,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    AtLeast,
    AtMost,
    IsTrue,
}

/// What a corpus must be before a gate about it means anything.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Requirements {
    /// Minimum count per provenance, e.g. `{"real_audio": 200}`.
    #[serde(default)]
    pub provenance_at_least: BTreeMap<String, u64>,
    #[serde(default)]
    pub min_speakers: Option<u64>,
    /// Whether the corpus must be the sealed frozen test.
    #[serde(default)]
    pub frozen: bool,
    /// Report paths that must hold at least this much, e.g.
    /// `{"metrics.false_scientific_rewrite_rate.denominator": 100}`.
    ///
    /// This is what keeps a safety gate from passing on nothing: a rewrite
    /// rate of 0 out of 0 is not evidence that ordinary speech is left alone.
    #[serde(default)]
    pub at_least: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    Pass {
        observed: String,
    },
    Fail {
        observed: String,
        reason: String,
    },
    /// The corpus cannot support this gate. Blocks a release just like a
    /// failure; kept as its own outcome because the fix is different —
    /// nothing about the software will change it.
    NotMeasurable {
        reason: String,
    },
}

impl Outcome {
    pub fn admits_release(&self) -> bool {
        matches!(self, Outcome::Pass { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Outcome::Pass { .. } => "ПРОЙДЕНО",
            Outcome::Fail { .. } => "НЕ ПРОЙДЕНО",
            Outcome::NotMeasurable { .. } => "НЕ ИЗМЕРИМО",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct GateResult {
    pub id: String,
    pub title: String,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Clone, Debug, Serialize)]
pub struct GateReport {
    pub profile: String,
    pub results: Vec<GateResult>,
}

impl GateReport {
    pub fn admits_release(&self) -> bool {
        self.results
            .iter()
            .all(|result| result.outcome.admits_release())
    }

    pub fn blocking(&self) -> Vec<&GateResult> {
        self.results
            .iter()
            .filter(|result| !result.outcome.admits_release())
            .collect()
    }
}

impl fmt::Display for GateReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "ворота допуска: {}", self.profile)?;
        for result in &self.results {
            let detail = match &result.outcome {
                Outcome::Pass { observed } => observed.clone(),
                Outcome::Fail { observed, reason } => format!("{observed} — {reason}"),
                Outcome::NotMeasurable { reason } => reason.clone(),
            };
            writeln!(
                f,
                "  {:<12} {:<24} {}",
                result.outcome.label(),
                result.id,
                detail
            )?;
        }
        Ok(())
    }
}

/// A sealed corpus: the file a release was measured on, named by digest so
/// that editing it after the fact is visible.
///
/// The seal is written **once**, before the numbers are known. A frozen test
/// that can be re-sealed after seeing a result is not frozen — it is a test
/// that gets adjusted until it agrees, which is the failure mode the whole
/// research protocol exists to prevent. [`seal`] therefore refuses to
/// overwrite.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FrozenSeal {
    pub file: String,
    pub sha256: String,
    #[serde(default)]
    pub sealed_at: String,
    #[serde(default)]
    pub sealed_for: String,
}

/// Seals `dataset` as the frozen test for `release`.
///
/// Refuses if a seal already exists, whatever it says. Re-sealing is not an
/// operation this program offers: a corpus that was already frozen and now
/// needs freezing again is a different corpus, and it needs a different file
/// and a visible decision to use it.
pub fn seal(
    dataset: &std::path::Path,
    out: &std::path::Path,
    release: &str,
    today: &str,
) -> Result<FrozenSeal, String> {
    if out.exists() {
        return Err(format!(
            "{} уже существует. Перепечатывание frozen test не предусмотрено: тест, который \
             можно запечатать заново после того, как результат стал известен, — это тест, \
             который подгоняют. Если корпус действительно другой, дайте ему другое имя.",
            out.display()
        ));
    }
    let bytes =
        std::fs::read(dataset).map_err(|error| format!("{}: {error}", dataset.display()))?;
    // The corpus must at least load before it can be frozen; sealing an
    // unparsable file would produce a digest nobody can ever evaluate.
    let text =
        String::from_utf8(bytes.clone()).map_err(|_| format!("{} не UTF-8", dataset.display()))?;
    let parsed = crate::schema::Dataset::parse_jsonl(&text).map_err(|error| error.to_string())?;
    let audit = crate::split::audit_splits(&parsed);
    if !audit.clean {
        return Err(format!(
            "{} не проходит проверку split: {} семейств и {} дикторов пересекают split. \
             Запечатывать корпус с утечкой — значит зафиксировать её навсегда.",
            dataset.display(),
            audit.leaking_families.len(),
            audit.leaking_speakers.len()
        ));
    }

    let sealed = FrozenSeal {
        file: dataset
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| dataset.display().to_string()),
        sha256: crate::report::sha256_hex(&bytes),
        sealed_at: today.to_string(),
        sealed_for: release.to_string(),
    };
    let json = serde_json::to_string_pretty(&sealed).map_err(|error| error.to_string())?;
    std::fs::write(out, format!("{json}\n"))
        .map_err(|error| format!("{}: {error}", out.display()))?;
    Ok(sealed)
}

/// Runs every gate against one report.
///
/// `seal` is the sealed frozen test, if there is one. `None` means no corpus
/// has been sealed yet, and every gate that requires one reports *not
/// measurable* rather than being skipped.
pub fn evaluate(
    file: &GateFile,
    report: &serde_json::Value,
    seal: Option<&FrozenSeal>,
) -> Result<GateReport, String> {
    check_schema(file)?;
    if let Some(benchmark) = &file.benchmark {
        return Err(format!(
            "профиль {} судит набор из {} корпусов, а не один отчёт: передайте отчёт по каждому ({})",
            file.profile,
            benchmark.corpora.len(),
            benchmark
                .corpora
                .iter()
                .map(|corpus| corpus.corpus_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let results = file
        .gates
        .iter()
        .map(|gate| GateResult {
            id: gate.id.clone(),
            title: gate.title.clone(),
            outcome: evaluate_one(gate, report, seal),
        })
        .collect();
    Ok(GateReport {
        profile: file.profile.clone(),
        results,
    })
}

fn evaluate_one(gate: &Gate, report: &serde_json::Value, seal: Option<&FrozenSeal>) -> Outcome {
    if let Some(reason) = unmet_requirement(&gate.requires, report, seal) {
        return Outcome::NotMeasurable { reason };
    }
    let Some(node) = lookup(report, &gate.metric) else {
        return Outcome::NotMeasurable {
            reason: format!("в отчёте нет {}", gate.metric),
        };
    };
    match gate.comparison {
        Comparison::IsTrue => match node.as_bool() {
            Some(true) => Outcome::Pass {
                observed: format!("{} = true", gate.metric),
            },
            Some(false) => Outcome::Fail {
                observed: format!("{} = false", gate.metric),
                reason: "требуется true".into(),
            },
            None => Outcome::NotMeasurable {
                reason: format!("{} не логическое значение", gate.metric),
            },
        },
        Comparison::AtLeast | Comparison::AtMost => {
            let Some((observed, note)) = judged_value(node, gate.judge) else {
                return Outcome::NotMeasurable {
                    reason: format!("{} не содержит {:?}", gate.metric, gate.judge),
                };
            };
            let ok = match gate.comparison {
                Comparison::AtLeast => observed >= gate.threshold,
                Comparison::AtMost => observed <= gate.threshold,
                Comparison::IsTrue => unreachable!(),
            };
            let shown = format!("{observed:.4}{note}");
            if ok {
                Outcome::Pass { observed: shown }
            } else {
                Outcome::Fail {
                    observed: shown,
                    reason: match gate.comparison {
                        Comparison::AtLeast => format!("нужно ≥ {:.4}", gate.threshold),
                        _ => format!("нужно ≤ {:.4}", gate.threshold),
                    },
                }
            }
        }
    }
}

fn check_schema(file: &GateFile) -> Result<(), String> {
    if file.gates_schema_version != GATES_SCHEMA_VERSION {
        return Err(format!(
            "gates_schema_version {} не поддерживается этой сборкой (нужна {GATES_SCHEMA_VERSION})",
            file.gates_schema_version
        ));
    }
    Ok(())
}

/// Id of the result that says whether the reports are the benchmark.
pub const BENCHMARK_GATE_ID: &str = "benchmark";

/// Runs every gate against the pooled benchmark.
///
/// The reports are pooled into one sample first — counts summed, proportions
/// re-divided from summed numerators and denominators, flags combined with
/// AND — and every gate is then judged on that sample by the same code that
/// judges a single report. A gate is therefore never passed by a corpus that
/// happens to lack the thing the gate is about: 146 sentences of ordinary
/// prose and 83 dictated formulas are one sample with both in it.
///
/// The first result says whether the reports *are* the benchmark. If they
/// are not — a corpus missing, one that is not in the list, one evaluated
/// with another configuration — nothing else is judged: a pooled sample of
/// the wrong corpora answers a question nobody asked.
pub fn evaluate_benchmark(
    file: &GateFile,
    reports: &[serde_json::Value],
) -> Result<GateReport, String> {
    check_schema(file)?;
    let benchmark = file
        .benchmark
        .as_ref()
        .ok_or_else(|| format!("профиль {} не объявляет benchmark", file.profile))?;
    for gate in &file.gates {
        if gate.judge != Judge::Value {
            return Err(format!(
                "ворота {} читают {:?}: при сложении корпусов интервалы не пересчитываются, \
                 судить можно только по счётчикам и долям",
                gate.id, gate.judge
            ));
        }
    }

    let coverage = benchmark_coverage(benchmark, reports);
    let mut results = vec![GateResult {
        id: BENCHMARK_GATE_ID.into(),
        title: "Отчёты получены ровно на закреплённых корпусах".into(),
        outcome: coverage.clone(),
    }];
    if !coverage.admits_release() {
        results.extend(file.gates.iter().map(|gate| GateResult {
            id: gate.id.clone(),
            title: gate.title.clone(),
            outcome: Outcome::NotMeasurable {
                reason: "не на чем судить: набор отчётов не совпадает с benchmark".into(),
            },
        }));
        return Ok(GateReport {
            profile: file.profile.clone(),
            results,
        });
    }

    let pooled = pool(file, reports);
    results.extend(file.gates.iter().map(|gate| GateResult {
        id: gate.id.clone(),
        title: gate.title.clone(),
        outcome: evaluate_one(gate, &pooled, None),
    }));
    Ok(GateReport {
        profile: file.profile.clone(),
        results,
    })
}

fn benchmark_coverage(benchmark: &Benchmark, reports: &[serde_json::Value]) -> Outcome {
    let mut seen: Vec<&str> = Vec::new();
    for report in reports {
        let Some(digest) = lookup(report, "dataset.sha256").and_then(serde_json::Value::as_str)
        else {
            return Outcome::Fail {
                observed: "отчёт без dataset.sha256".into(),
                reason: "корпус отчёта нельзя опознать".into(),
            };
        };
        let Some(corpus) = benchmark
            .corpora
            .iter()
            .find(|corpus| corpus.sha256 == digest)
        else {
            let file = lookup(report, "dataset.file")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?");
            return Outcome::Fail {
                observed: format!("{file} ({digest})"),
                reason: "этого корпуса нет в benchmark; другой корпус его не заменяет".into(),
            };
        };
        if seen.contains(&corpus.corpus_id.as_str()) {
            return Outcome::Fail {
                observed: corpus.corpus_id.clone(),
                reason: "корпус передан дважды и был бы посчитан дважды".into(),
            };
        }
        for (key, wanted) in &benchmark.config {
            let have = lookup(report, &format!("config.{key}"));
            if have != Some(wanted) {
                return Outcome::Fail {
                    observed: format!(
                        "{}: config.{key} = {}",
                        corpus.corpus_id,
                        have.map(|value| value.to_string())
                            .unwrap_or_else(|| "нет".into())
                    ),
                    reason: format!("benchmark требует {wanted}"),
                };
            }
        }
        seen.push(&corpus.corpus_id);
    }
    let missing: Vec<&str> = benchmark
        .corpora
        .iter()
        .map(|corpus| corpus.corpus_id.as_str())
        .filter(|id| !seen.contains(id))
        .collect();
    if !missing.is_empty() {
        return Outcome::NotMeasurable {
            reason: format!("нет отчёта по {}", missing.join(", ")),
        };
    }
    Outcome::Pass {
        observed: seen.join(" + "),
    }
}

/// One report that stands for all of them, holding only the paths the gates
/// read. A path that cannot be pooled is left out, and a gate that reads it
/// then reports it as not measurable instead of judging a wrong number.
fn pool(file: &GateFile, reports: &[serde_json::Value]) -> serde_json::Value {
    let mut paths: Vec<String> = Vec::new();
    for gate in &file.gates {
        paths.push(gate.metric.clone());
        paths.extend(gate.requires.at_least.keys().cloned());
        paths.extend(
            gate.requires
                .provenance_at_least
                .keys()
                .map(|provenance| format!("dataset.counts_by_provenance.{provenance}")),
        );
    }
    paths.sort();
    paths.dedup();
    let mut pooled = serde_json::json!({});
    for path in paths {
        let nodes: Option<Vec<&serde_json::Value>> =
            reports.iter().map(|report| lookup(report, &path)).collect();
        // A provenance absent from one corpus is zero rows of it, not an
        // unknown: the manifests only list what is present.
        let nodes = match nodes {
            Some(nodes) => nodes,
            None if path.starts_with("dataset.counts_by_provenance.") => reports
                .iter()
                .filter_map(|report| lookup(report, &path))
                .collect(),
            None => continue,
        };
        if let Some(value) = pool_nodes(&nodes) {
            insert(&mut pooled, &path, value);
        }
    }
    pooled
}

/// Pools one field across corpora. Only shapes whose pooled meaning is
/// unambiguous are pooled: whole counts are summed, `Proportion` objects are
/// re-divided from summed parts, flags are combined with AND. A bare
/// fraction has lost its denominator and cannot be pooled.
fn pool_nodes(nodes: &[&serde_json::Value]) -> Option<serde_json::Value> {
    use serde_json::Value;
    if nodes.iter().all(|node| node.is_boolean()) {
        return Some(Value::Bool(
            nodes.iter().all(|node| node.as_bool() == Some(true)),
        ));
    }
    if nodes.iter().all(|node| node.is_u64()) {
        return Some(Value::from(
            nodes.iter().filter_map(|node| node.as_u64()).sum::<u64>(),
        ));
    }
    let parts: Option<Vec<(u64, u64)>> = nodes
        .iter()
        .map(|node| {
            Some((
                node.get("numerator")?.as_u64()?,
                node.get("denominator")?.as_u64()?,
            ))
        })
        .collect();
    let parts = parts?;
    let numerator: u64 = parts.iter().map(|(n, _)| n).sum();
    let denominator: u64 = parts.iter().map(|(_, d)| d).sum();
    Some(serde_json::json!({
        "numerator": numerator,
        "denominator": denominator,
        "value": if denominator == 0 {
            Value::Null
        } else {
            Value::from(numerator as f64 / denominator as f64)
        },
    }))
}

fn insert(root: &mut serde_json::Value, path: &str, value: serde_json::Value) {
    let mut node = root;
    let parts: Vec<&str> = path.split('.').collect();
    for part in &parts[..parts.len() - 1] {
        node = node
            .as_object_mut()
            .expect("pooled report is built from objects")
            .entry(part.to_string())
            .or_insert_with(|| serde_json::json!({}));
    }
    node.as_object_mut()
        .expect("pooled report is built from objects")
        .insert(parts[parts.len() - 1].to_string(), value);
}

/// Reads the number a gate judges on, plus a note naming which end of the
/// interval it was — so a printed result cannot be mistaken for the point
/// estimate.
fn judged_value(node: &serde_json::Value, judge: Judge) -> Option<(f64, String)> {
    match judge {
        Judge::Value => {
            let value = node
                .as_f64()
                .or_else(|| node.get("value").and_then(serde_json::Value::as_f64))?;
            Some((value, String::new()))
        }
        Judge::Ci95Low => {
            let value = node.get("ci95_low")?.as_f64()?;
            let point = node.get("value").and_then(serde_json::Value::as_f64);
            Some((
                value,
                match point {
                    Some(point) => format!(" (нижняя граница 95% CI; наблюдалось {point:.4})"),
                    None => " (нижняя граница 95% CI)".into(),
                },
            ))
        }
        Judge::Ci95High => {
            let value = node.get("ci95_high")?.as_f64()?;
            let point = node.get("value").and_then(serde_json::Value::as_f64);
            Some((
                value,
                match point {
                    Some(point) => format!(" (верхняя граница 95% CI; наблюдалось {point:.4})"),
                    None => " (верхняя граница 95% CI)".into(),
                },
            ))
        }
    }
}

/// The first unmet precondition, in words a person can act on.
fn unmet_requirement(
    requires: &Requirements,
    report: &serde_json::Value,
    seal: Option<&FrozenSeal>,
) -> Option<String> {
    for (provenance, wanted) in &requires.provenance_at_least {
        let have = lookup(
            report,
            &format!("dataset.counts_by_provenance.{provenance}"),
        )
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
        if have < *wanted {
            return Some(format!(
                "корпус содержит {have} записей provenance={provenance}, нужно {wanted}"
            ));
        }
    }
    if let Some(wanted) = requires.min_speakers {
        let have = lookup(report, "split_audit.speakers")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if have < wanted {
            return Some(format!("дикторов в корпусе {have}, нужно {wanted}"));
        }
    }
    for (path, wanted) in &requires.at_least {
        let have = lookup(report, path).and_then(serde_json::Value::as_f64);
        match have {
            Some(have) if have >= *wanted => {}
            Some(have) => {
                return Some(format!("{path} = {have}, нужно не меньше {wanted}"));
            }
            None => return Some(format!("в отчёте нет {path}")),
        }
    }
    if requires.frozen {
        let Some(seal) = seal else {
            return Some("frozen test не запечатан: нечего сверять".into());
        };
        let digest = lookup(report, "dataset.sha256").and_then(serde_json::Value::as_str);
        match digest {
            Some(digest) if digest == seal.sha256 => {}
            Some(digest) => {
                return Some(format!(
                    "отчёт получен на корпусе {digest}, а запечатан {} ({})",
                    seal.sha256, seal.file
                ))
            }
            None => return Some("в отчёте нет dataset.sha256".into()),
        }
    }
    None
}

/// Dotted path lookup. Deliberately dumb: a gate names a field, and if the
/// field is not there the gate is not measurable rather than silently zero.
fn lookup<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut node = root;
    for part in path.split('.') {
        node = node.get(part)?;
    }
    Some(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn gates() -> GateFile {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../research/schema/release-gates-v1.json");
        let text = std::fs::read_to_string(&path).expect("release-gates-v1.json must exist");
        serde_json::from_str(&text).expect("release-gates-v1.json must parse")
    }

    fn seal() -> FrozenSeal {
        FrozenSeal {
            file: "voice-frozen-v1.jsonl".into(),
            sha256: "a".repeat(64),
            sealed_at: "2026-09-06".into(),
            sealed_for: "0.5".into(),
        }
    }

    /// A corpus that satisfies every precondition, so individual gates can be
    /// tested on their numbers rather than on their requirements.
    fn voice_report(overrides: serde_json::Value) -> serde_json::Value {
        let mut report = json!({
            "dataset": {
                "sha256": "a".repeat(64),
                "counts_by_provenance": { "real_audio": 400 }
            },
            "split_audit": { "speakers": 12, "clean": true },
            "severity": { "count_by_severity": { "S4": 0 } },
            "user_path": {
                "mixed_exact_match": { "value": 0.95, "ci95_low": 0.93, "ci95_high": 0.97 },
                "false_scientific_rewrite_rate": { "value": 0.0, "ci95_low": 0.0, "ci95_high": 0.01 }
            },
            "selective_prediction": { "calibration": { "errors": 25 } },
            "metrics": {
                "pre_insertion_end_to_end_exact_match": {
                    "unicode": { "value": 0.95, "ci95_low": 0.93, "ci95_high": 0.97 }
                },
                "auto_insert_exact_match": { "value": 0.99, "ci95_low": 0.98, "ci95_high": 0.999 },
                "false_scientific_rewrite_rate": { "value": 0.0, "ci95_low": 0.0, "ci95_high": 0.01 },
                "ast_validity": { "value": 1.0, "ci95_low": 0.995, "ci95_high": 1.0 }
            }
        });
        merge(&mut report, &overrides);
        report
    }

    fn merge(into: &mut serde_json::Value, from: &serde_json::Value) {
        match (into, from) {
            (serde_json::Value::Object(target), serde_json::Value::Object(source)) => {
                for (key, value) in source {
                    merge(target.entry(key.clone()).or_insert(json!(null)), value);
                }
            }
            (target, source) => *target = source.clone(),
        }
    }

    fn outcome<'a>(report: &'a GateReport, id: &str) -> &'a Outcome {
        &report
            .results
            .iter()
            .find(|result| result.id == id)
            .unwrap_or_else(|| panic!("no gate {id}"))
            .outcome
    }

    #[test]
    fn the_shipped_gate_file_loads_and_covers_the_roadmap_criteria() {
        let file = gates();
        assert_eq!(file.gates_schema_version, GATES_SCHEMA_VERSION);
        let ids: Vec<&str> = file.gates.iter().map(|gate| gate.id.as_str()).collect();
        for required in [
            "end-to-end-accuracy",
            "shipped-end-to-end-accuracy",
            "auto-insert-precision",
            "no-dangerous-rewrites-shipped",
            "no-dangerous-rewrites-parser",
            "no-s4-errors",
        ] {
            assert!(ids.contains(&required), "missing gate {required}: {ids:?}");
        }
    }

    /// The gate that makes the rest honest: a corpus of written text cannot
    /// answer a question about speech, and reporting that as a pass is how a
    /// benchmark becomes a lie.
    #[test]
    fn a_text_corpus_cannot_pass_a_gate_about_speech() {
        let text_report = json!({
            "dataset": {
                "sha256": "b".repeat(64),
                "counts_by_provenance": { "handcrafted_text": 113 }
            },
            "split_audit": { "speakers": 0, "clean": true },
            "severity": { "count_by_severity": { "S4": 0 } },
            "user_path": {
                "false_scientific_rewrite_rate": { "value": 0.0, "ci95_low": 0.0, "ci95_high": 0.121 }
            },
            "metrics": {
                "pre_insertion_end_to_end_exact_match": {
                    "unicode": { "value": 0.982, "ci95_low": 0.938, "ci95_high": 0.995 }
                },
                "auto_insert_exact_match": { "value": 1.0, "ci95_low": 0.956, "ci95_high": 1.0 },
                "false_scientific_rewrite_rate": { "value": 0.0, "ci95_low": 0.0, "ci95_high": 0.121 },
                "ast_validity": { "value": 1.0, "ci95_low": 0.956, "ci95_high": 1.0 }
            }
        });
        let report = evaluate(&gates(), &text_report, None).unwrap();
        assert!(!report.admits_release());
        for id in [
            "end-to-end-accuracy",
            "auto-insert-precision",
            "no-dangerous-rewrites-shipped",
            "no-dangerous-rewrites-parser",
        ] {
            assert!(
                matches!(outcome(&report, id), Outcome::NotMeasurable { .. }),
                "{id}: {:?}",
                outcome(&report, id)
            );
        }
        // Split hygiene needs no voice, so it is answered on any corpus.
        assert!(outcome(&report, "split-hygiene").admits_release());
    }

    /// A point estimate above the threshold is not enough when the interval
    /// says the truth could be far below it.
    #[test]
    fn a_thin_sample_does_not_clear_a_gate_it_only_appears_to_meet() {
        let thin = voice_report(json!({
            "metrics": {
                "pre_insertion_end_to_end_exact_match": {
                    // 19 of 20: looks like 95%, supports as little as 76%.
                    "unicode": { "value": 0.95, "ci95_low": 0.764, "ci95_high": 0.992 }
                }
            }
        }));
        let report = evaluate(&gates(), &thin, Some(&seal())).unwrap();
        let outcome = outcome(&report, "end-to-end-accuracy");
        assert!(matches!(outcome, Outcome::Fail { .. }), "{outcome:?}");
        let Outcome::Fail { observed, .. } = outcome else {
            unreachable!()
        };
        // The message must not let 0.95 be mistaken for what was judged.
        assert!(observed.contains("нижняя граница"), "{observed}");
        assert!(observed.contains("0.7640"), "{observed}");
    }

    /// Zero observed failures is not evidence of safety on a small sample.
    #[test]
    fn an_observed_zero_is_judged_on_what_it_could_still_hide() {
        let few = voice_report(json!({
            "user_path": {
                // 0/28: the true rate could be 10%.
                "false_scientific_rewrite_rate": { "value": 0.0, "ci95_low": 0.0, "ci95_high": 0.121 }
            }
        }));
        let report = evaluate(&gates(), &few, Some(&seal())).unwrap();
        let outcome = outcome(&report, "no-dangerous-rewrites-shipped");
        assert!(matches!(outcome, Outcome::Fail { .. }), "{outcome:?}");
        let Outcome::Fail { observed, .. } = outcome else {
            unreachable!()
        };
        assert!(observed.contains("верхняя граница"), "{observed}");
    }

    #[test]
    fn a_corpus_that_meets_every_precondition_can_pass() {
        let report = evaluate(&gates(), &voice_report(json!({})), Some(&seal())).unwrap();
        assert!(report.admits_release(), "{report}");
        assert!(report.blocking().is_empty());
    }

    /// One dangerous error is one too many, and it is counted exactly rather
    /// than smoothed into a rate.
    #[test]
    fn a_single_s4_error_blocks_the_release() {
        let with_s4 = voice_report(json!({ "severity": { "count_by_severity": { "S4": 1 } } }));
        let report = evaluate(&gates(), &with_s4, Some(&seal())).unwrap();
        assert!(!report.admits_release());
        assert!(matches!(
            outcome(&report, "no-s4-errors"),
            Outcome::Fail { .. }
        ));
    }

    /// The frozen test is identified by digest. A corpus edited after
    /// sealing is a different corpus, whatever it is called.
    #[test]
    fn a_corpus_edited_after_sealing_is_not_the_frozen_test() {
        let edited = voice_report(json!({ "dataset": { "sha256": "c".repeat(64) } }));
        let report = evaluate(&gates(), &edited, Some(&seal())).unwrap();
        let outcome = outcome(&report, "end-to-end-accuracy");
        let Outcome::NotMeasurable { reason } = outcome else {
            panic!("{outcome:?}")
        };
        assert!(reason.contains("запечатан"), "{reason}");
    }

    #[test]
    fn nothing_is_frozen_until_something_is_sealed() {
        let report = evaluate(&gates(), &voice_report(json!({})), None).unwrap();
        assert!(!report.admits_release());
        let outcome = outcome(&report, "end-to-end-accuracy");
        assert!(
            matches!(outcome, Outcome::NotMeasurable { reason } if reason.contains("не запечатан")),
            "{outcome:?}"
        );
    }

    /// A metric the report does not carry is not zero, and not a pass.
    #[test]
    fn a_missing_metric_is_reported_rather_than_assumed() {
        let mut without = voice_report(json!({}));
        without["metrics"]
            .as_object_mut()
            .unwrap()
            .remove("ast_validity");
        let report = evaluate(&gates(), &without, Some(&seal())).unwrap();
        let outcome = outcome(&report, "structural-validity");
        assert!(
            matches!(outcome, Outcome::NotMeasurable { reason } if reason.contains("нет metrics.ast_validity")),
            "{outcome:?}"
        );
    }

    #[test]
    fn a_gate_file_from_a_future_build_is_refused() {
        let mut file = gates();
        file.gates_schema_version = 99;
        let error = evaluate(&file, &voice_report(json!({})), Some(&seal())).unwrap_err();
        assert!(error.contains("не поддерживается"), "{error}");
    }
}

#[cfg(test)]
mod compiler_gates {
    use super::*;
    use serde_json::json;

    /// The current compiler profile — the default of `gate --gates`.
    fn file() -> GateFile {
        profile("compiler-gates-v2.json")
    }

    /// The profile the reports before 2026-10-01 were judged by. Kept on disk
    /// unchanged, so it has to keep matching its own corpora.
    fn file_v1() -> GateFile {
        profile("compiler-gates-v1.json")
    }

    fn profile(name: &str) -> GateFile {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../research/schema")
            .join(name);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn benchmark() -> Benchmark {
        file()
            .benchmark
            .expect("the compiler profile pins a benchmark")
    }

    fn published(names: &[&str]) -> Vec<serde_json::Value> {
        let results =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research/results");
        names
            .iter()
            .map(|name| {
                serde_json::from_str(&std::fs::read_to_string(results.join(name)).unwrap()).unwrap()
            })
            .collect()
    }

    fn merge(into: &mut serde_json::Value, from: &serde_json::Value) {
        match (into, from) {
            (serde_json::Value::Object(target), serde_json::Value::Object(source)) => {
                for (key, value) in source {
                    merge(target.entry(key.clone()).or_insert(json!(null)), value);
                }
            }
            (target, source) => *target = source.clone(),
        }
    }

    /// A clean report for one pinned corpus. `raw` ordinary sentences and
    /// `inserted` emitted trees — the two denominators the gates depend on.
    fn report(corpus: &BenchmarkCorpus, raw: u64, inserted: u64) -> serde_json::Value {
        let config = benchmark().config;
        json!({
            "dataset": {
                "file": format!("{}.jsonl", corpus.corpus_id),
                "sha256": corpus.sha256,
                "counts_by_provenance": { "handcrafted_text": raw + inserted }
            },
            "config": config,
            "split_audit": { "clean": true },
            "severity": { "count_by_severity": { "S4": 0 } },
            "metrics": {
                "false_scientific_rewrite_rate": proportion(0, raw),
                "ast_validity": proportion(inserted, inserted),
                "ast_exact_match": { "value": 0.6, "ci95_low": 0.47 }
            },
            "user_path": { "false_scientific_rewrite_rate": proportion(0, raw) }
        })
    }

    fn proportion(numerator: u64, denominator: u64) -> serde_json::Value {
        json!({
            "numerator": numerator,
            "denominator": denominator,
            "value": if denominator == 0 { json!(null) } else { json!(numerator as f64 / denominator as f64) }
        })
    }

    /// One clean report per pinned corpus, in roughly the benchmark's real
    /// proportions: prose-heavy on one side, formula-heavy on the other.
    fn reports() -> Vec<serde_json::Value> {
        benchmark()
            .corpora
            .iter()
            .map(|corpus| match corpus.corpus_id.as_str() {
                "prose-negatives-v1" => report(corpus, 146, 0),
                _ => report(corpus, 25, 40),
            })
            .collect()
    }

    fn outcome<'a>(report: &'a GateReport, id: &str) -> &'a Outcome {
        &report
            .results
            .iter()
            .find(|result| result.id == id)
            .unwrap_or_else(|| panic!("no gate {id}"))
            .outcome
    }

    #[test]
    fn the_compiler_profile_does_not_ask_for_a_microphone_or_for_accuracy() {
        let file = file();
        assert_eq!(file.profile, "compiler-v2");
        assert_eq!(file.gates_schema_version, GATES_SCHEMA_VERSION);
        for gate in &file.gates {
            assert!(
                gate.requires.min_speakers.is_none(),
                "{} asks for speakers",
                gate.id
            );
            assert!(
                !gate.requires.frozen,
                "{} waits for a sealed voice test",
                gate.id
            );
            assert!(
                !gate.requires.provenance_at_least.contains_key("real_audio"),
                "{} requires real_audio",
                gate.id
            );
            assert!(
                !gate.metric.contains("ast_exact_match"),
                "{} judges author-text accuracy",
                gate.id
            );
            assert_eq!(gate.judge, Judge::Value, "{} reads an interval", gate.id);
        }
    }

    /// Every gate that can be satisfied by an absence has to say how much
    /// presence it needs. Found the hard way: 20 formulas and no prose passed
    /// «no ordinary speech became a formula».
    #[test]
    fn every_counting_gate_names_the_denominator_it_needs() {
        for gate in &file().gates {
            if gate.comparison == Comparison::IsTrue {
                continue;
            }
            assert!(
                gate.requires
                    .at_least
                    .keys()
                    .any(|path| path.ends_with(".denominator")),
                "{} has no denominator floor, so an empty sample passes it",
                gate.id
            );
        }
    }

    #[test]
    fn a_benchmark_that_invents_nothing_passes() {
        let outcome = evaluate_benchmark(&file(), &reports()).unwrap();
        assert!(outcome.admits_release(), "{outcome}");
        assert_eq!(outcome.results[0].id, BENCHMARK_GATE_ID);
    }

    #[test]
    fn one_invented_formula_in_any_corpus_blocks() {
        for index in 0..benchmark().corpora.len() {
            let mut reports = reports();
            merge(
                &mut reports[index],
                &json!({ "severity": { "count_by_severity": { "S4": 1 } } }),
            );
            let outcome = evaluate_benchmark(&file(), &reports).unwrap();
            assert!(
                matches!(outcome_of(&outcome, "no-s4-errors"), Outcome::Fail { .. }),
                "an S4 in corpus {index} was pooled away: {outcome}"
            );
        }
    }

    fn outcome_of<'a>(report: &'a GateReport, id: &str) -> &'a Outcome {
        outcome(report, id)
    }

    /// The hole this whole mechanism closes.
    #[test]
    fn a_hand_picked_corpus_cannot_stand_in_for_the_benchmark() {
        // Through the single-report entry point: refused outright.
        let friendly = report(
            &BenchmarkCorpus {
                corpus_id: "friendly".into(),
                sha256: "0".repeat(64),
            },
            0,
            20,
        );
        assert!(evaluate(&file(), &friendly, None).is_err());

        // In place of a pinned corpus: the benchmark result fails, and
        // nothing is judged on the substituted sample.
        let mut substituted = reports();
        substituted[0] = friendly;
        let outcome = evaluate_benchmark(&file(), &substituted).unwrap();
        assert!(matches!(
            outcome_of(&outcome, BENCHMARK_GATE_ID),
            Outcome::Fail { .. }
        ));
        for result in &outcome.results[1..] {
            assert!(
                matches!(result.outcome, Outcome::NotMeasurable { .. }),
                "{} was judged on the wrong corpora",
                result.id
            );
        }
    }

    #[test]
    fn a_missing_corpus_is_not_measurable() {
        let mut short = reports();
        short.pop();
        let outcome = evaluate_benchmark(&file(), &short).unwrap();
        assert!(matches!(
            outcome_of(&outcome, BENCHMARK_GATE_ID),
            Outcome::NotMeasurable { .. }
        ));
        assert!(!outcome.admits_release());
    }

    #[test]
    fn a_corpus_passed_twice_is_refused() {
        let mut doubled = reports();
        doubled[1] = doubled[0].clone();
        let outcome = evaluate_benchmark(&file(), &doubled).unwrap();
        assert!(matches!(
            outcome_of(&outcome, BENCHMARK_GATE_ID),
            Outcome::Fail { .. }
        ));
    }

    #[test]
    fn a_report_evaluated_with_another_configuration_is_refused() {
        // A threshold nobody ships would insert nothing and make the rewrite
        // gates vacuous; a single split would count a fraction of the file.
        for (key, value) in [
            ("auto_insert_threshold", json!(1.01)),
            ("evaluated_split", json!("train")),
            ("domain_policy", json!("lattice_v1")),
        ] {
            let mut altered = reports();
            merge(&mut altered[0], &json!({ "config": { key: value } }));
            let outcome = evaluate_benchmark(&file(), &altered).unwrap();
            assert!(
                matches!(
                    outcome_of(&outcome, BENCHMARK_GATE_ID),
                    Outcome::Fail { .. }
                ),
                "config.{key} was not checked"
            );
        }
    }

    #[test]
    fn a_benchmark_without_ordinary_speech_cannot_pass_the_rewrite_gates() {
        let mut no_prose: Vec<_> = benchmark()
            .corpora
            .iter()
            .map(|corpus| report(corpus, 0, 40))
            .collect();
        let outcome = evaluate_benchmark(&file(), &no_prose).unwrap();
        for id in [
            "no-s4-errors",
            "no-false-rewrites-parser",
            "no-false-rewrites-utterance",
        ] {
            assert!(
                matches!(outcome_of(&outcome, id), Outcome::NotMeasurable { .. }),
                "{id} passed on a benchmark with no ordinary speech"
            );
        }
        // …and one sentence short of the floor is still not enough.
        no_prose[0] = report(&benchmark().corpora[0], 99, 40);
        let outcome = evaluate_benchmark(&file(), &no_prose).unwrap();
        assert!(matches!(
            outcome_of(&outcome, "no-false-rewrites-parser"),
            Outcome::NotMeasurable { .. }
        ));
    }

    #[test]
    fn prose_and_formulas_answer_the_validity_gate_together() {
        // The prose corpus inserts nothing, so on its own its validity is
        // null. Pooled, the formulas carry the gate and the prose does not
        // block it.
        let outcome = evaluate_benchmark(&file(), &reports()).unwrap();
        assert!(matches!(
            outcome_of(&outcome, "inserted-ast-well-formed"),
            Outcome::Pass { .. }
        ));
        // One malformed tree anywhere fails it.
        let mut broken = reports();
        merge(
            &mut broken[0],
            &json!({ "metrics": { "ast_validity": proportion(39, 40) } }),
        );
        let outcome = evaluate_benchmark(&file(), &broken).unwrap();
        assert!(matches!(
            outcome_of(&outcome, "inserted-ast-well-formed"),
            Outcome::Fail { .. }
        ));
    }

    #[test]
    fn pooling_sums_parts_and_never_averages_fractions() {
        let a = json!({ "numerator": 1, "denominator": 3, "value": 0.3333 });
        let b = json!({ "numerator": 0, "denominator": 97, "value": 0.0 });
        let pooled = pool_nodes(&[&a, &b]).unwrap();
        // 1/100, not the mean of 1/3 and 0.
        assert_eq!(pooled["numerator"], 1);
        assert_eq!(pooled["denominator"], 100);
        assert_eq!(pooled["value"], json!(0.01));
        assert_eq!(pool_nodes(&[&json!(2), &json!(3)]), Some(json!(5)));
        assert_eq!(
            pool_nodes(&[&json!(true), &json!(false)]),
            Some(json!(false))
        );
        // A bare fraction has lost its denominator.
        assert_eq!(pool_nodes(&[&json!(0.5), &json!(0.25)]), None);
        let empty = pool_nodes(&[
            &json!({ "numerator": 0, "denominator": 0 }),
            &json!({ "numerator": 0, "denominator": 0 }),
        ])
        .unwrap();
        assert_eq!(empty["value"], json!(null));
    }

    /// The gate file and the corpora cannot drift apart silently: every
    /// pinned digest is the digest of the file its manifest names, today.
    #[test]
    fn the_pinned_digests_match_the_corpora_on_disk() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research/data");
        let retired = file_v1().benchmark.expect("v1 pins a benchmark");
        for corpus in benchmark().corpora.into_iter().chain(retired.corpora) {
            let manifest: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(data.join(format!("{}.manifest.json", corpus.corpus_id)))
                    .unwrap_or_else(|error| panic!("{}: {error}", corpus.corpus_id)),
            )
            .unwrap();
            assert_eq!(
                manifest["sha256"],
                json!(corpus.sha256),
                "{}",
                corpus.corpus_id
            );
            let bytes = std::fs::read(data.join(manifest["file"].as_str().unwrap())).unwrap();
            assert_eq!(
                crate::report::sha256_hex(&bytes),
                corpus.sha256,
                "{} changed on disk",
                corpus.corpus_id
            );
            crate::schema::Dataset::parse_jsonl(&String::from_utf8(bytes).unwrap())
                .unwrap_or_else(|error| panic!("{}: {error}", corpus.corpus_id));
        }
    }

    /// A family in two corpora would be one example counted twice, and a
    /// split leak the per-report audit cannot see.
    #[test]
    fn benchmark_corpora_share_no_family() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research/data");
        let mut owner: BTreeMap<String, String> = BTreeMap::new();
        for corpus in benchmark().corpora {
            let text =
                std::fs::read_to_string(data.join(format!("{}.jsonl", corpus.corpus_id))).unwrap();
            let loaded = crate::schema::Dataset::parse_jsonl(&text).unwrap();
            for record in &loaded.records {
                if let Some(previous) =
                    owner.insert(record.family_id.clone(), corpus.corpus_id.clone())
                {
                    assert_eq!(
                        previous, corpus.corpus_id,
                        "family {} is in both {previous} and {}",
                        record.family_id, corpus.corpus_id
                    );
                }
            }
        }
    }

    /// The published reports are the benchmark, and they pass.
    #[test]
    fn the_published_benchmark_reports_pass() {
        let reports = published(&[
            "deterministic-v7.json",
            "ambiguous-auto-v3.json",
            "nomenclature-v2.json",
            "prose-negatives-v2.json",
        ]);
        let outcome = evaluate_benchmark(&file(), &reports).unwrap();
        assert!(outcome.admits_release(), "{outcome}");
    }

    /// The reports judged under v1 still pass the profile they were judged by.
    #[test]
    fn the_retired_profile_still_admits_its_own_reports() {
        let reports = published(&[
            "deterministic-v6.json",
            "ambiguous-auto-v2.json",
            "nomenclature-v1.json",
            "prose-negatives-v1.json",
        ]);
        let outcome = evaluate_benchmark(&file_v1(), &reports).unwrap();
        assert!(outcome.admits_release(), "{outcome}");
    }

    /// The trap a new profile version opens: keep handing in the old prose
    /// report, which has none of the new sentences and passes trivially. v1
    /// is a strict subset of v2, so its report looks like a smaller v2 — the
    /// pinned digest is what refuses it.
    #[test]
    fn the_old_prose_report_cannot_stand_in_for_the_new_one() {
        let reports = published(&[
            "deterministic-v7.json",
            "ambiguous-auto-v3.json",
            "nomenclature-v2.json",
            "prose-negatives-v1.json",
        ]);
        let refused = match evaluate_benchmark(&file(), &reports) {
            Err(_) => true,
            Ok(outcome) => !outcome.admits_release(),
        };
        assert!(
            refused,
            "compiler-v2 admitted the prose-negatives-v1 report"
        );
    }
}

#[cfg(test)]
mod seal_tests {
    use super::*;

    const RECORD: &str = r#"{"dataset_schema_version":1,"id":"fam-1-a","family_id":"fam-1","provenance":"handcrafted_text","human_transcript":"вода","asr_hypotheses":[],"target_domain":"plain","target_action":"raw","target_ast":null,"split":"train","tags":[],"speaker_id":null}"#;

    fn leaking() -> String {
        let a = RECORD.to_string();
        let b = RECORD
            .replace("fam-1-a", "fam-1-b")
            .replace("\"train\"", "\"dev_holdout\"");
        format!("{a}\n{b}")
    }

    #[test]
    fn sealing_records_the_digest_of_what_was_frozen() {
        let dir = tempfile::tempdir().unwrap();
        let dataset = dir.path().join("voice-frozen-v1.jsonl");
        std::fs::write(&dataset, RECORD).unwrap();
        let out = dir.path().join("seal.json");

        let sealed = seal(&dataset, &out, "0.5", "2026-09-06").unwrap();
        assert_eq!(sealed.file, "voice-frozen-v1.jsonl");
        assert_eq!(sealed.sha256.len(), 64);
        assert_eq!(sealed.sealed_for, "0.5");

        // What was written is what a later gate run will read.
        let written: FrozenSeal =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(written.sha256, sealed.sha256);
    }

    /// The property that makes "frozen" mean anything: a test that can be
    /// re-sealed after the result is known is a test that gets adjusted.
    #[test]
    fn a_frozen_test_cannot_be_sealed_a_second_time() {
        let dir = tempfile::tempdir().unwrap();
        let dataset = dir.path().join("voice-frozen-v1.jsonl");
        std::fs::write(&dataset, RECORD).unwrap();
        let out = dir.path().join("seal.json");
        let first = seal(&dataset, &out, "0.5", "2026-09-06").unwrap();

        // Even with a different corpus, and even for a different release.
        std::fs::write(
            &dataset,
            format!("{RECORD}\n{RECORD}").replace("fam-1-a", "fam-1-b"),
        )
        .unwrap();
        let error = seal(&dataset, &out, "0.6", "2026-10-01").unwrap_err();
        assert!(error.contains("уже существует"), "{error}");

        let unchanged: FrozenSeal =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(unchanged.sha256, first.sha256);
        assert_eq!(unchanged.sealed_for, "0.5");
    }

    /// Freezing a corpus with a split leak would fix the leak in place
    /// forever, and every number measured on it afterwards would be wrong in
    /// the same way.
    #[test]
    fn a_corpus_with_a_split_leak_is_not_sealed() {
        let dir = tempfile::tempdir().unwrap();
        let dataset = dir.path().join("leaky.jsonl");
        std::fs::write(&dataset, leaking()).unwrap();
        let out = dir.path().join("seal.json");
        let error = seal(&dataset, &out, "0.5", "2026-09-06").unwrap_err();
        assert!(error.contains("split"), "{error}");
        assert!(!out.exists(), "a refused seal must leave no file behind");
    }

    #[test]
    fn a_corpus_that_does_not_load_is_not_sealed() {
        let dir = tempfile::tempdir().unwrap();
        let dataset = dir.path().join("broken.jsonl");
        std::fs::write(&dataset, "{not json").unwrap();
        let out = dir.path().join("seal.json");
        assert!(seal(&dataset, &out, "0.5", "2026-09-06").is_err());
        assert!(!out.exists());
    }

    /// Sealing the same bytes twice, in different places, gives the same
    /// digest — the seal names the content, not the run.
    #[test]
    fn the_seal_names_the_content_not_the_moment() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jsonl");
        let b = dir.path().join("b.jsonl");
        std::fs::write(&a, RECORD).unwrap();
        std::fs::write(&b, RECORD).unwrap();
        let one = seal(&a, &dir.path().join("one.json"), "0.5", "2026-09-06").unwrap();
        let two = seal(&b, &dir.path().join("two.json"), "0.5", "2027-01-01").unwrap();
        assert_eq!(one.sha256, two.sha256);
    }
}

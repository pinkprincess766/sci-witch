//! `rq2-report`: risk–coverage curve and AURC with a phrase-family bootstrap
//! (question 2, plan item 7) from per-example scores.
//!
//! Input is JSON Lines, one object per answered-or-not example:
//!
//! ```text
//! {"id": "...", "family_id": "...", "stage": 0, "score": 0.95, "correct": true}
//! ```
//!
//! - `id`: unique example id (string);
//! - `family_id`: phrase family, the bootstrap unit (string);
//! - `stage`: synthesis stage 0…7 of voice-v2 (unsigned integer);
//! - `score`: confidence the curve ranks by, higher answers first (number, not NaN);
//! - `correct`: whether the parse is right (bool);
//! - `split` (optional): if present it must be `train`. Anything else,
//!   including `dev_holdout`, is refused: the frozen test is never read here.
//!
//! Unknown fields are refused, so a typo cannot silently drop a value. Empty
//! lines are skipped. `--max-stage s` keeps stages 0…s (the stop stage).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::metrics::group_by_family;
use crate::selective::{
    aurc_family_bootstrap, risk_coverage_curve, AurcBootstrap, RiskCoveragePoint,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rq2Example {
    pub id: String,
    pub family_id: String,
    pub stage: u32,
    pub score: f64,
    pub correct: bool,
    #[serde(default)]
    pub split: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Rq2Report {
    pub report: &'static str,
    pub max_stage: Option<u32>,
    pub stages: Vec<u32>,
    pub examples: usize,
    pub errors: usize,
    pub families: usize,
    pub aurc: AurcBootstrap,
    pub risk_coverage: Vec<RiskCoveragePoint>,
}

pub fn parse(text: &str) -> Result<Vec<Rq2Example>, String> {
    let mut out = Vec::new();
    let mut ids = BTreeSet::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let ex: Rq2Example =
            serde_json::from_str(line).map_err(|e| format!("line {}: {e}", n + 1))?;
        if let Some(split) = &ex.split {
            if split != "train" {
                return Err(format!(
                    "line {}: split `{split}` refused, rq2-report reads train only",
                    n + 1
                ));
            }
        }
        if ex.score.is_nan() {
            return Err(format!("line {}: score is NaN", n + 1));
        }
        if !ids.insert(ex.id.clone()) {
            return Err(format!("line {}: duplicate id `{}`", n + 1, ex.id));
        }
        out.push(ex);
    }
    Ok(out)
}

pub fn build(
    examples: &[Rq2Example],
    max_stage: Option<u32>,
    seed: u64,
    resamples: usize,
) -> Result<Rq2Report, String> {
    let kept: Vec<&Rq2Example> = examples
        .iter()
        .filter(|e| max_stage.is_none_or(|s| e.stage <= s))
        .collect();
    let scores: Vec<f64> = kept.iter().map(|e| e.score).collect();
    let correct: Vec<bool> = kept.iter().map(|e| e.correct).collect();
    let risk_coverage =
        risk_coverage_curve(&scores, &correct).ok_or("no examples left after --max-stage")?;
    let families: Vec<Vec<(f64, bool)>> = group_by_family(&kept, |e| e.family_id.clone())
        .into_iter()
        .map(|f| f.iter().map(|e| (e.score, e.correct)).collect())
        .collect();
    let aurc = aurc_family_bootstrap(&families, seed, resamples)
        .map_err(|e| format!("AURC bootstrap refused: {e:?}"))?;
    Ok(Rq2Report {
        report: "rq2-risk-coverage-v1",
        max_stage,
        stages: kept
            .iter()
            .map(|e| e.stage)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        examples: kept.len(),
        errors: aurc.errors,
        families: aurc.families,
        aurc,
        risk_coverage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_non_train_split_unknown_fields_and_duplicates() {
        let ok = r#"{"id":"a","family_id":"f","stage":0,"score":1.0,"correct":true}"#;
        assert!(parse(ok).is_ok());
        let test = r#"{"id":"a","family_id":"f","stage":0,"score":1.0,"correct":true,"split":"dev_holdout"}"#;
        assert!(parse(test).unwrap_err().contains("train only"));
        let typo = r#"{"id":"a","family_id":"f","stage":0,"score":1.0,"corect":true}"#;
        assert!(parse(typo).is_err());
        assert!(parse(&format!("{ok}\n{ok}"))
            .unwrap_err()
            .contains("duplicate"));
    }

    #[test]
    fn one_family_gives_an_error_not_an_interval() {
        let ex = parse(
            "{\"id\":\"a\",\"family_id\":\"f\",\"stage\":0,\"score\":1.0,\"correct\":true}\n\
             {\"id\":\"b\",\"family_id\":\"f\",\"stage\":1,\"score\":0.5,\"correct\":false}",
        )
        .unwrap();
        assert!(build(&ex, None, 1, 10)
            .unwrap_err()
            .contains("TooFewFamilies"));
    }
}

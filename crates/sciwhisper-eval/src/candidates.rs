//! Candidate generation on top of the real `sciwhisper-core`.
//!
//! There is no second scientific parser here. Every candidate comes from
//! `sciwhisper_core::interpret`, plus the `RAW` safety action, plus the
//! alternatives the core parser itself offers. The gold answer is never
//! inserted into an ordinary candidate set — that only happens inside the
//! explicitly named oracle in `oracle.rs`.

use sciwhisper_core::{interpret, Domain, InterpretOptions, Node};

use crate::canonical::{canonical_target_v1, Target};
use crate::schema::AsrHypothesis;

/// Which domains the generator is allowed to try.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomainPolicy {
    /// Exactly what the shipped application does: one automatic routing pass.
    Auto,
    /// Automatic routing plus one pass per scientific domain. This is a
    /// laboratory setting: it buys recall by spending latency, and the report
    /// has to say so.
    AutoThenExplicit,
    /// The corpus domain, used only by oracle experiments.
    Oracle(Domain),
}

impl DomainPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            DomainPolicy::Auto => "auto",
            DomainPolicy::AutoThenExplicit => "auto_then_explicit",
            DomainPolicy::Oracle(_) => "oracle_domain",
        }
    }

    fn passes(self) -> Vec<(Domain, CandidateSource)> {
        match self {
            DomainPolicy::Auto => vec![(Domain::Auto, CandidateSource::PrimaryParse)],
            DomainPolicy::AutoThenExplicit => vec![
                (Domain::Auto, CandidateSource::PrimaryParse),
                (Domain::Chemistry, CandidateSource::ExplicitDomainParse),
                (Domain::Mathematics, CandidateSource::ExplicitDomainParse),
                (Domain::Physics, CandidateSource::ExplicitDomainParse),
            ],
            DomainPolicy::Oracle(domain) => vec![(domain, CandidateSource::PrimaryParse)],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateSource {
    PrimaryParse,
    ParserAlternative,
    ExplicitDomainParse,
    Raw,
    /// Only ever produced by `oracle::oracle_candidates`.
    OracleGold,
}

impl CandidateSource {
    pub fn as_str(self) -> &'static str {
        match self {
            CandidateSource::PrimaryParse => "primary_parse",
            CandidateSource::ParserAlternative => "parser_alternative",
            CandidateSource::ExplicitDomainParse => "explicit_domain_parse",
            CandidateSource::Raw => "raw",
            CandidateSource::OracleGold => "oracle_gold",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub action: Target,
    pub canonical: String,
    pub transcript: String,
    pub transcript_index: usize,
    /// The domain that produced this candidate. `None` for `RAW`, which is
    /// not the product of any domain.
    pub domain: Option<Domain>,
    pub resolved_domain: Option<Domain>,
    pub source: CandidateSource,
    /// Position in the generated order, before the `K` cut.
    pub order: usize,
    pub warnings: Vec<String>,
    pub structural_confidence: f32,
    /// The AST satisfies the structural invariants of the core validator.
    /// This is not a claim that the science is right: a deliberately
    /// unbalanced reaction is structurally valid and merely warned about.
    pub structurally_valid: bool,
}

impl Candidate {
    pub fn is_raw(&self) -> bool {
        matches!(self.action, Target::Raw)
    }
}

/// Builds the ordered, deduplicated candidate list for one utterance.
///
/// The order is fully determined by the inputs: hypotheses in their given
/// order, domains in the policy's fixed order, the primary parse before the
/// parser's own alternatives, and `RAW` last. No hash iteration is involved,
/// so two runs on the same corpus produce the same list.
pub fn generate_candidates(
    transcripts: &[AsrHypothesis],
    domain_policy: DomainPolicy,
    k: usize,
) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let push = |candidate: Candidate, out: &mut Vec<Candidate>, seen: &mut Vec<String>| {
        if seen.iter().any(|key| key == &candidate.canonical) {
            return;
        }
        seen.push(candidate.canonical.clone());
        out.push(candidate);
    };

    for (transcript_index, hypothesis) in transcripts.iter().enumerate() {
        for (domain, source) in domain_policy.passes() {
            let result = interpret(
                &hypothesis.text,
                InterpretOptions {
                    domain,
                    allow_shortcuts: true,
                },
            );
            let warnings: Vec<String> = result
                .warnings
                .iter()
                .map(|warning| warning.code.clone())
                .collect();
            if let Some(node) = ast_candidate(&result) {
                if let Some(candidate) = build(
                    Target::Ast(node),
                    hypothesis,
                    transcript_index,
                    Some(domain),
                    Some(result.domain),
                    source,
                    warnings.clone(),
                    result.confidence,
                ) {
                    push(candidate, &mut out, &mut seen);
                }
            }
            for alternative in &result.alternatives {
                if matches!(alternative, Node::Text(_)) {
                    continue;
                }
                if let Some(candidate) = build(
                    Target::Ast(alternative.clone()),
                    hypothesis,
                    transcript_index,
                    Some(domain),
                    Some(result.domain),
                    CandidateSource::ParserAlternative,
                    warnings.clone(),
                    result.confidence,
                ) {
                    push(candidate, &mut out, &mut seen);
                }
            }
        }
    }

    // `RAW` is a real action, not a filler: keeping the dictated words is the
    // right answer for ordinary speech. It goes last because the deterministic
    // baseline prefers a successful scientific parse, which is what the
    // shipped application does.
    let fallback = transcripts
        .first()
        .map(|hypothesis| hypothesis.text.clone())
        .unwrap_or_default();
    let raw = Candidate {
        action: Target::Raw,
        canonical: crate::canonical::RAW_KEY.to_string(),
        transcript: fallback,
        transcript_index: 0,
        domain: None,
        resolved_domain: None,
        source: CandidateSource::Raw,
        order: out.len(),
        warnings: Vec::new(),
        structural_confidence: 0.0,
        structurally_valid: true,
    };
    push(raw, &mut out, &mut seen);

    for (position, candidate) in out.iter_mut().enumerate() {
        candidate.order = position;
    }
    out.truncate(k);
    out
}

/// The AST the core actually decided on, or `None` when the core fell back to
/// the raw transcript. A zero-confidence result and a `Node::Text` are both
/// the system saying "I did not build a scientific structure here", which is
/// the `RAW` action, not an AST candidate.
fn ast_candidate(result: &sciwhisper_core::InterpretationResult) -> Option<Node> {
    if result.confidence <= 0.0 {
        return None;
    }
    match &result.ast {
        Node::Text(_) => None,
        other => Some(other.clone()),
    }
}

#[allow(clippy::too_many_arguments)]
fn build(
    action: Target,
    hypothesis: &AsrHypothesis,
    transcript_index: usize,
    domain: Option<Domain>,
    resolved_domain: Option<Domain>,
    source: CandidateSource,
    warnings: Vec<String>,
    structural_confidence: f32,
) -> Option<Candidate> {
    // A candidate that cannot be canonicalised cannot be compared, so it is
    // dropped rather than counted as an unnamed answer.
    let canonical = canonical_target_v1(&action).ok()?;
    let structurally_valid = match action.ast() {
        Some(node) => sciwhisper_core::validate::semantic_warnings(node)
            .iter()
            .all(|warning| !warning.code.starts_with("math.")),
        None => true,
    };
    Some(Candidate {
        action,
        canonical,
        transcript: hypothesis.text.clone(),
        transcript_index,
        domain,
        resolved_domain,
        source,
        order: 0,
        warnings,
        structural_confidence,
        structurally_valid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hyp(text: &str) -> Vec<AsrHypothesis> {
        vec![AsrHypothesis {
            text: text.into(),
            score: None,
        }]
    }

    #[test]
    fn raw_is_always_offered_as_its_own_action() {
        let candidates = generate_candidates(&hyp("вода"), DomainPolicy::Auto, 16);
        assert!(candidates.iter().any(Candidate::is_raw));
        // and it is the last one, because a successful parse ranks first
        assert!(candidates.last().unwrap().is_raw());
    }

    #[test]
    fn an_unparsable_utterance_yields_raw_only() {
        let candidates = generate_candidates(&hyp("предел терпения"), DomainPolicy::Auto, 16);
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].is_raw());
        assert_eq!(candidates[0].canonical, crate::canonical::RAW_KEY);
    }

    #[test]
    fn candidates_are_deduplicated_by_canonical_form() {
        // Every explicit domain that parses «вода» produces the same AST, so
        // the set must not carry four copies of one answer.
        let candidates = generate_candidates(&hyp("вода"), DomainPolicy::AutoThenExplicit, 16);
        let mut keys: Vec<&str> = candidates.iter().map(|c| c.canonical.as_str()).collect();
        let before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(
            before,
            keys.len(),
            "duplicate canonical keys: {candidates:?}"
        );
    }

    #[test]
    fn generation_is_deterministic() {
        let first = generate_candidates(
            &hyp("икс в квадрате плюс два икс минус три равно нулю"),
            DomainPolicy::AutoThenExplicit,
            16,
        );
        let second = generate_candidates(
            &hyp("икс в квадрате плюс два икс минус три равно нулю"),
            DomainPolicy::AutoThenExplicit,
            16,
        );
        let keys = |list: &[Candidate]| {
            list.iter()
                .map(|c| (c.canonical.clone(), c.source.as_str()))
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&first), keys(&second));
    }

    #[test]
    fn k_truncates_the_list_but_keeps_the_prefix() {
        let full = generate_candidates(&hyp("вода"), DomainPolicy::AutoThenExplicit, 16);
        for k in [1usize, 2, 4, 8, 16] {
            let cut = generate_candidates(&hyp("вода"), DomainPolicy::AutoThenExplicit, k);
            assert!(cut.len() <= k);
            assert_eq!(cut.len(), full.len().min(k));
            for (a, b) in cut.iter().zip(full.iter()) {
                assert_eq!(a.canonical, b.canonical);
                assert_eq!(a.order, b.order);
            }
        }
    }

    #[test]
    fn the_generator_never_sees_or_inserts_the_gold_answer() {
        // The signature carries transcripts, a policy and K — there is no way
        // to pass the gold in, which is the structural guarantee behind
        // "no gold leakage". The oracle variant is a separate function.
        let candidates = generate_candidates(&hyp("предел терпения"), DomainPolicy::Auto, 16);
        assert!(candidates
            .iter()
            .all(|c| c.source != CandidateSource::OracleGold));
    }

    #[test]
    fn an_ordinary_sentence_does_not_produce_a_scientific_candidate() {
        for text in ["производная была опубликована", "порядок величины"]
        {
            let candidates = generate_candidates(&hyp(text), DomainPolicy::Auto, 16);
            assert!(
                candidates.iter().all(Candidate::is_raw),
                "{text} produced {candidates:?}"
            );
        }
    }
}

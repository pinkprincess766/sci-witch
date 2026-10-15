//! Text-side features for calibration question 2.
//!
//! Only features that a named function in this repository computes are here.
//! Each one comes from the same core call the harness already makes:
//!
//! - `warning_codes`: `interpret` with `Domain::Auto`, as `candidates.rs`
//!   calls it. Codes only, in the order the core produced them.
//! - `routed_domain`: `result.domain` from that same call. This is what
//!   `candidates.rs` stores as `resolved_domain`.
//! - `lattice_candidates`: `lattice::build` with `LatticeOptions::default()`,
//!   as `main.rs` (`dump-lattice`) calls it. It is the length of
//!   `Lattice::candidates` as built: `RAW` is included and the harness's `K`
//!   cut is not applied.
//! - `words`: `text.split_whitespace().count()` on the raw transcript. There
//!   is no normalisation, so case, `ё` and punctuation stay as spoken, and
//!   «два,» is one word. This is not the split `word_error_rate` uses, which
//!   lowercases and folds `ё` first.
//! - `earley_parses`: `count_parses` on `docs/grammar/math.ebnf` from its start
//!   symbol, over `math_token_classes` of the text. The mode follows
//!   `routed_domain`: `Mathematics` is math mode, `Physics` is physics mode,
//!   and any other domain is `None`, because the math grammar does not
//!   describe it. A tokenizer error is `TokenizeFailed`. The grammar is
//!   compiled into the binary. No bound is hidden: `AtLeastMax`, `TooLong`
//!   and `TooManyItems` are separate values, and `Rejected` is not `Exact(0)`.
//!
//! - `PcfgScorer`: `ln P(text)` per token class under a PCFG over
//!   `docs/grammar/math.ebnf`, a separate scorer and not a field of
//!   `TextFeatures`, because it carries a fitted model. One PCFG per token
//!   mode (math, physics). `fit` takes training-split transcripts only; that
//!   is the caller's duty, nothing here can check it. Each text is tokenized
//!   in both modes, its derivations are enumerated, and each mode is fitted
//!   by `em_estimate` with add-one smoothing and `EM_ITERATIONS`: the `em`
//!   model of `research/results/pcfg-v2.json`. Texts that fail to tokenize,
//!   are rejected or have too many trees are skipped. A text is scored under
//!   the mode of its routed domain (`Mathematics` is math, `Physics` is
//!   physics). The value is `None` for any other domain, and whenever
//!   `earley_parses` for the same text and domain is not `Exact`: tokenizer
//!   error, rejected, too many trees, too long, too many items.
//!
//! Not computed, so absent here. Nothing fills them with a mean or a guess:
//!
//! - Whisper token log-prob (mean and minimum): the harness has no per-token
//!   output. `AsrHypothesis::score` is one number per hypothesis, not that.
//! - `no_speech_prob`: not in the eval report.
//! - Domain agreement in `route`: `route` returns only the chosen domain and
//!   keeps no counter. No counter is invented here, because the protocol does
//!   not define one.

use std::sync::OnceLock;

use sciwhisper_core::lattice::{self, LatticeOptions};
use sciwhisper_core::research::math_token_classes;
use sciwhisper_core::{interpret, Domain, InterpretOptions};
use sciwhisper_grammar::likelihood::sentence_log_prob;
use sciwhisper_grammar::pcfg::{em_estimate, EM_ITERATIONS, PSEUDOCOUNT};
use sciwhisper_grammar::{
    count_parses, derivations, productions, CountOutcome, DerivationOutcome, Grammar, ParseCount,
};

/// What `count_parses` said about the text, one value per outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EarleyParses {
    /// Derivation trees, up to and including `MAX_PARSES`.
    Exact(u32),
    /// More than `MAX_PARSES` trees, or a nullable cycle.
    AtLeastMax,
    /// The grammar does not accept the token classes.
    Rejected,
    /// More tokens than `MAX_TOKENS`; the chart was not built.
    TooLong,
    /// The chart passed `MAX_ITEMS`; it was abandoned.
    TooManyItems,
    /// `math_token_classes` returned an error, so there was nothing to parse.
    TokenizeFailed,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TextFeatures {
    pub warning_codes: Vec<String>,
    pub lattice_candidates: usize,
    pub routed_domain: Domain,
    pub words: usize,
    /// `None` when `routed_domain` is neither `Mathematics` nor `Physics`.
    pub earley_parses: Option<EarleyParses>,
}

fn math_grammar() -> &'static Grammar {
    static GRAMMAR: OnceLock<Grammar> = OnceLock::new();
    GRAMMAR.get_or_init(|| {
        sciwhisper_grammar::parse(include_str!("../../../docs/grammar/math.ebnf"))
            .expect("docs/grammar/math.ebnf must parse")
    })
}

fn earley_parses(text: &str, domain: Domain) -> Option<EarleyParses> {
    let physics = match domain {
        Domain::Mathematics => false,
        Domain::Physics => true,
        _ => return None,
    };
    let Ok(classes) = math_token_classes(text, physics) else {
        return Some(EarleyParses::TokenizeFailed);
    };
    let refs: Vec<&str> = classes.iter().map(String::as_str).collect();
    let grammar = math_grammar();
    let start = grammar
        .start()
        .expect("docs/grammar/math.ebnf must have a start symbol");
    Some(match count_parses(grammar, start, &refs) {
        CountOutcome::Counted(ParseCount::Exact(n)) => EarleyParses::Exact(n),
        CountOutcome::Counted(ParseCount::AtLeastMax) => EarleyParses::AtLeastMax,
        CountOutcome::Rejected { .. } => EarleyParses::Rejected,
        CountOutcome::TooLong => EarleyParses::TooLong,
        CountOutcome::TooManyItems => EarleyParses::TooManyItems,
    })
}

pub fn text_features(text: &str) -> TextFeatures {
    let result = interpret(
        text,
        InterpretOptions {
            domain: Domain::Auto,
            allow_shortcuts: true,
        },
    );
    let lattice = lattice::build(text, LatticeOptions::default());
    TextFeatures {
        warning_codes: result
            .warnings
            .iter()
            .map(|warning| warning.code.clone())
            .collect(),
        lattice_candidates: lattice.candidates.len(),
        routed_domain: result.domain,
        words: text.split_whitespace().count(),
        earley_parses: earley_parses(text, result.domain),
    }
}

fn grammar_start() -> &'static str {
    math_grammar()
        .start()
        .expect("docs/grammar/math.ebnf must have a start symbol")
}

/// Derivation trees of the text's token classes in one mode, and the token
/// count. `None` when the tokenizer fails or `derivations` is not
/// `Derivations`.
fn derive(text: &str, physics: bool) -> Option<(Vec<Vec<usize>>, usize)> {
    let classes = math_token_classes(text, physics).ok()?;
    let refs: Vec<&str> = classes.iter().map(String::as_str).collect();
    match derivations(math_grammar(), grammar_start(), &refs) {
        DerivationOutcome::Derivations(trees) => Some((trees, refs.len())),
        _ => None,
    }
}

/// One PCFG per token mode (math, physics), fitted by EM on training
/// transcripts.
pub struct PcfgScorer {
    /// Probability of each production, indexed by `productions` id.
    math: Vec<f64>,
    physics: Vec<f64>,
}

impl PcfgScorer {
    /// Fit on `train_texts`, which must be training-split transcripts only
    /// (the caller's duty: scoring a text the model was fitted on is not a
    /// held-out score). Each text is tokenized in both modes; texts that fail
    /// to tokenize, are rejected or have too many trees are skipped in that
    /// mode. Then `em_estimate` with `PSEUDOCOUNT` and `EM_ITERATIONS`.
    pub fn fit(train_texts: &[&str]) -> Result<Self, String> {
        let prods = productions(math_grammar());
        let fit_mode = |physics: bool, mode: &str| -> Result<Vec<f64>, String> {
            let sentences: Vec<Vec<Vec<usize>>> = train_texts
                .iter()
                .filter_map(|text| derive(text, physics).map(|(trees, _)| trees))
                .collect();
            em_estimate(&prods, &sentences, PSEUDOCOUNT, EM_ITERATIONS)
                .map(|fit| fit.prob)
                .map_err(|error| format!("{mode}: {error}"))
        };
        Ok(Self {
            math: fit_mode(false, "math")?,
            physics: fit_mode(true, "physics")?,
        })
    }

    /// `ln P(text)` per token class, nats, under the mode of `routed_domain`
    /// (`Mathematics` is math, `Physics` is physics). `None` for any other
    /// domain, and whenever `earley_parses` for the same text and domain is
    /// not `Exact`.
    pub fn log_prob_per_token(&self, text: &str, routed_domain: Domain) -> Option<f64> {
        let physics = match routed_domain {
            Domain::Mathematics => false,
            Domain::Physics => true,
            _ => return None,
        };
        let (total, tokens) = self.log_prob_total(text, physics)?;
        Some(total / tokens as f64)
    }

    /// `ln P(text)` in nats and the token count, in a forced mode. A text with
    /// no token classes has no per-token value, so it is `None`.
    fn log_prob_total(&self, text: &str, physics: bool) -> Option<(f64, usize)> {
        let (trees, tokens) = derive(text, physics)?;
        if tokens == 0 {
            return None;
        }
        let prob = if physics { &self.physics } else { &self.math };
        Some((sentence_log_prob(prob, &trees), tokens))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values are read from the current core, by running
    // `text_features` on these phrases in this tree. They are not derived by
    // hand. If the core changes them, the change is a change of the
    // calibration inputs and should be seen here.

    #[test]
    fn a_formula_has_no_warning_two_lattice_readings_and_five_words() {
        assert_eq!(
            text_features("икс в квадрате плюс один"),
            TextFeatures {
                warning_codes: vec![],
                lattice_candidates: 2,
                routed_domain: Domain::Mathematics,
                words: 5,
                earley_parses: Some(EarleyParses::Exact(1)),
            }
        );
    }

    #[test]
    fn an_unbalanced_reaction_reports_its_warning_code() {
        assert_eq!(
            text_features("уксусная кислота окисляется до аш два о"),
            TextFeatures {
                warning_codes: vec!["chemistry.unbalanced_atoms".to_string()],
                lattice_candidates: 2,
                routed_domain: Domain::Chemistry,
                words: 7,
                earley_parses: None,
            }
        );
    }

    // Nothing parses this, so the core reports `unresolved`. The routed domain
    // is then the keyword guess («вода» is a chemistry word), not a decision.
    // The lattice holds only `RAW`, which is why its count is 1.
    #[test]
    fn plain_prose_is_unresolved_and_routed_by_keyword_guess() {
        assert_eq!(
            text_features("вода закипела в чайнике"),
            TextFeatures {
                warning_codes: vec!["unresolved".to_string()],
                lattice_candidates: 1,
                routed_domain: Domain::Chemistry,
                words: 4,
                earley_parses: None,
            }
        );
    }

    #[test]
    fn an_empty_transcript_has_zero_words() {
        assert_eq!(
            text_features(""),
            TextFeatures {
                warning_codes: vec!["unresolved".to_string()],
                lattice_candidates: 1,
                routed_domain: Domain::Auto,
                words: 0,
                earley_parses: None,
            }
        );
    }

    // The split is `split_whitespace` on the raw text: case, `ё` and
    // punctuation are kept, so «Два,» is one word.
    #[test]
    fn words_are_whitespace_pieces_of_the_raw_text() {
        assert_eq!(text_features("  Два,   ёж  ").words, 2);
    }

    // Ambiguity is the point of this feature: the count is the number of
    // derivation trees, 6 for both modes in `grammar-differential-v2.json`.
    #[test]
    fn an_ambiguous_phrase_has_its_known_parse_count() {
        let features = text_features("интеграл эф дэ икс");
        assert_eq!(features.routed_domain, Domain::Mathematics);
        assert_eq!(features.earley_parses, Some(EarleyParses::Exact(6)));
    }

    // The math grammar does not describe chemistry, prose that the keyword
    // guess sends to chemistry, or domains that are not decided.
    #[test]
    fn chemistry_has_no_parse_count() {
        let features = text_features("хлорид натрия");
        assert_eq!(features.routed_domain, Domain::Chemistry);
        assert_eq!(features.earley_parses, None);
    }

    #[test]
    fn a_math_phrase_is_not_counted_under_a_domain_the_grammar_does_not_describe() {
        for domain in [Domain::Chemistry, Domain::Plain, Domain::Auto] {
            assert_eq!(earley_parses("икс плюс один", domain), None, "{domain:?}");
        }
    }

    // «три метра» is a quantity, so the math tokenizer refuses the unit word.
    // The count 4 exists only because `Physics` selects physics mode.
    #[test]
    fn a_physics_phrase_is_counted_in_physics_mode() {
        let text = "три метра плюс четыре сантиметра";
        assert!(math_token_classes(text, false).is_err());
        let features = text_features(text);
        assert_eq!(features.routed_domain, Domain::Physics);
        assert_eq!(features.earley_parses, Some(EarleyParses::Exact(4)));
        assert_eq!(
            earley_parses(text, Domain::Mathematics),
            Some(EarleyParses::TokenizeFailed)
        );
    }

    // A rejected sentence is not a sentence with zero trees.
    #[test]
    fn a_sentence_the_grammar_rejects_is_rejected_not_zero() {
        let features = text_features("икс плюс");
        assert_eq!(features.routed_domain, Domain::Mathematics);
        assert_eq!(features.earley_parses, Some(EarleyParses::Rejected));
    }

    // Prose that the router sends to mathematics has no token classes.
    #[test]
    fn prose_routed_to_mathematics_fails_to_tokenize() {
        let features = text_features("я вчера ходил в магазин");
        assert_eq!(features.routed_domain, Domain::Mathematics);
        assert_eq!(features.earley_parses, Some(EarleyParses::TokenizeFailed));
    }

    // 129 «икс» and 128 «плюс» are 257 tokens, one past `MAX_TOKENS`.
    #[test]
    fn a_sentence_past_the_token_cap_is_too_long() {
        let text = vec!["икс"; 129].join(" плюс ");
        assert_eq!(
            earley_parses(&text, Domain::Mathematics),
            Some(EarleyParses::TooLong)
        );
    }

    // PcfgScorer. The corpora and splits are read as
    // `sciwhisper-grammar/tests/pcfg_report_v2.rs` reads them: `dev_holdout`
    // records are counted and never looked at.

    const CORPORA: [&str; 4] = [
        "research/data/dev-seed-v3.jsonl",
        "research/data/ambiguous-v2.jsonl",
        "research/data/nomenclature-v1.jsonl",
        "research/data/prose-negatives-v2.jsonl",
    ];
    const MODES: [(&str, bool); 2] = [("math", false), ("physics", true)];
    // The report rounds to 6 decimals, so each total is off by at most 5e-7.
    const REPORT_TOLERANCE: f64 = 1e-5;

    fn repo_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    struct Splits {
        train: Vec<String>,
        validation: Vec<String>,
    }

    fn splits() -> &'static Splits {
        static SPLITS: OnceLock<Splits> = OnceLock::new();
        SPLITS.get_or_init(|| {
            let mut splits = Splits {
                train: vec![],
                validation: vec![],
            };
            for rel in CORPORA {
                let path = repo_root().join(rel);
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                for (i, line) in text
                    .lines()
                    .enumerate()
                    .filter(|(_, l)| !l.trim().is_empty())
                {
                    let value: serde_json::Value = serde_json::from_str(line)
                        .unwrap_or_else(|e| panic!("{}:{i}: {e}", path.display()));
                    let target = match value["split"].as_str() {
                        Some("dev_holdout") => continue,
                        Some("train") => &mut splits.train,
                        Some("validation") => &mut splits.validation,
                        other => panic!("{}:{i}: unknown split {other:?}", path.display()),
                    };
                    let transcript = value["human_transcript"]
                        .as_str()
                        .unwrap_or_else(|| panic!("{}:{i}: no human_transcript", path.display()));
                    target.push(transcript.to_string());
                }
            }
            splits
        })
    }

    fn refs(texts: &[String]) -> Vec<&str> {
        texts.iter().map(String::as_str).collect()
    }

    fn train_scorer() -> &'static PcfgScorer {
        static SCORER: OnceLock<PcfgScorer> = OnceLock::new();
        SCORER.get_or_init(|| PcfgScorer::fit(&refs(&splits().train)).expect("fit on train"))
    }

    /// Sum of `ln P` over the validation texts that parse in the forced mode,
    /// and how many that is.
    fn validation_total(scorer: &PcfgScorer, physics: bool) -> (f64, usize) {
        let scored: Vec<f64> = splits()
            .validation
            .iter()
            .filter_map(|text| scorer.log_prob_total(text, physics))
            .map(|(total, _)| total)
            .collect();
        (scored.iter().sum(), scored.len())
    }

    fn stored_report() -> serde_json::Value {
        let path = repo_root().join("research/results/pcfg-v2.json");
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_str(&text).expect("pcfg-v2.json must parse")
    }

    // The key test: the scorer is the `em` model of the published report.
    // The `used` count guards against a sum over nothing.
    #[test]
    fn pcfg_validation_total_equals_the_published_em_model() {
        let report = stored_report();
        for (mode, physics) in MODES {
            let (total, used) = validation_total(train_scorer(), physics);
            let stored = &report["modes"][mode];
            assert_eq!(
                used as u64,
                stored["validation"]["used"].as_u64().expect("used"),
                "{mode}: used validation sentences"
            );
            let published = stored["models"]["em"]["validation_log_prob"]
                .as_f64()
                .expect("validation_log_prob");
            assert!(
                (total - published).abs() < REPORT_TOLERANCE,
                "{mode}: live {total} vs published {published}"
            );
        }
    }

    #[test]
    fn pcfg_has_no_value_outside_mathematics_and_physics() {
        let text = "хлорид натрия";
        assert_eq!(text_features(text).routed_domain, Domain::Chemistry);
        for domain in [Domain::Chemistry, Domain::Plain, Domain::Auto] {
            assert_eq!(
                train_scorer().log_prob_per_token("икс плюс один", domain),
                None,
                "{domain:?}"
            );
        }
        assert_eq!(
            train_scorer().log_prob_per_token(text, Domain::Chemistry),
            None
        );
    }

    #[test]
    fn a_math_phrase_has_a_negative_finite_value() {
        let text = "интеграл эф дэ икс";
        let domain = text_features(text).routed_domain;
        assert_eq!(domain, Domain::Mathematics);
        let value = train_scorer()
            .log_prob_per_token(text, domain)
            .expect("math phrase is scored");
        assert!(value.is_finite() && value < 0.0, "{value}");
    }

    // Math mode cannot tokenize «метра», so only the routed mode scores it.
    #[test]
    fn a_physics_phrase_is_scored_in_physics_mode_only() {
        let text = "три метра плюс четыре сантиметра";
        let domain = text_features(text).routed_domain;
        assert_eq!(domain, Domain::Physics);
        let scorer = train_scorer();
        let value = scorer.log_prob_per_token(text, domain).expect("physics");
        assert!(value.is_finite() && value < 0.0, "{value}");
        assert_eq!(scorer.log_prob_per_token(text, Domain::Mathematics), None);
        assert_eq!(scorer.log_prob_total(text, false), None);
    }

    // The value exists exactly where `earley_parses` says `Exact`.
    #[test]
    fn pcfg_is_none_exactly_when_the_parse_count_is_not_exact() {
        let (mut some, mut none) = (0, 0);
        for text in &splits().validation {
            for domain in [Domain::Mathematics, Domain::Physics] {
                let has_value = train_scorer().log_prob_per_token(text, domain).is_some();
                let exact = matches!(earley_parses(text, domain), Some(EarleyParses::Exact(_)));
                assert_eq!(has_value, exact, "{domain:?}: {text}");
                if has_value {
                    some += 1;
                } else {
                    none += 1;
                }
            }
        }
        // Both sides of the equivalence are exercised.
        assert!(some > 0 && none > 0, "some {some}, none {none}");
    }

    // Fitting on the validation texts too raises their likelihood. That the
    // totals differ is why `fit` must get the train split only.
    #[test]
    fn fitting_on_validation_texts_changes_the_validation_score() {
        let splits = splits();
        let all: Vec<&str> = splits
            .train
            .iter()
            .chain(&splits.validation)
            .map(String::as_str)
            .collect();
        let leaked = PcfgScorer::fit(&all).expect("fit on train and validation");
        for (mode, physics) in MODES {
            let (held_out, _) = validation_total(train_scorer(), physics);
            let (seen, _) = validation_total(&leaked, physics);
            assert!(
                seen > held_out,
                "{mode}: seen {seen} vs held out {held_out}"
            );
        }
    }
}

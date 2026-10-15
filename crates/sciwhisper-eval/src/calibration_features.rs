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
//! Not computed, so absent here. Nothing fills them with a mean or a guess:
//!
//! - Whisper token log-prob (mean and minimum): the harness has no per-token
//!   output. `AsrHypothesis::score` is one number per hypothesis, not that.
//! - `no_speech_prob`: not in the eval report.
//! - PCFG `log P`: the repository has no PCFG.
//! - Domain agreement in `route`: `route` returns only the chosen domain and
//!   keeps no counter. No counter is invented here, because the protocol does
//!   not define one.

use std::sync::OnceLock;

use sciwhisper_core::lattice::{self, LatticeOptions};
use sciwhisper_core::research::math_token_classes;
use sciwhisper_core::{interpret, Domain, InterpretOptions};
use sciwhisper_grammar::{count_parses, CountOutcome, Grammar, ParseCount};

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
}

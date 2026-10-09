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
//!
//! Not computed, so absent here. Nothing fills them with a mean or a guess:
//!
//! - Whisper token log-prob (mean and minimum): the harness has no per-token
//!   output. `AsrHypothesis::score` is one number per hypothesis, not that.
//! - `no_speech_prob`: not in the eval report.
//! - Earley parse count: `count_parses` lives in `sciwhisper-grammar`, which
//!   this crate does not depend on; adding that dependency is the owner's call.
//! - PCFG `log P`: the repository has no PCFG.
//! - Domain agreement in `route`: `route` returns only the chosen domain and
//!   keeps no counter. No counter is invented here, because the protocol does
//!   not define one.

use sciwhisper_core::lattice::{self, LatticeOptions};
use sciwhisper_core::{interpret, Domain, InterpretOptions};

#[derive(Debug, PartialEq, Eq)]
pub struct TextFeatures {
    pub warning_codes: Vec<String>,
    pub lattice_candidates: usize,
    pub routed_domain: Domain,
    pub words: usize,
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
            }
        );
    }

    // The split is `split_whitespace` on the raw text: case, `ё` and
    // punctuation are kept, so «Два,» is one word.
    #[test]
    fn words_are_whitespace_pieces_of_the_raw_text() {
        assert_eq!(text_features("  Два,   ёж  ").words, 2);
    }
}

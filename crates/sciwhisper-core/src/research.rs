//! Research-only entry points. Not part of the stable compiler surface;
//! signatures here may change without notice.

use crate::lexicon::Lexicon;
use crate::normalize::words as split_words;
use crate::numbers::NumberLex;
use crate::parser::math::{tokenize, MathMode, Tok};

/// Classes of the tokens the mathematics parser sees for `text`, by the names
/// `docs/grammar/math.ebnf` uses for its terminals (`Tok::Plus` → `PLUS`, …).
/// Research API: not part of the stable surface, may change without notice.
pub fn math_token_classes(text: &str, physics: bool) -> Result<Vec<String>, String> {
    let words = split_words(text);
    let lex = Lexicon::builtin();
    let nums = NumberLex::new();
    let mode = if physics {
        MathMode::Physics
    } else {
        MathMode::Math
    };
    let toks = tokenize(&words, lex, &nums, mode).map_err(|e| e.to_string())?;
    Ok(toks
        .iter()
        .map(Tok::class_name)
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::math_token_classes;

    #[test]
    fn token_classes_match_the_ebnf_terminal_names() {
        assert_eq!(
            math_token_classes("два плюс два", false).unwrap(),
            ["NUM", "PLUS", "NUM"]
        );
        assert_eq!(math_token_classes("икс", false).unwrap(), ["SYM"]);
        assert_eq!(math_token_classes("а", false).unwrap(), ["WEAK_SYM"]);
        assert_eq!(
            math_token_classes("синус икс", false).unwrap(),
            ["FUNCTION", "SYM"]
        );
    }

    #[test]
    fn a_word_the_tokenizer_does_not_know_is_an_error() {
        let err = math_token_classes("водкqxyz", false).unwrap_err();
        assert!(err.contains("unknown word"), "{err}");
    }

    #[test]
    fn physics_mode_emits_unit_where_math_mode_refuses() {
        let physics = math_token_classes("два метра", true).unwrap();
        assert_eq!(physics, ["NUM", "UNIT"]);
        assert!(math_token_classes("два метра", false).is_err());
    }
}

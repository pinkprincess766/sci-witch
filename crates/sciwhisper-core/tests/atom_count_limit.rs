//! Spoken coefficients and subscripts stop at [`MAX_ATOM_COUNT`].
//!
//! Digits, not number-words: `consume_int` parses a decimal token before it
//! consults the school lexicon, so this file does not depend on that lexicon.

use sciwhisper_core::ast::{Chemical, Node, Part};
use sciwhisper_core::lexicon::Lexicon;
use sciwhisper_core::numbers::NumberLex;
use sciwhisper_core::parser::chemistry::{parse_chemistry, MAX_ATOM_COUNT};

fn parse(text: &str) -> Result<Node, String> {
    let words: Vec<String> = text.split_whitespace().map(str::to_string).collect();
    parse_chemistry(&words, Lexicon::builtin(), &NumberLex::new()).map_err(|err| err.to_string())
}

fn species(text: &str) -> sciwhisper_core::ast::Species {
    match parse(text) {
        Ok(Node::Chemical(Chemical::Species(species))) => species,
        other => panic!("{text} did not parse as one species: {other:?}"),
    }
}

fn atom_count(text: &str, symbol: &str) -> u32 {
    let species = species(text);
    species
        .formula
        .parts
        .iter()
        .find_map(|part| match part {
            Part::Atom { symbol: got, count } if got == symbol => Some(*count),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{text} has no {symbol}: {:?}", species.formula.parts))
}

#[test]
fn a_coefficient_of_999_is_kept_and_1000_is_refused() {
    let kept = species("999 аш");
    assert_eq!(kept.coefficient, MAX_ATOM_COUNT);
    let err = parse("1000 аш").expect_err("1000 is past the coefficient limit");
    assert!(
        err.contains("коэффициент 1000") && err.contains("999"),
        "{err}"
    );
}

#[test]
fn a_subscript_of_999_is_kept_and_1000_is_not_a_formula() {
    assert_eq!(atom_count("аш 999", "H"), MAX_ATOM_COUNT);
    assert!(
        parse("аш 1000").is_err(),
        "a subscript of 1000 must not become a formula: {:?}",
        parse("аш 1000")
    );
}

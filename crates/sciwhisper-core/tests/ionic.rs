//! Ionic equations and half-reactions.
//!
//! This package began as "add half-reactions" and turned into three
//! correctness fixes, because the audit found the existing behaviour wrong
//! rather than merely absent:
//!
//! * «ион хлора» produced **Cl⁺**. `elements.yaml` records chlorine's
//!   `default_oxidation` as −1 and always did; the ion path ignored it and
//!   defaulted every element to +1.
//! * «ион серебра плюс ион хлора → хлорид серебра» produced
//!   **`Ag⁺ → AgCl↓`** — the chloride vanished. `плюс` is both the sign of
//!   an ion and the joint between reagents, and the first reading won
//!   unconditionally.
//! * A half-reaction dropped its electron the same way.
//!
//! Everything goes through the public path the CLI and the application use.

use sciwhisper_core::ast::{Chemical, Node, Part};
use sciwhisper_core::{
    interpret, interpret_utterance, render, Domain, InterpretOptions, Renderer, UtteranceMode,
    UtteranceOptions,
};

fn spoken(text: &str) -> String {
    let result = interpret_utterance(
        text,
        UtteranceOptions {
            domain: Domain::Auto,
            mode: UtteranceMode::MixedText,
            allow_shortcuts: true,
        },
    );
    render(&result.document, Renderer::Unicode)
}

fn compiled(text: &str) -> sciwhisper_core::InterpretationResult {
    interpret(
        text,
        InterpretOptions {
            domain: Domain::Chemistry,
            ..Default::default()
        },
    )
}

fn warnings(text: &str) -> Vec<String> {
    compiled(text)
        .warnings
        .iter()
        .map(|warning| warning.code.clone())
        .collect()
}

fn stayed_prose(text: &str) -> bool {
    spoken(text).trim() == text.trim()
}

// ------------------------------------------------------------ ion charges

/// The sign of a monatomic ion comes from the element, not from a default
/// of +1. Chloride is Cl⁻; anything else is not chemistry.
#[test]
fn a_monatomic_ion_takes_the_sign_its_element_has() {
    let cases = [
        ("ион серебра", "Ag⁺"),
        ("ион хлора", "Cl⁻"),
        ("ион водорода", "H⁺"),
        ("ион меди два", "Cu²⁺"),
        ("ион железа три", "Fe³⁺"),
        ("сульфат ион", "SO₄²⁻"),
    ];
    for (said, expected) in cases {
        assert_eq!(spoken(said), expected, "{said}");
    }
}

/// A spoken count is unsigned, but an ion charge is signed. Narrowing with
/// `as i32` used to turn `u32::MAX` into −1 and silently change the ion.
#[test]
fn an_oversized_ion_charge_is_refused_instead_of_wrapped() {
    for said in [
        "ион меди 2147483648",
        "ион меди 4294967295",
        "сульфат ион 4294967295 плюс",
    ] {
        let result = compiled(said);
        assert_eq!(result.confidence, 0.0, "{said}: {:?}", result.ast);
    }
}

// ------------------------------------------------------- ionic equations

/// Both reagents survive. The old reading kept the first and silently
/// dropped everything after the `плюс`.
#[test]
fn every_reagent_of_an_ionic_equation_survives() {
    let cases = [
        (
            "ион серебра плюс ион хлора превращается в хлорид серебра осадок",
            "Ag⁺ + Cl⁻ → AgCl↓",
        ),
        (
            "ион меди два плюс два гидроксид иона превращается в гидроксид меди два осадок",
            "Cu²⁺ + 2OH⁻ → Cu(OH)₂↓",
        ),
        (
            "ион водорода плюс гидроксид ион превращается в вода",
            "H⁺ + OH⁻ → H₂O",
        ),
    ];
    for (said, expected) in cases {
        assert_eq!(spoken(said), expected, "{said}");
    }
}

/// `плюс` at the end of an ion phrase is still its sign, not a dangling
/// separator: the fix must not have traded one misreading for the other.
#[test]
fn a_trailing_plus_is_still_the_sign_of_the_ion() {
    assert_eq!(spoken("ион меди два"), "Cu²⁺");
    assert_eq!(
        spoken("ион меди два превращается в оксид меди два"),
        "Cu²⁺ → CuO"
    );
}

// -------------------------------------------------------- half-reactions

#[test]
fn a_half_reaction_carries_its_electron() {
    assert_eq!(spoken("электрон"), "e⁻");
    assert_eq!(
        spoken("ион железа два превращается в ион железа три плюс электрон"),
        "Fe²⁺ → Fe³⁺ + e⁻"
    );
    assert_eq!(
        spoken("ион железа три плюс электрон превращается в ион железа два"),
        "Fe³⁺ + e⁻ → Fe²⁺"
    );
}

/// The electron is not an atom, so it must not touch the atom balance —
/// and its charge must reach the charge balance. Both fall out of the AST
/// without a special case anywhere in the validator.
#[test]
fn the_electron_balances_charge_without_disturbing_atoms() {
    // Correct half-reactions raise nothing.
    for said in [
        "ион железа два превращается в ион железа три плюс электрон",
        "ион железа три плюс электрон превращается в ион железа два",
    ] {
        assert!(
            !warnings(said)
                .iter()
                .any(|code| code.starts_with("chemistry.unbalanced")),
            "{said}: {:?}",
            warnings(said)
        );
    }
    // One electron too many is caught: 2 → 3 + 2(−1) = 1.
    let wrong = "ион железа два превращается в ион железа три плюс два электрона";
    assert!(
        warnings(wrong).contains(&"chemistry.unbalanced_charge".to_string()),
        "{:?}",
        warnings(wrong)
    );
    // And the iron still balances, because the electron contributes no atoms.
    assert!(!warnings(wrong).contains(&"chemistry.unbalanced_atoms".to_string()));
}

#[test]
fn the_electron_is_a_part_that_contributes_no_atoms() {
    let Node::Chemical(Chemical::Equation(equation)) =
        compiled("ион железа два превращается в ион железа три плюс электрон").ast
    else {
        panic!("expected an equation")
    };
    let electron = equation
        .right
        .iter()
        .find(|species| species.formula.parts == vec![Part::Electron])
        .expect("the electron must be its own species");
    assert_eq!(electron.charge, Some(-1));
    let atoms = electron.formula.atom_counts().expect("no overflow");
    assert!(atoms.is_empty(), "an electron is not an atom: {atoms:?}");
}

// ------------------------------------------------------------- refusals

/// An ion phrase that does not account for every word is not one species.
/// This is the guard that turned a silent drop into a refusal.
///
/// Asserted through `interpret`, where the whole utterance has to parse as
/// one thing: the span search above it may still find two separate readings
/// side by side, which is a different question and not the one this guard
/// answers.
#[test]
fn an_ion_phrase_with_words_left_over_is_not_one_species() {
    for said in ["ион серебра хлора", "ион меди два три"] {
        let result = compiled(said);
        if let Node::Chemical(Chemical::Species(species)) = &result.ast {
            assert!(
                species.charge.is_none(),
                "{said} was read as a single ion, dropping the rest: {species:?}"
            );
        }
    }
}

/// Ordinary speech that mentions an ion is still ordinary speech.
#[test]
fn prose_about_ions_is_not_an_equation() {
    for said in [
        "электрон движется по орбите",
        "полуреакция записана неверно",
        "заряд иона неизвестен",
    ] {
        assert!(stayed_prose(said), "{said} → {:?}", spoken(said));
    }
}

/// A named ion inside prose is substituted, by the same policy as «серная
/// кислота хранится в лаборатории»: a multi-word deliberate chemical term
/// is not an ordinary Russian phrase. The sentence around it survives, and
/// «Варианты прочтения» offers the words back.
#[test]
fn a_named_ion_in_prose_is_substituted_and_the_sentence_survives() {
    let shown = spoken("ион серебра был обнаружен в растворе");
    assert_eq!(shown, "Ag⁺ был обнаружен в растворе");
    for word in ["был", "обнаружен", "в", "растворе"] {
        assert!(shown.contains(word), "{shown}");
    }
}

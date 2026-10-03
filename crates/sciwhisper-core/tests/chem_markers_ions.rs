//! Two ways the chemistry reading used to change a formula or eat a word.
//!
//! 1. A state marker changed the substance. «водород» is `H₂`, but «водород
//!    газ» was `H↑`: `try_spelled` read the named element as one atom before
//!    the diatomic rule could apply. The same went for кислород, азот, фтор,
//!    хлор, бром and йод.
//! 2. «ион натрия плюс ион хлора» was `Na⁺ Cl₂`. `try_ion` filtered ion
//!    markers out of the words wherever they stood, so the span «ион натрия
//!    плюс ион» was read as Na⁺ with the «плюс» as its charge sign and the
//!    second «ион» swallowed; «хлора» was then left alone and became a
//!    molecule. «ион хлора плюс ион серебра» was `Cl⁺ Ag`: the charge of
//!    chlorine wrong, the silver ion without its charge.
//!
//! Each fix has a test that tries to pass it the wrong way: the letter
//! dictation «аш газ» stays an atom, the neighbours keep their readings, and
//! ordinary prose with «ион», «газ» and «плюс» stays words.

use sciwhisper_core::{interpret_utterance, render, Renderer, UtteranceMode, UtteranceOptions};

fn shown(text: &str) -> String {
    let result = interpret_utterance(
        text,
        UtteranceOptions {
            mode: UtteranceMode::MixedText,
            ..UtteranceOptions::default()
        },
    );
    render(&result.document, Renderer::Unicode)
}

/// Every element whose `elements.yaml` entry says `diatomic: true`, by the
/// name used in speech and by its genitive, with the molecule it names.
const DIATOMIC: [(&str, &str, &str); 7] = [
    ("водород", "водорода", "H₂"),
    ("кислород", "кислорода", "O₂"),
    ("азот", "азота", "N₂"),
    ("фтор", "фтора", "F₂"),
    ("хлор", "хлора", "Cl₂"),
    ("бром", "брома", "Br₂"),
    ("йод", "йода", "I₂"),
];

#[test]
fn a_state_marker_does_not_change_the_diatomic_formula() {
    for (name, genitive, molecule) in DIATOMIC {
        assert_eq!(shown(name), molecule, "«{name}» alone");
        assert_eq!(shown(&format!("{name} газ")), format!("{molecule}↑"));
        assert_eq!(shown(&format!("{name} осадок")), format!("{molecule}↓"));
        // The invariant itself: the marker adds an arrow and nothing else.
        assert_eq!(
            shown(&format!("{name} газ")),
            format!("{}↑", shown(name)),
            "«{name} газ»"
        );
        assert_eq!(
            shown(&format!("два {name} газ")),
            format!("2{molecule}↑"),
            "a coefficient does not turn the molecule back into an atom"
        );
        assert_eq!(
            shown(&format!("два {genitive} газ")),
            format!("2{molecule}↑")
        );
    }
}

#[test]
fn the_diatomic_rule_is_in_a_reaction_too() {
    assert_eq!(
        shown("водород газ плюс кислород газ равно вода"),
        "H₂↑ плюс O₂↑ равно H₂O"
    );
    assert_eq!(
        shown("цинк плюс соляная кислота равно хлорид цинка плюс водород газ"),
        "Zn плюс HCl равно ZnCl₂ плюс H₂↑"
    );
}

#[test]
fn a_letter_dictation_stays_as_dictated() {
    // The wrong way to fix the above: make every «газ» double the atom. A
    // person who spells the letter said an atom, and one who adds a
    // subscript said the subscript.
    for (spoken, expected) in [
        ("аш газ", "H↑"),
        ("аш два газ", "H₂↑"),
        ("эф газ", "F↑"),
        ("эн два газ", "N₂↑"),
        ("о два газ", "O₂↑"),
        ("аш", "H"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn a_monatomic_name_with_a_marker_stays_an_atom() {
    for (spoken, expected) in [
        ("натрий газ", "Na↑"),
        ("железо осадок", "Fe↓"),
        ("купрум газ", "Cu↑"),
        // A compound is not an element.
        ("натрий хлор", "NaCl"),
        ("натрий хлор осадок", "NaCl↓"),
        ("натрий хлор газ", "NaCl↑"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn spelled_formulas_keep_their_readings() {
    for (spoken, expected) in [
        ("аш два о", "H₂O"),
        ("це о два", "CO₂"),
        ("эс о два", "SO₂"),
        ("аш два эс о четыре", "H₂SO₄"),
        ("калий йод", "KI"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
    // Bare names with «плюс» and «равно» between them used to come back as
    // «H₂ плюс O₂ равно H₂O» and «Na плюс Cl₂ равно NaCl»: the names were more
    // than half of the words. That was the share rule. These sentences have
    // words in them that are neither names nor «и», so they are not lists and
    // the names stay words. A compound said in two words is not strong either:
    // «NaCl» is a bare species. With an arrow it is still an equation, see
    // `tests/comma_names.rs`.
    for spoken in [
        "водород плюс кислород равно вода",
        "натрий плюс хлор равно натрий хлор",
    ] {
        assert_eq!(shown(spoken), spoken, "«{spoken}»");
    }
}

#[test]
fn a_sum_of_ions_keeps_the_plus_and_both_ions() {
    for (spoken, expected) in [
        ("ион натрия плюс ион хлора", "Na⁺ плюс Cl⁻"),
        ("ион хлора плюс ион серебра", "Cl⁻ плюс Ag⁺"),
        ("ион калия плюс ион брома", "K⁺ плюс Br⁻"),
        ("ион железа три плюс ион хлора", "Fe³⁺ плюс Cl⁻"),
        ("ион кальция плюс два иона хлора", "Ca²⁺ плюс 2Cl⁻"),
        ("ион калия плюс два иона хлора", "K⁺ плюс 2Cl⁻"),
        ("два иона хлора плюс ион серебра", "2Cl⁻ плюс Ag⁺"),
        (
            "ион серебра плюс ион хлора равно хлорид серебра",
            "Ag⁺ плюс Cl⁻ равно AgCl",
        ),
        (
            "в растворе есть ион натрия плюс ион хлора",
            "в растворе есть Na⁺ плюс Cl⁻",
        ),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn a_sum_of_ions_never_loses_a_word_or_a_sign() {
    // What went wrong was never one specific answer, it was a word
    // disappearing and a charge changing. Check those directly.
    for spoken in [
        "ион натрия плюс ион хлора",
        "ион хлора плюс ион серебра",
        "ион калия плюс ион брома",
        "ион кальция плюс два иона хлора",
        "ион натрия плюс ион",
    ] {
        let out = shown(spoken);
        assert!(out.contains("плюс"), "«{spoken}» lost its «плюс»: {out}");
        assert!(!out.contains("Cl₂"), "«{spoken}» made a molecule: {out}");
        assert!(
            !out.contains("Cl⁺"),
            "«{spoken}» made chlorine positive: {out}"
        );
        assert!(
            !out.contains("Br⁺") && !out.contains("Br "),
            "«{spoken}» broke bromine: {out}"
        );
    }
    // A trailing «ион» with nothing after it is a word, not a charge.
    assert_eq!(shown("ион натрия плюс ион"), "Na⁺ плюс ион");
    // The words with «и» always worked and still do.
    for (spoken, expected) in [
        ("ион натрия и ион хлора", "Na⁺ и Cl⁻"),
        ("ион хлора и ион серебра", "Cl⁻ и Ag⁺"),
        ("ион калия и ион брома", "K⁺ и Br⁻"),
        ("ион кальция и два иона хлора", "Ca²⁺ и 2Cl⁻"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn single_ions_and_dictated_signs_keep_their_readings() {
    // The wrong way to fix the sum: forbid «плюс» after an ion altogether.
    // «ион меди два плюс» is Cu²⁺ and the sign is what was said.
    for (spoken, expected) in [
        ("ион натрия", "Na⁺"),
        ("ион хлора", "Cl⁻"),
        ("ион серебра", "Ag⁺"),
        ("ион брома", "Br⁻"),
        ("ион кальция", "Ca²⁺"),
        ("два иона хлора", "2Cl⁻"),
        ("ион натрия плюс", "Na⁺"),
        ("ион меди два плюс", "Cu²⁺"),
        ("ион железа три плюс", "Fe³⁺"),
        ("ион натрия плюс хлор", "Na⁺ плюс Cl₂"),
        ("ион меди два плюс сульфат ион", "Cu²⁺ плюс SO₄²⁻"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn cation_and_anion_are_not_claimed() {
    // «катион» and «анион» are not in the ion markers; the phrases stay words
    // rather than being read as something nearby.
    for spoken in [
        "катион натрия плюс анион хлора",
        "катион натрия и анион хлора",
        "анион хлора",
    ] {
        assert_eq!(shown(spoken), spoken);
    }
}

#[test]
fn ordinary_prose_with_ion_gas_and_plus_stays_words() {
    for spoken in [
        "Плюс в том, что газ не пахнет.",
        "Ион — это заряженная частица.",
        "Ион плюс ион дают пару.",
        "Газ плюс жидкость — это смесь.",
        "Мы взяли плюс ещё один ион.",
        "Йод помогает при ранах.",
        "Фтор есть в зубной пасте.",
        "Бром жидкий при комнатной температуре.",
        "Хлор и бром — галогены.",
        "в воздухе много азота",
        "водород газообразный",
    ] {
        assert_eq!(shown(spoken), spoken);
    }
}

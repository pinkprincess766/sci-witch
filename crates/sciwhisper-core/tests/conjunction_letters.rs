//! A conjunction between two operands is a conjunction, not a letter.
//!
//! «и», «а», «в», «о», «с», «у», «б» and «же» are Russian function words, and
//! `symbols.yaml` lists each of them as the *name of a Latin letter*, so that
//! a person can dictate `a + b` as «а плюс бэ». The two readings collided in
//! the mathematics and physics parser: «икс и игрек» became `xiy`, «три метра
//! и четыре метра» became `3 мi4 м`, «два моля а три моля» became
//! `2 мольa3 моль` — notation nobody dictated, at the confidence that inserts
//! without asking. (Chemistry closed the same trap earlier; see
//! `prose_safety.rs`.)
//!
//! The rule now: a bare function-word letter does not extend a
//! juxtaposition when an operand follows it. The leftover word is left as a
//! word, and the operands around it are compiled on their own. These tests
//! pin both directions, because a fix that only silences «икс и игрек» would
//! also take «а бэ» and «эф равно эм а» with it.

use sciwhisper_core::{
    interpret, interpret_utterance, render, Domain, InterpretOptions, Renderer, UtteranceMode,
    UtteranceOptions,
};

fn shown_in(text: &str, domain: Domain) -> String {
    let result = interpret_utterance(
        text,
        UtteranceOptions {
            mode: UtteranceMode::MixedText,
            domain,
            ..UtteranceOptions::default()
        },
    );
    render(&result.document, Renderer::Unicode)
}

fn shown(text: &str) -> String {
    shown_in(text, Domain::Auto)
}

// ------------------------------------------------------------ the conjunction

#[test]
fn a_conjunction_between_two_letters_stays_a_word() {
    // Before: xiy, x + yiz, xiyiz, aib.
    for (spoken, expected) in [
        ("икс и игрек", "икс и игрек"),
        ("икс и игрек и зет", "икс и игрек и зет"),
        ("а и бэ", "а и бэ"),
        // The operands that compile on their own are compiled; the
        // conjunction and the lone letter it leaves behind stay as said.
        ("икс плюс игрек и зет", "x + y и зет"),
        ("икс в квадрате и игрек в квадрате", "x² и y²"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn a_conjunction_between_two_functions_stays_a_word() {
    // Before: sin(x)icos(x).
    assert_eq!(shown("синус икс и косинус икс"), "sin(x) и cos(x)");
}

#[test]
fn a_conjunction_between_two_quantities_stays_a_word() {
    // Before: 3 мi4 м, 2 мольi3 моль, 2 мольa3 моль, 5 кгi3 кг, 3 м/сi4 м/с.
    //
    // Two amounts are not a list (`holds_an_enumeration` skips bare
    // quantities: the owner's decision that an amount in a sentence stays
    // words), so with the conjunction kept as a word each phrase comes back
    // as said. The test still guards the conjunction: without the weak-letter
    // rule the letter glues both amounts into one `Juxt`, which is not a bare
    // quantity, and the phrase is rewritten as «3 мi4 м».
    for spoken in [
        "три метра и четыре метра",
        "два моля и три моля",
        "два моля а три моля",
        "пять килограммов и три килограмма",
        "два килограмма а три килограмма",
        "три метра в секунду и четыре метра в секунду",
    ] {
        assert_eq!(shown(spoken), spoken);
    }
}

#[test]
fn a_conjunction_inside_a_sentence_about_quantities_stays_a_word() {
    // As above: amounts in a sentence stay words, and without the weak-letter
    // rule «три моля и пять молей» became one span, «3 мольi5 моль».
    for spoken in [
        "Навески: два моля, три моля и пять молей.",
        "Сначала три метра, потом четыре метра и пять метров.",
        "Массу взяли два килограмма и три килограмма.",
    ] {
        assert_eq!(shown(spoken), spoken);
    }
}

#[test]
fn a_conjunction_between_two_equations_stays_a_word() {
    // Before: x = 1iy = 2 and x = 1ay = 2.
    assert_eq!(shown("икс равно один и игрек равно два"), "x = 1 и y = 2");
    assert_eq!(shown("икс равно один а игрек равно два"), "x = 1 а y = 2");
}

#[test]
fn a_conjunction_between_two_coefficients_stays_a_word() {
    // Before: 2xi3y, 3xi2y. The number in front of the letter does not make
    // the conjunction a factor: an operand follows it.
    assert_eq!(shown("два икс и три игрек"), "два икс и три игрек");
    assert_eq!(shown("три икс и два игрек"), "три икс и два игрек");
}

#[test]
fn a_conjunction_between_two_numbers_is_neither_a_product_nor_iodine() {
    // Before: 2i3 and 3i4. Closing that alone would have exposed the chemistry
    // reading — «и три» as I₃, a bare number being taken as a subscript — so
    // the auto domain is checked as well as the explicit ones.
    for (spoken, expected) in [("два и три", "два и три"), ("три и четыре", "три и четыре")]
    {
        for domain in [Domain::Auto, Domain::Mathematics, Domain::Physics] {
            assert_eq!(
                shown_in(spoken, domain),
                expected,
                "«{spoken}» in {domain:?}"
            );
        }
    }
}

/// Every operand paired with every conjunction: the conjunction has to come
/// out as a word of its own, whatever the operands compile to.
#[test]
fn no_pair_of_operands_swallows_the_conjunction() {
    let operands = [
        "икс",
        "игрек",
        "зет",
        "тэ",
        "икс в квадрате",
        "синус икс",
        "два икс",
        "икс плюс один",
        "три метра",
        "два моля",
        "пять килограммов",
        "три метра в секунду",
    ];
    for left in operands {
        for right in operands {
            for conjunction in ["и", "а"] {
                let spoken = format!("{left} {conjunction} {right}");
                for domain in [
                    Domain::Auto,
                    Domain::Chemistry,
                    Domain::Mathematics,
                    Domain::Physics,
                ] {
                    let out = shown_in(&spoken, domain);
                    let separate = out.split(' ').any(|word| word == conjunction);
                    assert!(
                        separate,
                        "«{spoken}» in {domain:?} lost its conjunction: {out}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_strict_interpreter_does_not_multiply_across_a_conjunction() {
    // `interpret` is the whole-string entry point the utterance layer builds
    // on; the parser rule lives below both, so it has to hold here too.
    for (spoken, forbidden) in [
        ("икс и игрек", "xiy"),
        ("синус икс и косинус икс", "sin(x)icos(x)"),
        ("три метра и четыре метра", "3 мi4 м"),
    ] {
        let result = interpret(
            spoken,
            InterpretOptions {
                domain: Domain::Physics,
                ..InterpretOptions::default()
            },
        );
        let out = render(&result.ast, Renderer::Unicode);
        assert!(
            !out.contains(forbidden),
            "«{spoken}» became a product with a letter: {out}"
        );
    }
}

// --------------------------------------------------------- a real dictation

#[test]
fn a_function_word_letter_at_the_start_of_a_term_is_still_a_letter() {
    for (spoken, expected) in [
        ("а плюс бэ", "a + b"),
        ("а плюс бэ равно цэ", "a + b = c"),
        ("а в квадрате", "a²"),
        ("и в степени два", "i²"),
        ("а бэ", "ab"),
        ("а бэ цэ", "abc"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn a_coefficient_before_a_function_word_letter_is_still_a_product() {
    // A bare number in front and no operand behind: nothing to be a
    // conjunction between.
    for (spoken, expected) in [
        ("два а", "2a"),
        ("два а плюс три бэ", "2a + 3b"),
        ("два а в квадрате", "2a²"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn a_letter_after_a_letter_is_still_a_product_of_quantities() {
    assert_eq!(shown_in("эф равно эм а", Domain::Physics), "F = ma");
    assert_eq!(shown("эф равно эм а плюс эм бэ"), "f = ma + mb");
    assert_eq!(shown("пэ равно эм в"), "p = mv");
    assert_eq!(shown("эс равно а тэ"), "s = at");
    assert_eq!(shown("эф равно эм а в квадрате"), "f = ma²");
}

#[test]
fn an_index_named_by_a_function_word_letter_is_still_an_index() {
    assert_eq!(shown("икс индекс и"), "xᵢ");
    assert_eq!(
        shown("сумма от и равно один до эн икс индекс и"),
        "∑_{i=1}^{n} xᵢ"
    );
}

#[test]
fn a_function_word_letter_as_an_argument_is_still_a_letter() {
    assert_eq!(shown("предел при икс стремящемся к а эф"), "lim_{x→a} f");
    assert_eq!(
        shown_in("вектор эф равен эм умножить на вектор а", Domain::Physics),
        "F⃗ = m·a⃗"
    );
    assert_eq!(
        shown_in("дельта же равно минус эн эф е", Domain::Physics),
        "ΔG = −nFE"
    );
}

#[test]
fn a_modifier_is_evidence_enough_for_the_letter() {
    // The speaker said which letter is meant, so the word is not a
    // conjunction any more.
    assert_eq!(shown("икс и латинская игрек"), "xiy");
}

#[test]
fn a_spelled_formula_is_untouched() {
    assert_eq!(shown("аш два о"), "H₂O");
    assert_eq!(shown("эн а хлор"), "NaCl");
}

/// «и» is iodine when a formula is being spelled, and a conjunction the rest of
/// the time; a number after it cannot tell the two apart («два и три»), but a
/// second letter with a subscript of its own can. The first version of the
/// conjunction rule refused the number outright and turned «и два о пять»
/// into «и 2O₅» — neither a formula nor the words that were said.
///
/// Every expectation here is what the compiler said before the rule existed,
/// except the last two rows, which read as words or garbage before and now
/// read as the formula that was spelled.
#[test]
fn a_spelled_formula_with_a_conjunction_letter_is_still_a_formula() {
    for (spoken, expected) in [
        ("и два о пять", "I₂O₅"),
        ("ка и о три", "KIO₃"),
        ("аш и", "HI"),
        ("эс и два", "SI₂"),
        ("в два о пять", "V₂O₅"),
        ("у два о три", "U₂O₃"),
        ("эс о два", "SO₂"),
        ("эс о три", "SO₃"),
        ("аш два эс о четыре", "H₂SO₄"),
        ("ка о аш", "KOH"),
        ("цэ у о", "CuO"),
        ("эм гэ и два", "MgI₂"),
        ("эм гэ б два", "MgB₂"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
    // Asked for chemistry outright, a bare «и два» is the iodine molecule,
    // as it was before.
    assert_eq!(shown_in("и два", Domain::Chemistry), "I₂");
}

#[test]
fn a_conjunction_letter_between_numbers_needs_more_than_the_numbers() {
    // The second number belongs to «в два раза», a preposition, and does not
    // vouch for the conjunction.
    assert_eq!(shown("три и пять в два раза"), "три и пять в два раза");
    for domain in [Domain::Auto, Domain::Chemistry] {
        assert_eq!(shown_in("пять а шесть", domain), "пять а шесть");
    }
}

// ------------------------------------------------------------- known limits

/// Not a virtue: «два а бэ» is a legitimate `2ab`, and it is refused because
/// «два и игрек» and «два а икс» cannot be told apart from it by anything the
/// parser sees. The phrase stays as words, which is visible and costs one
/// retype; the other choice is notation nobody dictated. If a later rule finds
/// positive evidence, this is the test to change.
#[test]
fn a_coefficient_and_two_letters_after_a_function_word_stay_words() {
    assert_eq!(shown("два а бэ"), "два а бэ");
    assert_eq!(shown("два а икс"), "два а икс");
}

#[test]
fn a_weak_letter_does_not_take_the_number_after_it() {
    // «и два» came out as `i2`: mathematics glued the number onto the letter
    // and won Auto against the iodine chemistry read. A function-word letter
    // followed by a bare number is not a product anybody dictates.
    for (spoken, expected) in [
        ("и два", "I₂"),
        ("а два", "а два"),
        ("Мы взяли и два образца.", "Мы взяли и два образца."),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
    // The letter itself is untouched where it is a letter.
    for (spoken, expected) in [
        ("а бэ", "ab"),
        ("и в квадрате", "i²"),
        ("два а", "2a"),
        ("а плюс бэ", "a + b"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

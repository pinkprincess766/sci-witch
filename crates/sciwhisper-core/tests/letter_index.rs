//! A letter followed by a bare integer is a subscript.
//!
//! «икс два» used to come out as the juxtaposition `x2`, which is not how
//! anyone dictates `2x` and is not what `x₂` means. The same tree as «икс
//! индекс два» is the right reading. These tests pin both directions: the
//! index appears where a letter was said, and it does not appear for a weak
//! letter, a fraction, an operator in between, a chemical formula, or a
//! sentence that merely mentions a letter-word next to a number.

use sciwhisper_core::ast::{Math, Node};
use sciwhisper_core::{
    interpret, interpret_utterance, render, Domain, InterpretOptions, Renderer, UtteranceMode,
    UtteranceOptions,
};

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

fn compiled(text: &str) -> sciwhisper_core::InterpretationResult {
    interpret(text, InterpretOptions::default())
}

fn math_of(text: &str) -> Math {
    let result = interpret(
        text,
        InterpretOptions {
            domain: Domain::Mathematics,
            ..Default::default()
        },
    );
    match result.ast {
        Node::Math(math) => math,
        other => panic!("expected mathematics for «{text}», got {other:?}"),
    }
}

fn is_symbol_integer_subscript(math: &Math) -> bool {
    matches!(
        math,
        Math::Subscript { base, sub }
            if matches!(base.as_ref(), Math::Symbol(_))
                && matches!(sub.as_ref(), Math::Number(_))
    )
}

// -------------------------------------------------------------- the index

#[test]
fn a_letter_then_an_integer_is_a_subscript() {
    for (spoken, expected) in [
        ("икс два", "x₂"),
        ("икс один", "x₁"),
        ("икс двадцать один", "x₂₁"),
        ("эм один плюс эм два", "m₁ + m₂"),
        ("тэ один", "t₁"),
        ("альфа один", "α₁"),
        ("икс один плюс икс два", "x₁ + x₂"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
    assert_eq!(compiled("икс два").domain, Domain::Mathematics);
}

#[test]
fn the_implicit_index_is_the_same_tree_as_saying_индекс() {
    for (implicit, explicit) in [
        ("икс два", "икс индекс два"),
        ("икс один", "икс индекс один"),
        ("эм два", "эм индекс два"),
        ("альфа один", "альфа индекс один"),
    ] {
        assert_eq!(
            math_of(implicit),
            math_of(explicit),
            "«{implicit}» vs «{explicit}»"
        );
        assert!(
            is_symbol_integer_subscript(&math_of(implicit)),
            "«{implicit}» was not Subscript(Symbol, Number): {:?}",
            math_of(implicit)
        );
    }
}

#[test]
fn a_coefficient_a_power_and_an_operator_still_read_as_before() {
    assert_eq!(shown("два икс"), "2x");
    assert_eq!(shown("икс в квадрате"), "x²");
    assert_eq!(shown("икс два в квадрате"), "x₂²");
    assert_eq!(shown("икс равно два"), "x = 2");
}

#[test]
fn a_named_function_takes_the_indexed_letter_as_its_argument() {
    assert_eq!(shown("эф от икс два"), "f(x₂)");
    let Math::Apply { args, .. } = math_of("эф от икс два") else {
        panic!("expected Apply, got {:?}", math_of("эф от икс два"));
    };
    assert_eq!(args.len(), 1);
    assert!(
        is_symbol_integer_subscript(&args[0]),
        "argument was {:?}, not x₂",
        args[0]
    );
}

// ----------------------------------------------- must not pass incorrectly

#[test]
fn a_weak_letter_does_not_take_an_index() {
    // «а» is a function-word letter (`Tok::WeakSym`). The new rule does not
    // apply, the number does not join, mathematics fails, and the phrase
    // stays words. «и два» is the same shape in mathematics and the iodine
    // I₂ that chemistry already reads.
    assert_eq!(shown("а два"), "а два");
    assert_eq!(shown("и два"), "I₂");
    assert!(!shown("а два").contains("a₂"), "{}", shown("а два"));
}

#[test]
fn spelled_formulas_stay_chemistry_in_auto() {
    for (spoken, expected) in [
        ("аш два о", "H₂O"),
        ("о два", "O₂"),
        ("эн два", "N₂"),
        ("цэ о два", "CO₂"),
        ("эс о два", "SO₂"),
        ("аш два эс о четыре", "H₂SO₄"),
        ("ка и", "KI"),
        ("и два", "I₂"),
        ("и два о пять", "I₂O₅"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
        assert_eq!(
            compiled(spoken).domain,
            Domain::Chemistry,
            "«{spoken}» left chemistry"
        );
    }
}

#[test]
fn an_operator_between_the_letter_and_the_number_is_not_an_index() {
    assert_eq!(shown("икс равно два"), "x = 2");
    assert_eq!(shown("икс плюс два"), "x + 2");
    assert_eq!(shown("икс минус два"), "x − 2");
    assert!(!is_symbol_integer_subscript(&math_of("икс равно два")));
}

#[test]
fn a_decimal_after_a_letter_is_not_an_index() {
    // «два целых пять» is `Num("2,5")`, so the integer rule must refuse it.
    // The trailing «десятых» is not a number word this lexicon consumes, so
    // the longer phrase is not a formula either.
    let decimal = shown("икс два целых пять");
    assert!(
        !decimal.contains('₂'),
        "«икс два целых пять» became a subscript: {decimal}"
    );
    assert!(
        !is_symbol_integer_subscript(&math_of("икс два целых пять")),
        "{:?}",
        math_of("икс два целых пять")
    );
    let with_tenths = shown("икс два целых пять десятых");
    assert!(
        !with_tenths.contains('₂'),
        "«икс два целых пять десятых» became a subscript: {with_tenths}"
    );
    // «десятых» is not consumed as part of the number, so the whole
    // phrase is not a formula; a prefix «икс два» is not a math cue
    // either (the next word is a number, not an operator).
    assert_eq!(with_tenths, "икс два целых пять десятых");
}

#[test]
fn a_letter_word_in_a_sentence_stays_a_word() {
    // `symbols.yaml` names «эм», «тэ», «икс», «альфа» as letters; «вариант»
    // and «пункт» are not letters at all. `is_math_cue` does not open a
    // span on a letter followed by a number, so a sentence that happens to
    // contain that pair is not rewritten even when the same pair compiles
    // on its own.
    assert_eq!(shown("икс два"), "x₂");
    assert_eq!(shown("эм два"), "m₂");
    for spoken in [
        "Вариант два мы отбросили",
        "Пункт три выполнен",
        "Эм два дня прошло",
        "Эм, два дня прошло",
        "Икс два мы отбросили",
    ] {
        assert_eq!(shown(spoken), spoken, "«{spoken}» was rewritten");
    }
}

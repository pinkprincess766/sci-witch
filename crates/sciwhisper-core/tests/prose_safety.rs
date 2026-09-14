//! Ordinary Russian must not turn into chemistry.
//!
//! Every single-letter Russian preposition, conjunction and particle also
//! names a Latin letter, and `symbols.yaml` has to list them so that a person
//! can spell out `Na` as «эн а». The two readings collided: «натрий и калий
//! стоят рядом в таблице» came out as `NaIK`, «медь, о которой я говорил»
//! came out as `CuO` — a *different substance* — and both at the confidence
//! level that inserts without asking.
//!
//! The rule now is that an ambiguous letter must earn its element reading:
//! either it carries a subscript, or the same chunk holds a letter name that
//! is not a Russian word. These tests pin both directions, because a fix that
//! only silences the bad cases would also take «аш два о» with it.

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

#[test]
fn a_one_letter_russian_word_never_becomes_an_element() {
    // Each of these produced a formula before: NaIK, CuO, HU, NaSCl.
    for (spoken, forbidden) in [
        ("натрий и калий стоят рядом в таблице", "NaIK"),
        ("медь, о которой я говорил, лежит в шкафу", "CuO"),
        ("водород у нас закончился", "HU"),
        ("натрий с хлором реагируют бурно", "NaSCl"),
    ] {
        let out = shown(spoken);
        assert!(
            !out.contains(forbidden),
            "«{spoken}» still produces {forbidden}: {out}"
        );
    }
}

#[test]
fn an_enumeration_of_substances_in_prose_is_left_alone() {
    // With the conjunction no longer edible, each name is a single ordinary
    // word with no other science around it, and the existing in-prose guard
    // keeps the sentence whole. That is the safe answer, and it is the one
    // the corpus asks for; whether a lone unambiguous term should still be
    // substituted is a policy question, not a parser one.
    let spoken = "натрий и калий стоят рядом в таблице";
    assert_eq!(shown(spoken), spoken);
}

#[test]
fn a_spelled_formula_still_reads_as_one() {
    for (spoken, expected) in [
        ("аш два о", "H₂O"),
        ("аш два эс о четыре", "H₂SO₄"),
        ("цэ о два", "CO₂"),
        ("марганец о два", "MnO₂"),
        // A bare «о два» has no spelled neighbour, but the subscript alone is
        // proof enough: no preposition is followed by a bare number.
        ("о два", "O₂"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn a_spelled_element_name_is_reachable_through_a_function_word_letter() {
    // «эн а» is Na and «цэ у» is Cu: the second letter of each is a Russian
    // word, and the first one is not, which is exactly the evidence the rule
    // asks for.
    assert_eq!(shown("эн а хлор"), "NaCl");
    assert_eq!(shown("цэ у о"), "CuO");
}

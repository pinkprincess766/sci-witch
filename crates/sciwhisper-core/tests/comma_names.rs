//! A comma between two names is a boundary, not a joint.
//!
//! The chemistry parser dropped every comma before it read the words, because
//! Whisper puts commas around «превращается в». The price was that «йод, бром»
//! became «йод бром», which is read the way «натрий хлор» is — as one compound
//! named by its elements. «Принесли йод, бром и хлор» came back as «Принесли
//! IBr и Cl₂» and «Примеры: йод, бром, хлор и фтор» as «Примеры: IBrCl и F₂»:
//! an ordinary list of elements turned into formulas of substances nobody had
//! named.
//!
//! These tests pin both sides. A fix that only silences the glue would also
//! take «натрий хлор» with it, and a fix that only keeps the compounds would
//! leave the list glued.

use sciwhisper_core::{
    interpret, interpret_utterance, render, Domain, InterpretOptions, Node, Renderer,
    UtteranceMode, UtteranceOptions,
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

/// What the strict chemistry entry makes of the words, as a string, or `None`
/// when it keeps them as text.
fn chemistry(text: &str) -> Option<String> {
    let result = interpret(
        text,
        InterpretOptions {
            domain: Domain::Chemistry,
            allow_shortcuts: false,
        },
    );
    match result.ast {
        Node::Text(_) => None,
        ast => Some(render(&ast, Renderer::Unicode)),
    }
}

// ------------------------------------------------------------ the glue

#[test]
fn names_split_by_a_comma_never_form_one_compound() {
    // Each of these produced a formula of the neighbouring names joined:
    // IBr, IBrCl, NaCl, NaK, NaClK, ONH, FeCuZnSn, LiNaKRbCs, SCl.
    for (spoken, forbidden) in [
        ("Принесли йод, бром и хлор.", "IBr"),
        ("Примеры: йод, бром, хлор и фтор.", "IBr"),
        ("Купили натрий, хлор и калий.", "NaCl"),
        ("Купили натрий, калий и литий.", "NaK"),
        ("Примеры: натрий, хлор, калий и йод.", "NaCl"),
        ("Примеры: кислород, азот, водород и гелий.", "ON"),
        (
            "Примеры элементов: железо, медь, цинк, олово и свинец.",
            "FeCu",
        ),
        ("Литий, натрий, калий, рубидий, цезий.", "LiNa"),
        ("Возьми серу, фосфор и хлор.", "SP"),
        ("йод, бром", "IBr"),
        ("сера, хлор", "SCl"),
    ] {
        let out = shown(spoken);
        assert!(
            !out.contains(forbidden),
            "«{spoken}» glued its names into {forbidden}: {out}"
        );
    }
}

#[test]
fn a_list_of_elements_keeps_each_element_and_its_comma() {
    // Each of these is a dictated list — names after a colon, or nothing but
    // names — so the names are formulas, one each, in the order said. The
    // rule that makes them a list is in `tests/enumeration_rule.rs`.
    for (spoken, expected) in [
        ("Нам нужны: йод, бром и хлор.", "Нам нужны: I₂, Br₂ и Cl₂."),
        (
            "Примеры: йод, бром, хлор и фтор.",
            "Примеры: I₂, Br₂, Cl₂ и F₂.",
        ),
        ("Примеры: натрий, хлор и калий.", "Примеры: Na, Cl₂ и K."),
        (
            "Литий, натрий, калий, рубидий, цезий.",
            "Li, Na, K, Rb, Cs.",
        ),
        ("йод, бром", "I₂, Br₂"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn the_strict_interpreter_does_not_read_two_names_with_a_comma_as_one() {
    // The rule lives in the parser, below the utterance layer; it has to
    // hold at the whole-string entry point too.
    for spoken in [
        "йод, бром",
        "натрий, хлор",
        "кислород, азот",
        "железо, медь, цинк",
        "йод , бром",
    ] {
        assert_eq!(
            chemistry(spoken),
            None,
            "«{spoken}» was read as one compound"
        );
    }
}

#[test]
fn a_comma_between_two_formulas_spelled_out_letter_by_letter_separates_them() {
    // `H₂OCO₂` before: the comma was dropped and two formulas became one.
    assert_eq!(shown("аш два о, це о два"), "H₂O, CO₂");
    assert_eq!(shown("натрий хлор, калий йод"), "NaCl, KI");
}

// ----------------------------------------------- what must keep working

#[test]
fn a_compound_named_by_its_elements_still_joins_without_a_comma() {
    // The other side of the rule: the same words with no comma between them
    // are the compound. A fix that cut every pair of adjacent names apart
    // would pass the tests above and break these.
    for (spoken, expected) in [
        ("натрий хлор", "NaCl"),
        ("калий йод", "KI"),
        ("сера хлор", "SCl"),
        ("йод бром", "IBr"),
        ("железо три хлор", "Fe₃Cl"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
        assert_eq!(chemistry(spoken).as_deref(), Some(expected), "«{spoken}»");
    }
    // A conjunction is not a comma: the name list with «и» keeps working as
    // it did.
    assert_eq!(
        shown("Примеры: натрий хлор и калий йод."),
        "Примеры: NaCl и KI."
    );
}

#[test]
fn a_list_of_named_substances_is_unchanged() {
    for (spoken, expected) in [
        (
            "Примеры: павликова кислота, уксусная кислота, ацетон и глицерин.",
            "Примеры: HF, CH₃COOH, CH₃COCH₃ и C₃H₈O₃.",
        ),
        (
            "Нам нужны: серная кислота, соляная кислота и азотная кислота.",
            "Нам нужны: H₂SO₄, HCl и HNO₃.",
        ),
        (
            "Примеры: хлорид натрия, хлорид калия, гидроксид кальция.",
            "Примеры: NaCl, KCl, Ca(OH)₂.",
        ),
        ("гидроксид натрия, хлорид калия", "NaOH, KCl"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn a_comma_beside_a_connective_or_a_condition_does_not_break_a_reaction() {
    // Whisper puts commas around the phrases that join the sides. The comma
    // rule is about commas between names, not about these.
    let nacl = "NaCl + KI → NaI + KCl";
    for spoken in [
        "натрий хлор, плюс, калий йод, стрелка, натрий йод, плюс, калий хлор",
        "натрий хлор, плюс калий йод, превращается в, натрий йод плюс калий хлор",
        "натрий хлор плюс калий йод, превращается в натрий йод плюс калий хлор",
        "натрий хлор и калий йод, превращается в, натрий йод и калий хлор",
    ] {
        assert_eq!(shown(spoken), nacl, "«{spoken}»");
    }
    for spoken in [
        "аш два о, при нагревании, стрелка, аш два плюс о два",
        "аш два о, при нагревании стрелка аш два плюс о два",
    ] {
        assert_eq!(shown(spoken), "H₂O → H₂ + O₂", "«{spoken}»");
    }
    assert_eq!(
        shown("натрий, плюс хлор, стрелка, натрий хлор"),
        "Na + Cl₂ → NaCl"
    );
}

#[test]
fn a_charge_or_a_hydrate_after_a_comma_still_belongs_to_the_name() {
    // They are said about a substance, they are not a second one.
    assert_eq!(shown("ион железа, три плюс"), "Fe³⁺");
    assert_eq!(shown("ион меди, два плюс"), "Cu²⁺");
    assert_eq!(shown("сульфат меди, пентагидрат"), "CuSO₄·5H₂O");
}

#[test]
fn a_number_after_a_comma_is_not_the_charge_of_a_name_without_an_ion() {
    // The charge exception needs an ion marker. Without it «йод, два плюс»
    // is two items, not an iodine ion.
    // Read without the comma, «натрий два плюс хлор» is Na₂ + Cl₂: a number
    // and a «плюс» after a name are only a charge when an ion was named.
    assert_eq!(chemistry("натрий, два плюс хлор стрелка натрий хлор"), None);
    assert_eq!(chemistry("йод, два плюс хлор"), None);
}

#[test]
fn a_comma_ends_the_name_before_an_oxidation_state_is_looked_for() {
    // «аш два о, три»: the three is not the oxidation state of the oxygen.
    // The span used to refuse to stop on «о» and was cut into «H₂» and «O₂».
    let out = shown("аш два о, три");
    assert!(!out.contains("H₂ O₂"), "the formula was cut in two: {out}");
}

// -------------------------------------------- prose stays prose

#[test]
fn prose_that_mentions_a_substance_or_two_stays_words() {
    for spoken in [
        "Мы взяли железо, медь.",
        "Мы взяли железо, медь и цинк.",
        "Нам нужны йод, бром и хлор.",
        "Принесли йод, а бром забыли.",
        "Принесли кислород, а потом азот.",
        "Добавь азот, потом кислород.",
        "Возьми хлор, и натрий.",
        "Принесли два йода, бром и хлор.",
        "Принесли серную кислоту, соляную кислоту и азотную кислоту.",
        "Натрий и калий мы держим отдельно, медь и цинк можно рядом, а кислоты вообще в другом шкафу.",
    ] {
        assert_eq!(shown(spoken), spoken, "prose was rewritten: «{spoken}»");
    }
}

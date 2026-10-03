//! When a row of substance names is a dictated list and when it is a sentence.
//!
//! The rule used to be a share: the names had to cover more than half of the
//! words of their sentence. A share cannot tell a list from a sentence with a
//! verb in it — «Принесли йод, бром и хлор» is three words of five,
//! «Мы обсуждали углерод, водород, кислород, азот» is four of five — and both
//! came back with formulas in them. The owner's decision of 2 October 2026:
//!
//! * a list is names **after a colon** in their sentence («Примеры: …»,
//!   «Нам нужны: …», «Список: …»), or a sentence that is **nothing but names**
//!   and the separators between them (commas, «и», «а», the final stop);
//! * everything else is prose, and a name in prose is mentioned, not dictated;
//! * a framing («запиши …») still makes dictation, whatever follows it;
//! * a counted name or an amount does not vote.
//!
//! Both sides are pinned. A rule that only keeps the prose would throw the
//! lists away, and one that only keeps the lists would let the verbs through.

use sciwhisper_core::{
    interpret_utterance, render, utterance::MIN_ENUMERATION_SPANS, Renderer, UtteranceMode,
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

// ------------------------------------------------------- prose with a verb

#[test]
fn a_sentence_with_a_verb_is_prose_however_many_names_it_holds() {
    // Each of these is names-over-half-the-words, which was the old test for
    // a list, and each has a word in it that is not a name.
    for spoken in [
        "Принесли йод, бром и хлор.",
        "Принесли йод, бром.",
        "Принесли кислород, азот и водород.",
        "Принесли ацетон, этанол и глицерин.",
        "Принесли хлор, натрий.",
        "Купили натрий, хлор и калий.",
        "Купили натрий, калий и литий.",
        "Возьми серу, фосфор и хлор.",
        "Возьми железо, медь и цинк.",
        "Нужны хлор, бром, йод.",
        "Нужны кислород, азот.",
        "Нужны фтор, хлор, бром, йод и астат.",
        "Нам нужны йод, бром и хлор.",
        "Мы обсуждали углерод, водород, кислород, азот.",
        "Мы взяли железо, медь и цинк.",
    ] {
        assert_eq!(shown(spoken), spoken, "prose was rewritten: «{spoken}»");
    }
}

#[test]
fn a_sentence_of_names_and_an_extra_word_is_prose() {
    // «потом», «плюс» and «равно» are not separators. The sentence is not
    // nothing but names, so it is not a list.
    for spoken in [
        "Железо, медь, а потом цинк.",
        "Йод, бром, потом хлор.",
        "Железо, медь и еще цинк.",
        "водород плюс кислород равно вода",
        "Принесли серную кислоту, соляную кислоту и азотную кислоту.",
    ] {
        assert_eq!(shown(spoken), spoken, "«{spoken}»");
    }
}

#[test]
fn the_share_of_the_sentence_does_not_decide() {
    // A short prose sentence is not made a list by being mostly names, and a
    // long sentence is not made prose by being mostly other words: what
    // decides is the shape. A list after a colon is a list however many
    // words stand before the colon; words after the names make it prose
    // however few they are.
    assert_eq!(
        shown("Принесли йод, бром и хлор."),
        "Принесли йод, бром и хлор."
    );
    assert_eq!(
        shown(
            "Я долго думал над тем, что же именно нам понадобится в лаборатории: йод, бром и хлор."
        ),
        "Я долго думал над тем, что же именно нам понадобится в лаборатории: I₂, Br₂ и Cl₂."
    );
    let spoken = "Примеры: йод, бром и хлор и еще многое другое из лаборатории.";
    assert_eq!(shown(spoken), spoken);
}

// ------------------------------------------------------------ a colon

#[test]
fn names_after_a_colon_are_a_list() {
    for (spoken, expected) in [
        ("Нам нужны: йод, бром и хлор.", "Нам нужны: I₂, Br₂ и Cl₂."),
        (
            "Примеры: йод, бром, хлор и фтор.",
            "Примеры: I₂, Br₂, Cl₂ и F₂.",
        ),
        ("Список: железо, кобальт, никель.", "Список: Fe, Co, Ni."),
        (
            "Примеры элементов: железо, медь, цинк, олово и свинец.",
            "Примеры элементов: Fe, Cu, Zn, Sn и Pb.",
        ),
        (
            "Примеры: павликова кислота, уксусная кислота, ацетон и глицерин.",
            "Примеры: HF, CH₃COOH, CH₃COCH₃ и C₃H₈O₃.",
        ),
        (
            "Нам нужны: серная кислота, соляная кислота и азотная кислота.",
            "Нам нужны: H₂SO₄, HCl и HNO₃.",
        ),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn a_colon_with_words_after_the_last_name_is_prose() {
    // The colon is not enough. After the last name nothing but «и» / «а» may
    // stand before the end of the sentence; a verb or any other word there
    // means the names are mentioned in a sentence, not listed. Two names or
    // more follow the colon in each of these, and none is a list.
    for spoken in [
        "Он сказал: йод и бром стоят рядом.",
        "Мы взяли: железо и медь, а потом ушли.",
        "Он сказал: йод, бром и хлор стоят рядом.",
        "Нам нужны: йод, бром и хлор, остальное потом.",
        "Мы решили: железо, медь, цинк уже есть.",
        // The verb sits before the first name.
        "Он сказал: вчера мы взяли йод, бром и хлор.",
        // The verb sits between two names, and a name ends the sentence.
        "Вывод: цинк, олово и свинец плавятся ниже железа.",
        "Мы взяли: железо и медь и ушли.",
    ] {
        assert_eq!(shown(spoken), spoken, "«{spoken}»");
    }
}

#[test]
fn a_lead_in_before_the_names_and_a_joiner_between_them_do_not_break_a_list() {
    // The two small sets of words that a list may hold besides names and
    // «и» / «а»: see `LIST_LEAD_IN_WORDS` and `LIST_JOINER_WORDS` in
    // `utterance.rs`. Both sentences are dictated lists, and a word of the
    // same kind outside its place is not allowed: «или» after the last name,
    // «на примере» between two names.
    for (spoken, expected) in [
        (
            "Попытка вставки: на примере гидроксида железа три, оксида меди два или перманганата калия.",
            "Попытка вставки: на примере Fe(OH)₃, CuO или KMnO₄.",
        ),
        (
            "Попытка записи: пермангнат калия или же уксусная кислота, павликовая кислота.",
            "Попытка записи: KMnO₄ или же CH₃COOH, HF.",
        ),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
    for spoken in [
        "Примеры: йод, бром или.",
        "Примеры: йод, на примере бром и хлор.",
        "Примеры: йод, например бром.",
    ] {
        assert_eq!(shown(spoken), spoken, "«{spoken}»");
    }
}

#[test]
fn a_colon_list_keeps_its_closing_conjunction_and_the_stop() {
    // The other side of the same narrowing: the lists that were lists stay
    // lists. A final stop and a comma before «и» are not words after the
    // names.
    for (spoken, expected) in [
        ("Примеры: йод, бром.", "Примеры: I₂, Br₂."),
        ("Список: железо и медь.", "Список: Fe и Cu."),
        (
            "Нам нужны: йод, бром, и хлор.",
            "Нам нужны: I₂, Br₂, и Cl₂.",
        ),
        ("Запиши: йод, бром.", "Запиши: I₂, Br₂."),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn a_colon_needs_the_names_after_it_in_the_same_sentence() {
    // The colon of another sentence announces nothing here, and one name
    // after a colon is not a list.
    for spoken in [
        "Примеры: ладно. Принесли йод, бром.",
        "Нужны: йод.",
        "Принесли: серу.",
    ] {
        assert_eq!(shown(spoken), spoken, "«{spoken}»");
    }
}

#[test]
fn names_before_the_colon_do_not_make_a_list() {
    // The colon announces what follows it; names said before it are part of
    // the announcement.
    let spoken = "Йод и бром: вот что мы взяли.";
    assert_eq!(shown(spoken), spoken);
}

#[test]
fn a_list_lends_no_company_to_another_sentence() {
    // The list is a property of its sentence. The first sentence is prose
    // about sulphur and has to stay words even though the second is a list.
    assert_eq!(
        shown("Принесли серу. Примеры: йод, бром."),
        "Принесли серу. Примеры: I₂, Br₂."
    );
}

// ------------------------------------------------- nothing but names

#[test]
fn a_sentence_of_nothing_but_names_is_a_list() {
    for (spoken, expected) in [
        (
            "Железо, медь, цинк, олово и свинец.",
            "Fe, Cu, Zn, Sn и Pb.",
        ),
        (
            "Литий, натрий, калий, рубидий, цезий.",
            "Li, Na, K, Rb, Cs.",
        ),
        ("Натрий и калий.", "Na и K."),
        ("Йод и бром.", "I₂ и Br₂."),
        ("йод, бром и хлор", "I₂, Br₂ и Cl₂"),
        ("Железо и медь, цинк и олово.", "Fe и Cu, Zn и Sn."),
        ("Хлорид натрия, калий.", "NaCl, K."),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn one_name_is_not_a_list() {
    // The minimum is not a detail: a list of one is a lone ordinary word.
    assert_eq!(MIN_ENUMERATION_SPANS, 2);
    for spoken in ["Принесли железо.", "Нужна серная кислота."] {
        assert_eq!(shown(spoken), spoken, "«{spoken}»");
    }
}

// -------------------------------------------------------------- a framing

#[test]
fn a_framing_still_makes_dictation() {
    // «запиши» says that what follows is to be written down, whatever it is
    // and however many other words are in the sentence.
    for (spoken, expected) in [
        ("запиши йод, бром и хлор", "запиши I₂, Br₂ и Cl₂"),
        ("ну запиши йод, бром и хлор", "ну запиши I₂, Br₂ и Cl₂"),
        ("Запиши: йод, бром.", "Запиши: I₂, Br₂."),
        ("запиши натрий хлор", "запиши NaCl"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

// --------------------------------------------------------- counted names

#[test]
fn a_counted_name_does_not_vote_in_either_shape() {
    // Two counted names are not a list, after a colon or without one. With
    // the counted names left out there is one name or none.
    for spoken in [
        "Примеры: два йода и три хлора.",
        "Принесли два йода, бром и хлор.",
        "Мы взяли три глицерина и два ацетона.",
    ] {
        assert_eq!(shown(spoken), spoken, "«{spoken}»");
    }
}

#[test]
fn a_counted_name_stands_inside_a_list_that_has_enough_other_names() {
    // The decision of `tests/prose_coefficients.rs`, kept: the counted name
    // does not vote, but the list it stands in is still a list.
    assert_eq!(
        shown("Примеры: два йода, бром и хлор."),
        "Примеры: 2I₂, Br₂ и Cl₂."
    );
}

//! A count in front of a substance name, inside ordinary Russian, is an amount
//! that is mentioned, and it stays words.
//!
//! «Принесли два йода и бром» came back as «Принесли 2I₂ и Br₂», and «Купили
//! два хлора и три ведра» as «Купили 2ClI₃ ведра». Two mechanisms did it, and
//! each was enough by itself:
//!
//! 1. A counted name was not a *bare* substance (the coefficient was not one),
//!    so it was taken for something dictated and made strong. A strong span
//!    keeps the weak names beside it.
//! 2. «два йода» and «бром» are two names covering three words of five, which
//!    is what the enumeration rule calls a list.
//!
//! The owner's decision is that an amount inside a sentence stays words, and a
//! counted name is an amount. These tests pin both directions: the sentences
//! above stay as they were said, and everything that really is dictation
//! (alone, after a framing, an equation, a charge) keeps its notation, so a
//! fix that only silences the first would not pass the second.

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
fn a_counted_substance_in_a_sentence_stays_words() {
    // Each of these was rewritten before, and several came back as nonsense
    // (`2ClI₃`, `3NaI₅`, `2KI₃Na`: the «и» of the sentence read as iodine).
    for spoken in [
        "Принесли два йода и бром.",
        "Мы заказали два йода.",
        "Два йода принесли вчера.",
        "Он купил три натрия и пять ведер.",
        "Купили два хлора и три ведра.",
        "Взяли три хлора и два ведра.",
        "В кладовой лежали два йода и бром.",
        "Лаборант принёс два магния.",
        "Вчера привезли три цинка, а сегодня четыре меди.",
        "Для опыта нужны два йода.",
        "Он пересчитал два калия и три натрия.",
        "Нам выдали два лития и бром.",
        "Положи два йода в сейф.",
        "Нам нужны три хлора, два ведра и швабра.",
        "Я разделил два йода между студентами.",
        "Оставьте два фтора на складе.",
        "Он положил два йода и бром в шкаф, потом закрыл дверь.",
        "Принесли два натрий хлор и бром.",
        "Мы хранили два аш два о в колбе.",
        "Принесли два йода и бром и хлор.",
        "Нам нужны два ацетона и три ведра.",
        "Мы взяли три глицерина и два ацетона.",
        "Привезли четыре натрия.",
        "Сколько стоят два йода?",
        "Принесли два йода и серную кислоту.",
    ] {
        assert_eq!(
            shown(spoken),
            spoken,
            "«{spoken}» is a sentence that mentions an amount of a substance"
        );
    }
}

#[test]
fn the_same_phrase_is_notation_when_it_is_the_whole_utterance() {
    // The count is dictation when nothing else is said around it. These are
    // the other half of the test above: the exemption is for prose only.
    for (spoken, expected) in [
        ("два йода", "2I₂"),
        ("два аш два о", "2H₂O"),
        ("два натрий хлор", "2NaCl"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn a_counted_name_beside_another_name_needs_a_framing() {
    // Without a framing nothing says «два йода и бром» is dictation and not a
    // fragment of a sentence, and a counted name is an amount, which does not
    // make company for the weak name beside it. With the framing it is
    // dictation again. This is the price of the rule, and the safe side of it.
    assert_eq!(shown("два йода и бром"), "два йода и бром");
    assert_eq!(shown("запиши два йода и бром"), "запиши 2I₂ и Br₂");
}

#[test]
fn a_counted_substance_after_a_framing_is_notation() {
    for (spoken, expected) in [
        ("запиши два йода", "запиши 2I₂"),
        ("запиши два аш два о", "запиши 2H₂O"),
        ("запиши два йода и бром", "запиши 2I₂ и Br₂"),
        (
            "запиши два натрий плюс хлор два стрелка два натрий хлор",
            "запиши 2Na + Cl₂ → 2NaCl",
        ),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn an_equation_with_counts_is_still_an_equation() {
    // A reaction is not a counted substance, it is an equation, and nobody
    // says one by accident, inside a sentence as well.
    for (spoken, expected) in [
        (
            "два натрий плюс хлор два стрелка два натрий хлор",
            "2Na + Cl₂ → 2NaCl",
        ),
        (
            "Реакция: два натрий плюс хлор два стрелка два натрий хлор.",
            "Реакция: 2Na + Cl₂ → 2NaCl.",
        ),
        (
            "Мы записали два натрий плюс хлор два стрелка два натрий хлор в тетрадь.",
            "Мы записали 2Na + Cl₂ → 2NaCl в тетрадь.",
        ),
        (
            "два аш два о стрелка аш два о плюс о два",
            "2H₂O → H₂O + O₂",
        ),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn a_charge_still_says_the_speaker_is_writing() {
    // «два иона натрия» carries a charge; a count with a charge is not a count
    // of «something we bought», it is an ion, and it keeps its strength.
    assert_eq!(
        shown("Здесь два иона натрия и один ион хлора."),
        "Здесь 2Na⁺ и Cl⁻."
    );
}

#[test]
fn a_list_of_names_is_still_a_list() {
    // The enumeration rule is not switched off. The counted name simply does
    // not vote, so a list with enough uncounted names covering the sentence
    // is still read as one.
    assert_eq!(
        shown("Примеры: павликова кислота, уксусная кислота, ацетон и глицерин."),
        "Примеры: HF, CH₃COOH, CH₃COCH₃ и C₃H₈O₃."
    );
    assert_eq!(
        shown("Примеры: павликова кислота, уксусная кислота, ацетон и два глицерина."),
        "Примеры: HF, CH₃COOH, CH₃COCH₃ и 2C₃H₈O₃."
    );
    assert_eq!(
        shown("Примеры: два ацетона, павликова кислота, уксусная кислота и глицерин."),
        "Примеры: 2CH₃COCH₃, HF, CH₃COOH и C₃H₈O₃."
    );
}

#[test]
fn counted_names_alone_do_not_make_a_list() {
    // The wrong way through the enumeration rule: two or three counted names
    // are a lot of words and not a list. If the amount voted, the second
    // sentence would be read as a list of two names covering four words of
    // six.
    for spoken in [
        "Мы взяли три глицерина и два ацетона.",
        "Вчера привезли три цинка, а сегодня четыре меди.",
    ] {
        assert_eq!(shown(spoken), spoken, "«{spoken}»");
    }
}

#[test]
fn an_amount_with_a_unit_is_unchanged() {
    // The policy the counted name follows. It must not have moved.
    for spoken in [
        "Налей два литра воды.",
        "Налили три литра, потом ещё два литра.",
    ] {
        assert_eq!(shown(spoken), spoken, "«{spoken}»");
    }
}

//! Grams, litres, hours, minutes and degrees Celsius.
//!
//! «пять граммов», «два литра», «три часа» and «минус пять градусов
//! Цельсия» used to stay words even when they were the whole utterance,
//! while «пять километров» and «два моля» compiled. These units are also
//! the most ordinary words in the vocabulary — every kitchen, clinic and
//! timetable sentence has one — so the tests below are mostly about what
//! must **not** happen:
//!
//! * a quantity inside an ordinary sentence stays words (the owner's rule,
//!   `is_bare_quantity`), including a quantity of something
//!   («налей два литра воды») and a compound one («два часа тридцать минут»);
//! * a word that merely resembles a unit is not a unit;
//! * a bare «градус» is never a temperature nobody said.

use sciwhisper_core::lexicon::Lexicon;
use sciwhisper_core::nbest::choose_hypothesis;
use sciwhisper_core::{
    interpret, interpret_utterance, render, render_result, Domain, InterpretOptions, Renderer,
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

fn physics_codes(text: &str) -> Vec<String> {
    let result = interpret(
        text,
        InterpretOptions {
            domain: Domain::Physics,
            ..Default::default()
        },
    );
    assert!(result.confidence > 0.0, "«{text}» did not parse");
    result
        .warnings
        .into_iter()
        .filter(|warning| warning.code.starts_with("physics."))
        .map(|warning| warning.code)
        .collect()
}

// ------------------------------------------------------------- (a) said alone

#[test]
fn each_new_quantity_said_alone_compiles() {
    for (spoken, expected) in [
        // грамм: the three forms a numeral governs, and the colloquial
        // genitive plural without an ending.
        ("один грамм", "1 г"),
        ("два грамма", "2 г"),
        ("пять граммов", "5 г"),
        ("пять грамм", "5 г"),
        ("двадцать один грамм", "21 г"),
        ("пять миллиграммов", "5 мг"),
        ("два микрограмма", "2 мкг"),
        // литр
        ("один литр", "1 л"),
        ("два литра", "2 л"),
        ("пять литров", "5 л"),
        ("двух литров", "2 л"),
        ("три миллилитра", "3 мл"),
        ("десять микролитров", "10 мкл"),
        // время
        ("один час", "1 ч"),
        ("три часа", "3 ч"),
        ("пять часов", "5 ч"),
        ("одна минута", "1 мин"),
        ("одну минуту", "1 мин"),
        ("две минуты", "2 мин"),
        ("пять минут", "5 мин"),
        // градус Цельсия, both spoken ways and Whisper's capital letter
        ("один градус цельсия", "1 °C"),
        ("два градуса цельсия", "2 °C"),
        ("двадцать градусов цельсия", "20 °C"),
        ("двадцать градусов Цельсия", "20 °C"),
        ("тридцать шесть градусов по цельсию", "36 °C"),
        ("минус пять градусов цельсия", "\u{2212}5 °C"),
        ("минус пять градусов по Цельсию", "\u{2212}5 °C"),
        // compound units built from the new ones
        ("два грамма на литр", "2 г/л"),
        ("пять километров в час", "5 км/ч"),
        ("три метра в минуту", "3 м/мин"),
        // framing keeps its old meaning: «запиши …» is dictation
        ("запиши два грамма", "запиши 2 г"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn the_other_renderers_carry_the_new_symbols() {
    let result = interpret(
        "три часа",
        InterpretOptions {
            domain: Domain::Physics,
            ..Default::default()
        },
    );
    assert_eq!(render_result(&result, Renderer::Unicode), "3 ч");
    assert_eq!(render_result(&result, Renderer::Latex), "3\\mathrm{ч}");
}

/// `°` is a text symbol; inside `\mathrm` it does not compile in pdfLaTeX.
/// The LaTeX renderer writes a degree as the superscript circle plus an
/// upright letter, and OMML (which is Unicode) carries the symbol as it is.
#[test]
fn degrees_celsius_render_portably_in_latex_and_omml() {
    let result = interpret(
        "двадцать градусов Цельсия",
        InterpretOptions {
            domain: Domain::Physics,
            ..Default::default()
        },
    );
    assert_eq!(render_result(&result, Renderer::Unicode), "20 °C");
    let latex = render_result(&result, Renderer::Latex);
    assert_eq!(latex, "20{}^{\\circ}\\mathrm{C}");
    assert!(!latex.contains('°'), "a raw degree sign in LaTeX: {latex}");
    let omml = render_result(&result, Renderer::Omml);
    assert!(omml.contains(">20<"), "{omml}");
    assert!(omml.contains(">°C<"), "{omml}");
    // A unit without a degree sign is untouched by the special case.
    let plain = interpret(
        "два литра",
        InterpretOptions {
            domain: Domain::Physics,
            ..Default::default()
        },
    );
    assert_eq!(render_result(&plain, Renderer::Latex), "2\\mathrm{л}");
}

// -------------------------------------------- (b) inside an ordinary sentence

/// One sentence per new unit form. Each returns word for word. Without
/// `is_bare_quantity` every one of them has a two-word parse that is long
/// enough to count as "dictated", and comes back with digits in it:
/// «он ждал 3 ч и ушёл».
#[test]
fn a_new_quantity_inside_a_sentence_stays_words() {
    for sentence in [
        "он ждал три часа и ушёл",
        "поезд стоит один час на станции",
        "варить надо пять часов",
        "через две минуты будет готово",
        "подождите одну минуту пожалуйста",
        "доклад занял пятнадцать минут",
        "мне нужно пять граммов на пробу",
        "в рецепте один грамм соли",
        "на весах два грамма",
        "выдали сто граммов хлеба",
        "принеси два литра молока из магазина",
        "в бутылке пять литров",
        "в банке осталось три миллилитра",
        "врач назначил пять миллиграммов утром",
        "доза составила два микрограмма на кг",
        "нужно десять микролитров реагента",
        "за окном минус пять градусов цельсия",
        "сегодня двадцать градусов по цельсию на улице",
        "вчера было три градуса цельсия",
    ] {
        assert_eq!(shown(sentence), sentence, "«{sentence}» was rewritten");
    }
}

/// A quantity **of something** is still a phrase. The quantity and the
/// substance were two weak spans, and two spans covering most of a short
/// sentence read as a dictated list: «налей два литра воды» came back as
/// «налей 2 лH₂O». Quantities are now counted apart from other spans, so
/// this pair is not a list of two.
#[test]
fn a_quantity_of_a_substance_is_not_a_list() {
    for sentence in [
        "налей два литра воды",
        "выпил три литра воды за день",
        "добавили пять граммов хлорида натрия в воду",
        "растворите два грамма хлорида натрия",
        "смешали два литра воды и три литра спирта",
        "смешали два моля воды и три моля спирта",
        "взяли пять миллилитров серной кислоты",
    ] {
        assert_eq!(shown(sentence), sentence, "«{sentence}» was rewritten");
    }
}

/// «два часа тридцать минут» is one amount made of two `number unit`
/// pairs. Judged as a single pair it was "not bare", so it was rewritten
/// inside a sentence («встреча длилась 2 ч30 мин»).
#[test]
fn a_compound_quantity_inside_a_sentence_stays_words() {
    for sentence in [
        "встреча длилась два часа тридцать минут",
        "в восемь часов пять минут мы вышли",
        "нагревали один час пятнадцать минут",
        // the same shape with a unit that existed before
        "длина стола три метра пять сантиметров",
    ] {
        assert_eq!(shown(sentence), sentence, "«{sentence}» was rewritten");
    }
}

/// Amounts do not make a list. The enumeration rule is for names
/// («ацетон и глицерин»); amounts in a row cover most of a short sentence,
/// and that used to make it a "dictated list". Each of these sentences is
/// prose with a time or a mass in it. With the old count every one of them
/// came back with digits («нагревали 5 мин, потом 10 мин, потом 2 ч»).
#[test]
fn a_run_of_quantities_in_prose_stays_words() {
    for sentence in [
        "нагревали пять минут, потом десять минут, потом два часа",
        "сначала три часа, потом два часа, потом пять минут",
        "налили три литра, потом ещё два литра",
        "навески: два грамма, три грамма, пять граммов",
        "сначала три метра, потом четыре метра, потом пять метров",
    ] {
        assert_eq!(shown(sentence), sentence, "«{sentence}» was rewritten");
    }
}

/// A quantity **and its substance**, said as the whole utterance, stays
/// words: two weak spans with nothing strong beside them. Before this it
/// was glued together as «2 мольH₂SO₄». After a framing it is still a
/// record, and the space said between the amount and the substance is kept.
#[test]
fn a_quantity_of_a_substance_said_alone_stays_words() {
    for spoken in [
        "два моля серной кислоты",
        "пять граммов хлорида натрия",
        "два литра воды",
    ] {
        assert_eq!(shown(spoken), spoken, "«{spoken}»");
    }
    // Dictated after a framing it is still a record.
    assert_eq!(
        shown("запиши два моля серной кислоты"),
        "запиши 2 моль H₂SO₄"
    );
}

// ------------------------------------- (c) resembling a unit is not being one

/// Every phrase here has a word that is a unit form, or that starts with
/// one, but is not used as a unit. Checked by breaking the code on purpose:
///
/// * a parser that accepts a bare unit as an atom turns «литр» and «час пик»
///   into notation («литр» → `л`);
/// * the phrases with a numeral in `a_number_before_an_ordinary_word_…` fail
///   if the unit lookup matches by prefix: «три минутки» → `3 мин`,
///   «пять граммофонов» → `5 г`;
/// * accepting the bare «градус» prints a temperature (or an angle) that
///   nobody said — see `a_bare_degree_is_never_a_temperature`.
///
/// The rest are here because they are the words most likely to be added to
/// the vocabulary by mistake («час» is in «который час», «минута» in
/// «минута молчания», «грамм» in «ни грамма совести»). They pass today
/// because a unit needs a numeral in front of it, and they are the phrases
/// that would say so if that stopped being true.
#[test]
fn words_that_look_like_units_are_not_units() {
    for phrase in [
        "час пик",
        "который час",
        "в час пик метро переполнено",
        "через час после обеда",
        "прошёл целый час",
        "градус напряжения вырос",
        "градус",
        "градусы",
        "грамм совести",
        "ни грамма совести",
        "литр",
        "литр молока",
        "литровая банка стоит на полке",
        "программа граммофон грамматика",
        "литература и литератор",
        "часовой на посту, часы идут",
        "минута молчания",
        "объявили минуту молчания",
        "минута в минуту",
        "минутка отдыха",
        "полчаса и полминуты",
        "несколько часов и много литров",
        "сколько граммов положить",
        "градусник показывает температуру",
        // the single letters that are the printed symbols are not spoken
        // forms: «эл» is the letter, not a litre
        "эл",
    ] {
        assert_eq!(shown(phrase), phrase, "«{phrase}» became notation");
    }
}

/// «Градус» is an angle in «синус тридцати градусов» and a temperature in
/// «минус пять градусов»; the two have different dimensions (1 and Θ). The
/// bare word is therefore not a unit, and only «Цельсия» makes one. Without
/// this test a later change that maps the bare word to `°C` would pass every
/// other test in this file.
#[test]
fn a_bare_degree_is_never_a_temperature() {
    for spoken in [
        "пять градусов",
        "минус пять градусов",
        "двадцать один градус",
        "тридцать градусов",
        "два градуса",
        "пять градусов кельвина",
        "температура упала на пять градусов",
        "пентан кипит при тридцати шести градусах",
    ] {
        let out = shown(spoken);
        assert_eq!(out, spoken, "«{spoken}» became «{out}»");
        assert!(!out.contains('°'), "«{spoken}» invented a degree sign");
    }
}

/// The bare quantity stays words as a whole utterance too when the noun is
/// a sentence, not a unit: nothing about a number in front of an ordinary
/// word makes it a quantity.
#[test]
fn a_number_before_an_ordinary_word_is_not_a_quantity() {
    for phrase in [
        "три минутки",
        "пять часовых",
        "два литератора",
        "пять граммофонов",
    ] {
        assert_eq!(shown(phrase), phrase, "«{phrase}»");
    }
}

/// An n-best repair may fix science that was heard; it may not create it.
/// The new units are common words, so they must not become the "already
/// heard science" that lets a neighbouring word turn into a formula.
#[test]
fn a_new_unit_does_not_license_a_repair_into_a_formula() {
    for pair in [
        ["два литра водка", "два литра вода"],
        ["один литр года", "один литр вода"],
        ["пять граммов сода", "пять граммов вода"],
        ["два часа года", "два часа вода"],
        ["пять минут водка", "пять минут вода"],
    ] {
        let choice = choose_hypothesis(&pair);
        assert_eq!(choice.index, 0, "{pair:?} repaired the top hypothesis");
    }
}

// --------------------------------------------------- dimensions and friends

#[test]
fn the_new_units_carry_the_right_dimensions() {
    let lex = Lexicon::builtin();
    for (symbol, dimension) in [
        ("г", "M"),
        ("мг", "M"),
        ("мкг", "M"),
        ("л", "L^3"),
        ("мл", "L^3"),
        ("мкл", "L^3"),
        ("ч", "T"),
        ("мин", "T"),
        ("°C", "Θ"),
    ] {
        assert_eq!(
            lex.unit_dimension(symbol).map(|d| d.to_string()).as_deref(),
            Some(dimension),
            "{symbol}"
        );
    }
    // The same dimension as the units they are multiples of.
    assert_eq!(lex.unit_dimension("г"), lex.unit_dimension("кг"));
    assert_eq!(lex.unit_dimension("ч"), lex.unit_dimension("с"));
    assert_eq!(lex.unit_dimension("°C"), lex.unit_dimension("К"));
}

#[test]
fn compatible_new_units_add_without_a_warning() {
    for spoken in [
        "два литра плюс три миллилитра",
        "два грамма плюс три килограмма",
        "два миллиграмма плюс три микрограмма",
        "три часа плюс пять минут",
        "три часа плюс пять секунд",
        "двадцать градусов цельсия плюс пять кельвинов",
        "три микролитра плюс два миллилитра",
    ] {
        assert_eq!(physics_codes(spoken), Vec::<String>::new(), "«{spoken}»");
    }
}

/// The wrong way round: incompatible sums must be reported, and reported
/// for the right reason. A unit whose dimension were mistyped as `1` would
/// make every one of these silent.
#[test]
fn incompatible_new_units_are_reported() {
    for spoken in [
        "два грамма плюс три литра",
        "два литра плюс три метра",
        "три часа плюс два литра",
        "три минуты плюс два грамма",
        "двадцать градусов цельсия плюс три метра",
        "два грамма плюс три часа",
    ] {
        assert_eq!(
            physics_codes(spoken),
            ["physics.dimension_mismatch"],
            "«{spoken}»"
        );
    }
}

/// A litre is not a cubic metre with a decimal exponent in this table, and
/// an hour is 3600 seconds, not a power of ten: the equivalent-unit hint
/// would print a wrong number. Nothing may be offered for them, and the
/// hint the smaller-than-base rule already suppresses stays suppressed.
#[test]
fn no_equivalent_is_offered_for_the_new_units() {
    for spoken in [
        "два грамма",
        "пять миллиграммов",
        "два литра",
        "три миллилитра",
        "три часа",
        "пять минут",
        "двадцать градусов цельсия",
        "два грамма на литр",
    ] {
        let result = interpret(
            spoken,
            InterpretOptions {
                domain: Domain::Physics,
                ..Default::default()
            },
        );
        assert!(
            !result
                .warnings
                .iter()
                .any(|warning| warning.code == "physics.unit_equivalent"),
            "«{spoken}» was offered an equivalent: {:?}",
            result.warnings
        );
    }
}

/// A compound amount is two quantities, and each keeps its own space in every
/// format: `2 ч30 мин` ran the second number into the first unit. The other
/// side is pinned too, because the easy fix — a space between every pair of
/// factors — would turn `2x` into `2 x` and `3 м + 4 с` into something else.
#[test]
fn a_compound_amount_is_spaced_in_every_format_and_nothing_else_is() {
    use sciwhisper_core::{interpret_utterance, render, Renderer, UtteranceOptions};
    let whole = |text: &str, renderer: Renderer| {
        render(
            &interpret_utterance(text, UtteranceOptions::default()).document,
            renderer,
        )
    };
    assert_eq!(
        whole("два часа тридцать минут", Renderer::Unicode),
        "2 ч 30 мин"
    );
    let latex = whole("два часа тридцать минут", Renderer::Latex);
    assert!(latex.contains("\\mathrm{ч}\\,30"), "{latex}");
    let omml = whole("два часа тридцать минут", Renderer::Omml);
    let (unit, number) = (omml.find(">ч<").unwrap(), omml.find(">30<").unwrap());
    assert!(omml[unit..number].contains("> <"), "{omml}");

    assert_eq!(
        whole("три метра пять сантиметров", Renderer::Unicode),
        "3 м 5 см"
    );
    assert_eq!(whole("два моля", Renderer::Unicode), "2 моль");
    assert_eq!(whole("два икс", Renderer::Unicode), "2x");
    assert_eq!(
        whole("три метра плюс четыре секунды", Renderer::Unicode),
        "3 м + 4 с"
    );
    assert_eq!(whole("два литра", Renderer::Latex), "2\\mathrm{л}");
}

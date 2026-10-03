//! Ordinary Russian must not turn into notation.
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
        // «ка» had no spelled name, so potassium earned no chemistry evidence
        // and Auto chose the letter product `ki` over KI, while «аш и» was
        // already HI. «це» is how a recogniser often writes «цэ».
        ("ка и", "KI"),
        ("ка о аш", "KOH"),
        ("це о два", "CO₂"),
        ("це у о", "CuO"),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

#[test]
fn a_letter_name_that_became_an_element_is_still_a_letter_in_mathematics() {
    // The other side of «ка» → K and «це» → C: the same words are the Latin
    // letters k and c, and an expression must keep reading them that way.
    // The Ukrainian «це» ("this") in a sentence is not carbon either.
    for (spoken, expected) in [
        ("ка плюс один", "k + 1"),
        ("эф равно ка икс", "f = kx"),
        ("ка в квадрате", "k²"),
        ("це плюс один", "c + 1"),
        ("Ну це вже інше питання.", "Ну це вже інше питання."),
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

#[test]
fn a_greek_letter_name_that_is_also_a_russian_word_stays_a_word() {
    // Found by `research/data/prose-negatives-v1.jsonl`: «дельта» was the one
    // entry in the strong-cue list that parses on its own, so a one-word span
    // was enough to rewrite it in the middle of an ordinary sentence.
    for spoken in [
        "Дельта между планом и фактом оказалась заметной.",
        "Дельта реки за лето сильно обмелела.",
        "Эта работа ещё не закончена.",
        "Альфа и омега всей методики — чистота посуды.",
        "Сигма в отчёте посчитана по неправильной формуле.",
    ] {
        assert_eq!(shown(spoken), spoken, "«{spoken}» was rewritten");
    }
}

#[test]
fn delta_applied_to_something_is_still_notation() {
    // The fix must cost nothing that can actually be dictated. Δ takes an
    // operand, so a real one is always more than one word — and «дельта»
    // alone is a whole utterance, which never reaches the strong-cue rule.
    assert_eq!(shown("дельта же равно минус эн эф е"), "ΔG = −nFE");
    assert_eq!(shown("дельта аш"), "ΔH");
    assert_eq!(shown("дельта"), "δ");
    assert_eq!(
        shown("Мы записали дельта же равно минус эн эф е на доске."),
        "Мы записали ΔG = −nFE на доске."
    );
}

#[test]
fn names_are_substituted_only_when_they_are_most_of_the_sentence() {
    // Counting the names cannot tell these apart: both name four substances.
    // The cupboard sentence spends its words on where things are kept, so
    // the names are mentions and have to stay words. The examples sentence
    // announces a list with a colon, which is what was dictated, and those
    // have to become formulas. A fix that only keeps the first would throw
    // the list away; a fix that only compiles the second would rewrite the
    // cupboard. (The name of this test is the share-of-words rule it was
    // written under, and PAIR_WORKFLOW_RU.md cites it; the rule now is a
    // colon or a sentence of nothing but names, see
    // `tests/enumeration_rule.rs`.)
    let cupboard = "Натрий и калий мы держим отдельно, медь и цинк можно рядом, а кислоты вообще в другом шкафу.";
    assert_eq!(shown(cupboard), cupboard);
    assert_eq!(
        shown("Примеры: павликова кислота, уксусная кислота, ацетон и глицерин."),
        "Примеры: HF, CH₃COOH, CH₃COCH₃ и C₃H₈O₃."
    );
}

#[test]
fn an_amount_in_prose_stays_words() {
    // Found in the owner's coursework, the first text in this project not
    // written to test the parser: «один моль» became «1 моль», and a sentence
    // with «два моля …, — три» came back half digits, half words. These are
    // the same shapes in other words; the coursework itself is not published.
    //
    // Each amount is first shown to compile on its own. Without that the test
    // passes for any amount the lexicon happens not to know — «три часа» and
    // «двух граммов» do not parse at all, so a sentence built on them proves
    // nothing about this rule.
    for (amount, compiled, sentence) in [
        (
            "один моль",
            "1 моль",
            "На каждый моль хлора расходуется один моль водорода.",
        ),
        (
            "два моля",
            "2 моль",
            "Для первой стадии нужно два моля кислорода, для второй — три.",
        ),
        (
            "пять километров",
            "5 км",
            "Мы прошли пять километров до лаборатории.",
        ),
        (
            "три метра",
            "3 м",
            "Шланг оказался длиной три метра, пришлось взять другой.",
        ),
        (
            "пять вольт",
            "5 В",
            "Блок питания выдаёт пять вольт, этого хватает.",
        ),
        (
            "минус пять вольт",
            "−5 В",
            "На втором выводе было минус пять вольт, как и ожидали.",
        ),
        // Amounts do not make a list. The enumeration rule is for names
        // («ацетон и глицерин»); two or three amounts in a sentence cover
        // most of its words and used to be read as a dictated list.
        (
            "три метра",
            "3 м",
            "Сначала три метра, потом четыре метра, потом пять метров.",
        ),
        ("три литра", "3 л", "Налили три литра, потом ещё два литра."),
        // A deliberate price: a list of weighed amounts written out in words
        // is not rewritten either.
        (
            "два моля",
            "2 моль",
            "Навески: два моля, три моля, пять молей.",
        ),
    ] {
        assert_eq!(
            shown(amount),
            compiled,
            "«{amount}» must still compile alone"
        );
        assert_eq!(shown(sentence), sentence);
    }
}

#[test]
fn an_amount_that_was_dictated_still_compiles() {
    // The other side. A rule that simply refused every amount would pass the
    // test above and break dictation, so each way of *meaning* an amount is
    // pinned: said alone, after a framing, corrected mid-way, as part of an
    // expression inside a sentence. A list of amounts is not on this list: it
    // is prose, see `an_amount_in_prose_stays_words`.
    for (spoken, expected) in [
        ("два моля", "2 моль"),
        ("запиши два моля", "запиши 2 моль"),
        ("два моля, нет, три моля", "3 моль"),
        (
            "Получилось три метра плюс четыре секунды, и это ошибка.",
            "Получилось 3 м + 4 с, и это ошибка.",
        ),
    ] {
        assert_eq!(shown(spoken), expected, "«{spoken}»");
    }
}

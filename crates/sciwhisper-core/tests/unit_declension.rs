//! Russian numerals govern the case of the unit after them, and a unit that
//! knows only its dictionary form fails on four numbers out of five.
//!
//! «сто паскалей» used to come back unparsed while «двести джоулей» worked,
//! for no reason a user could see: the joule happened to have its genitive
//! plural in `units.yaml` and the pascal did not. Coverage that depends on
//! who typed which line is not coverage.
//!
//! Every form below was written from the grammar, not from the parser's
//! output. The last test refuses to let a new unit into `units.yaml`
//! without its declension.

use std::collections::BTreeSet;

use sciwhisper_core::lexicon::Lexicon;
use sciwhisper_core::{interpret, render_result, Domain, InterpretOptions, Renderer};

/// The three forms a Russian numeral can demand, in the order
/// 1 / 2–4 / 5–20:
///
/// - `один паскаль` — nominative singular
/// - `два паскаля` — genitive singular
/// - `пять паскалей` — genitive plural
struct Government {
    symbol: &'static str,
    one: &'static str,
    few: &'static str,
    many: &'static str,
}

const fn unit(
    symbol: &'static str,
    one: &'static str,
    few: &'static str,
    many: &'static str,
) -> Government {
    Government {
        symbol,
        one,
        few,
        many,
    }
}

const UNITS: &[Government] = &[
    unit("м", "метр", "метра", "метров"),
    unit("кг", "килограмм", "килограмма", "килограммов"),
    unit("с", "секунда", "секунды", "секунд"),
    unit("А", "ампер", "ампера", "амперов"),
    unit("К", "кельвин", "кельвина", "кельвинов"),
    unit("моль", "моль", "моля", "молей"),
    unit("кд", "кандела", "канделы", "кандел"),
    unit("Н", "ньютон", "ньютона", "ньютонов"),
    unit("Дж", "джоуль", "джоуля", "джоулей"),
    unit("Вт", "ватт", "ватта", "ваттов"),
    unit("Па", "паскаль", "паскаля", "паскалей"),
    unit("Кл", "кулон", "кулона", "кулонов"),
    unit("В", "вольт", "вольта", "вольтов"),
    unit("Ом", "ом", "ома", "омов"),
    // Гц, кГц and МГц have the same form in the nominative singular and the
    // genitive plural: «пять герц», not «пять герцев».
    unit("Гц", "герц", "герца", "герц"),
    unit("нм", "нанометр", "нанометра", "нанометров"),
    unit("мм", "миллиметр", "миллиметра", "миллиметров"),
    unit("см", "сантиметр", "сантиметра", "сантиметров"),
    unit("км", "километр", "километра", "километров"),
    unit("мс", "миллисекунда", "миллисекунды", "миллисекунд"),
    unit("кГц", "килогерц", "килогерца", "килогерц"),
    unit("МГц", "мегагерц", "мегагерца", "мегагерц"),
];

fn compile(spoken: &str) -> String {
    let result = interpret(
        spoken,
        InterpretOptions {
            domain: Domain::Auto,
            ..Default::default()
        },
    );
    render_result(&result, Renderer::Unicode)
}

#[test]
fn every_unit_survives_the_numeral_it_follows() {
    let mut failures = Vec::new();
    for u in UNITS {
        for (number, digits, form) in [
            ("один", "1", u.one),
            ("два", "2", u.few),
            ("пять", "5", u.many),
        ] {
            let spoken = format!("{number} {form}");
            let expected = format!("{digits} {}", u.symbol);
            let actual = compile(&spoken);
            if actual != expected {
                failures.push(format!("  {spoken:?} → {actual:?}, expected {expected:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} numeral+unit forms did not compile:\n{}",
        failures.len(),
        UNITS.len() * 3,
        failures.join("\n")
    );
}

/// Feminine numerals take the same forms: «одна секунда», «две секунды».
#[test]
fn the_feminine_numeral_reaches_the_same_unit() {
    assert_eq!(compile("одна секунда"), "1 с");
    assert_eq!(compile("две секунды"), "2 с");
    assert_eq!(compile("одна кандела"), "1 кд");
}

/// The table above is the contract. A unit added to `units.yaml` without a
/// row here is a unit nobody has checked against Russian grammar, so this
/// fails rather than quietly leaving it half-usable.
#[test]
fn no_unit_reaches_the_lexicon_without_its_declension() {
    let declined: BTreeSet<&str> = UNITS.iter().map(|u| u.symbol).collect();
    let loaded: BTreeSet<&str> = Lexicon::builtin()
        .units
        .iter()
        .map(|u| u.symbol.as_str())
        .collect();
    let missing: Vec<&&str> = loaded.difference(&declined).collect();
    let stale: Vec<&&str> = declined.difference(&loaded).collect();
    assert!(
        missing.is_empty(),
        "units.yaml defines {missing:?} but no numeral forms were checked for them"
    );
    assert!(
        stale.is_empty(),
        "this table names {stale:?}, which units.yaml no longer defines"
    );
}

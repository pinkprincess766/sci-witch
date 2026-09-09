//! The same quantity, offered a second way.
//!
//! This is the one place in the project that does arithmetic on a value the
//! user dictated, and it does not change that value. `1 км` stays `1 км`;
//! `1000 м (тысяча метров)` arrives beside it as a warning, exactly as an
//! unbalanced equation is offered coefficients and left alone.

use sciwhisper_core::{interpret, render, Domain, InterpretOptions, Renderer};

fn compiled(text: &str) -> sciwhisper_core::InterpretationResult {
    interpret(
        text,
        InterpretOptions {
            domain: Domain::Physics,
            ..Default::default()
        },
    )
}

fn suggestion(text: &str) -> Option<String> {
    compiled(text)
        .warnings
        .iter()
        .find(|warning| warning.code == "physics.unit_equivalent")
        .map(|warning| warning.message.clone())
}

fn inserted(text: &str) -> String {
    render(&compiled(text).ast, Renderer::Unicode)
}

/// The suggestion is offered and the dictated value is untouched.
#[test]
fn the_equivalent_is_offered_and_nothing_is_replaced() {
    let cases = [
        (
            "один километр",
            "1 км",
            "то же самое: 1000 м (тысяча метров)",
        ),
        (
            "два километра",
            "2 км",
            "то же самое: 2000 м (две тысячи метров)",
        ),
        (
            "пять километров",
            "5 км",
            "то же самое: 5000 м (пять тысяч метров)",
        ),
        (
            "два килогерца",
            "2 кГц",
            "то же самое: 2000 Гц (две тысячи герц)",
        ),
    ];
    for (said, kept, offered) in cases {
        assert_eq!(inserted(said), kept, "{said} must be inserted as dictated");
        assert_eq!(suggestion(said).as_deref(), Some(offered), "{said}");
    }
}

/// Converting towards a smaller unit makes the number harder to read, so
/// nothing is offered: `300 нм` is clearer than `0.0000003 м`.
#[test]
fn a_conversion_that_would_read_worse_is_not_offered() {
    for said in ["триста нанометров", "сто миллисекунд", "пять миллиметров"]
    {
        assert_eq!(suggestion(said), None, "{said}");
    }
}

/// A base unit is already the simplest form of itself.
#[test]
fn a_base_unit_gets_no_suggestion() {
    for said in ["пять метров", "три секунды", "семь джоулей", "сто паскалей"]
    {
        assert_eq!(suggestion(said), None, "{said}");
    }
}

/// A compound unit is not converted: turning one part of `км/с` into metres
/// would state something the speaker did not.
#[test]
fn a_compound_unit_is_left_alone() {
    assert_eq!(inserted("один километр в секунду"), "1 км/с");
    assert_eq!(suggestion("один километр в секунду"), None);
}

/// The unit word agrees with the number it ends up next to, not with the
/// one that was spoken.
#[test]
fn the_spelled_form_agrees_with_the_converted_number() {
    // 1 км is a thousand metres — «метров», though «километр» was singular.
    assert!(suggestion("один километр").is_some_and(|text| text.ends_with("(тысяча метров)")));
    // 2 км is two thousand — «две тысячи», feminine, agreeing with «тысяча».
    assert!(suggestion("два километра").is_some_and(|text| text.contains("две тысячи метров")));
}

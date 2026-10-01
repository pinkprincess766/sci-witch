//! Genuine ambiguity is offered as a choice, not resolved by guessing.
//!
//! «корень из икс плюс один» has two readings — `√x + 1` and `√(x+1)` — and
//! spoken Russian carries no bracket to tell them apart. Until now the
//! parser picked the narrow one, warned about it, and dropped its confidence
//! below the auto-insert threshold; the second reading was described in a
//! comment and never built. The result was the worst of both: the user was
//! told the answer might be wrong and given no way to say which one they
//! meant.
//!
//! What must stay true, and is asserted below:
//!
//! * the **primary** reading does not change — the narrow one is still what
//!   the system answers with, because it alters the least of what was said;
//! * the second reading is a real, complete parse of the whole utterance,
//!   not the first tree with a bracket pasted in;
//! * an utterance with no ambiguity gains no alternatives, so a choice is
//!   never offered where there is nothing to choose.

use sciwhisper_core::{interpret, render, Domain, InterpretOptions, Renderer};

fn readings(spoken: &str) -> (String, Vec<String>) {
    let result = interpret(
        spoken,
        InterpretOptions {
            domain: Domain::Mathematics,
            ..Default::default()
        },
    );
    (
        render(&result.ast, Renderer::Unicode),
        result
            .alternatives
            .iter()
            .map(|node| render(node, Renderer::Unicode))
            .collect(),
    )
}

#[test]
fn an_open_root_offers_the_other_reading_without_changing_the_answer() {
    let (primary, alternatives) = readings("корень из икс плюс один");
    assert_eq!(primary, "√x + 1", "the narrow reading stays the answer");
    assert_eq!(alternatives, vec!["√(x + 1)".to_string()]);
}

/// The alternative is a parse of the **whole** utterance, so everything
/// outside the radical survives it.
#[test]
fn the_other_reading_keeps_the_rest_of_the_equation() {
    let (primary, alternatives) = readings("корень из икс плюс один равно два");
    assert_eq!(primary, "√x + 1 = 2");
    assert_eq!(alternatives, vec!["√(x + 1) = 2".to_string()]);
}

/// A speaker who said where the root ends has already answered the
/// question; offering them a choice would be noise.
#[test]
fn an_explicitly_closed_root_is_not_ambiguous() {
    let (primary, alternatives) = readings("начало корня икс плюс один конец корня");
    assert_eq!(primary, "√(x + 1)");
    assert!(alternatives.is_empty(), "{alternatives:?}");
}

#[test]
fn an_unambiguous_utterance_gains_nothing() {
    for spoken in [
        "корень из икс",
        "икс в квадрате плюс два икс минус три равно нулю",
        "два плюс два",
    ] {
        let (_, alternatives) = readings(spoken);
        assert!(alternatives.is_empty(), "{spoken}: {alternatives:?}");
    }
}

/// A minus binds exactly as a plus does; the ambiguity is about where the
/// radical ends, not about which operator follows it.
#[test]
fn subtraction_after_a_root_is_equally_ambiguous() {
    let (primary, alternatives) = readings("корень из икс минус один");
    assert_eq!(primary, "√x − 1");
    assert_eq!(alternatives, vec!["√(x − 1)".to_string()]);
}

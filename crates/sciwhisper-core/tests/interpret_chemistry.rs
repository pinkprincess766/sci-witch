//! `interpret_chemistry` forces chemistry. A math phrase must not become a
//! math tree here, while the same phrase through `interpret` with `Domain::Auto`
//! must: otherwise this file would pass even if the helper fell back.

use sciwhisper_core::ast::Node;
use sciwhisper_core::interpret::{interpret, interpret_chemistry, InterpretOptions};
use sciwhisper_core::{render_result, Domain, Renderer};

#[test]
fn copper_hydroxide_stays_chemistry() {
    let result = interpret_chemistry("гидроксид меди два");
    assert_eq!(result.domain, Domain::Chemistry);
    assert!(result.confidence > 0.0, "{:?}", result.warnings);
    assert_eq!(render_result(&result, Renderer::Unicode), "Cu(OH)₂");
}

#[test]
fn a_math_phrase_does_not_become_a_math_tree() {
    let forced = interpret_chemistry("икс плюс один");
    assert_eq!(forced.domain, Domain::Chemistry);
    assert!(
        !matches!(forced.ast, Node::Math(_)),
        "chemistry-only entry produced a math tree: {:?}",
        forced.ast
    );

    let auto = interpret(
        "икс плюс один",
        InterpretOptions {
            domain: Domain::Auto,
            allow_shortcuts: true,
        },
    );
    assert_eq!(auto.domain, Domain::Mathematics);
    assert!(
        matches!(auto.ast, Node::Math(_)),
        "auto did not parse the math phrase: {:?}",
        auto.ast
    );
}

#[test]
fn ordinary_speech_stays_text() {
    let result = interpret_chemistry("сегодня хорошая погода");
    assert_eq!(result.domain, Domain::Chemistry);
    assert!(matches!(result.ast, Node::Text(_)), "{:?}", result.ast);
    assert!(result.confidence <= 0.0);
}

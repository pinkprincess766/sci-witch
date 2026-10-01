//! The recursive descent has a fixed depth, so a pathological utterance
//! comes back as words instead of killing the process.
//!
//! `MAX_PARSE_DEPTH` is `pub` on `parser::math`, the same path as
//! `MAX_FUNCTION_ARGS`. It is not re-exported from the crate root.

use sciwhisper_core::parser::math::MAX_PARSE_DEPTH;
use sciwhisper_core::{
    interpret, render_result, Domain, InterpretOptions, InterpretationResult, Node, Renderer,
};

fn compile(text: &str) -> InterpretationResult {
    interpret(
        text,
        InterpretOptions {
            domain: Domain::Mathematics,
            allow_shortcuts: false,
        },
    )
}

fn assert_formula(text: &str) -> InterpretationResult {
    let result = compile(text);
    assert!(
        result.confidence > 0.0,
        "should parse, got {:?}",
        result.unresolved_spans
    );
    assert!(
        matches!(result.ast, Node::Math(_)),
        "expected a formula, got {:?}",
        result.ast
    );
    result
}

fn assert_words(text: &str) {
    let result = compile(text);
    assert_eq!(result.confidence, 0.0, "{:?}", result.warnings);
    assert_eq!(result.raw_transcript, text);
    assert!(matches!(result.ast, Node::Text(ref got) if got == text));
    assert_eq!(render_result(&result, Renderer::Unicode), text);
    let reason = &result.unresolved_spans[0].reason;
    assert!(
        reason.contains(&MAX_PARSE_DEPTH.to_string()),
        "the refusal must be the depth limit, not some other parse error: {reason}"
    );
}

fn nested_parens(depth: usize) -> String {
    let mut text = String::new();
    for _ in 0..depth {
        text.push_str("открыть скобку ");
    }
    text.push_str("икс");
    for _ in 0..depth {
        text.push_str(" закрыть скобку");
    }
    text
}

fn nested_fractions(depth: usize) -> String {
    let mut text = String::new();
    for _ in 0..depth {
        text.push_str("начало дроби числитель ");
    }
    text.push_str("икс");
    for _ in 0..depth {
        text.push_str(" знаменатель один конец дроби");
    }
    text
}

/// Root and absolute value, alternating, `levels` deep. No parentheses:
/// a limit that only counted brackets would still parse this.
fn nested_root_abs(levels: usize) -> String {
    let mut text = String::new();
    for i in 0..levels {
        if i % 2 == 0 {
            text.push_str("корень из ");
        } else {
            text.push_str("модуль ");
        }
    }
    text.push_str("икс");
    text
}

fn nested_powers(depth: usize) -> String {
    let mut text = String::from("икс");
    for _ in 1..depth {
        text.push_str(" начало степени игрек");
    }
    text.push_str(" начало степени один");
    for _ in 0..depth {
        text.push_str(" конец степени");
    }
    text
}

/// The three constructs in one chain, so the bomb is not only brackets.
fn nested_mix(depth: usize) -> String {
    let mut text = String::new();
    for _ in 0..depth {
        text.push_str("корень из модуль открыть скобку ");
    }
    text.push_str("икс");
    for _ in 0..depth {
        text.push_str(" закрыть скобку");
    }
    text
}

fn on_small_stack(text: String) -> InterpretationResult {
    // 8 MiB is the stack on which a thousand nested parentheses used to abort
    // the process. Overflow is not a catchable panic: if the limit is gone,
    // this thread takes the whole test with it.
    let handle = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || compile(&text))
        .expect("spawn the depth-limit thread");
    handle
        .join()
        .expect("the depth limit must refuse instead of panicking")
}

#[test]
fn one_pair_of_parentheses_is_unchanged() {
    let result = assert_formula("открыть скобку икс закрыть скобку");
    assert_eq!(render_result(&result, Renderer::Unicode), "(x)");
}

#[test]
fn parentheses_at_the_depth_limit_parse_and_one_more_stays_words() {
    let at_limit = nested_parens(MAX_PARSE_DEPTH);
    let result = assert_formula(&at_limit);
    let shown = render_result(&result, Renderer::Unicode);
    assert_eq!(shown.chars().filter(|c| *c == '(').count(), MAX_PARSE_DEPTH);
    assert_eq!(shown.chars().filter(|c| *c == ')').count(), MAX_PARSE_DEPTH);
    assert!(shown.contains('x'), "{shown}");

    assert_words(&nested_parens(MAX_PARSE_DEPTH + 1));
}

#[test]
fn the_default_interpret_returns_the_raw_text_past_the_limit() {
    let text = nested_parens(MAX_PARSE_DEPTH + 1);
    let result = interpret(&text, InterpretOptions::default());
    assert_eq!(result.confidence, 0.0);
    assert!(matches!(result.ast, Node::Text(ref got) if got == &text));
    assert_eq!(render_result(&result, Renderer::Unicode), text);
}

#[test]
fn fractions_at_the_depth_limit_parse_and_one_more_stays_words() {
    let one = assert_formula(&nested_fractions(1));
    assert_eq!(render_result(&one, Renderer::Unicode), "(x)/(1)");

    let at_limit = assert_formula(&nested_fractions(MAX_PARSE_DEPTH));
    let shown = render_result(&at_limit, Renderer::Unicode);
    assert_eq!(shown.chars().filter(|c| *c == '/').count(), MAX_PARSE_DEPTH);

    assert_words(&nested_fractions(MAX_PARSE_DEPTH + 1));
}

#[test]
fn roots_and_absolute_values_share_one_depth_counter() {
    let one = assert_formula("корень из модуль икс");
    assert_eq!(render_result(&one, Renderer::Unicode), "√|x|");

    let at_limit = assert_formula(&nested_root_abs(MAX_PARSE_DEPTH));
    let shown = render_result(&at_limit, Renderer::Unicode);
    let roots = MAX_PARSE_DEPTH.div_ceil(2);
    let absolutes = MAX_PARSE_DEPTH / 2;
    assert_eq!(
        shown.chars().filter(|c| *c == '√').count(),
        roots,
        "{shown}"
    );
    assert_eq!(
        shown.chars().filter(|c| *c == '|').count(),
        absolutes * 2,
        "{shown}"
    );

    // One more construct, still with no extra parentheses. 65 frames do not
    // overflow; only the shared counter can refuse this.
    assert_words(&nested_root_abs(MAX_PARSE_DEPTH + 1));
}

#[test]
fn a_root_of_an_absolute_value_of_a_parenthesis_parses() {
    let result = assert_formula("корень из модуль открыть скобку икс закрыть скобку");
    assert_eq!(render_result(&result, Renderer::Unicode), "√|(x)|");
}

#[test]
fn powers_at_the_depth_limit_parse_and_one_more_stays_words() {
    let at_limit = assert_formula(&nested_powers(MAX_PARSE_DEPTH));
    let shown = render_result(&at_limit, Renderer::Unicode);
    assert!(shown.contains('x'), "{shown}");
    assert!(!shown.contains("начало"), "{shown}");

    assert_words(&nested_powers(MAX_PARSE_DEPTH + 1));
}

/// Sibling groups must not consume the budget. Forgetting to lower the
/// counter — the same shape as `stop_at_differential` being cleared on one
/// branch only — would reject a flat sum of `MAX_PARSE_DEPTH + 1` pairs.
#[test]
fn sibling_groups_past_the_limit_still_parse() {
    let text = std::iter::repeat_n("открыть скобку икс закрыть скобку", MAX_PARSE_DEPTH + 1)
        .collect::<Vec<_>>()
        .join(" плюс ");
    let result = assert_formula(&text);
    let shown = render_result(&result, Renderer::Unicode);
    assert_eq!(
        shown.chars().filter(|c| *c == '(').count(),
        MAX_PARSE_DEPTH + 1,
        "{shown}"
    );
}

#[test]
fn an_open_root_still_keeps_its_alternative() {
    let result = assert_formula("корень из икс плюс один");
    assert_eq!(render_result(&result, Renderer::Unicode), "√x + 1");
    assert_eq!(result.alternatives.len(), 1);
}

#[test]
fn ten_thousand_nested_constructs_refuse_without_panic() {
    let bomb = 10_000;
    for text in [
        nested_parens(bomb),
        nested_fractions(bomb),
        nested_root_abs(bomb),
        nested_mix(bomb),
        nested_powers(bomb),
    ] {
        let result = on_small_stack(text);
        assert_eq!(result.confidence, 0.0);
        assert!(matches!(result.ast, Node::Text(_)), "{:?}", result.ast);
        assert_eq!(
            render_result(&result, Renderer::Unicode),
            result.raw_transcript
        );
        let reason = &result.unresolved_spans[0].reason;
        assert!(reason.contains(&MAX_PARSE_DEPTH.to_string()), "{reason}");
    }
}

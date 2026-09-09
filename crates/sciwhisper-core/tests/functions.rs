//! Named functions and subscripts.
//!
//! Two gaps this closes. «эф от икс» produced nothing at all, and «эф икс»
//! produced `fx` — a product, which is a different statement. And a
//! subscript that was not a digit came out as `x_{n}`: LaTeX syntax leaking
//! into a string that is supposed to be plain Unicode.
//!
//! Everything goes through the public path the CLI and the application use.

use sciwhisper_core::ast::{Math, Node};
use sciwhisper_core::{
    interpret, interpret_utterance, render, Domain, InterpretOptions, Renderer, UtteranceMode,
    UtteranceOptions,
};

fn spoken(text: &str) -> String {
    let result = interpret_utterance(
        text,
        UtteranceOptions {
            domain: Domain::Auto,
            mode: UtteranceMode::MixedText,
            allow_shortcuts: true,
        },
    );
    render(&result.document, Renderer::Unicode)
}

fn compiled(text: &str) -> sciwhisper_core::InterpretationResult {
    interpret(
        text,
        InterpretOptions {
            domain: Domain::Mathematics,
            ..Default::default()
        },
    )
}

fn rendered(text: &str, renderer: Renderer) -> String {
    render(&compiled(text).ast, renderer)
}

// ------------------------------------------------------ function application

#[test]
fn a_named_function_is_applied_not_multiplied() {
    assert_eq!(spoken("эф от икс"), "f(x)");
    assert_eq!(spoken("же от тэ"), "g(t)");
    assert_eq!(spoken("же от тэ равно ноль"), "g(t) = 0");
    // The introducer is not part of the expression.
    assert_eq!(spoken("функция эф от икс"), "f(x)");
}

#[test]
fn several_arguments_are_separated_by_a_spoken_comma() {
    assert_eq!(spoken("эф от икс запятая игрек"), "f(x, y)");
    assert_eq!(spoken("эф от икс запятая игрек запятая зет"), "f(x, y, z)");
}

/// Arguments bind narrowly, the same convention the spoken root uses and
/// for the same reason: speech carries no closing bracket. «эф от икс плюс
/// один» is `f(x) + 1`, and a speaker who meant `f(x + 1)` says so with an
/// explicit bracket.
#[test]
fn an_argument_ends_where_the_next_term_begins() {
    assert_eq!(spoken("эф от икс плюс один"), "f(x) + 1");
    assert_eq!(
        spoken("эф от открыть скобку икс плюс один закрыть скобку"),
        "f((x + 1))"
    );
}

#[test]
fn the_tree_says_apply_rather_than_multiply() {
    let Node::Math(math) = compiled("эф от икс").ast else {
        panic!("expected mathematics")
    };
    let Math::Apply { name, args } = math else {
        panic!("expected an application, got {math:?}")
    };
    assert!(matches!(*name, Math::Symbol(_)));
    assert_eq!(args.len(), 1);

    // The old reading is still available where it is what was said.
    let Node::Math(product) = compiled("эф икс").ast else {
        panic!("expected mathematics")
    };
    assert!(matches!(product, Math::Juxt(_)), "{product:?}");
}

#[test]
fn every_renderer_shows_the_application() {
    assert_eq!(
        rendered("эф от икс запятая игрек", Renderer::Unicode),
        "f(x, y)"
    );
    assert_eq!(
        rendered("эф от икс запятая игрек", Renderer::Latex),
        r"f\left(x, y\right)"
    );
    let omml = rendered("эф от икс", Renderer::Omml);
    assert!(omml.starts_with("<m:oMath"), "{omml}");
    assert!(omml.contains(">f<"), "{omml}");
    assert!(omml.contains(">(<") && omml.contains(">)<"), "{omml}");
}

/// The dimension of `f(x)` depends on what `f` is, and nothing here knows
/// that. Unknown is the honest answer; "dimensionless" would be a claim.
#[test]
fn an_applied_function_has_no_proven_dimension() {
    use sciwhisper_core::dimension::{infer, Inferred};
    let Node::Math(math) = compiled("эф от икс").ast else {
        panic!("expected mathematics")
    };
    assert!(matches!(infer(&math), Inferred::Unknown));
}

/// «от» introduces bounds for an integral, a sum and a product, and those
/// must not start being read as function application.
#[test]
fn the_constructions_that_already_used_this_word_still_work() {
    assert_eq!(
        spoken("интеграл от нуля до единицы икс в квадрате по икс"),
        "∫₀¹ x² dx"
    );
    assert_eq!(
        spoken("сумма от и равно один до эн икс индекс и"),
        "∑_{i=1}^{n} xᵢ"
    );
}

/// A run of commas is a misrecognition, not an argument list.
///
/// The parser refuses outright. What the *span search* then does is take
/// the longest prefix that did parse and leave the rest as prose — so the
/// user sees a visibly truncated `f(x, y, …) запятая игрек`, not a
/// plausible-looking wrong answer. That is a different situation from
/// «гидроксид железа минус три», where the short reading looked complete
/// and had to be suppressed; here the leftovers are the tell.
#[test]
fn the_argument_list_has_a_stated_limit() {
    use sciwhisper_core::parser::math::MAX_FUNCTION_ARGS;
    let many = std::iter::repeat_n("запятая игрек", MAX_FUNCTION_ARGS + 3)
        .collect::<Vec<_>>()
        .join(" ");
    let said = format!("эф от икс {many}");

    // The parser itself refuses: there is no argument list this long.
    assert_eq!(compiled(&said).confidence, 0.0);

    // And nothing anywhere produces more arguments than the limit allows.
    let shown = spoken(&said);
    assert!(shown.matches(", ").count() < MAX_FUNCTION_ARGS, "{shown}");
    assert!(
        shown.contains("запятая"),
        "the rest must stay visible: {shown}"
    );

    // One below the limit still compiles whole.
    let ok = std::iter::repeat_n("запятая игрек", MAX_FUNCTION_ARGS - 1)
        .collect::<Vec<_>>()
        .join(" ");
    let full = spoken(&format!("эф от икс {ok}"));
    assert!(full.starts_with("f(x, y"), "{full}");
    assert!(!full.contains("запятая"), "{full}");
}

// ------------------------------------------------------------- subscripts

/// Unicode has subscript letters for part of the alphabet. Using them is
/// what makes the Unicode renderer produce Unicode; `x_{n}` was LaTeX.
#[test]
fn a_letter_subscript_uses_a_real_unicode_subscript() {
    assert_eq!(spoken("икс индекс эн"), "xₙ");
    assert_eq!(spoken("икс индекс и"), "xᵢ");
    assert_eq!(spoken("а индекс ка"), "aₖ");
    // Digits were already right.
    assert_eq!(spoken("икс индекс один"), "x₁");
}

/// The subscript block is incomplete — there is no subscript `b` — and the
/// fallback stays rather than half-subscripting the result.
#[test]
fn a_letter_with_no_subscript_keeps_the_explicit_form() {
    assert_eq!(spoken("икс индекс бэ"), "x_{b}");
    assert_eq!(spoken("икс индекс зет"), "x_{z}");
}

/// LaTeX and OMML are unaffected by the Unicode change: each has its own
/// way of writing a subscript and always did.
#[test]
fn the_other_renderers_write_subscripts_their_own_way() {
    assert_eq!(rendered("икс индекс эн", Renderer::Latex), "x_{n}");
    let omml = rendered("икс индекс эн", Renderer::Omml);
    assert!(omml.contains("<m:sSub>"), "{omml}");
}

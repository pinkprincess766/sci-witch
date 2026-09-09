//! Calculus slice: spoken Russian → typed derivative/limit AST → three
//! renderers, plus the conservative behaviour that keeps an incomplete
//! construct as raw text.
//!
//! Nothing in this suite expects a *computed* derivative or limit: the
//! system records what was dictated and never evaluates it.

use sciwhisper_core::ast::{
    Case, DerivativeKind, DerivativeVariable, FunctionKind, LimitDirection, Math, Node, Symbol,
};
use sciwhisper_core::{interpret, render_result, Domain, InterpretOptions, Renderer};

fn parsed(domain: Domain, text: &str) -> sciwhisper_core::InterpretationResult {
    let result = interpret(
        text,
        InterpretOptions {
            domain,
            allow_shortcuts: false,
        },
    );
    assert!(
        result.confidence > 0.0,
        "failed to parse '{text}': {:?}",
        result.unresolved_spans
    );
    result
}

fn ast(text: &str) -> Math {
    match parsed(Domain::Mathematics, text).ast {
        Node::Math(math) => math,
        other => panic!("expected a math node for '{text}', got {other:?}"),
    }
}

fn unicode(text: &str) -> String {
    render_result(&parsed(Domain::Mathematics, text), Renderer::Unicode)
}

fn latex(text: &str) -> String {
    render_result(&parsed(Domain::Mathematics, text), Renderer::Latex)
}

fn omml(text: &str) -> String {
    render_result(&parsed(Domain::Mathematics, text), Renderer::Omml)
}

fn sym(letter: char) -> Math {
    Math::Symbol(Symbol::latin(letter, Case::Lower))
}

fn render_all(math: &Math) -> [String; 3] {
    let node = Node::Math(math.clone());
    [
        sciwhisper_core::render(&node, Renderer::Unicode),
        sciwhisper_core::render(&node, Renderer::Latex),
        sciwhisper_core::render(&node, Renderer::Omml),
    ]
}

fn derivative(kind: DerivativeKind, expr: Math, variables: &[(Math, u32)]) -> Math {
    Math::derivative(
        kind,
        expr,
        variables
            .iter()
            .cloned()
            .map(|(variable, order)| DerivativeVariable::new(variable, order))
            .collect(),
    )
    .expect("the test builds a well-formed derivative")
}

// ------------------------------------------------------------------ parser
// Every required speech form is checked against the AST, not only against
// the rendered string.

#[test]
fn spoken_first_derivative_builds_the_typed_node() {
    for text in ["производная эф по икс", "первая производная эф по икс"]
    {
        assert_eq!(
            ast(text),
            derivative(DerivativeKind::Ordinary, sym('f'), &[(sym('x'), 1)]),
            "{text}"
        );
    }
}

#[test]
fn spoken_second_derivative_builds_order_two() {
    assert_eq!(
        ast("вторая производная игрек по икс"),
        derivative(DerivativeKind::Ordinary, sym('y'), &[(sym('x'), 2)])
    );
}

#[test]
fn spoken_order_after_the_noun_builds_the_same_order() {
    assert_eq!(
        ast("производная третьего порядка игрек по тэ"),
        derivative(DerivativeKind::Ordinary, sym('y'), &[(sym('t'), 3)])
    );
}

#[test]
fn spoken_partial_derivative_builds_the_partial_kind() {
    assert_eq!(
        ast("частная производная тэ по икс"),
        derivative(DerivativeKind::Partial, sym('t'), &[(sym('x'), 1)])
    );
    assert_eq!(
        ast("вторая частная производная тэ по икс"),
        derivative(DerivativeKind::Partial, sym('t'), &[(sym('x'), 2)])
    );
}

#[test]
fn spoken_mixed_partial_keeps_the_variables_apart() {
    let node = ast("частная производная второго порядка тэ по икс и по игрек");
    assert_eq!(
        node,
        derivative(
            DerivativeKind::Partial,
            sym('t'),
            &[(sym('x'), 1), (sym('y'), 1)]
        )
    );
    match node {
        Math::Derivative { variables, .. } => {
            // Two structural entries, never one fused «dxdy».
            assert_eq!(variables.len(), 2);
            assert_eq!(*variables[0].variable, sym('x'));
            assert_eq!(*variables[1].variable, sym('y'));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn spoken_two_sided_limit_builds_the_typed_node() {
    assert_eq!(
        ast("предел при икс стремящемся к нулю синуса икс делённого на икс"),
        Math::limit(
            sym('x'),
            Math::Number("0".into()),
            LimitDirection::TwoSided,
            Math::Binary {
                op: sciwhisper_core::ast::BinOp::Div,
                left: Box::new(Math::Function {
                    kind: FunctionKind::Sin,
                    arg: Box::new(sym('x')),
                }),
                right: Box::new(sym('x')),
            },
        )
    );
}

#[test]
fn spoken_limit_body_may_precede_the_approach_clause() {
    assert_eq!(
        ast("предел функции эф при тэ стремящемся к бесконечности"),
        Math::limit(sym('t'), Math::Infinity, LimitDirection::TwoSided, sym('f'),)
    );
}

#[test]
fn spoken_one_sided_limits_are_typed_directions() {
    assert_eq!(
        ast("предел слева при икс стремящемся к нулю эф"),
        Math::limit(
            sym('x'),
            Math::Number("0".into()),
            LimitDirection::FromLeft,
            sym('f'),
        )
    );
    assert_eq!(
        ast("предел справа при икс стремящемся к двум эф"),
        Math::limit(
            sym('x'),
            Math::Number("2".into()),
            LimitDirection::FromRight,
            sym('f'),
        )
    );
    // The adjective may also precede the noun.
    assert_eq!(
        ast("левый предел при икс стремящемся к нулю эф"),
        ast("предел слева при икс стремящемся к нулю эф")
    );
}

#[test]
fn spoken_limit_targets_cover_zero_number_symbol_and_both_infinities() {
    let target = |text: &str| match ast(text) {
        Math::Limit { target, .. } => *target,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        target("предел при икс стремящемся к нулю эф"),
        Math::Number("0".into())
    );
    assert_eq!(
        target("предел при икс стремящемся к двум эф"),
        Math::Number("2".into())
    );
    assert_eq!(target("предел при икс стремящемся к а эф"), sym('a'));
    assert_eq!(
        target("предел при икс стремящемся к бесконечности эф"),
        Math::Infinity
    );
    assert_eq!(
        target("предел при икс стремящемся к плюс бесконечности эф"),
        Math::Infinity
    );
    assert_eq!(
        target("предел при икс стремящемся к минус бесконечности эф"),
        Math::UnaryMinus(Box::new(Math::Infinity))
    );
}

#[test]
fn a_derivative_binds_tighter_than_addition() {
    // «производная эф по икс плюс один» is df/dx + 1, not d(f)/d(x+1).
    match ast("производная эф по икс плюс один") {
        Math::Binary { op, left, right } => {
            assert_eq!(op, sciwhisper_core::ast::BinOp::Add);
            assert_eq!(
                *left,
                derivative(DerivativeKind::Ordinary, sym('f'), &[(sym('x'), 1)])
            );
            assert_eq!(*right, Math::Number("1".into()));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_derivative_expression_may_be_a_product_or_a_power() {
    assert_eq!(
        ast("производная икс в квадрате по икс"),
        derivative(
            DerivativeKind::Ordinary,
            Math::Power {
                base: Box::new(sym('x')),
                exp: Box::new(Math::Number("2".into())),
            },
            &[(sym('x'), 1)]
        )
    );
}

#[test]
fn by_still_means_the_integration_variable_and_a_word_fraction() {
    // The new «по» rules must not disturb the constructs that already used
    // «по» and «на».
    assert_eq!(unicode("интеграл эф по икс"), "∫ f dx");
    assert_eq!(
        unicode("интеграл от нуля до единицы икс в квадрате по икс"),
        "∫₀¹ x² dx"
    );
    assert_eq!(unicode("икс делённое на игрек"), "x/y");
}

// --------------------------------------------------------------- renderers

#[test]
fn unicode_renders_the_linear_calculus_forms() {
    assert_eq!(unicode("производная эф по икс"), "df/dx");
    assert_eq!(unicode("вторая производная игрек по икс"), "d²y/dx²");
    assert_eq!(unicode("частная производная тэ большое по икс"), "∂T/∂x");
    assert_eq!(
        unicode("частная производная второго порядка тэ большое по икс и по игрек"),
        "∂²T/(∂x∂y)"
    );
    assert_eq!(
        unicode("предел при икс стремящемся к нулю синуса икс делённого на икс"),
        "lim_{x→0} sin(x)/x"
    );
    assert_eq!(
        unicode("предел слева при икс стремящемся к нулю эф"),
        "lim_{x→0⁻} f"
    );
    assert_eq!(
        unicode("предел справа при икс стремящемся к нулю эф"),
        "lim_{x→0⁺} f"
    );
}

#[test]
fn latex_renders_structural_fractions_and_limits() {
    assert_eq!(latex("производная эф по икс"), "\\frac{d f}{d x}");
    assert_eq!(
        latex("вторая производная игрек по икс"),
        "\\frac{d^{2} y}{d x^{2}}"
    );
    assert_eq!(
        latex("частная производная тэ большое по икс"),
        "\\frac{\\partial T}{\\partial x}"
    );
    assert_eq!(
        latex("частная производная второго порядка тэ большое по икс и по игрек"),
        "\\frac{\\partial^{2} T}{\\partial x\\,\\partial y}"
    );
    assert_eq!(
        latex("предел при икс стремящемся к нулю синуса икс делённого на икс"),
        "\\lim_{x \\to 0} \\frac{\\sin\\left(x\\right)}{x}"
    );
    assert_eq!(
        latex("предел слева при икс стремящемся к нулю эф"),
        "\\lim_{x \\to 0^-} f"
    );
    assert_eq!(
        latex("предел справа при икс стремящемся к двум эф"),
        "\\lim_{x \\to 2^+} f"
    );
}

#[test]
fn omml_uses_native_word_structures_for_a_derivative() {
    let d = |text: &str| format!("<m:r><m:t xml:space=\"preserve\">{text}</m:t></m:r>");
    assert_eq!(
        omml("производная эф по икс"),
        format!(
            "<m:oMath xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\">\
             <m:f><m:num>{}{}</m:num><m:den>{}{}</m:den></m:f></m:oMath>",
            d("d"),
            d("f"),
            d("d"),
            d("x")
        )
    );
}

#[test]
fn omml_puts_the_order_in_real_superscripts() {
    let xml = omml("вторая производная игрек по икс");
    assert!(xml.contains("<m:f><m:num><m:sSup>"), "{xml}");
    assert!(
        xml.contains("<m:sup><m:r><m:t xml:space=\"preserve\">2</m:t></m:r></m:sup>"),
        "{xml}"
    );
    // The `d` and the variable each carry their own run.
    assert!(
        xml.contains("<m:t xml:space=\"preserve\">d</m:t>")
            && xml.contains("<m:t xml:space=\"preserve\">y</m:t>"),
        "{xml}"
    );
}

#[test]
fn omml_uses_the_partial_sign_and_one_run_per_mixed_variable() {
    let xml = omml("частная производная второго порядка тэ большое по икс и по игрек");
    assert!(xml.contains("<m:t xml:space=\"preserve\">∂</m:t>"), "{xml}");
    assert_eq!(
        xml.matches("<m:t xml:space=\"preserve\">∂</m:t>").count(),
        3,
        "one ∂ in the numerator and one per variable: {xml}"
    );
}

#[test]
fn omml_uses_lim_low_for_a_limit() {
    let xml = omml("предел слева при икс стремящемся к нулю эф");
    assert!(
        xml.contains("<m:func><m:fName><m:limLow>") && xml.contains("</m:limLow></m:fName>"),
        "{xml}"
    );
    assert!(
        xml.contains("<m:t xml:space=\"preserve\">lim</m:t>"),
        "{xml}"
    );
    assert!(xml.contains("<m:t xml:space=\"preserve\">→</m:t>"), "{xml}");
    // The one-sided marker is a superscript run, not a character glued to 0.
    assert!(
        xml.contains(
            "<m:sSup><m:e><m:r><m:t xml:space=\"preserve\">0</m:t></m:r></m:e>\
             <m:sup><m:r><m:t xml:space=\"preserve\">-</m:t></m:r></m:sup></m:sSup>"
        ),
        "{xml}"
    );
}

#[test]
fn omml_never_embeds_latex_as_text() {
    for text in [
        "производная эф по икс",
        "вторая производная игрек по икс",
        "частная производная второго порядка тэ большое по икс и по игрек",
        "предел при икс стремящемся к нулю синуса икс делённого на икс",
        "предел справа при икс стремящемся к двум эф",
    ] {
        let xml = omml(text);
        for forbidden in ["\\frac", "\\lim", "\\partial", "^{", "_{"] {
            assert!(
                !xml.contains(forbidden),
                "OMML for '{text}' contains LaTeX '{forbidden}': {xml}"
            );
        }
    }
}

#[test]
fn omml_escapes_xml_metacharacters_in_calculus_nodes() {
    // A hand-built symbol carrying XML metacharacters must be escaped in
    // every position a calculus node can put it.
    let hostile = Math::Symbol(Symbol::latin('x', Case::Lower));
    let hostile = match hostile {
        Math::Symbol(mut symbol) => {
            symbol.letter = "<&>".into();
            Math::Symbol(symbol)
        }
        other => other,
    };
    let node = Node::Math(derivative(
        DerivativeKind::Partial,
        hostile.clone(),
        &[(hostile.clone(), 2)],
    ));
    let xml = sciwhisper_core::render(&node, Renderer::Omml);
    assert!(xml.contains("&lt;&amp;&gt;"), "{xml}");
    assert!(!xml.contains("<&>"), "{xml}");

    let node = Node::Math(Math::limit(
        hostile.clone(),
        hostile.clone(),
        LimitDirection::FromRight,
        hostile,
    ));
    let xml = sciwhisper_core::render(&node, Renderer::Omml);
    assert_eq!(xml.matches("&lt;&amp;&gt;").count(), 3, "{xml}");
    assert!(!xml.contains("<&>"), "{xml}");
}

#[test]
fn every_renderer_groups_a_composite_operand() {
    let node = derivative(
        DerivativeKind::Ordinary,
        Math::Power {
            base: Box::new(sym('x')),
            exp: Box::new(Math::Number("2".into())),
        },
        &[(sym('x'), 1)],
    );
    let [unicode, latex, omml] = render_all(&node);
    assert_eq!(unicode, "d(x²)/dx");
    assert_eq!(latex, "\\frac{d \\left(x^{2}\\right)}{d x}");
    // Real parenthesis runs, so no renderer silently reads it as (dx)².
    assert!(
        omml.contains("<m:t xml:space=\"preserve\">(</m:t>"),
        "{omml}"
    );
    assert!(
        omml.contains("<m:t xml:space=\"preserve\">)</m:t>"),
        "{omml}"
    );
}

#[test]
fn every_renderer_brackets_an_additive_limit_body_and_leaves_a_quotient_alone() {
    let additive = Math::limit(
        sym('x'),
        Math::Number("0".into()),
        LimitDirection::TwoSided,
        Math::Binary {
            op: sciwhisper_core::ast::BinOp::Add,
            left: Box::new(sym('x')),
            right: Box::new(Math::Number("1".into())),
        },
    );
    let [unicode, latex, _] = render_all(&additive);
    assert_eq!(unicode, "lim_{x→0} (x + 1)");
    assert_eq!(latex, "\\lim_{x \\to 0} \\left(x + 1\\right)");

    let quotient = Math::limit(
        sym('x'),
        Math::Number("0".into()),
        LimitDirection::TwoSided,
        Math::Binary {
            op: sciwhisper_core::ast::BinOp::Div,
            left: Box::new(sym('x')),
            right: Box::new(sym('y')),
        },
    );
    let [unicode, latex, _] = render_all(&quotient);
    assert_eq!(unicode, "lim_{x→0} x/y");
    assert_eq!(latex, "\\lim_{x \\to 0} \\frac{x}{y}");
}

#[test]
fn a_calculus_node_under_a_power_is_bracketed_in_every_renderer() {
    // `(df/dx)²` and `df/dx²` are different expressions, so the base of a
    // power is always bracketed.
    let squared = Math::Power {
        base: Box::new(derivative(
            DerivativeKind::Ordinary,
            sym('f'),
            &[(sym('x'), 1)],
        )),
        exp: Box::new(Math::Number("2".into())),
    };
    let [unicode, latex, omml] = render_all(&squared);
    assert_eq!(unicode, "(df/dx)²");
    assert_eq!(latex, "\\left(\\frac{d f}{d x}\\right)^{2}");
    // OMML delimits with its own structure: the fraction sits inside the
    // superscript base.
    assert!(omml.contains("<m:sSup><m:e><m:f>"), "{omml}");

    let squared = Math::Power {
        base: Box::new(Math::limit(
            sym('x'),
            Math::Number("0".into()),
            LimitDirection::TwoSided,
            sym('f'),
        )),
        exp: Box::new(Math::Number("2".into())),
    };
    let [unicode, latex, omml] = render_all(&squared);
    assert_eq!(unicode, "(lim_{x→0} f)²");
    assert_eq!(latex, "\\left(\\lim_{x \\to 0} f\\right)^{2}");
    assert!(omml.contains("<m:sSup><m:e><m:func>"), "{omml}");
}

#[test]
fn a_derivative_may_be_the_right_operand_of_an_operator() {
    // «вторая» is a prefix token, so it must still start a construct after
    // a plus, where the juxtaposition loop is not what opens the atom.
    match ast("икс плюс вторая производная эф по икс") {
        Math::Binary { op, left, right } => {
            assert_eq!(op, sciwhisper_core::ast::BinOp::Add);
            assert_eq!(*left, sym('x'));
            assert_eq!(
                *right,
                derivative(DerivativeKind::Ordinary, sym('f'), &[(sym('x'), 2)])
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        unicode("икс плюс вторая производная эф по икс"),
        "x + d²f/dx²"
    );
}

#[test]
fn a_structurally_invalid_derivative_renders_without_panicking() {
    // No variable and a zero order are impossible from dictation but
    // possible over serde. Every renderer must degrade, not panic, and must
    // not invent the missing part.
    for node in [
        Math::Derivative {
            kind: DerivativeKind::Ordinary,
            expr: Box::new(sym('f')),
            variables: vec![],
        },
        Math::Derivative {
            kind: DerivativeKind::Ordinary,
            expr: Box::new(sym('f')),
            variables: vec![DerivativeVariable::new(sym('x'), 0)],
        },
    ] {
        for rendered in render_all(&node) {
            assert!(!rendered.is_empty());
        }
    }
    let [unicode, latex, _] = render_all(&Math::Derivative {
        kind: DerivativeKind::Ordinary,
        expr: Box::new(sym('f')),
        variables: vec![],
    });
    // The total order of an empty variable list is 0, and that is what is
    // shown: a malformed node stays visibly malformed instead of being
    // silently normalised to a first derivative.
    assert_eq!(unicode, "d⁰f");
    assert_eq!(latex, "d^{0} f");
}

// -------------------------------------------------------------- end-to-end

#[test]
fn end_to_end_spoken_russian_to_three_formats() {
    let cases: [(Domain, &str, &str, &str); 8] = [
        (
            Domain::Mathematics,
            "производная эф по икс",
            "df/dx",
            "\\frac{d f}{d x}",
        ),
        (
            Domain::Mathematics,
            "вторая производная игрек по икс",
            "d²y/dx²",
            "\\frac{d^{2} y}{d x^{2}}",
        ),
        (
            Domain::Mathematics,
            "частная производная тэ большое по икс",
            "∂T/∂x",
            "\\frac{\\partial T}{\\partial x}",
        ),
        (
            Domain::Mathematics,
            "частная производная второго порядка тэ большое по икс и по игрек",
            "∂²T/(∂x∂y)",
            "\\frac{\\partial^{2} T}{\\partial x\\,\\partial y}",
        ),
        (
            Domain::Mathematics,
            "предел при икс стремящемся к нулю синуса икс делённого на икс",
            "lim_{x→0} sin(x)/x",
            "\\lim_{x \\to 0} \\frac{\\sin\\left(x\\right)}{x}",
        ),
        (
            Domain::Mathematics,
            "предел слева при икс стремящемся к нулю синус от икс",
            "lim_{x→0⁻} sin(x)",
            "\\lim_{x \\to 0^-} \\sin\\left(x\\right)",
        ),
        (
            Domain::Mathematics,
            "предел справа при икс стремящемся к двум эф",
            "lim_{x→2⁺} f",
            "\\lim_{x \\to 2^+} f",
        ),
        (
            Domain::Physics,
            "интеграл пять ньютонов по два метра",
            "∫ 5 Н d2 м",
            "\\int 5\\mathrm{Н}\\,d2\\mathrm{м}",
        ),
    ];
    for (domain, spoken, expected_unicode, expected_latex) in cases {
        let result = parsed(domain, spoken);
        assert_eq!(
            render_result(&result, Renderer::Unicode),
            expected_unicode,
            "unicode: {spoken}"
        );
        assert_eq!(
            render_result(&result, Renderer::Latex),
            expected_latex,
            "latex: {spoken}"
        );
        let xml = render_result(&result, Renderer::Omml);
        assert!(xml.starts_with("<m:oMath"), "omml: {spoken}: {xml}");
        assert!(!xml.contains("\\"), "omml must not carry LaTeX: {xml}");
    }
}

#[test]
fn the_dimensioned_integral_and_derivative_are_proven_end_to_end() {
    use sciwhisper_core::dimension::{infer, Inferred};

    // ∫ (5 Н) d(2 м) = M L² T⁻²: both parts are dictated with units.
    let math = match parsed(Domain::Physics, "интеграл пять ньютонов по два метра").ast
    {
        Node::Math(math) => math,
        other => panic!("{other:?}"),
    };
    match infer(&math) {
        Inferred::Known(dimension) => assert_eq!(dimension.to_string(), "M L^2 T^-2"),
        other => panic!("expected a proven dimension, got {other:?}"),
    }

    // d(10 м)/d(5 с) = L T⁻¹.
    let math = match parsed(Domain::Physics, "производная десять метров по пять секунд").ast
    {
        Node::Math(math) => math,
        other => panic!("{other:?}"),
    };
    match infer(&math) {
        Inferred::Known(dimension) => assert_eq!(dimension.to_string(), "L T^-1"),
        other => panic!("expected a proven dimension, got {other:?}"),
    }

    // A bare symbol stays unproven, and that is silent.
    let result = parsed(Domain::Mathematics, "производная эф по икс");
    assert!(result
        .warnings
        .iter()
        .all(|warning| !warning.code.starts_with("physics.")));
}

#[test]
fn a_dimensional_warning_leaves_the_dictated_calculus_payload_untouched() {
    let result = parsed(
        Domain::Physics,
        "производная открыть скобку три метра плюс четыре секунды закрыть скобку по икс",
    );
    assert_eq!(render_result(&result, Renderer::Unicode), "d(3 м + 4 с)/dx");
    assert!(result
        .warnings
        .iter()
        .any(|warning| warning.code == "physics.dimension_mismatch"));
}

// --------------------------------------------------- counterfactual pairs
// Exactly one element of meaning changes; exactly one field of the AST may
// change with it.

fn derivative_parts(text: &str) -> (DerivativeKind, Math, Vec<(Math, u32)>) {
    match ast(text) {
        Math::Derivative {
            kind,
            expr,
            variables,
        } => (
            kind,
            *expr,
            variables
                .into_iter()
                .map(|variable| (*variable.variable, variable.order))
                .collect(),
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn counterfactual_first_versus_second_changes_only_the_order() {
    let (first_kind, first_expr, first_variables) =
        derivative_parts("первая производная эф по икс");
    let (second_kind, second_expr, second_variables) =
        derivative_parts("вторая производная эф по икс");
    assert_eq!(first_kind, second_kind);
    assert_eq!(first_expr, second_expr);
    assert_eq!(first_variables[0].0, second_variables[0].0);
    assert_eq!(first_variables[0].1, 1);
    assert_eq!(second_variables[0].1, 2);
}

#[test]
fn counterfactual_x_versus_t_changes_only_the_variable() {
    let (kind_x, expr_x, variables_x) = derivative_parts("производная эф по икс");
    let (kind_t, expr_t, variables_t) = derivative_parts("производная эф по тэ");
    assert_eq!(kind_x, kind_t);
    assert_eq!(expr_x, expr_t);
    assert_eq!(variables_x[0].1, variables_t[0].1);
    assert_eq!(variables_x[0].0, sym('x'));
    assert_eq!(variables_t[0].0, sym('t'));
}

#[test]
fn counterfactual_ordinary_versus_partial_changes_only_the_kind() {
    let (ordinary, expr_a, variables_a) = derivative_parts("производная тэ по икс");
    let (partial, expr_b, variables_b) = derivative_parts("частная производная тэ по икс");
    assert_eq!(ordinary, DerivativeKind::Ordinary);
    assert_eq!(partial, DerivativeKind::Partial);
    assert_eq!(expr_a, expr_b);
    assert_eq!(variables_a, variables_b);
}

fn limit_parts(text: &str) -> (Math, Math, LimitDirection, Math) {
    match ast(text) {
        Math::Limit {
            variable,
            target,
            direction,
            body,
        } => (*variable, *target, direction, *body),
        other => panic!("{other:?}"),
    }
}

#[test]
fn counterfactual_zero_versus_infinity_changes_only_the_target() {
    let (variable_a, target_a, direction_a, body_a) =
        limit_parts("предел при икс стремящемся к нулю эф");
    let (variable_b, target_b, direction_b, body_b) =
        limit_parts("предел при икс стремящемся к бесконечности эф");
    assert_eq!(variable_a, variable_b);
    assert_eq!(direction_a, direction_b);
    assert_eq!(body_a, body_b);
    assert_eq!(target_a, Math::Number("0".into()));
    assert_eq!(target_b, Math::Infinity);
}

#[test]
fn counterfactual_left_versus_right_changes_only_the_direction() {
    let (variable_a, target_a, direction_a, body_a) =
        limit_parts("предел слева при икс стремящемся к нулю эф");
    let (variable_b, target_b, direction_b, body_b) =
        limit_parts("предел справа при икс стремящемся к нулю эф");
    assert_eq!(variable_a, variable_b);
    assert_eq!(target_a, target_b);
    assert_eq!(body_a, body_b);
    assert_eq!(direction_a, LimitDirection::FromLeft);
    assert_eq!(direction_b, LimitDirection::FromRight);
}

// ------------------------------------------------------------ negative/OOD

#[test]
fn an_incomplete_or_ordinary_sentence_stays_raw_text() {
    let refused = [
        // Structurally incomplete constructs.
        "производная по",
        "производная эф",
        "вторая производная",
        "предел при икс",
        "предел при икс стремящемся к нулю",
        "предел функции при тэ стремящемся к бесконечности",
        // An order that does not split unambiguously over the variables.
        "производная третьего порядка эф по икс и по игрек",
        // Ordinary Russian that merely contains the same nouns.
        "предел терпения",
        "производная была опубликована",
        "порядок величины",
        "предел терпения был исчерпан",
    ];
    for text in refused {
        for domain in [Domain::Auto, Domain::Mathematics] {
            let result = interpret(
                text,
                InterpretOptions {
                    domain,
                    allow_shortcuts: false,
                },
            );
            assert_eq!(
                result.confidence, 0.0,
                "'{text}' ({domain:?}) must not become a formula: {:?}",
                result.ast
            );
            assert_eq!(
                render_result(&result, Renderer::Unicode),
                text,
                "'{text}' must be preserved verbatim"
            );
            assert!(matches!(result.ast, Node::Text(_)), "'{text}'");
        }
    }
}

#[test]
fn a_conflicting_order_statement_is_refused() {
    for text in [
        "вторая производная третьего порядка эф по икс",
        "предел слева при икс стремящемся к нулю справа эф",
    ] {
        let result = interpret(
            text,
            InterpretOptions {
                domain: Domain::Mathematics,
                allow_shortcuts: false,
            },
        );
        assert_eq!(result.confidence, 0.0, "{text}");
    }
}

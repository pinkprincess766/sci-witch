//! MVP acceptance corpus from SciWhisper_TZ.md §16.

use sciwhisper_core::ast::{Chemical, Math, Node};
use sciwhisper_core::{interpret, render_result, Domain, InterpretOptions, Renderer};

fn fmt(domain: Domain, text: &str) -> String {
    let r = interpret(
        text,
        InterpretOptions {
            domain,
            allow_shortcuts: true,
        },
    );
    assert!(
        r.confidence > 0.0,
        "failed to parse '{text}': {:?}",
        r.warnings
    );
    render_result(&r, Renderer::Unicode)
}

fn latex(domain: Domain, text: &str) -> String {
    let r = interpret(
        text,
        InterpretOptions {
            domain,
            allow_shortcuts: true,
        },
    );
    assert!(r.confidence > 0.0, "failed to parse '{text}'");
    render_result(&r, Renderer::Latex)
}

fn ast(domain: Domain, text: &str) -> Node {
    interpret(
        text,
        InterpretOptions {
            domain,
            allow_shortcuts: true,
        },
    )
    .ast
}

#[test]
fn chem_copper_hydroxide() {
    assert_eq!(fmt(Domain::Chemistry, "гидроксид меди два"), "Cu(OH)₂");
}

#[test]
fn chem_observed_whisper_hydroxide_typos() {
    assert_eq!(fmt(Domain::Chemistry, "гидраксид железа 3"), "Fe(OH)₃");
    assert_eq!(fmt(Domain::Chemistry, "кидроксидж лезо три"), "Fe(OH)₃");
}

#[test]
fn chem_copper_ion() {
    assert_eq!(fmt(Domain::Chemistry, "ион меди два плюс"), "Cu²⁺");
}

#[test]
fn chem_sulfate_ion() {
    assert_eq!(fmt(Domain::Chemistry, "сульфат ион два минус"), "SO₄²⁻");
}

#[test]
fn chem_copper_vitriol() {
    assert_eq!(fmt(Domain::Chemistry, "медный купорос"), "CuSO₄·5H₂O");
}

#[test]
fn chem_zinc_ferrite_material_name() {
    assert_eq!(fmt(Domain::Chemistry, "феррит Zn"), "ZnFe₂O₄");
    assert_eq!(fmt(Domain::Chemistry, "два феррита Zn"), "2ZnFe₂O₄");
}

#[test]
fn chem_observed_asr_hydrofluoric_acid_alias() {
    assert_eq!(fmt(Domain::Chemistry, "кавликовая кислота"), "HF");
}

#[test]
fn chem_copper_hydroxide_decomp() {
    assert_eq!(
        fmt(
            Domain::Chemistry,
            "гидроксид меди два превращается в оксид меди два плюс вода"
        ),
        "Cu(OH)₂ → CuO + H₂O"
    );
}

#[test]
fn chem_plural_reaction_phrase() {
    assert_eq!(
        fmt(
            Domain::Chemistry,
            "два феррита Zn плюс калий марганец о четыре превращаются в оксид меди два плюс гидроксид железа три плюс гидроксид кобальта два плюс два гидроксид натрия"
        ),
        "2ZnFe₂O₄ + KMnO₄ → CuO + Fe(OH)₃ + Co(OH)₂ + 2NaOH"
    );
}

#[test]
fn chem_oxidation_phrase_is_a_forward_arrow() {
    assert_eq!(
        fmt(Domain::Chemistry, "уксусная кислота окисляется до аш два о"),
        "CH₃COOH → H₂O"
    );
}

#[test]
fn chem_zinc_hcl() {
    assert_eq!(
        fmt(
            Domain::Chemistry,
            "цинк плюс два аш хлор превращается в цинк хлор два плюс аш два газ"
        ),
        "Zn + 2HCl → ZnCl₂ + H₂↑"
    );
}

#[test]
fn chem_kmno4_heat() {
    assert_eq!(
        fmt(
            Domain::Chemistry,
            "два калий марганец о четыре при нагревании превращается в калий два марганец о четыре плюс марганец о два плюс о два"
        ),
        "2KMnO₄ → K₂MnO₄ + MnO₂ + O₂"
    );
}

#[test]
fn chem_literal_not_replaced() {
    let out = fmt(
        Domain::Chemistry,
        "метан плюс кислород превращается в углекислый газ плюс вода",
    );
    assert_eq!(out, "CH₄ + O₂ → CO₂ + H₂O");
}

#[test]
fn chem_unknown_shortcut_not_invented() {
    let r = interpret(
        "горение флогистона",
        InterpretOptions {
            domain: Domain::Chemistry,
            allow_shortcuts: true,
        },
    );
    assert!(r.confidence <= 0.0);
    assert_eq!(r.raw_transcript, "горение флогистона");
    assert!(!render_result(&r, Renderer::Unicode).contains("→"));
}

#[test]
fn chem_known_shortcut() {
    assert_eq!(
        fmt(Domain::Chemistry, "горение метана"),
        "CH₄ + 2O₂ → CO₂ + 2H₂O"
    );
}

#[test]
fn chem_latex_ce() {
    let t = latex(Domain::Chemistry, "гидроксид меди два");
    assert!(t.starts_with("\\ce{"), "{t}");
    assert!(t.contains("Cu(OH)2"), "{t}");
}

#[test]
fn chem_omml_native() {
    let r = interpret(
        "гидроксид меди два",
        InterpretOptions {
            domain: Domain::Chemistry,
            allow_shortcuts: true,
        },
    );
    let omml = render_result(&r, Renderer::Omml);
    assert!(omml.contains("oMath"), "{omml}");
    assert!(omml.contains("Cu"), "{omml}");
}

#[test]
fn math_quadratic() {
    assert_eq!(
        fmt(
            Domain::Mathematics,
            "икс в квадрате плюс два икс минус три равно нулю"
        ),
        "x² + 2x − 3 = 0"
    );
}

#[test]
fn math_greek_and_ordinal_power() {
    assert_eq!(
        fmt(
            Domain::Mathematics,
            "пси в квадрате умножить на x в кубе равно 10 в четвертой степени"
        ),
        "ψ²·x³ = 10⁴"
    );
}

#[test]
fn math_operator_alias_is_loaded_from_yaml() {
    assert_eq!(
        fmt(
            Domain::Mathematics,
            "10 в третьей степени умноженное на икс"
        ),
        "10³·x"
    );
}

#[test]
fn bare_delta_number_is_marked_ambiguous() {
    let result = interpret(
        "дельта три",
        InterpretOptions {
            domain: Domain::Mathematics,
            allow_shortcuts: true,
        },
    );
    assert_eq!(result.confidence, 0.7);
    assert!(result
        .warnings
        .iter()
        .any(|warning| warning.message.contains("ambiguous 'delta <number>'")));
}

#[test]
fn math_fraction_ast() {
    let node = ast(
        Domain::Mathematics,
        "начало дроби числитель два во второй степени умножить на эн знаменатель икс в кубе конец дроби",
    );
    match node {
        Node::Math(Math::Fraction { num, den }) => {
            let u = sciwhisper_core::render::unicode::render(&Node::Math(*num));
            let d = sciwhisper_core::render::unicode::render(&Node::Math(*den));
            assert!(u.contains('2'), "{u}");
            assert!(d.contains('x'), "{d}");
        }
        other => panic!("expected fraction AST, got {other:?}"),
    }
}

#[test]
fn math_root_binds_atom() {
    assert_eq!(
        fmt(Domain::Mathematics, "корень из икс плюс один"),
        "√x + 1"
    );
}

#[test]
fn math_root_comma_same() {
    assert_eq!(
        fmt(Domain::Mathematics, "корень из икс, плюс один"),
        "√x + 1"
    );
}

#[test]
fn math_root_bounded() {
    assert_eq!(
        fmt(
            Domain::Mathematics,
            "начало корня икс плюс один конец корня"
        ),
        "√(x + 1)"
    );
}

#[test]
fn math_sum() {
    let t = latex(
        Domain::Mathematics,
        "сумма от ка равно нулю до бесконечности",
    );
    assert_eq!(t, "\\sum_{k=0}^{\\infty}");
}

#[test]
fn math_integral() {
    let spoken = "интеграл от нуля до единицы икс в квадрате по икс";
    let t = latex(Domain::Mathematics, spoken);
    assert_eq!(t, "\\int_{0}^{1} x^{2}\\,dx");
    assert_eq!(fmt(Domain::Mathematics, spoken), "∫₀¹ x² dx");
}

#[test]
fn math_integral_accepts_spoken_differential_and_observed_asr_by() {
    assert_eq!(fmt(Domain::Mathematics, "интеграл эф дэ икс"), "∫ f dx");
    assert_eq!(fmt(Domain::Mathematics, "интеграл от эф дэ икс"), "∫ f dx");
    assert_eq!(fmt(Domain::Mathematics, "интеграл эф под икс"), "∫ f dx");
    assert_eq!(
        fmt(
            Domain::Mathematics,
            "интеграл от нуля до единицы экспонента от икс дэ икс"
        ),
        "∫₀¹ exp(x) dx"
    );
}

#[test]
fn math_standard_functions_and_observed_zeta_alias() {
    let spoken = "сета умноженное на 3x равно 10 в третьей степени плюс экспонента от икс деленное на x в квадрате";
    assert_eq!(fmt(Domain::Mathematics, spoken), "ζ·3x = 10³ + exp(x)/x²");
    assert_eq!(
        latex(Domain::Mathematics, spoken),
        "\\zeta \\cdot 3x = 10^{3} + \\frac{\\exp\\left(x\\right)}{x^{2}}"
    );
    assert_eq!(fmt(Domain::Mathematics, "синус от икс"), "sin(x)");
    assert_eq!(
        fmt(Domain::Mathematics, "натуральный логарифм от икс"),
        "ln(x)"
    );
}

#[test]
fn latex_renders_complete_greek_commands() {
    assert_eq!(
        latex(
            Domain::Mathematics,
            "зета плюс эта плюс кси плюс тау плюс ипсилон плюс хи плюс пси"
        ),
        "\\zeta + \\eta + \\xi + \\tau + \\upsilon + \\chi + \\psi"
    );
}

#[test]
fn natural_division_is_a_native_word_fraction() {
    let result = interpret(
        "экспонента от икс деленное на x в квадрате",
        InterpretOptions {
            domain: Domain::Mathematics,
            allow_shortcuts: false,
        },
    );
    assert!(result.confidence > 0.0);
    let omml = render_result(&result, Renderer::Omml);
    assert!(omml.contains("<m:f>"), "{omml}");
    assert_eq!(
        render_result(&result, Renderer::Latex),
        "\\frac{\\exp\\left(x\\right)}{x^{2}}"
    );
}

#[test]
fn math_precedence_and_incomplete_construct_safety() {
    assert_eq!(
        fmt(Domain::Mathematics, "два плюс три умножить на четыре"),
        "2 + 3·4"
    );
    assert_eq!(
        fmt(
            Domain::Mathematics,
            "открыть скобку два плюс три закрыть скобку умножить на четыре"
        ),
        "(2 + 3)·4"
    );

    for incomplete in ["экспонента", "икс деленное на", "интеграл"] {
        let result = interpret(
            incomplete,
            InterpretOptions {
                domain: Domain::Mathematics,
                allow_shortcuts: false,
            },
        );
        assert!(
            result.confidence < 0.9,
            "incomplete input must not be auto-inserted: {incomplete}"
        );
    }
}

#[test]
fn math_factorial() {
    assert_eq!(fmt(Domain::Mathematics, "факториал четырёх ка"), "(4k)!");
}

#[test]
fn math_ramanujan_primitives() {
    // Exact grammar assembling 1/π = … enough primitives for the TZ check.
    let t = latex(
        Domain::Mathematics,
        "начало дроби числитель один знаменатель пи конец дроби равно сумма от ка равно нулю до бесконечности",
    );
    assert!(t.contains("\\frac{1}{\\pi}"), "{t}");
    assert!(t.contains("\\sum_{k=0}^{\\infty}"), "{t}");
}

#[test]
fn phys_delta_g() {
    assert_eq!(
        fmt(Domain::Physics, "дельта же равно минус эн эф е"),
        "ΔG = −nFE"
    );
}

#[test]
fn phys_lambda() {
    assert_eq!(
        fmt(
            Domain::Physics,
            "лямбда равно шестьсот тридцать два нанометра"
        ),
        "λ = 632 нм"
    );
}

#[test]
fn phys_nu() {
    assert_eq!(
        fmt(Domain::Physics, "ню греческая равно ц делённое на лямбда"),
        "ν = c/λ"
    );
}

#[test]
fn phys_vector() {
    assert_eq!(
        fmt(Domain::Physics, "вектор эф равен эм умножить на вектор а"),
        "F⃗ = m·a⃗"
    );
}

#[test]
fn phys_g_units() {
    assert_eq!(
        fmt(
            Domain::Physics,
            "девять целых восемьдесят одна метра на секунду в квадрате"
        ),
        "9,81 м/с²"
    );
}

#[test]
fn phys_letter_commands() {
    assert_eq!(fmt(Domain::Mathematics, "эн большое"), "N");
    assert_eq!(fmt(Domain::Mathematics, "эн малое"), "n");
    assert_eq!(fmt(Domain::Mathematics, "ню греческая"), "ν");
    assert_eq!(fmt(Domain::Mathematics, "эн русская большое"), "Н");
}

#[test]
fn whisper_artifacts_chemistry_comma() {
    assert_eq!(
        fmt(
            Domain::Chemistry,
            "гидроксид меди два превращается в, оксид меди два плюс вода."
        ),
        "Cu(OH)₂ → CuO + H₂O"
    );
}

#[test]
fn whisper_artifacts_glued_math() {
    assert_eq!(
        fmt(Domain::Mathematics, "Икс в квадрате плюс 2x-3 равно нулю."),
        "x² + 2x − 3 = 0"
    );
}

#[test]
fn negative_unclosed_fraction_preserves_raw() {
    let r = interpret(
        "начало дроби числитель два",
        InterpretOptions {
            domain: Domain::Mathematics,
            allow_shortcuts: false,
        },
    );
    assert!(r.confidence <= 0.0);
    assert_eq!(r.raw_transcript, "начало дроби числитель два");
}

#[test]
fn negative_unknown_symbol() {
    let r = interpret(
        "кварк плюс глюон",
        InterpretOptions {
            domain: Domain::Chemistry,
            allow_shortcuts: false,
        },
    );
    assert!(r.confidence <= 0.0);
    assert!(!render_result(&r, Renderer::Unicode).contains("→"));
}

#[test]
fn same_ast_three_renderers() {
    let r = interpret(
        "гидроксид меди два",
        InterpretOptions {
            domain: Domain::Chemistry,
            allow_shortcuts: true,
        },
    );
    match r.ast {
        Node::Chemical(Chemical::Species(ref s)) => {
            assert_eq!(s.formula.parts.len(), 2);
        }
        other => panic!("{other:?}"),
    }
    assert!(render_result(&r, Renderer::Unicode).contains("Cu"));
    assert!(render_result(&r, Renderer::Latex).contains("\\ce{"));
    assert!(render_result(&r, Renderer::Omml).contains("oMath"));
}

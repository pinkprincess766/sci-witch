//! Acceptance corpus for natural Russian dictation.
//!
//! Every expected value here was written by hand from what the sentence
//! means. Nothing in this file was produced by running the parser and pasting
//! its output: a corpus generated that way would only prove that the parser
//! agrees with itself.
//!
//! This is a **text** corpus. There is no audio behind any line, so it says
//! nothing about recognition accuracy — only about what happens once Whisper
//! has produced a transcript.

use sciwhisper_core::{
    interpret_utterance, render, Domain, Renderer, UtteranceMode, UtteranceOptions,
};

const S: UtteranceMode = UtteranceMode::ScientificOnly;
const M: UtteranceMode = UtteranceMode::MixedText;

struct Case {
    spoken: &'static str,
    mode: UtteranceMode,
    expect: &'static str,
}

const fn case(spoken: &'static str, mode: UtteranceMode, expect: &'static str) -> Case {
    Case {
        spoken,
        mode,
        expect,
    }
}

fn unicode(spoken: &str, mode: UtteranceMode) -> String {
    let result = interpret_utterance(
        spoken,
        UtteranceOptions {
            domain: Domain::Auto,
            mode,
            allow_shortcuts: true,
        },
    );
    render(&result.document, Renderer::Unicode)
}

/// Runs a whole block and reports every failure at once, so a corpus run says
/// what is broken rather than only what broke first.
fn run(name: &str, cases: &[Case]) {
    let mut failures = Vec::new();
    for case in cases {
        let actual = unicode(case.spoken, case.mode);
        if actual != case.expect {
            failures.push(format!(
                "  [{}] {:?}\n      expected {:?}\n      actual   {:?}",
                case.mode.as_str(),
                case.spoken,
                case.expect,
                actual
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{name}: {} of {} cases failed\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

// ------------------------------------------------------------------ chemistry

const CHEMISTRY: [Case; 40] = [
    case("ну запиши перманганат калия", S, "KMnO₄"),
    case("запиши пожалуйста серную кислоту", S, "H₂SO₄"),
    case("давайте запишем гидроксид натрия", S, "NaOH"),
    case("введи хлорид алюминия", S, "AlCl₃"),
    case("напиши сульфат калия", S, "K₂SO₄"),
    case("например оксид железа три", S, "Fe₂O₃"),
    case("пусть будет углекислый газ", S, "CO₂"),
    case("запиши воду", S, "H₂O"),
    case("запиши уксусную кислоту", S, "CH₃COOH"),
    case("аммиак", S, "NH₃"),
    case("поваренная соль", S, "NaCl"),
    case("азотная кислота", S, "HNO₃"),
    case("перманганат калия", S, "KMnO₄"),
    case("пермангнат калия", S, "KMnO₄"),
    case("феррит цинка", S, "ZnFe₂O₄"),
    case("оксид алюминия", S, "Al₂O₃"),
    case("оксид серы шесть", S, "SO₃"),
    case("гидроксид кальция", S, "Ca(OH)₂"),
    case("медный купорос", S, "CuSO₄·5H₂O"),
    case("калий марганец о четыре", S, "KMnO₄"),
    case("два аш два о", S, "2H₂O"),
    case("гидроксид меди два осадок", S, "Cu(OH)₂↓"),
    case("ион меди два плюс", S, "Cu²⁺"),
    case("сульфат ион два минус", S, "SO₄²⁻"),
    // self-corrections
    case("гидроксид железа два, нет, железа три", S, "Fe(OH)₃"),
    case(
        "Так, запиши, пожалуйста, гидроксид железа два, нет, железа три",
        S,
        "Fe(OH)₃",
    ),
    case("оксид меди два, точнее оксид меди один", S, "Cu₂O"),
    case("гидроксид железа не два, а три", S, "Fe(OH)₃"),
    case("гидроксид железа не три, а два", S, "Fe(OH)₂"),
    case("хлорид натрия, нет, хлорид калия", S, "KCl"),
    case("серная кислота, точнее азотная кислота", S, "HNO₃"),
    // reactions in several natural shapes
    case(
        "гидроксид меди два превращается в оксид меди два плюс вода",
        S,
        "Cu(OH)₂ → CuO + H₂O",
    ),
    case(
        "гидроксид меди два при нагревании превращается в оксид меди два плюс вода",
        S,
        "Cu(OH)₂ → CuO + H₂O",
    ),
    case(
        "перекись водорода разлагается на воду и кислород",
        S,
        "H₂O₂ → H₂O + O₂",
    ),
    case(
        "гидроксид меди два реагирует с серной кислотой в результате образуется сульфат меди два и вода",
        S,
        "Cu(OH)₂ + H₂SO₄ → CuSO₄ + H₂O",
    ),
    case("из воды получается водород и кислород", S, "H₂O → H₂ + O₂"),
    case(
        "между водородом и кислородом протекает реакция с образованием воды",
        S,
        "H₂ + O₂ → H₂O",
    ),
    case(
        "метан взаимодействует с кислородом с образованием углекислого газа и воды",
        S,
        "CH₄ + O₂ → CO₂ + H₂O",
    ),
    case("уксусная кислота окисляется до аш два о", S, "CH₃COOH → H₂O"),
    case(
        "два калий марганец о четыре при нагревании превращается в калий два марганец о четыре плюс марганец о два плюс о два",
        S,
        "2KMnO₄ → K₂MnO₄ + MnO₂ + O₂",
    ),
];

#[test]
fn chemistry_corpus() {
    run("chemistry", &CHEMISTRY);
}

// ---------------------------------------------------------------- mathematics

const MATHEMATICS: [Case; 30] = [
    case(
        "запиши икс в квадрате плюс два икс минус три равно нулю",
        S,
        "x² + 2x − 3 = 0",
    ),
    case("ну давайте запишем икс в кубе", S, "x³"),
    case("икс возвести в квадрат плюс два икс", S, "x² + 2x"),
    case("икс делённое на игрек", S, "x/y"),
    case("икс поделить на игрек", S, "x/y"),
    case("два плюс три умножить на четыре", S, "2 + 3·4"),
    case(
        "открыть скобку два плюс три закрыть скобку умножить на четыре",
        S,
        "(2 + 3)·4",
    ),
    case("икс индекс один", S, "x₁"),
    case("икс больше или равно нулю", S, "x ≥ 0"),
    case("пи", S, "π"),
    case("модуль икс", S, "|x|"),
    case("факториал четырёх ка", S, "(4k)!"),
    case("синус от икс", S, "sin(x)"),
    case("натуральный логарифм от икс", S, "ln(x)"),
    case("экспонента от икс", S, "exp(x)"),
    case("корень из икс плюс один", S, "√x + 1"),
    case("начало корня икс плюс один конец корня", S, "√(x + 1)"),
    case("сумма от ка равно нулю до бесконечности", S, "∑_{k=0}^{∞}"),
    case(
        "интеграл от нуля до единицы икс в квадрате по икс",
        S,
        "∫₀¹ x² dx",
    ),
    case(
        "запиши дробь: в числителе два во второй степени умножить на эн, в знаменателе икс в кубе",
        S,
        "(2²·n)/(x³)",
    ),
    // calculus in natural word order
    case("вторая производная эф по икс", S, "d²f/dx²"),
    case("производная третьего порядка игрек по тэ", S, "d³y/dt³"),
    case("частная производная тэ большое по икс", S, "∂T/∂x"),
    case(
        "частная производная второго порядка тэ большое по икс и по игрек",
        S,
        "∂²T/(∂x∂y)",
    ),
    case(
        "предел при икс, стремящемся к нулю, синуса икс поделить на икс",
        S,
        "lim_{x→0} sin(x)/x",
    ),
    case(
        "предел слева при икс стремящемся к нулю эф",
        S,
        "lim_{x→0⁻} f",
    ),
    case(
        "предел справа при икс стремящемся к двум эф",
        S,
        "lim_{x→2⁺} f",
    ),
    // self-corrections
    case("икс в квадрате, нет, в кубе", S, "x³"),
    case("икс во второй степени, нет, в третьей", S, "x³"),
    case(
        "производная эф по икс, точнее вторая производная эф по икс",
        S,
        "d²f/dx²",
    ),
];

#[test]
fn mathematics_corpus() {
    run("mathematics", &MATHEMATICS);
}

// -------------------------------------------------------------------- physics

const PHYSICS: [Case; 20] = [
    case("вэ равно три метра в секунду", S, "v = 3 м/с"),
    case("запиши три метра", S, "3 м"),
    case("четыре сантиметра", S, "4 см"),
    case("пять ньютонов", S, "5 Н"),
    case("десять джоулей", S, "10 Дж"),
    case("сто ватт", S, "100 Вт"),
    case("запиши пять вольт", S, "5 В"),
    case("два моля", S, "2 моль"),
    case(
        "лямбда равно шестьсот тридцать два нанометра",
        S,
        "λ = 632 нм",
    ),
    case(
        "девять целых восемьдесят одна метра на секунду в квадрате",
        S,
        "9,81 м/с²",
    ),
    case("дельта же равно минус эн эф е", S, "ΔG = −nFE"),
    case("вектор эф равен эм умножить на вектор а", S, "F⃗ = m·a⃗"),
    case("три метра плюс четыре сантиметра", S, "3 м + 4 см"),
    case("три метра плюс четыре секунды", S, "3 м + 4 с"),
    case("пять вольт плюс три ампера", S, "5 В + 3 А"),
    case("один метр в минус первой", S, "(1 м)^{-1}"),
    case("синус трёх метров", S, "sin(3 м)"),
    case("два в степени три метра", S, "2^{3 м}"),
    case(
        "производная десять метров по пять секунд",
        S,
        "d(10 м)/d(5 с)",
    ),
    case("интеграл пять ньютонов по два метра", S, "∫ 5 Н d2 м"),
];

#[test]
fn physics_corpus() {
    run("physics", &PHYSICS);
}

// ------------------------------------------------------------- mixed and OOD

/// Ordinary Russian that happens to contain a scientific word, plus mixed
/// sentences where exactly one span is proven. Not one of these may become a
/// formula it does not contain.
const MIXED_AND_OOD: [Case; 30] = [
    // a scientific word is not permission to rewrite the sentence
    case("Реакция идёт быстрее при нагревании.", M, "Реакция идёт быстрее при нагревании."),
    case("Предел терпения закончился.", M, "Предел терпения закончился."),
    case("Нужно найти корень проблемы.", M, "Нужно найти корень проблемы."),
    case("Сумма заказа изменилась.", M, "Сумма заказа изменилась."),
    case("Он исправил два на три в отчёте.", M, "Он исправил два на три в отчёте."),
    case("Мы получили хороший результат.", M, "Мы получили хороший результат."),
    case("Медь оказалась слишком дорогой.", M, "Медь оказалась слишком дорогой."),
    case("Сегодня вода холодная.", M, "Сегодня вода холодная."),
    case("Это обычная фраза без научной формулы.", M, "Это обычная фраза без научной формулы."),
    case("Добавим спирт и соду.", M, "Добавим спирт и соду."),
    case("Он изучает интеграл в университете.", M, "Он изучает интеграл в университете."),
    case("Степень доверия выросла.", M, "Степень доверия выросла."),
    case("Функция организма нарушена.", M, "Функция организма нарушена."),
    case("Модуль памяти сгорел.", M, "Модуль памяти сгорел."),
    case("Заряд аккумулятора упал.", M, "Заряд аккумулятора упал."),
    case("Мы обсудили реакцию коллектива.", M, "Мы обсудили реакцию коллектива."),
    case("Аммиак имеет резкий запах.", M, "Аммиак имеет резкий запах."),
    case("производная была опубликована", M, "производная была опубликована"),
    // unfinished constructs are kept, never completed
    case("порядок величины", M, "порядок величины"),
    case("предел при икс", M, "предел при икс"),
    case("производная по", M, "производная по"),
    case("вторая производная", M, "вторая производная"),
    case("интеграл", M, "интеграл"),
    case("экспонента", M, "экспонента"),
    case("гидроксид", M, "гидроксид"),
    // a correction marker with nothing to correct
    case("не два а три", M, "не два а три"),
    // and the mixed sentences that do carry one proven span
    case(
        "Сегодня рассмотрим перманганат калия, а затем продолжим опыт.",
        M,
        "Сегодня рассмотрим KMnO₄, а затем продолжим опыт.",
    ),
    case("Например, калий марганец о четыре.", M, "Например, KMnO₄."),
    case(
        "Попытка вставки: на примере гидроксида железа три, оксида меди два или перманганата калия.",
        M,
        "Попытка вставки: на примере Fe(OH)₃, CuO или KMnO₄.",
    ),
    case(
        "Примеры: павликова кислота, уксусная кислота, ацетон и глицерин.",
        M,
        "Примеры: HF, CH₃COOH, CH₃COCH₃ и C₃H₈O₃.",
    ),
];

#[test]
fn mixed_and_out_of_domain_corpus() {
    run("mixed/OOD", &MIXED_AND_OOD);
}

#[test]
fn the_corpus_has_the_promised_shape() {
    assert_eq!(CHEMISTRY.len(), 40);
    assert_eq!(MATHEMATICS.len(), 30);
    assert_eq!(PHYSICS.len(), 20);
    assert_eq!(MIXED_AND_OOD.len(), 30);
    assert_eq!(
        CHEMISTRY.len() + MATHEMATICS.len() + PHYSICS.len() + MIXED_AND_OOD.len(),
        120
    );
}

#[test]
fn no_ordinary_sentence_is_rewritten_in_either_mode() {
    // The protection must not depend on the mode: ScientificOnly may drop a
    // recognised shell, never a sentence it failed to understand.
    for case in MIXED_AND_OOD.iter().filter(|c| c.spoken == c.expect) {
        for mode in [M, S] {
            assert_eq!(
                unicode(case.spoken, mode),
                case.expect,
                "{:?} in {}",
                case.spoken,
                mode.as_str()
            );
        }
    }
}

// -------------------------------------------------- one structure, three views

fn all_formats(spoken: &str, mode: UtteranceMode) -> (String, String, String) {
    let result = interpret_utterance(
        spoken,
        UtteranceOptions {
            domain: Domain::Auto,
            mode,
            allow_shortcuts: true,
        },
    );
    (
        render(&result.document, Renderer::Unicode),
        render(&result.document, Renderer::Latex),
        render(&result.document, Renderer::Omml),
    )
}

#[test]
fn the_headline_examples_render_in_every_format_from_one_structure() {
    let (unicode, latex, omml) = all_formats("ну запиши перманганат калия", S);
    assert_eq!(unicode, "KMnO₄");
    assert_eq!(latex, "\\ce{KMnO4}");
    // A purely scientific utterance keeps real OMML markup, not a string of
    // LaTeX pasted into a run.
    assert!(omml.starts_with("<m:oMath"), "{omml}");
    assert!(omml.contains("<m:sSub>"), "{omml}");
    assert!(!omml.contains("\\ce"), "{omml}");

    let (unicode, latex, omml) = all_formats("гидроксид железа два, нет, железа три", S);
    assert_eq!(unicode, "Fe(OH)₃");
    assert_eq!(latex, "\\ce{Fe(OH)3}");
    assert!(omml.contains("<m:sSub>"), "{omml}");

    let (unicode, latex, omml) = all_formats("перекись водорода разлагается на воду и кислород", S);
    assert_eq!(unicode, "H₂O₂ → H₂O + O₂");
    assert_eq!(latex, "\\ce{H2O2 -> H2O + O2}");
    assert!(omml.contains("→"), "{omml}");

    let (unicode, latex, omml) = all_formats("икс в квадрате, нет, в кубе", S);
    assert_eq!(unicode, "x³");
    assert_eq!(latex, "x^{3}");
    assert!(omml.contains("<m:sSup>"), "{omml}");
}

#[test]
fn a_mixed_sentence_renders_as_prose_plus_structure_in_every_format() {
    let (unicode, latex, omml) = all_formats(
        "Сегодня рассмотрим перманганат калия, а затем продолжим опыт.",
        M,
    );
    assert_eq!(unicode, "Сегодня рассмотрим KMnO₄, а затем продолжим опыт.");
    assert_eq!(
        latex,
        "Сегодня рассмотрим \\ce{KMnO4}, а затем продолжим опыт."
    );
    // The prose survives in OMML too; the Word layer is what falls back to a
    // plain string, and that decision lives in the pipeline, not here.
    assert!(omml.contains("Сегодня рассмотрим"), "{omml}");
    assert!(omml.contains("<m:sSub>"), "{omml}");
}

#[test]
fn an_unbalanced_dictated_reaction_is_kept_and_only_warned_about() {
    let result = interpret_utterance(
        "перекись водорода разлагается на воду и кислород",
        UtteranceOptions {
            domain: Domain::Auto,
            mode: S,
            allow_shortcuts: true,
        },
    );
    // The dictated coefficients are never silently replaced.
    assert_eq!(
        render(&result.document, Renderer::Unicode),
        "H₂O₂ → H₂O + O₂"
    );
    let codes: Vec<&str> = result
        .warnings
        .iter()
        .map(|warning| warning.code.as_str())
        .collect();
    assert!(
        codes.contains(&"chemistry.unbalanced_atoms"),
        "the imbalance must be reported: {codes:?}"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.code == "chemistry.balance_suggestion"
                && warning.message.contains("2H₂O₂ → 2H₂O + O₂")),
        "a suggestion is offered, not applied: {:?}",
        result.warnings
    );
}

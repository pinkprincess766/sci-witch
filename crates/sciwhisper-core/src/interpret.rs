use crate::ast::{Domain, InterpretationResult, Node, Renderer, UnresolvedSpan, Warning};
use crate::lexicon::Lexicon;
use crate::normalize::{normalize, words as split_words};
use crate::numbers::NumberLex;
use crate::parser::{math, parse_domain};
use crate::render;

#[derive(Clone, Debug)]
pub struct InterpretOptions {
    pub domain: Domain,
    pub allow_shortcuts: bool,
}

impl Default for InterpretOptions {
    fn default() -> Self {
        Self {
            domain: Domain::Auto,
            allow_shortcuts: true,
        }
    }
}

pub fn interpret(text: &str, opts: InterpretOptions) -> InterpretationResult {
    let lex = Lexicon::builtin();
    let nums = NumberLex::new();
    let normalized = normalize(text);
    let words = split_words(text);

    if words.is_empty() {
        return InterpretationResult::failed_raw(text, &normalized, opts.domain, "empty input");
    }

    if opts.allow_shortcuts {
        if let Some(sc) = lex.shortcut_exact(&normalized) {
            return InterpretationResult {
                ast: Node::Chemical(crate::ast::Chemical::Equation(sc.equation.clone())),
                raw_transcript: text.to_string(),
                normalized_transcript: normalized,
                domain: Domain::Chemistry,
                confidence: 1.0,
                unresolved_spans: vec![],
                warnings: vec![],
                alternatives: vec![],
            };
        }
    }

    let resolved = match opts.domain {
        Domain::Auto => detect_domain(&words, lex),
        other => other,
    };

    match parse_domain(&words, resolved, lex, &nums) {
        Ok(ast) => {
            let mut warnings = Vec::new();
            let mut alternatives = Vec::new();
            if resolved == Domain::Mathematics || resolved == Domain::Physics {
                if let Ok(p) = math::parse_math(
                    &words,
                    lex,
                    &nums,
                    if resolved == Domain::Physics {
                        math::MathMode::Physics
                    } else {
                        math::MathMode::Math
                    },
                ) {
                    warnings.extend(p.warnings.into_iter().map(|message| Warning {
                        code: "math".into(),
                        message,
                    }));
                    alternatives.extend(p.alternatives.into_iter().map(Node::Math));
                }
            }
            let structural_confidence = if warnings.is_empty() { 0.95 } else { 0.7 };
            warnings.extend(crate::validate::semantic_warnings(&ast));
            InterpretationResult {
                ast,
                raw_transcript: text.to_string(),
                normalized_transcript: normalized,
                domain: resolved,
                confidence: structural_confidence,
                unresolved_spans: vec![],
                warnings,
                alternatives,
            }
        }
        Err(e) => InterpretationResult {
            ast: Node::Text(text.to_string()),
            raw_transcript: text.to_string(),
            normalized_transcript: normalized,
            domain: resolved,
            confidence: 0.0,
            unresolved_spans: vec![UnresolvedSpan {
                text: text.to_string(),
                reason: e.to_string(),
            }],
            warnings: vec![Warning {
                code: "unresolved".into(),
                message: e.to_string(),
            }],
            alternatives: vec![],
        },
    }
}

pub fn format_text(text: &str, domain: Domain, renderer: Renderer) -> InterpretationResult {
    let r = interpret(
        text,
        InterpretOptions {
            domain,
            allow_shortcuts: true,
        },
    );
    let _ = renderer;
    r
}

pub fn render_result(r: &InterpretationResult, renderer: Renderer) -> String {
    if r.confidence <= 0.0 {
        return r.raw_transcript.clone();
    }
    render::render(&r.ast, renderer)
}

fn detect_domain(words: &[String], lex: &Lexicon) -> Domain {
    let mut chem = 0;
    let mut math = 0;
    let mut phys = 0;
    for (i, w) in words.iter().enumerate() {
        if matches!(
            w.as_str(),
            "превращается"
                | "превращаются"
                | "окисляется"
                | "окисляются"
                | "восстанавливается"
                | "восстанавливаются"
                | "ион"
                | "кислота"
                | "оксид"
                | "гидроксид"
                | "хлорид"
                | "сульфат"
                | "газ"
                | "осадок"
                | "стрелка"
        ) {
            chem += 3;
        }
        if lex.element(w).is_some() {
            chem += 1;
        }
        if lex.anion(w).is_some() {
            chem += 2;
        }
        if matches!(
            w.as_str(),
            "дробь"
                | "числитель"
                | "знаменатель"
                | "интеграл"
                | "сумма"
                | "факториал"
                | "корень"
                | "синус"
                | "косинус"
                | "тангенс"
                | "котангенс"
                | "логарифм"
                | "экспонента"
                | "степени"
                | "квадрате"
                | "скобку"
        ) {
            math += 3;
        }
        if matches!(w.as_str(), "вектор" | "метра" | "нанометра" | "ньютон")
        {
            phys += 3;
        }
        if w == "дельта" {
            phys += 1;
            chem += 1;
        }
        if lex.longest_unit(words, i).is_some() {
            phys += 2;
        }
    }
    if chem >= math && chem >= phys && chem > 0 {
        Domain::Chemistry
    } else if phys > math {
        Domain::Physics
    } else if math > 0 {
        Domain::Mathematics
    } else if chem > 0 {
        Domain::Chemistry
    } else {
        Domain::Mathematics
    }
}

/// Chemistry-only entry used by tests that must not fall back to math.
pub fn interpret_chemistry(text: &str) -> InterpretationResult {
    interpret(
        text,
        InterpretOptions {
            domain: Domain::Chemistry,
            allow_shortcuts: true,
        },
    )
}

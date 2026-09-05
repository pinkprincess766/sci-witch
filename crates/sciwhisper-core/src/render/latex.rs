use crate::ast::{
    derivative_total_order, Arrow, BinOp, Chemical, DerivativeKind, DerivativeVariable, Equation,
    Formula, GroupKind, LimitDirection, Math, Node, Part, Species, StateMarker, UnitExpr,
};

pub fn render(node: &Node) -> String {
    match node {
        Node::Document(xs) => xs.iter().map(render).collect::<Vec<_>>().join(""),
        Node::Text(s) => s.clone(),
        Node::Chemical(c) => format!("\\ce{{{}}}", chemical(c)),
        Node::Math(m) => math(m),
    }
}

fn chemical(c: &Chemical) -> String {
    match c {
        Chemical::Species(s) => species(s),
        Chemical::Equation(eq) => equation(eq),
    }
}

fn equation(eq: &Equation) -> String {
    let left = eq.left.iter().map(species).collect::<Vec<_>>().join(" + ");
    let right = eq.right.iter().map(species).collect::<Vec<_>>().join(" + ");
    let arrow = match eq.arrow {
        Arrow::Forward => "->",
        Arrow::Equilibrium => "<=>",
    };
    format!("{left} {arrow} {right}")
}

fn species(s: &Species) -> String {
    let mut out = String::new();
    if s.coefficient != 1 {
        out.push_str(&s.coefficient.to_string());
    }
    out.push_str(&formula(&s.formula));
    if let Some(ch) = s.charge {
        out.push_str(&charge(ch));
    }
    match s.marker {
        Some(StateMarker::Gas) => out.push_str(" ^"),
        Some(StateMarker::Precipitate) => out.push_str(" v"),
        None => {}
    }
    out
}

fn formula(f: &Formula) -> String {
    let mut out = String::new();
    for p in &f.parts {
        match p {
            Part::Atom { symbol, count } => {
                out.push_str(symbol);
                if *count != 1 {
                    out.push_str(&count.to_string());
                }
            }
            Part::Group { inner, count } => {
                out.push('(');
                out.push_str(&formula(inner));
                out.push(')');
                if *count != 1 {
                    out.push_str(&count.to_string());
                }
            }
            Part::Hydrate { count } => {
                out.push('.');
                if *count != 1 {
                    out.push_str(&count.to_string());
                }
                out.push_str("H2O");
            }
        }
    }
    out
}

fn charge(ch: i32) -> String {
    let mag = ch.unsigned_abs();
    let sign = if ch > 0 { '+' } else { '-' };
    if mag == 1 {
        format!("^{sign}")
    } else {
        format!("^{{{mag}{sign}}}")
    }
}

fn math(m: &Math) -> String {
    match m {
        Math::Number(n) => n.replace(',', "{,}"),
        Math::Symbol(s) => greek_or_letter(&s.letter),
        Math::Delta(inner) => format!("\\Delta {}", math(inner)),
        Math::Vector(inner) => format!("\\vec{{{}}}", math(inner)),
        Math::UnaryMinus(inner) => format!("-{}", math_tight(inner)),
        Math::Binary { op, left, right } => {
            if *op == BinOp::Div {
                return format!("\\frac{{{}}}{{{}}}", math(left), math(right));
            }
            let op_s = match op {
                BinOp::Add => " + ",
                BinOp::Sub => " - ",
                BinOp::Mul => " \\cdot ",
                BinOp::Div => "/",
                BinOp::Eq => " = ",
                BinOp::Ne => " \\neq ",
                BinOp::Lt => " < ",
                BinOp::Gt => " > ",
                BinOp::Le => " \\leq ",
                BinOp::Ge => " \\geq ",
                BinOp::PlusMinus => " \\pm ",
            };
            format!("{}{}{}", math(left), op_s, math(right))
        }
        Math::Juxt(xs) => xs.iter().map(math_tight).collect::<Vec<_>>().join(""),
        Math::Fraction { num, den } => format!("\\frac{{{}}}{{{}}}", math(num), math(den)),
        Math::Power { base, exp } => {
            format!("{}^{{{}}}", math_maybe_group(base), math(exp))
        }
        Math::Subscript { base, sub } => {
            format!("{}_{{{}}}", math_maybe_group(base), math(sub))
        }
        Math::Root { index, radicand } => {
            if let Some(i) = index {
                format!("\\sqrt[{}]{{{}}}", math(i), math(radicand))
            } else {
                format!("\\sqrt{{{}}}", math(radicand))
            }
        }
        Math::Group { kind, inner } => {
            let (a, b) = match kind {
                GroupKind::Paren => ("\\left(", "\\right)"),
                GroupKind::Bracket => ("\\left[", "\\right]"),
                GroupKind::Brace => ("\\left\\{", "\\right\\}"),
            };
            format!("{a}{}{b}", math(inner))
        }
        Math::Abs(inner) => format!("\\left|{}\\right|", math(inner)),
        Math::Factorial(inner) => format!("{}!", math_maybe_group(inner)),
        Math::Function { kind, arg } => {
            format!("\\{}\\left({}\\right)", kind.name(), math(arg))
        }
        Math::Sum {
            var,
            from,
            to,
            body,
        } => nary("\\sum", var, from, to, body),
        Math::Product {
            var,
            from,
            to,
            body,
        } => nary("\\prod", var, from, to, body),
        Math::Integral {
            from,
            to,
            integrand,
            wrt,
        } => {
            let mut s = String::from("\\int");
            if let Some(f) = from {
                s.push_str(&format!("_{{{}}}", math(f)));
            }
            if let Some(t) = to {
                s.push_str(&format!("^{{{}}}", math(t)));
            }
            if let Some(i) = integrand {
                s.push(' ');
                s.push_str(&math(i));
            }
            if let Some(w) = wrt {
                s.push_str("\\,d");
                s.push_str(&math(w));
            }
            s
        }
        Math::Derivative {
            kind,
            expr,
            variables,
        } => derivative(*kind, expr, variables),
        Math::Limit {
            variable,
            target,
            direction,
            body,
        } => limit(variable, target, *direction, body),
        Math::Unit(u) => unit(u),
        Math::Infinity => "\\infty".into(),
        Math::Ellipsis => "\\ldots".into(),
    }
}

fn nary(
    cmd: &str,
    var: &Option<Box<Math>>,
    from: &Option<Box<Math>>,
    to: &Option<Box<Math>>,
    body: &Option<Box<Math>>,
) -> String {
    let mut s = String::from(cmd);
    let sub = match (var, from) {
        (Some(v), Some(f)) => Some(format!("{}={}", math(v), math(f))),
        (None, Some(f)) => Some(math(f)),
        (Some(v), None) => Some(math(v)),
        _ => None,
    };
    if let Some(sub) = sub {
        s.push_str(&format!("_{{{sub}}}"));
    }
    if let Some(t) = to {
        s.push_str(&format!("^{{{}}}", math(t)));
    }
    if let Some(b) = body {
        s.push(' ');
        s.push_str(&math(b));
    }
    s
}

/// `\frac{d f}{d x}`, `\frac{d^{2} y}{d x^{2}}`,
/// `\frac{\partial^{2} T}{\partial x\,\partial y}` — a real fraction,
/// with the order as an exponent on the operator and on the variable.
fn derivative(kind: DerivativeKind, expr: &Math, variables: &[DerivativeVariable]) -> String {
    let operator = match kind {
        DerivativeKind::Ordinary => "d",
        DerivativeKind::Partial => "\\partial",
    };
    let mut numerator = String::from(operator);
    // `None` means the total order overflowed: no exponent is printed rather
    // than a wrapped, wrong one.
    if let Some(total) = derivative_total_order(variables) {
        if total != 1 {
            numerator.push_str(&format!("^{{{total}}}"));
        }
    }
    numerator.push(' ');
    numerator.push_str(&derivative_operand(expr));
    if variables.is_empty() {
        // Structurally invalid (the validator reports it); no denominator is
        // invented for it.
        return numerator;
    }
    let denominator = variables
        .iter()
        .map(|variable| {
            let mut part = format!("{operator} {}", derivative_operand(&variable.variable));
            if variable.order != 1 {
                part.push_str(&format!("^{{{}}}", variable.order));
            }
            part
        })
        .collect::<Vec<_>>()
        .join("\\,");
    format!("\\frac{{{numerator}}}{{{denominator}}}")
}

/// Only a simple atom sits next to the `d`: `\frac{d \left(x^{2}\right)}{d x}`.
fn derivative_operand(operand: &Math) -> String {
    if super::derivative_operand_needs_group(operand) {
        format!("\\left({}\\right)", math(operand))
    } else {
        math(operand)
    }
}

/// `\lim_{x \to 0} \frac{\sin\left(x\right)}{x}`,
/// `\lim_{x \to 0^-} f`.
fn limit(variable: &Math, target: &Math, direction: LimitDirection, body: &Math) -> String {
    let mut approach = format!("{} \\to {}", math(variable), math_tight(target));
    if let Some(marker) = direction.marker() {
        approach.push_str(&format!("^{marker}"));
    }
    format!("\\lim_{{{approach}}} {}", construct_body(body))
}

/// Additive and relational bodies need brackets; a quotient already renders
/// as a self-delimiting `\frac`.
fn construct_body(body: &Math) -> String {
    match body {
        Math::Binary { op, .. } if !matches!(op, BinOp::Mul | BinOp::Div) => {
            format!("\\left({}\\right)", math(body))
        }
        other => math(other),
    }
}

fn unit(u: &UnitExpr) -> String {
    let mut s = String::new();
    for (i, f) in u.factors.iter().enumerate() {
        if i > 0 {
            if f.divide {
                s.push('/');
            } else {
                s.push_str("\\cdot ");
            }
        } else if f.divide {
            s.push('/');
        }
        s.push_str(&format!("\\mathrm{{{}}}", f.symbol));
        if f.power != 1 {
            s.push_str(&format!("^{{{}}}", f.power));
        }
    }
    s
}

fn math_tight(m: &Math) -> String {
    match m {
        Math::Binary { .. } => format!("\\left({}\\right)", math(m)),
        _ => math(m),
    }
}

fn math_maybe_group(m: &Math) -> String {
    match m {
        Math::Binary { .. }
        | Math::Juxt(_)
        | Math::Fraction { .. }
        | Math::UnaryMinus(_)
        // The same grouping decision as the Unicode renderer makes.
        | Math::Derivative { .. }
        | Math::Limit { .. } => {
            format!("\\left({}\\right)", math(m))
        }
        _ => math(m),
    }
}

fn greek_or_letter(letter: &str) -> String {
    match letter {
        "α" => "\\alpha".into(),
        "Α" => "A".into(),
        "β" => "\\beta".into(),
        "Β" => "B".into(),
        "γ" => "\\gamma".into(),
        "Γ" => "\\Gamma".into(),
        "δ" => "\\delta".into(),
        "Δ" => "\\Delta".into(),
        "ε" => "\\varepsilon".into(),
        "Ε" => "E".into(),
        "ζ" => "\\zeta".into(),
        "Ζ" => "Z".into(),
        "η" => "\\eta".into(),
        "Η" => "H".into(),
        "θ" => "\\theta".into(),
        "Θ" => "\\Theta".into(),
        "ι" => "\\iota".into(),
        "Ι" => "I".into(),
        "κ" => "\\kappa".into(),
        "Κ" => "K".into(),
        "λ" => "\\lambda".into(),
        "Λ" => "\\Lambda".into(),
        "μ" => "\\mu".into(),
        "Μ" => "M".into(),
        "ν" => "\\nu".into(),
        "Ν" => "N".into(),
        "ξ" => "\\xi".into(),
        "Ξ" => "\\Xi".into(),
        "ο" => "o".into(),
        "Ο" => "O".into(),
        "π" => "\\pi".into(),
        "Π" => "\\Pi".into(),
        "ρ" => "\\rho".into(),
        "Ρ" => "P".into(),
        "σ" => "\\sigma".into(),
        "Σ" => "\\Sigma".into(),
        "τ" => "\\tau".into(),
        "Τ" => "T".into(),
        "υ" => "\\upsilon".into(),
        "Υ" => "\\Upsilon".into(),
        "φ" => "\\phi".into(),
        "Φ" => "\\Phi".into(),
        "χ" => "\\chi".into(),
        "Χ" => "X".into(),
        "ψ" => "\\psi".into(),
        "Ψ" => "\\Psi".into(),
        "ω" => "\\omega".into(),
        "Ω" => "\\Omega".into(),
        "∞" => "\\infty".into(),
        other => other.to_string(),
    }
}

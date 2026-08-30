use crate::ast::{
    Arrow, BinOp, Chemical, Equation, Formula, GroupKind, Math, Node, Part, Species, StateMarker,
    UnitExpr,
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
        Math::Binary { .. } | Math::Juxt(_) | Math::Fraction { .. } | Math::UnaryMinus(_) => {
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

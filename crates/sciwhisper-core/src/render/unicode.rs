use crate::ast::{
    Arrow, BinOp, Chemical, Equation, Formula, GroupKind, Math, Node, Part, Species, StateMarker,
    UnitExpr,
};

pub fn render(node: &Node) -> String {
    match node {
        Node::Document(xs) => xs.iter().map(render).collect::<Vec<_>>().join(""),
        Node::Text(s) => s.clone(),
        Node::Chemical(c) => chemical(c),
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
        Arrow::Forward => "→",
        Arrow::Equilibrium => "⇌",
    };
    format!("{left} {arrow} {right}")
}

/// Render one species with its own coefficient, e.g. for a balancing suggestion.
pub(crate) fn render_species(s: &Species) -> String {
    species(s)
}

fn species(s: &Species) -> String {
    let mut out = String::new();
    if s.coefficient != 1 {
        out.push_str(&s.coefficient.to_string());
    }
    out.push_str(&formula(&s.formula));
    if let Some(ch) = s.charge {
        out.push_str(&charge_super(ch));
    }
    if let Some(m) = s.marker {
        out.push(match m {
            StateMarker::Gas => '↑',
            StateMarker::Precipitate => '↓',
        });
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
                    out.push_str(&sub_num(*count));
                }
            }
            Part::Group { inner, count } => {
                out.push('(');
                out.push_str(&formula(inner));
                out.push(')');
                if *count != 1 {
                    out.push_str(&sub_num(*count));
                }
            }
            Part::Hydrate { count } => {
                out.push('·');
                if *count != 1 {
                    out.push_str(&count.to_string());
                }
                out.push_str("H₂O");
            }
        }
    }
    out
}

fn math(m: &Math) -> String {
    match m {
        Math::Number(n) => n.clone(),
        Math::Symbol(s) => s.letter.clone(),
        Math::Delta(inner) => format!("Δ{}", math(inner)),
        Math::Vector(inner) => format!("{}⃗", math_maybe_group(inner)),
        Math::UnaryMinus(inner) => format!("−{}", math_tight(inner)),
        Math::Binary { op, left, right } => {
            let op_s = match op {
                BinOp::Add => " + ",
                BinOp::Sub => " − ",
                BinOp::Mul => "·",
                BinOp::Div => "/",
                BinOp::Eq => " = ",
                BinOp::Ne => " ≠ ",
                BinOp::Lt => " < ",
                BinOp::Gt => " > ",
                BinOp::Le => " ≤ ",
                BinOp::Ge => " ≥ ",
                BinOp::PlusMinus => " ± ",
            };
            format!("{}{}{}", math(left), op_s, math(right))
        }
        Math::Juxt(xs) => {
            let mut s = String::new();
            for (i, x) in xs.iter().enumerate() {
                if i > 0 && matches!(x, Math::Unit(_)) {
                    s.push(' ');
                }
                s.push_str(&math_tight(x));
            }
            s
        }
        Math::Fraction { num, den } => format!("({})/({})", math(num), math(den)),
        Math::Power { base, exp } => {
            let b = math_maybe_group(base);
            if let Math::Number(n) = exp.as_ref() {
                if n.len() == 1 && n.chars().next().unwrap().is_ascii_digit() {
                    return format!("{}{}", b, super_digit(n.chars().next().unwrap()));
                }
            }
            format!("{b}^{{{}}}", math(exp))
        }
        Math::Subscript { base, sub } => {
            let b = math_maybe_group(base);
            if let Math::Number(n) = sub.as_ref() {
                if n.chars().all(|c| c.is_ascii_digit()) {
                    return format!("{b}{}", sub_digits(n));
                }
            }
            format!("{b}_{{{}}}", math(sub))
        }
        Math::Root { index, radicand } => {
            if let Some(i) = index {
                format!("√[{}]({})", math(i), math(radicand))
            } else {
                format!("√{}", math_maybe_group(radicand))
            }
        }
        Math::Group { kind, inner } => {
            let (a, b) = match kind {
                GroupKind::Paren => ('(', ')'),
                GroupKind::Bracket => ('[', ']'),
                GroupKind::Brace => ('{', '}'),
            };
            format!("{a}{}{b}", math(inner))
        }
        Math::Abs(inner) => format!("|{}|", math(inner)),
        Math::Factorial(inner) => format!("{}!", math_maybe_group(inner)),
        Math::Function { kind, arg } => format!("{}({})", kind.name(), math(arg)),
        Math::Sum {
            var,
            from,
            to,
            body,
        } => nary('∑', var, from, to, body),
        Math::Product {
            var,
            from,
            to,
            body,
        } => nary('∏', var, from, to, body),
        Math::Integral {
            from,
            to,
            integrand,
            wrt,
        } => {
            let mut s = String::from("∫");
            if let Some(f) = from {
                s.push_str(&lower_bound(f));
            }
            if let Some(t) = to {
                s.push_str(&upper_bound(t));
            }
            if let Some(i) = integrand {
                s.push(' ');
                s.push_str(&math(i));
            }
            if let Some(w) = wrt {
                s.push_str(" d");
                s.push_str(&math(w));
            }
            s
        }
        Math::Unit(u) => unit(u),
        Math::Infinity => "∞".into(),
        Math::Ellipsis => "…".into(),
    }
}

fn nary(
    sym: char,
    var: &Option<Box<Math>>,
    from: &Option<Box<Math>>,
    to: &Option<Box<Math>>,
    body: &Option<Box<Math>>,
) -> String {
    let mut s = String::from(sym);
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
        s.push_str(&upper_bound(t));
    }
    if let Some(b) = body {
        s.push(' ');
        s.push_str(&math(b));
    }
    s
}

fn lower_bound(value: &Math) -> String {
    if let Math::Number(number) = value {
        if number.chars().all(|c| c.is_ascii_digit()) {
            return sub_digits(number);
        }
    }
    format!("_{{{}}}", math(value))
}

fn upper_bound(value: &Math) -> String {
    if let Math::Number(number) = value {
        if number.chars().all(|c| c.is_ascii_digit()) {
            return number.chars().map(super_digit).collect();
        }
    }
    format!("^{{{}}}", math(value))
}

fn unit(u: &UnitExpr) -> String {
    let mut s = String::new();
    for (i, f) in u.factors.iter().enumerate() {
        if i > 0 {
            s.push(if f.divide { '/' } else { '·' });
        } else if f.divide {
            s.push('/');
        }
        s.push_str(&f.symbol);
        if f.power != 1 {
            s.push_str(&super_num_signed(f.power));
        }
    }
    s
}

fn math_tight(m: &Math) -> String {
    match m {
        Math::Binary { .. } => format!("({})", math(m)),
        _ => math(m),
    }
}

fn math_maybe_group(m: &Math) -> String {
    match m {
        Math::Binary { .. } | Math::Juxt(_) | Math::Fraction { .. } | Math::UnaryMinus(_) => {
            format!("({})", math(m))
        }
        Math::Number(_)
        | Math::Symbol(_)
        | Math::Group { .. }
        | Math::Delta(_)
        | Math::Vector(_) => math(m),
        other => math(other),
    }
}

const SUB: [char; 10] = ['₀', '₁', '₂', '₃', '₄', '₅', '₆', '₇', '₈', '₉'];
const SUP: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];

fn sub_num(n: u32) -> String {
    sub_digits(&n.to_string())
}

fn sub_digits(n: &str) -> String {
    n.chars()
        .map(|c| c.to_digit(10).map(|d| SUB[d as usize]).unwrap_or(c))
        .collect()
}

fn super_digit(c: char) -> char {
    c.to_digit(10).map(|d| SUP[d as usize]).unwrap_or(c)
}

fn super_num_signed(n: i32) -> String {
    let mut s = String::new();
    if n < 0 {
        s.push('⁻');
    }
    for c in n.unsigned_abs().to_string().chars() {
        s.push(super_digit(c));
    }
    s
}

fn charge_super(ch: i32) -> String {
    let mut s = String::new();
    let mag = ch.unsigned_abs();
    if mag != 1 {
        s.push_str(super_num_signed(mag as i32).trim_start_matches('⁻'));
    }
    s.push(if ch > 0 { '⁺' } else { '⁻' });
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formula::parse_species_str;

    #[test]
    fn copper_hydroxide() {
        let s = parse_species_str("Cu(OH)2").unwrap();
        assert_eq!(species(&s), "Cu(OH)₂");
    }
}

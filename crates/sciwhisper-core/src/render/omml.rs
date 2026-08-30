//! Office Math ML (OMML) renderer. Insertion into Word is a later Windows layer;
//! this crate only produces the XML tree from the AST.

use crate::ast::{
    Arrow, BinOp, Chemical, Formula, GroupKind, Math, Node, Part, Species, StateMarker, UnitExpr,
};

const NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";

pub fn render(node: &Node) -> String {
    let inner = match node {
        Node::Document(xs) => xs.iter().map(render_inner).collect::<Vec<_>>().join(""),
        other => render_inner(other),
    };
    format!(r#"<m:oMath xmlns:m="{NS}">{inner}</m:oMath>"#)
}

/// Word `Selection.InsertXML` payload: a paragraph wrapping oMath.
pub fn word_insert_xml(node: &Node) -> String {
    let math = render(node);
    format!(
        r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="{NS}">{math}</w:p>"#
    )
}

fn render_inner(node: &Node) -> String {
    match node {
        Node::Document(xs) => xs.iter().map(render_inner).collect::<Vec<_>>().join(""),
        Node::Text(s) => run(s),
        Node::Chemical(c) => match c {
            Chemical::Species(s) => species(s),
            Chemical::Equation(eq) => {
                let mut out = String::new();
                for (i, sp) in eq.left.iter().enumerate() {
                    if i > 0 {
                        out.push_str(&run(" + "));
                    }
                    out.push_str(&species(sp));
                }
                let a = match eq.arrow {
                    Arrow::Forward => "→",
                    Arrow::Equilibrium => "⇌",
                };
                out.push_str(&run(&format!(" {a} ")));
                for (i, sp) in eq.right.iter().enumerate() {
                    if i > 0 {
                        out.push_str(&run(" + "));
                    }
                    out.push_str(&species(sp));
                }
                out
            }
        },
        Node::Math(m) => math(m),
    }
}

fn species(s: &Species) -> String {
    let mut out = String::new();
    if s.coefficient != 1 {
        out.push_str(&run(&s.coefficient.to_string()));
    }
    out.push_str(&formula(&s.formula));
    if let Some(ch) = s.charge {
        let mag = ch.unsigned_abs();
        let sign = if ch > 0 { "+" } else { "-" };
        let t = if mag == 1 {
            sign.to_string()
        } else {
            format!("{mag}{sign}")
        };
        out.push_str(&s_sup("", &t));
    }
    match s.marker {
        Some(StateMarker::Gas) => out.push_str(&run("↑")),
        Some(StateMarker::Precipitate) => out.push_str(&run("↓")),
        None => {}
    }
    out
}

fn formula(f: &Formula) -> String {
    let mut out = String::new();
    for p in &f.parts {
        match p {
            Part::Atom { symbol, count } => {
                if *count == 1 {
                    out.push_str(&run(symbol));
                } else {
                    out.push_str(&s_sub(symbol, &count.to_string()));
                }
            }
            Part::Group { inner, count } => {
                out.push_str(&run("("));
                out.push_str(&formula(inner));
                if *count == 1 {
                    out.push_str(&run(")"));
                } else {
                    out.push_str(&s_sub(")", &count.to_string()));
                }
            }
            Part::Hydrate { count } => {
                out.push_str(&run("·"));
                if *count != 1 {
                    out.push_str(&run(&count.to_string()));
                }
                out.push_str(&s_sub("H", "2"));
                out.push_str(&run("O"));
            }
        }
    }
    out
}

fn math(m: &Math) -> String {
    match m {
        Math::Number(n) => run(n),
        Math::Symbol(s) => run(&s.letter),
        Math::Delta(inner) => format!("{}{}", run("Δ"), math(inner)),
        Math::Vector(inner) => format!(
            "<m:acc><m:accPr><m:chr m:val=\"⃗\"/></m:accPr><m:e>{}</m:e></m:acc>",
            math(inner)
        ),
        Math::UnaryMinus(inner) => format!("{}{}", run("−"), math(inner)),
        Math::Binary { op, left, right } => {
            if *op == BinOp::Div {
                return frac(&math(left), &math(right));
            }
            let op_s = match op {
                BinOp::Add => " + ",
                BinOp::Sub => " − ",
                BinOp::Mul => "·",
                BinOp::Eq => " = ",
                BinOp::Ne => " ≠ ",
                BinOp::Lt => " < ",
                BinOp::Gt => " > ",
                BinOp::Le => " ≤ ",
                BinOp::Ge => " ≥ ",
                BinOp::PlusMinus => " ± ",
                BinOp::Div => "/",
            };
            format!("{}{}{}", math(left), run(op_s), math(right))
        }
        Math::Juxt(xs) => xs.iter().map(math).collect::<Vec<_>>().join(""),
        Math::Fraction { num, den } => frac(&math(num), &math(den)),
        Math::Power { base, exp } => format!(
            "<m:sSup><m:e>{}</m:e><m:sup>{}</m:sup></m:sSup>",
            math(base),
            math(exp)
        ),
        Math::Subscript { base, sub } => format!(
            "<m:sSub><m:e>{}</m:e><m:sub>{}</m:sub></m:sSub>",
            math(base),
            math(sub)
        ),
        Math::Root { index, radicand } => {
            if let Some(i) = index {
                format!(
                    "<m:rad><m:radPr><m:degHide m:val=\"0\"/></m:radPr><m:deg>{}</m:deg><m:e>{}</m:e></m:rad>",
                    math(i),
                    math(radicand)
                )
            } else {
                format!(
                    "<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e>{}</m:e></m:rad>",
                    math(radicand)
                )
            }
        }
        Math::Group { kind, inner } => {
            let (a, b) = match kind {
                GroupKind::Paren => ("(", ")"),
                GroupKind::Bracket => ("[", "]"),
                GroupKind::Brace => ("{", "}"),
            };
            format!("{}{}{}", run(a), math(inner), run(b))
        }
        Math::Abs(inner) => format!("{}{}{}", run("|"), math(inner), run("|")),
        Math::Factorial(inner) => format!("{}{}", math(inner), run("!")),
        Math::Function { kind, arg } => {
            format!("{}{}{}{}", run(kind.name()), run("("), math(arg), run(")"))
        }
        Math::Sum {
            var,
            from,
            to,
            body,
        } => nary("∑", var, from, to, body),
        Math::Product {
            var,
            from,
            to,
            body,
        } => nary("∏", var, from, to, body),
        Math::Integral {
            from,
            to,
            integrand,
            wrt,
        } => {
            let mut sub = String::new();
            if let Some(f) = from {
                sub = math(f);
            }
            let mut sup = String::new();
            if let Some(t) = to {
                sup = math(t);
            }
            let mut e = String::new();
            if let Some(i) = integrand {
                e.push_str(&math(i));
            }
            if let Some(w) = wrt {
                e.push_str(&run(" d"));
                e.push_str(&math(w));
            }
            format!(
                "<m:nary><m:naryPr><m:chr m:val=\"∫\"/></m:naryPr><m:sub>{sub}</m:sub><m:sup>{sup}</m:sup><m:e>{e}</m:e></m:nary>"
            )
        }
        Math::Unit(u) => run(&unit_plain(u)),
        Math::Infinity => run("∞"),
        Math::Ellipsis => run("…"),
    }
}

fn nary(
    chr: &str,
    var: &Option<Box<Math>>,
    from: &Option<Box<Math>>,
    to: &Option<Box<Math>>,
    body: &Option<Box<Math>>,
) -> String {
    let sub = match (var, from) {
        (Some(v), Some(f)) => format!("{}{}{}", math(v), run("="), math(f)),
        (None, Some(f)) => math(f),
        (Some(v), None) => math(v),
        _ => String::new(),
    };
    let sup = to.as_ref().map(|t| math(t)).unwrap_or_default();
    let e = body.as_ref().map(|b| math(b)).unwrap_or_default();
    format!(
        "<m:nary><m:naryPr><m:chr m:val=\"{chr}\"/></m:naryPr><m:sub>{sub}</m:sub><m:sup>{sup}</m:sup><m:e>{e}</m:e></m:nary>"
    )
}

fn frac(num: &str, den: &str) -> String {
    format!("<m:f><m:num>{num}</m:num><m:den>{den}</m:den></m:f>")
}

fn s_sub(base: &str, sub: &str) -> String {
    format!(
        "<m:sSub><m:e>{}</m:e><m:sub>{}</m:sub></m:sSub>",
        run(base),
        run(sub)
    )
}

fn s_sup(base: &str, sup: &str) -> String {
    format!(
        "<m:sSup><m:e>{}</m:e><m:sup>{}</m:sup></m:sSup>",
        run(base),
        run(sup)
    )
}

fn run(text: &str) -> String {
    let t = escape(text);
    format!("<m:r><m:t xml:space=\"preserve\">{t}</m:t></m:r>")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn unit_plain(u: &UnitExpr) -> String {
    let mut s = String::new();
    for (i, f) in u.factors.iter().enumerate() {
        if i > 0 {
            s.push(if f.divide { '/' } else { '·' });
        }
        s.push_str(&f.symbol);
        if f.power != 1 {
            s.push('^');
            s.push_str(&f.power.to_string());
        }
    }
    s
}

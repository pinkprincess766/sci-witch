//! Canonical AST form v1.
//!
//! Two results are the same answer when their canonical strings are equal.
//! The comparison happens on the AST, never on Unicode, LaTeX or OMML: a
//! renderer choosing `\left(` over `(` is not a scientific error, and a
//! changed charge is not excused by an identical-looking render.
//!
//! Normalisation is limited to rewrites whose equivalence is provable. No
//! algebraic identity is applied anywhere: this is not a CAS, and `x/x` is
//! not `1`.

use std::fmt;

use sciwhisper_core::Node;
use serde_json::Value;

pub const CANONICAL_SCHEMA_VERSION: u32 = 1;

/// Nesting guard. A hand-built or hostile AST abstains with an error rather
/// than exhausting the stack.
pub const MAX_CANONICAL_DEPTH: usize = 128;

/// The key for the safety action. It lives outside the AST value space on
/// purpose: `RAW` is "leave the words alone", which is a different kind of
/// answer from any `Node`, including `Node::Text`.
pub const RAW_KEY: &str = "RAW";

#[derive(Debug, PartialEq, Eq)]
pub enum CanonicalError {
    TooDeep(usize),
    NotSerialisable(String),
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CanonicalError::TooDeep(limit) => {
                write!(f, "AST nesting deeper than {limit} is not canonicalised")
            }
            CanonicalError::NotSerialisable(error) => write!(f, "AST is not serialisable: {error}"),
        }
    }
}

impl std::error::Error for CanonicalError {}

/// What the system, or the corpus, says should happen to an utterance.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Ast(Node),
    Raw,
}

impl Target {
    pub fn is_ast(&self) -> bool {
        matches!(self, Target::Ast(_))
    }

    pub fn ast(&self) -> Option<&Node> {
        match self {
            Target::Ast(node) => Some(node),
            Target::Raw => None,
        }
    }
}

/// Canonical key for a whole decision, so that `RAW` can never collide with
/// a scientific AST.
pub fn canonical_target_v1(target: &Target) -> Result<String, CanonicalError> {
    match target {
        Target::Raw => Ok(RAW_KEY.to_string()),
        Target::Ast(node) => Ok(format!("AST:{}", canonical_node_v1(node)?)),
    }
}

/// Canonical string for one AST.
///
/// Object keys are emitted in a stable (sorted) order, every enum variant tag
/// and every meaningful field is preserved, and the only rewrites applied are
/// the two proven-equivalent ones documented in
/// `research/schema/CANONICAL_AST_V1_RU.md`.
pub fn canonical_node_v1(node: &Node) -> Result<String, CanonicalError> {
    let value = serde_json::to_value(node)
        .map_err(|error| CanonicalError::NotSerialisable(error.to_string()))?;
    check_depth(&value, MAX_CANONICAL_DEPTH)?;
    let value = normalise(value);
    serde_json::to_string(&value)
        .map_err(|error| CanonicalError::NotSerialisable(error.to_string()))
}

/// Iterative depth check, so that measuring the depth cannot itself overflow
/// the stack.
fn check_depth(root: &Value, limit: usize) -> Result<(), CanonicalError> {
    let mut stack = vec![(root, 1usize)];
    while let Some((value, depth)) = stack.pop() {
        if depth > limit {
            return Err(CanonicalError::TooDeep(limit));
        }
        match value {
            Value::Object(map) => {
                for child in map.values() {
                    stack.push((child, depth + 1));
                }
            }
            Value::Array(items) => {
                for child in items {
                    stack.push((child, depth + 1));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// The two permitted rewrites.
///
/// 1. `Document([x])` becomes `x`. Every renderer concatenates a document's
///    children, so a one-element document and its child produce byte-identical
///    output in all three formats.
/// 2. A numeric literal's decimal separator becomes `.` and a leading `+` is
///    dropped. `Number("9,81")` and `Number("9.81")` denote the same number;
///    the surviving difference is presentation, which the render metric —
///    not the AST metric — is responsible for.
///
/// Nothing else is touched. `Juxt` is not turned into `Binary::Mul`, a
/// `Group` is not removed, and no algebraic identity is applied.
fn normalise(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(children)) = map.get("Document") {
                if children.len() == 1 {
                    return normalise(children[0].clone());
                }
            }
            if let Some(Value::String(text)) = map.get("Number") {
                if map.len() == 1 {
                    if let Some(normalised) = normalise_number(text) {
                        let mut replacement = serde_json::Map::new();
                        replacement.insert("Number".into(), Value::String(normalised));
                        return Value::Object(replacement);
                    }
                }
            }
            let mut out = serde_json::Map::new();
            for (key, child) in map {
                out.insert(key, normalise(child));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(normalise).collect()),
        other => other,
    }
}

/// `None` when the literal is not a plain decimal number, in which case it is
/// left exactly as dictated.
fn normalise_number(text: &str) -> Option<String> {
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", text.strip_prefix('+').unwrap_or(text)),
    };
    let (integer, fraction) = match digits.split_once([',', '.']) {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (digits, None),
    };
    if integer.is_empty() || !integer.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    match fraction {
        None => Some(format!("{sign}{integer}")),
        Some(fraction) => {
            if fraction.is_empty() || !fraction.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            Some(format!("{sign}{integer}.{fraction}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciwhisper_core::ast::{
        Arrow, Case, Chemical, DerivativeKind, DerivativeVariable, Equation, Formula,
        LimitDirection, Math, Part, Species, Symbol,
    };

    fn sym(letter: char) -> Math {
        Math::Symbol(Symbol::latin(letter, Case::Lower))
    }

    fn water() -> Species {
        Species::new(Formula {
            parts: vec![
                Part::Atom {
                    symbol: "H".into(),
                    count: 2,
                },
                Part::Atom {
                    symbol: "O".into(),
                    count: 1,
                },
            ],
        })
    }

    fn key(node: &Node) -> String {
        canonical_node_v1(node).expect("canonicalisable")
    }

    #[test]
    fn the_canonical_form_is_stable_across_calls_and_clones() {
        let node = Node::Math(Math::limit(
            sym('x'),
            Math::Number("0".into()),
            LimitDirection::FromLeft,
            sym('f'),
        ));
        let first = key(&node);
        assert_eq!(first, key(&node.clone()));
        assert_eq!(first, key(&node));
        // Keys are emitted sorted, so the string does not depend on the order
        // in which the struct happens to declare its fields.
        let position = |needle: &str| first.find(needle).expect(needle);
        assert!(position("\"body\"") < position("\"direction\""));
        assert!(position("\"direction\"") < position("\"target\""));
        assert!(position("\"target\"") < position("\"variable\""));
    }

    #[test]
    fn raw_is_its_own_action_and_never_an_ast() {
        let raw = canonical_target_v1(&Target::Raw).unwrap();
        assert_eq!(raw, RAW_KEY);
        let text = canonical_target_v1(&Target::Ast(Node::Text("предел терпения".into()))).unwrap();
        assert_ne!(raw, text);
        assert!(text.starts_with("AST:"));
    }

    #[test]
    fn a_changed_subscript_changes_the_canonical_form() {
        let h2o = Node::Chemical(Chemical::Species(water()));
        let mut wrong = water();
        wrong.formula.parts[0] = Part::Atom {
            symbol: "H".into(),
            count: 3,
        };
        assert_ne!(key(&h2o), key(&Node::Chemical(Chemical::Species(wrong))));
    }

    #[test]
    fn a_changed_charge_changes_the_canonical_form() {
        let mut plus_two = water();
        plus_two.charge = Some(2);
        let mut minus_two = water();
        minus_two.charge = Some(-2);
        assert_ne!(
            key(&Node::Chemical(Chemical::Species(plus_two))),
            key(&Node::Chemical(Chemical::Species(minus_two)))
        );
    }

    #[test]
    fn a_changed_coefficient_changes_the_canonical_form() {
        let mut two = water();
        two.coefficient = 2;
        assert_ne!(
            key(&Node::Chemical(Chemical::Species(water()))),
            key(&Node::Chemical(Chemical::Species(two)))
        );
    }

    #[test]
    fn a_swapped_reaction_side_changes_the_canonical_form() {
        let forward = Equation {
            left: vec![water()],
            arrow: Arrow::Forward,
            right: vec![Species::new(Formula::atom("O", 2))],
            condition: None,
        };
        let backward = Equation {
            left: forward.right.clone(),
            arrow: Arrow::Forward,
            right: forward.left.clone(),
            condition: None,
        };
        assert_ne!(
            key(&Node::Chemical(Chemical::Equation(forward.clone()))),
            key(&Node::Chemical(Chemical::Equation(backward)))
        );
        let equilibrium = Equation {
            arrow: Arrow::Equilibrium,
            ..forward.clone()
        };
        assert_ne!(
            key(&Node::Chemical(Chemical::Equation(forward))),
            key(&Node::Chemical(Chemical::Equation(equilibrium)))
        );
    }

    #[test]
    fn a_changed_limit_direction_changes_the_canonical_form() {
        let limit = |direction| {
            Node::Math(Math::limit(
                sym('x'),
                Math::Number("0".into()),
                direction,
                sym('f'),
            ))
        };
        assert_ne!(
            key(&limit(LimitDirection::FromLeft)),
            key(&limit(LimitDirection::FromRight))
        );
        assert_ne!(
            key(&limit(LimitDirection::TwoSided)),
            key(&limit(LimitDirection::FromLeft))
        );
    }

    #[test]
    fn a_changed_derivative_order_or_kind_changes_the_canonical_form() {
        let derivative = |kind, order| {
            Node::Math(
                Math::derivative(
                    kind,
                    sym('f'),
                    vec![DerivativeVariable::new(sym('x'), order)],
                )
                .unwrap(),
            )
        };
        assert_ne!(
            key(&derivative(DerivativeKind::Ordinary, 1)),
            key(&derivative(DerivativeKind::Ordinary, 2))
        );
        assert_ne!(
            key(&derivative(DerivativeKind::Ordinary, 1)),
            key(&derivative(DerivativeKind::Partial, 1))
        );
    }

    #[test]
    fn a_single_child_document_equals_its_child() {
        let child = Node::Math(sym('x'));
        assert_eq!(key(&Node::Document(vec![child.clone()])), key(&child));
        // Two children are a different answer and stay different.
        assert_ne!(
            key(&Node::Document(vec![child.clone(), child.clone()])),
            key(&child)
        );
    }

    #[test]
    fn only_the_decimal_separator_is_normalised() {
        assert_eq!(
            key(&Node::Math(Math::Number("9,81".into()))),
            key(&Node::Math(Math::Number("9.81".into())))
        );
        assert_eq!(
            key(&Node::Math(Math::Number("+7".into()))),
            key(&Node::Math(Math::Number("7".into())))
        );
        // Different numbers stay different, and a non-numeric literal is left
        // exactly as it was dictated.
        assert_ne!(
            key(&Node::Math(Math::Number("9,81".into()))),
            key(&Node::Math(Math::Number("981".into())))
        );
        assert_eq!(normalise_number("1e5"), None);
        assert_eq!(normalise_number(""), None);
        assert_eq!(normalise_number("-0,5").as_deref(), Some("-0.5"));
    }

    #[test]
    fn no_algebraic_identity_is_applied() {
        let quotient = Node::Math(Math::Binary {
            op: sciwhisper_core::ast::BinOp::Div,
            left: Box::new(sym('x')),
            right: Box::new(sym('x')),
        });
        assert_ne!(key(&quotient), key(&Node::Math(Math::Number("1".into()))));
        // Implicit multiplication is not folded into an explicit product.
        let juxt = Node::Math(Math::Juxt(vec![sym('a'), sym('b')]));
        let product = Node::Math(Math::Binary {
            op: sciwhisper_core::ast::BinOp::Mul,
            left: Box::new(sym('a')),
            right: Box::new(sym('b')),
        });
        assert_ne!(key(&juxt), key(&product));
        // A group is meaningful and is not silently removed.
        let grouped = Node::Math(Math::Group {
            kind: sciwhisper_core::ast::GroupKind::Paren,
            inner: Box::new(sym('a')),
        });
        assert_ne!(key(&grouped), key(&Node::Math(sym('a'))));
    }

    #[test]
    fn an_over_deep_ast_is_refused_instead_of_overflowing() {
        let mut node = sym('x');
        for _ in 0..(MAX_CANONICAL_DEPTH + 8) {
            node = Math::UnaryMinus(Box::new(node));
        }
        assert_eq!(
            canonical_node_v1(&Node::Math(node)),
            Err(CanonicalError::TooDeep(MAX_CANONICAL_DEPTH))
        );
    }
}

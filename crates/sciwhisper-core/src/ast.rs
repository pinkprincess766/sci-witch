//! Internal scientific notation tree.
//! Renderers consume this AST and must not re-parse a raw transcript.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Document(Vec<Node>),
    Text(String),
    Chemical(Chemical),
    Math(Math),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Chemical {
    Species(Species),
    Equation(Equation),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Equation {
    pub left: Vec<Species>,
    pub arrow: Arrow,
    pub right: Vec<Species>,
    pub condition: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Arrow {
    Forward,
    Equilibrium,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Species {
    pub coefficient: u32,
    pub formula: Formula,
    pub charge: Option<i32>,
    pub marker: Option<StateMarker>,
}

impl Species {
    pub fn new(formula: Formula) -> Self {
        Self {
            coefficient: 1,
            formula,
            charge: None,
            marker: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateMarker {
    Gas,
    Precipitate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Formula {
    pub parts: Vec<Part>,
}

impl Formula {
    pub fn atom(symbol: impl Into<String>, count: u32) -> Self {
        Self {
            parts: vec![Part::Atom {
                symbol: symbol.into(),
                count: count.max(1),
            }],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Element symbol -> total atom count, with groups and hydrates expanded.
    /// `None` on overflow (an artificially deep or wide `Formula`) rather
    /// than a wrapped, silently-wrong count.
    pub fn atom_counts(&self) -> Option<BTreeMap<String, u64>> {
        let mut atoms = BTreeMap::new();
        Self::collect_atoms(self, 1, &mut atoms)?;
        Some(atoms)
    }

    fn collect_atoms(
        formula: &Formula,
        multiplier: u64,
        atoms: &mut BTreeMap<String, u64>,
    ) -> Option<()> {
        for part in &formula.parts {
            match part {
                Part::Atom { symbol, count } => {
                    let contribution = multiplier.checked_mul(u64::from(*count))?;
                    let entry = atoms.entry(symbol.clone()).or_insert(0);
                    *entry = entry.checked_add(contribution)?;
                }
                Part::Group { inner, count } => {
                    let multiplier = multiplier.checked_mul(u64::from(*count))?;
                    Self::collect_atoms(inner, multiplier, atoms)?;
                }
                Part::Hydrate { count } => {
                    let waters = multiplier.checked_mul(u64::from(*count))?;
                    let h = waters.checked_mul(2)?;
                    let h_entry = atoms.entry("H".into()).or_insert(0);
                    *h_entry = h_entry.checked_add(h)?;
                    let o_entry = atoms.entry("O".into()).or_insert(0);
                    *o_entry = o_entry.checked_add(waters)?;
                }
            }
        }
        Some(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Part {
    Atom { symbol: String, count: u32 },
    Group { inner: Formula, count: u32 },
    Hydrate { count: u32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Math {
    Number(String),
    Symbol(Symbol),
    Delta(Box<Math>),
    Vector(Box<Math>),
    UnaryMinus(Box<Math>),
    Binary {
        op: BinOp,
        left: Box<Math>,
        right: Box<Math>,
    },
    Juxt(Vec<Math>),
    Fraction {
        num: Box<Math>,
        den: Box<Math>,
    },
    Power {
        base: Box<Math>,
        exp: Box<Math>,
    },
    Subscript {
        base: Box<Math>,
        sub: Box<Math>,
    },
    Root {
        index: Option<Box<Math>>,
        radicand: Box<Math>,
    },
    Group {
        kind: GroupKind,
        inner: Box<Math>,
    },
    Abs(Box<Math>),
    Factorial(Box<Math>),
    Function {
        kind: FunctionKind,
        arg: Box<Math>,
    },
    Sum {
        var: Option<Box<Math>>,
        from: Option<Box<Math>>,
        to: Option<Box<Math>>,
        body: Option<Box<Math>>,
    },
    Product {
        var: Option<Box<Math>>,
        from: Option<Box<Math>>,
        to: Option<Box<Math>>,
        body: Option<Box<Math>>,
    },
    Integral {
        from: Option<Box<Math>>,
        to: Option<Box<Math>>,
        integrand: Option<Box<Math>>,
        wrt: Option<Box<Math>>,
    },
    /// Structural derivative: *what* is differentiated with respect to
    /// *which* variables and *how many* times. Nothing here differentiates
    /// symbolically — `d/dx` of `x²` stays `d(x²)/dx`, never `2x`.
    ///
    /// A mixed partial keeps one entry per variable (`∂²T/∂x∂y` is two
    /// entries of order 1), never a single fused `dxdy` string.
    Derivative {
        kind: DerivativeKind,
        expr: Box<Math>,
        variables: Vec<DerivativeVariable>,
    },
    /// Structural limit. The one-sidedness is a type, not a character glued
    /// onto the target, and nothing here evaluates the limit.
    Limit {
        variable: Box<Math>,
        target: Box<Math>,
        direction: LimitDirection,
        body: Box<Math>,
    },
    Unit(UnitExpr),
    Infinity,
    Ellipsis,
}

/// Highest differentiation order a typed derivative accepts, both per
/// variable and summed over the whole node. Dictated mathematics never
/// needs more, and the cap keeps the order arithmetic — and every
/// renderer's superscript — bounded.
pub const MAX_DERIVATIVE_ORDER: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DerivativeKind {
    Ordinary,
    Partial,
}

impl DerivativeKind {
    /// The differential operator letter: `d` in `df/dx`, `∂` in `∂T/∂x`.
    pub fn operator(self) -> &'static str {
        match self {
            Self::Ordinary => "d",
            Self::Partial => "∂",
        }
    }
}

/// One variable of differentiation together with its own order. `∂²T/∂x∂y`
/// is two of these, each of order 1; `d²y/dx²` is one of order 2.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DerivativeVariable {
    pub variable: Box<Math>,
    pub order: u32,
}

impl DerivativeVariable {
    pub fn new(variable: Math, order: u32) -> Self {
        Self {
            variable: Box::new(variable),
            order,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LimitDirection {
    TwoSided,
    FromLeft,
    FromRight,
}

impl LimitDirection {
    /// Superscript marker on the target: `x → 0⁻` for a left-hand limit.
    pub fn marker(self) -> Option<&'static str> {
        match self {
            Self::TwoSided => None,
            Self::FromLeft => Some("-"),
            Self::FromRight => Some("+"),
        }
    }
}

/// Why a derivative's variable list violates the node's invariants. Used by
/// the constructor (which refuses to build such a node), by the structural
/// validator (which reports one that was built by hand) and by the
/// dimensional analyser (which abstains on one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DerivativeDefect {
    NoVariables,
    ZeroOrder,
    OrderTooHigh,
    TotalOrderTooHigh,
}

impl DerivativeDefect {
    pub fn code(self) -> &'static str {
        match self {
            Self::NoVariables => "math.derivative_without_variable",
            Self::ZeroOrder => "math.derivative_zero_order",
            Self::OrderTooHigh => "math.derivative_order_too_high",
            Self::TotalOrderTooHigh => "math.derivative_total_order_too_high",
        }
    }

    pub fn message(self) -> String {
        match self {
            Self::NoVariables => "a derivative needs at least one variable".into(),
            Self::ZeroOrder => "a derivative variable needs an order of at least 1".into(),
            Self::OrderTooHigh => {
                format!("a derivative order above {MAX_DERIVATIVE_ORDER} is not supported")
            }
            Self::TotalOrderTooHigh => {
                format!("a total derivative order above {MAX_DERIVATIVE_ORDER} is not supported")
            }
        }
    }
}

/// Sum of the per-variable orders. `None` on overflow, so an artificially
/// built list can never wrap around into a plausible-looking small total.
pub fn derivative_total_order(variables: &[DerivativeVariable]) -> Option<u32> {
    variables
        .iter()
        .try_fold(0u32, |total, variable| total.checked_add(variable.order))
}

/// `None` when the variable list satisfies every invariant of
/// `Math::Derivative`.
pub fn derivative_defect(variables: &[DerivativeVariable]) -> Option<DerivativeDefect> {
    if variables.is_empty() {
        return Some(DerivativeDefect::NoVariables);
    }
    for variable in variables {
        if variable.order == 0 {
            return Some(DerivativeDefect::ZeroOrder);
        }
        if variable.order > MAX_DERIVATIVE_ORDER {
            return Some(DerivativeDefect::OrderTooHigh);
        }
    }
    match derivative_total_order(variables) {
        Some(total) if total <= MAX_DERIVATIVE_ORDER => None,
        // Overflow and "too large to be real" are the same answer here:
        // refuse, rather than accept a total nobody can have dictated.
        _ => Some(DerivativeDefect::TotalOrderTooHigh),
    }
}

impl Math {
    /// The only way the parser builds a derivative: an invariant violation
    /// is an error, never a silently repaired node.
    pub fn derivative(
        kind: DerivativeKind,
        expr: Math,
        variables: Vec<DerivativeVariable>,
    ) -> std::result::Result<Math, DerivativeDefect> {
        match derivative_defect(&variables) {
            Some(defect) => Err(defect),
            None => Ok(Math::Derivative {
                kind,
                expr: Box::new(expr),
                variables,
            }),
        }
    }

    pub fn limit(variable: Math, target: Math, direction: LimitDirection, body: Math) -> Math {
        Math::Limit {
            variable: Box::new(variable),
            target: Box::new(target),
            direction,
            body: Box::new(body),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FunctionKind {
    Sin,
    Cos,
    Tan,
    Cot,
    Ln,
    Log,
    Exp,
}

impl FunctionKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Sin => "sin",
            Self::Cos => "cos",
            Self::Tan => "tan",
            Self::Cot => "cot",
            Self::Ln => "ln",
            Self::Log => "log",
            Self::Exp => "exp",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub letter: String,
    pub alphabet: Alphabet,
    pub case: Case,
}

impl Symbol {
    pub fn latin(ch: char, case: Case) -> Self {
        let letter = match case {
            Case::Upper => ch.to_ascii_uppercase().to_string(),
            Case::Lower => ch.to_ascii_lowercase().to_string(),
        };
        Self {
            letter,
            alphabet: Alphabet::Latin,
            case,
        }
    }

    pub fn greek(letter: impl Into<String>, case: Case) -> Self {
        Self {
            letter: letter.into(),
            alphabet: Alphabet::Greek,
            case,
        }
    }

    pub fn cyrillic(letter: impl Into<String>, case: Case) -> Self {
        Self {
            letter: letter.into(),
            alphabet: Alphabet::Cyrillic,
            case,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Alphabet {
    Latin,
    Greek,
    Cyrillic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Case {
    Upper,
    Lower,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    PlusMinus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupKind {
    Paren,
    Bracket,
    Brace,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnitExpr {
    pub factors: Vec<UnitFactor>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnitFactor {
    pub symbol: String,
    pub power: i32,
    pub divide: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnresolvedSpan {
    pub text: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Warning {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Domain {
    Auto,
    Chemistry,
    Mathematics,
    Physics,
    Plain,
}

impl Domain {
    pub fn as_str(self) -> &'static str {
        match self {
            Domain::Auto => "auto",
            Domain::Chemistry => "chemistry",
            Domain::Mathematics => "mathematics",
            Domain::Physics => "physics",
            Domain::Plain => "plain",
        }
    }
}

impl std::str::FromStr for Domain {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(Domain::Auto),
            "chemistry" | "chem" | "химия" => Ok(Domain::Chemistry),
            "mathematics" | "math" | "математика" => Ok(Domain::Mathematics),
            "physics" | "phys" | "физика" => Ok(Domain::Physics),
            "plain" | "text" => Ok(Domain::Plain),
            other => Err(format!("unknown domain '{other}'")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Renderer {
    Unicode,
    Latex,
    Omml,
}

impl std::str::FromStr for Renderer {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "unicode" | "plain" => Ok(Renderer::Unicode),
            "latex" | "tex" => Ok(Renderer::Latex),
            "omml" | "word" => Ok(Renderer::Omml),
            other => Err(format!("unknown renderer '{other}'")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InterpretationResult {
    pub ast: Node,
    pub raw_transcript: String,
    pub normalized_transcript: String,
    pub domain: Domain,
    pub confidence: f32,
    pub unresolved_spans: Vec<UnresolvedSpan>,
    pub warnings: Vec<Warning>,
    pub alternatives: Vec<Node>,
}

impl InterpretationResult {
    pub fn failed_raw(raw: &str, normalized: &str, domain: Domain, reason: &str) -> Self {
        Self {
            ast: Node::Text(raw.to_string()),
            raw_transcript: raw.to_string(),
            normalized_transcript: normalized.to_string(),
            domain,
            confidence: 0.0,
            unresolved_spans: vec![UnresolvedSpan {
                text: raw.to_string(),
                reason: reason.to_string(),
            }],
            warnings: vec![Warning {
                code: "unresolved".into(),
                message: reason.to_string(),
            }],
            alternatives: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atom_counts_reports_overflow_instead_of_wrapping() {
        // Three nested groups each multiplying by u32::MAX compound to well
        // past u64::MAX; this must surface as `None`, never a silently
        // wrapped (and therefore wrong) atom count.
        let deeply_nested = Formula {
            parts: vec![Part::Group {
                inner: Formula {
                    parts: vec![Part::Group {
                        inner: Formula {
                            parts: vec![Part::Group {
                                inner: Formula::atom("X", u32::MAX),
                                count: u32::MAX,
                            }],
                        },
                        count: u32::MAX,
                    }],
                },
                count: u32::MAX,
            }],
        };
        assert_eq!(deeply_nested.atom_counts(), None);
    }

    fn sym(letter: char) -> Math {
        Math::Symbol(Symbol::latin(letter, Case::Lower))
    }

    fn round_trip(math: &Math) -> Math {
        let json = serde_json::to_string(math).expect("a Math node must serialise");
        serde_json::from_str(&json).expect("a serialised Math node must deserialise")
    }

    #[test]
    fn ordinary_derivative_survives_serde() {
        let node = Math::derivative(
            DerivativeKind::Ordinary,
            sym('f'),
            vec![DerivativeVariable::new(sym('x'), 1)],
        )
        .expect("a first derivative is well formed");
        assert_eq!(round_trip(&node), node);
    }

    #[test]
    fn higher_order_derivative_survives_serde() {
        let node = Math::derivative(
            DerivativeKind::Ordinary,
            sym('y'),
            vec![DerivativeVariable::new(sym('x'), 2)],
        )
        .expect("a second derivative is well formed");
        assert_eq!(round_trip(&node), node);
        assert_eq!(
            derivative_total_order(match &node {
                Math::Derivative { variables, .. } => variables,
                other => panic!("{other:?}"),
            }),
            Some(2)
        );
    }

    #[test]
    fn mixed_partial_keeps_one_entry_per_variable() {
        let node = Math::derivative(
            DerivativeKind::Partial,
            Math::Symbol(Symbol::latin('T', Case::Upper)),
            vec![
                DerivativeVariable::new(sym('x'), 1),
                DerivativeVariable::new(sym('y'), 1),
            ],
        )
        .expect("a mixed second partial is well formed");
        match &node {
            Math::Derivative { variables, .. } => {
                // Structurally two variables, not one fused `dxdy` string.
                assert_eq!(variables.len(), 2);
                assert_eq!(*variables[0].variable, sym('x'));
                assert_eq!(*variables[1].variable, sym('y'));
                assert_eq!(derivative_total_order(variables), Some(2));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(round_trip(&node), node);
    }

    #[test]
    fn two_sided_and_one_sided_limits_survive_serde() {
        for direction in [
            LimitDirection::TwoSided,
            LimitDirection::FromLeft,
            LimitDirection::FromRight,
        ] {
            let node = Math::limit(
                sym('x'),
                Math::Number("0".into()),
                direction,
                Math::Function {
                    kind: FunctionKind::Sin,
                    arg: Box::new(sym('x')),
                },
            );
            assert_eq!(round_trip(&node), node);
            match &node {
                Math::Limit {
                    direction: kept, ..
                } => assert_eq!(*kept, direction),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn an_unfilled_derivative_cannot_be_built() {
        assert_eq!(
            Math::derivative(DerivativeKind::Ordinary, sym('f'), vec![]),
            Err(DerivativeDefect::NoVariables)
        );
    }

    #[test]
    fn a_zero_order_derivative_cannot_be_built() {
        assert_eq!(
            Math::derivative(
                DerivativeKind::Ordinary,
                sym('f'),
                vec![DerivativeVariable::new(sym('x'), 0)]
            ),
            Err(DerivativeDefect::ZeroOrder)
        );
    }

    #[test]
    fn an_order_above_the_cap_cannot_be_built() {
        assert_eq!(
            Math::derivative(
                DerivativeKind::Ordinary,
                sym('f'),
                vec![DerivativeVariable::new(sym('x'), MAX_DERIVATIVE_ORDER + 1)]
            ),
            Err(DerivativeDefect::OrderTooHigh)
        );
        // Exactly at the cap is still accepted.
        assert!(Math::derivative(
            DerivativeKind::Ordinary,
            sym('f'),
            vec![DerivativeVariable::new(sym('x'), MAX_DERIVATIVE_ORDER)]
        )
        .is_ok());
    }

    #[test]
    fn a_total_order_above_the_cap_cannot_be_built() {
        let variables: Vec<_> = "abcdefghijklmnopq"
            .chars()
            .map(|letter| DerivativeVariable::new(sym(letter), 1))
            .collect();
        assert_eq!(variables.len() as u32, MAX_DERIVATIVE_ORDER + 1);
        assert_eq!(
            Math::derivative(DerivativeKind::Partial, sym('f'), variables),
            Err(DerivativeDefect::TotalOrderTooHigh)
        );
    }

    #[test]
    fn total_order_reports_overflow_instead_of_wrapping() {
        // Two orders that sum past u32::MAX must surface as `None`, never as
        // a wrapped (and therefore plausible-looking, small) total.
        let variables = vec![
            DerivativeVariable::new(sym('x'), u32::MAX),
            DerivativeVariable::new(sym('y'), 2),
        ];
        assert_eq!(derivative_total_order(&variables), None);
        // And a node built by hand out of them is diagnosed, not accepted.
        assert_eq!(
            derivative_defect(&variables),
            Some(DerivativeDefect::OrderTooHigh)
        );
    }

    #[test]
    fn atom_counts_handles_ordinary_formulas() {
        let f = Formula {
            parts: vec![
                Part::Atom {
                    symbol: "Cu".into(),
                    count: 1,
                },
                Part::Group {
                    inner: Formula::atom("O", 1),
                    count: 1,
                },
                Part::Hydrate { count: 5 },
            ],
        };
        let counts = f.atom_counts().unwrap();
        assert_eq!(counts.get("Cu"), Some(&1));
        assert_eq!(counts.get("O"), Some(&(1 + 5)));
        assert_eq!(counts.get("H"), Some(&10));
    }
}

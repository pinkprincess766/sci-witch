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
    pub fn atom_counts(&self) -> BTreeMap<String, u64> {
        let mut atoms = BTreeMap::new();
        Self::collect_atoms(self, 1, &mut atoms);
        atoms
    }

    fn collect_atoms(formula: &Formula, multiplier: u64, atoms: &mut BTreeMap<String, u64>) {
        for part in &formula.parts {
            match part {
                Part::Atom { symbol, count } => {
                    *atoms.entry(symbol.clone()).or_default() += multiplier * u64::from(*count);
                }
                Part::Group { inner, count } => {
                    Self::collect_atoms(inner, multiplier * u64::from(*count), atoms);
                }
                Part::Hydrate { count } => {
                    let waters = multiplier * u64::from(*count);
                    *atoms.entry("H".into()).or_default() += waters * 2;
                    *atoms.entry("O".into()).or_default() += waters;
                }
            }
        }
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
    Unit(UnitExpr),
    Infinity,
    Ellipsis,
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

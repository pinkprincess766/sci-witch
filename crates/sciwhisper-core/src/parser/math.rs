use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::ast::{
    Alphabet, BinOp, Case, DerivativeKind, DerivativeVariable, FunctionKind, GroupKind,
    LimitDirection, Math, Node, Symbol, UnitExpr, UnitFactor, MAX_DERIVATIVE_ORDER,
};
use crate::error::{Error, Result};
use crate::lexicon::Lexicon;
use crate::numbers::NumberLex;

const OPERATORS_YAML: &str = include_str!("../../data/domains/mathematics/operators.yaml");
const SUPPORTED_OPERATOR_SCHEMA: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(String),
    Sym(Symbol),
    Unit(String),
    Plus,
    Minus,
    Times,
    Div,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    PlusMinus,
    Squared,
    Cubed,
    Degree,
    FracStart,
    Numer,
    Denom,
    FracEnd,
    PowStart,
    PowEnd,
    LParen,
    RParen,
    LBrack,
    RBrack,
    LBrace,
    RBrace,
    Root,
    RootStart,
    RootEnd,
    Sum,
    Product,
    Integral,
    From,
    To,
    By,
    SumEnd,
    ProdEnd,
    IntEnd,
    Fact,
    AbsKw,
    Function(FunctionKind),
    VectorKw,
    SubKw,
    Inf,
    Ellipsis,
    Comma,
    Delta,
    Derivative,
    Partial,
    OrderKw,
    Ordinal(u32),
    Limit,
    LimitLeft,
    LimitRight,
    LimitVar,
    Tends,
    AndBy,
    FuncFiller,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathMode {
    Math,
    Physics,
}

pub struct MathParse {
    pub ast: Math,
    pub alternatives: Vec<Math>,
    pub warnings: Vec<String>,
}

pub fn parse_math(
    words: &[String],
    lex: &Lexicon,
    nums: &NumberLex,
    mode: MathMode,
) -> Result<MathParse> {
    let toks = tokenize(words, lex, nums, mode)?;
    if toks.is_empty() {
        return Err(Error::Parse {
            domain: "mathematics",
            reason: "empty input".into(),
        });
    }
    let mut p = Parser {
        toks: &toks,
        i: 0,
        warnings: Vec::new(),
        alternatives: Vec::new(),
        stop_at_differential: false,
    };
    let ast = p.parse_eq()?;
    p.skip_commas();
    if p.i < p.toks.len() {
        return Err(Error::Parse {
            domain: "mathematics",
            reason: format!("trailing tokens from {:?}", p.peek()),
        });
    }
    Ok(MathParse {
        ast,
        alternatives: p.alternatives,
        warnings: p.warnings,
    })
}

pub fn parse_math_node(
    words: &[String],
    lex: &Lexicon,
    nums: &NumberLex,
    mode: MathMode,
) -> Result<Node> {
    Ok(Node::Math(parse_math(words, lex, nums, mode)?.ast))
}

struct Parser<'a> {
    toks: &'a [Tok],
    i: usize,
    warnings: Vec<String>,
    alternatives: Vec<Math>,
    stop_at_differential: bool,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a Tok> {
        self.toks.get(self.i)
    }
    fn bump(&mut self) -> Option<&'a Tok> {
        let t = self.toks.get(self.i);
        if t.is_some() {
            self.i += 1;
        }
        t
    }
    fn eat(&mut self, want: fn(&Tok) -> bool) -> bool {
        if self.peek().is_some_and(want) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn skip_commas(&mut self) {
        while self.eat(|t| matches!(t, Tok::Comma)) {}
    }

    fn parse_eq(&mut self) -> Result<Math> {
        let mut left = self.parse_add()?;
        loop {
            self.skip_commas();
            let op = match self.peek() {
                Some(Tok::Eq) => BinOp::Eq,
                Some(Tok::Ne) => BinOp::Ne,
                Some(Tok::Lt) => BinOp::Lt,
                Some(Tok::Gt) => BinOp::Gt,
                Some(Tok::Le) => BinOp::Le,
                Some(Tok::Ge) => BinOp::Ge,
                _ => break,
            };
            self.bump();
            let right = self.parse_add()?;
            left = Math::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<Math> {
        let mut left = self.parse_mul()?;
        loop {
            self.skip_commas();
            let op = match self.peek() {
                Some(Tok::Plus) => BinOp::Add,
                Some(Tok::Minus) => BinOp::Sub,
                Some(Tok::PlusMinus) => BinOp::PlusMinus,
                _ => break,
            };
            self.bump();
            let right = self.parse_mul()?;
            left = Math::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Math> {
        let mut left = self.parse_unary()?;
        loop {
            self.skip_commas();
            let op = match self.peek() {
                Some(Tok::Times) => BinOp::Mul,
                Some(Tok::Div) => BinOp::Div,
                _ => break,
            };
            self.bump();
            let right = self.parse_unary()?;
            left = Math::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Math> {
        self.skip_commas();
        if self.eat(|t| matches!(t, Tok::Minus)) {
            let inner = self.parse_unary()?;
            return Ok(Math::UnaryMinus(Box::new(inner)));
        }
        if self.eat(|t| matches!(t, Tok::Plus)) {
            return self.parse_unary();
        }
        self.parse_juxt()
    }

    fn parse_juxt(&mut self) -> Result<Math> {
        let first = self.parse_postfix()?;
        let mut items = vec![first];
        while self.starts_atom() {
            items.push(self.parse_postfix()?);
        }
        if items.len() == 1 {
            Ok(items.pop().unwrap())
        } else {
            Ok(Math::Juxt(items))
        }
    }

    fn starts_atom(&self) -> bool {
        if self.stop_at_differential && self.is_differential_here() {
            return false;
        }
        Self::atom_token_is_supported(self.peek())
    }

    fn is_differential_here(&self) -> bool {
        let Some(Tok::Sym(symbol)) = self.peek() else {
            return false;
        };
        symbol.alphabet == Alphabet::Latin
            && symbol.letter.eq_ignore_ascii_case("d")
            && Self::atom_token_is_supported(self.toks.get(self.i + 1))
    }

    fn parse_function(&mut self, kind: FunctionKind) -> Result<Math> {
        let arg = self.parse_postfix()?;
        Ok(Math::Function {
            kind,
            arg: Box::new(arg),
        })
    }

    fn atom_token_is_supported(token: Option<&Tok>) -> bool {
        matches!(
            token,
            Some(
                Tok::Num(_)
                    | Tok::Sym(_)
                    | Tok::Unit(_)
                    | Tok::LParen
                    | Tok::LBrack
                    | Tok::LBrace
                    | Tok::FracStart
                    | Tok::Root
                    | Tok::RootStart
                    | Tok::Sum
                    | Tok::Product
                    | Tok::Integral
                    | Tok::Fact
                    | Tok::AbsKw
                    | Tok::Function(_)
                    | Tok::VectorKw
                    | Tok::Delta
                    | Tok::Inf
                    | Tok::Ellipsis
                    | Tok::PowStart
                    // Only the construct heads. «вторая», «частная»,
                    // «слева» and «справа» are prefixes: they are handled
                    // where a construct starts, and must not pull the
                    // juxtaposition loop into a half-formed derivative.
                    | Tok::Derivative
                    | Tok::Limit
            )
        )
    }

    fn parse_postfix(&mut self) -> Result<Math> {
        let mut inner = self.parse_atom()?;
        loop {
            match self.peek() {
                Some(Tok::Squared) => {
                    self.bump();
                    inner = Math::Power {
                        base: Box::new(inner),
                        exp: Box::new(Math::Number("2".into())),
                    };
                }
                Some(Tok::Cubed) => {
                    self.bump();
                    inner = Math::Power {
                        base: Box::new(inner),
                        exp: Box::new(Math::Number("3".into())),
                    };
                }
                Some(Tok::Fact) => {
                    self.bump();
                    inner = Math::Factorial(Box::new(inner));
                }
                Some(Tok::Degree) => {
                    self.bump();
                    let exp = self.parse_atom()?;
                    inner = Math::Power {
                        base: Box::new(inner),
                        exp: Box::new(exp),
                    };
                }
                Some(Tok::PowStart) => {
                    self.bump();
                    let exp = self.parse_add()?;
                    if !self.eat(|t| matches!(t, Tok::PowEnd)) {
                        self.warnings
                            .push("unclosed power; inserting anyway".into());
                    }
                    inner = Math::Power {
                        base: Box::new(inner),
                        exp: Box::new(exp),
                    };
                }
                Some(Tok::SubKw) => {
                    self.bump();
                    let sub = self.parse_atom()?;
                    inner = Math::Subscript {
                        base: Box::new(inner),
                        sub: Box::new(sub),
                    };
                }
                Some(Tok::Unit(_)) => {
                    let unit = self.parse_unit_expr()?;
                    inner = Math::Juxt(vec![inner, Math::Unit(unit)]);
                }
                _ => break,
            }
        }
        Ok(inner)
    }

    fn parse_atom(&mut self) -> Result<Math> {
        self.skip_commas();
        match self.bump() {
            Some(Tok::Num(n)) => {
                let mut node = Math::Number(n.clone());
                if matches!(self.peek(), Some(Tok::Unit(_))) {
                    let unit = self.parse_unit_expr()?;
                    node = Math::Juxt(vec![node, Math::Unit(unit)]);
                }
                Ok(node)
            }
            Some(Tok::Sym(s)) => Ok(Math::Symbol(s.clone())),
            Some(Tok::Inf) => Ok(Math::Infinity),
            Some(Tok::Ellipsis) => Ok(Math::Ellipsis),
            Some(Tok::Delta) => {
                if matches!(self.peek(), Some(Tok::Num(_))) {
                    self.warnings.push(
                        "ambiguous 'delta <number>'; say 'delta lower index', 'delta multiplied by', or 'delta to the power'"
                            .into(),
                    );
                    return Ok(Math::Symbol(Symbol::greek("δ", Case::Lower)));
                }
                if !self.starts_atom() {
                    return Ok(Math::Symbol(Symbol::greek("δ", Case::Lower)));
                }
                let inner = self.parse_postfix()?;
                let inner = match inner {
                    Math::Symbol(s) if s.alphabet == Alphabet::Latin && s.letter.len() == 1 => {
                        Math::Symbol(Symbol::latin(s.letter.chars().next().unwrap(), Case::Upper))
                    }
                    other => other,
                };
                Ok(Math::Delta(Box::new(inner)))
            }
            Some(Tok::VectorKw) => {
                let inner = self.parse_postfix()?;
                Ok(Math::Vector(Box::new(inner)))
            }
            Some(Tok::Fact) => {
                let inner = self.parse_juxt()?;
                Ok(Math::Factorial(Box::new(inner)))
            }
            Some(Tok::AbsKw) => {
                let inner = self.parse_postfix()?;
                Ok(Math::Abs(Box::new(inner)))
            }
            Some(Tok::Function(kind)) => self.parse_function(*kind),
            Some(Tok::LParen) => {
                let inner = self.parse_eq()?;
                if !self.eat(|t| matches!(t, Tok::RParen)) {
                    self.warnings.push("unclosed parenthesis".into());
                }
                Ok(Math::Group {
                    kind: GroupKind::Paren,
                    inner: Box::new(inner),
                })
            }
            Some(Tok::LBrack) => {
                let inner = self.parse_eq()?;
                let _ = self.eat(|t| matches!(t, Tok::RBrack));
                Ok(Math::Group {
                    kind: GroupKind::Bracket,
                    inner: Box::new(inner),
                })
            }
            Some(Tok::LBrace) => {
                let inner = self.parse_eq()?;
                let _ = self.eat(|t| matches!(t, Tok::RBrace));
                Ok(Math::Group {
                    kind: GroupKind::Brace,
                    inner: Box::new(inner),
                })
            }
            Some(Tok::FracStart) => self.parse_fraction(),
            Some(Tok::RootStart) => {
                let rad = self.parse_add()?;
                if !self.eat(|t| matches!(t, Tok::RootEnd)) {
                    self.warnings.push("unclosed root".into());
                }
                Ok(Math::Root {
                    index: None,
                    radicand: Box::new(rad),
                })
            }
            Some(Tok::Root) => {
                let rad = self.parse_postfix()?;
                if matches!(self.peek(), Some(Tok::Plus | Tok::Minus)) {
                    // Ambiguous natural-speech root: default binds only the atom.
                    // Offer the grouping alternative for preview.
                    let saved = self.i;
                    // reconstruct alternative sqrt(atom ± rest) at higher grouping
                    // We only record a note; interpret.rs may also inspect.
                    let _ = saved;
                    self.warnings.push(
                        "root without end command binds the next atom only; use «начало корня» … «конец корня» for x+1 under the radical"
                            .into(),
                    );
                }
                Ok(Math::Root {
                    index: None,
                    radicand: Box::new(rad),
                })
            }
            Some(Tok::Derivative) => self.parse_derivative(DerivativeKind::Ordinary, None),
            Some(Tok::Partial) => {
                if !self.eat(|t| matches!(t, Tok::Derivative)) {
                    return Err(Error::Parse {
                        domain: "mathematics",
                        reason: "«частная» without «производная»".into(),
                    });
                }
                self.parse_derivative(DerivativeKind::Partial, None)
            }
            Some(Tok::Ordinal(order)) => {
                let order = *order;
                let kind = if self.eat(|t| matches!(t, Tok::Partial)) {
                    DerivativeKind::Partial
                } else {
                    DerivativeKind::Ordinary
                };
                if !self.eat(|t| matches!(t, Tok::Derivative)) {
                    return Err(Error::Parse {
                        domain: "mathematics",
                        reason: "an ordinal here only names a derivative order".into(),
                    });
                }
                self.parse_derivative(kind, Some(order))
            }
            Some(Tok::Limit) => self.parse_limit(None),
            Some(Tok::LimitLeft) => {
                if !self.eat(|t| matches!(t, Tok::Limit)) {
                    return Err(Error::Parse {
                        domain: "mathematics",
                        reason: "«слева» without «предел»".into(),
                    });
                }
                self.parse_limit(Some(LimitDirection::FromLeft))
            }
            Some(Tok::LimitRight) => {
                if !self.eat(|t| matches!(t, Tok::Limit)) {
                    return Err(Error::Parse {
                        domain: "mathematics",
                        reason: "«справа» without «предел»".into(),
                    });
                }
                self.parse_limit(Some(LimitDirection::FromRight))
            }
            Some(Tok::Sum) => self.parse_nary(Nary::Sum),
            Some(Tok::Product) => self.parse_nary(Nary::Product),
            Some(Tok::Integral) => self.parse_integral(),
            Some(other) => Err(Error::Parse {
                domain: "mathematics",
                reason: format!("unexpected token {other:?}"),
            }),
            None => Err(Error::Parse {
                domain: "mathematics",
                reason: "unexpected end".into(),
            }),
        }
    }

    fn parse_fraction(&mut self) -> Result<Math> {
        let _ = self.eat(|t| matches!(t, Tok::Numer));
        let num = self.parse_add()?;
        if !self.eat(|t| matches!(t, Tok::Denom)) {
            return Err(Error::Parse {
                domain: "mathematics",
                reason: "fraction missing знаменатель".into(),
            });
        }
        let den = self.parse_add()?;
        let _ = self.eat(|t| matches!(t, Tok::FracEnd));
        Ok(Math::Fraction {
            num: Box::new(num),
            den: Box::new(den),
        })
    }

    fn parse_nary(&mut self, kind: Nary) -> Result<Math> {
        let mut var = None;
        let mut from = None;
        let mut to = None;
        if self.eat(|t| matches!(t, Tok::From)) {
            let e = self.parse_eq()?;
            match e {
                Math::Binary {
                    op: BinOp::Eq,
                    left,
                    right,
                } => {
                    var = Some(left);
                    from = Some(right);
                }
                other => from = Some(Box::new(other)),
            }
        }
        if self.eat(|t| matches!(t, Tok::To)) {
            to = Some(Box::new(self.parse_postfix()?));
        }
        let end = match kind {
            Nary::Sum => |t: &Tok| {
                matches!(
                    t,
                    Tok::SumEnd | Tok::Plus | Tok::Minus | Tok::Eq | Tok::Comma
                )
            },
            Nary::Product => |t: &Tok| {
                matches!(
                    t,
                    Tok::ProdEnd | Tok::Plus | Tok::Minus | Tok::Eq | Tok::Comma
                )
            },
        };
        let body = if self.peek().is_some() && !self.peek().is_some_and(end) && self.starts_atom() {
            Some(Box::new(self.parse_mul()?))
        } else {
            None
        };
        match kind {
            Nary::Sum => {
                let _ = self.eat(|t| matches!(t, Tok::SumEnd));
                Ok(Math::Sum {
                    var,
                    from,
                    to,
                    body,
                })
            }
            Nary::Product => {
                let _ = self.eat(|t| matches!(t, Tok::ProdEnd));
                Ok(Math::Product {
                    var,
                    from,
                    to,
                    body,
                })
            }
        }
    }

    fn parse_integral(&mut self) -> Result<Math> {
        let mut from = None;
        let mut to = None;
        if self.eat(|t| matches!(t, Tok::From)) {
            let after_from = self.i;
            let possible_lower_bound = self.parse_postfix()?;
            if self.eat(|t| matches!(t, Tok::To)) {
                from = Some(Box::new(possible_lower_bound));
                to = Some(Box::new(self.parse_postfix()?));
            } else {
                // In natural Russian, «интеграл от f dx» means the integral
                // of a function, not a lower bound. A bound is accepted only
                // as the complete paired construction «от ... до ...».
                self.i = after_from;
            }
        }
        self.stop_at_differential = true;
        let integrand = if self.starts_atom() {
            let parsed = self.parse_mul()?;
            self.stop_at_differential = false;
            Some(Box::new(parsed))
        } else {
            self.stop_at_differential = false;
            None
        };
        let wrt = if self.is_differential_here() {
            self.bump();
            Some(Box::new(self.parse_postfix()?))
        } else if self.eat(|t| matches!(t, Tok::By)) {
            Some(Box::new(self.parse_postfix()?))
        } else {
            None
        };
        let _ = self.eat(|t| matches!(t, Tok::IntEnd));
        if integrand.is_none() {
            self.warnings
                .push("integral has no integrand; keep raw text or dictate the expression".into());
        }
        Ok(Math::Integral {
            from,
            to,
            integrand,
            wrt,
        })
    }

    /// `[<ordinal>] [частная] производная [<ordinal> порядка] <expr> по <var> (и по <var>)*`
    ///
    /// Records the structure only. No symbolic differentiation happens here
    /// or anywhere else: `d/dx` of `x²` stays `d(x²)/dx`.
    fn parse_derivative(
        &mut self,
        kind: DerivativeKind,
        prefix_order: Option<u32>,
    ) -> Result<Math> {
        let mut order = prefix_order;
        // «производная третьего порядка …» — the order may also follow the
        // noun. Both spellings mean the same thing, so they must agree.
        if let (Some(Tok::Ordinal(spoken)), Some(Tok::OrderKw)) =
            (self.peek(), self.toks.get(self.i + 1))
        {
            let spoken = *spoken;
            self.i += 2;
            if order.is_some_and(|prefix| prefix != spoken) {
                return Err(Error::Parse {
                    domain: "mathematics",
                    reason: "the derivative order is stated twice and the two disagree".into(),
                });
            }
            order = Some(spoken);
        }
        // «производная от эф по икс», «производная функции эф по икс».
        let _ = self.eat(|t| matches!(t, Tok::From));
        let _ = self.eat(|t| matches!(t, Tok::FuncFiller));
        if !self.starts_atom() {
            return Err(Error::Parse {
                domain: "mathematics",
                reason: "derivative without an expression".into(),
            });
        }
        let expr = self.parse_mul()?;
        let mut spoken_variables = Vec::new();
        loop {
            self.skip_commas();
            if !self.eat(|t| matches!(t, Tok::By | Tok::AndBy)) {
                break;
            }
            if !self.starts_atom() {
                return Err(Error::Parse {
                    domain: "mathematics",
                    reason: "derivative without a variable after «по»".into(),
                });
            }
            spoken_variables.push(self.parse_postfix()?);
            if spoken_variables.len() > MAX_DERIVATIVE_ORDER as usize {
                return Err(Error::Parse {
                    domain: "mathematics",
                    reason: "too many variables of differentiation".into(),
                });
            }
        }
        let variables = distribute_order(spoken_variables, order)?;
        Math::derivative(kind, expr, variables).map_err(|defect| Error::Parse {
            domain: "mathematics",
            reason: defect.message(),
        })
    }

    /// `[слева|справа] предел [слева|справа] [функции] [<body>] при <var>
    /// стремящемся к <target> [слева|справа] [<body>]`
    ///
    /// The body may be dictated before or after the approach clause; a
    /// construct missing the variable, the target or the body is an error,
    /// so the transcript survives verbatim instead of becoming a formula
    /// nobody said.
    fn parse_limit(&mut self, prefix_direction: Option<LimitDirection>) -> Result<Math> {
        let mut direction = prefix_direction.unwrap_or(LimitDirection::TwoSided);
        if let Some(spoken) = self.eat_direction() {
            if prefix_direction.is_some_and(|prefix| prefix != spoken) {
                return Err(Error::Parse {
                    domain: "mathematics",
                    reason: "the limit is said to be one-sided in both directions".into(),
                });
            }
            direction = spoken;
        }
        let _ = self.eat(|t| matches!(t, Tok::FuncFiller));
        let _ = self.eat(|t| matches!(t, Tok::From));
        self.skip_commas();
        let mut body = if self.starts_atom() {
            Some(self.parse_mul()?)
        } else {
            None
        };
        if !self.eat(|t| matches!(t, Tok::LimitVar)) {
            return Err(Error::Parse {
                domain: "mathematics",
                reason: "limit without «при <переменная>»".into(),
            });
        }
        if !self.starts_atom() {
            return Err(Error::Parse {
                domain: "mathematics",
                reason: "limit without a variable".into(),
            });
        }
        let variable = self.parse_postfix()?;
        // «предел при икс, стремящемся к нулю» — Whisper puts a comma there
        // and a comma is a pause, not the end of the construct.
        self.skip_commas();
        if !self.eat(|t| matches!(t, Tok::Tends)) {
            return Err(Error::Parse {
                domain: "mathematics",
                reason: "limit without «стремящемся к»".into(),
            });
        }
        let target = self.parse_limit_target()?;
        if let Some(spoken) = self.eat_direction() {
            if direction != LimitDirection::TwoSided && direction != spoken {
                return Err(Error::Parse {
                    domain: "mathematics",
                    reason: "the limit is said to be one-sided in both directions".into(),
                });
            }
            direction = spoken;
        }
        self.skip_commas();
        if body.is_none() && self.starts_atom() {
            body = Some(self.parse_mul()?);
        }
        let Some(body) = body else {
            return Err(Error::Parse {
                domain: "mathematics",
                reason: "limit without an expression".into(),
            });
        };
        Ok(Math::limit(variable, target, direction, body))
    }

    fn eat_direction(&mut self) -> Option<LimitDirection> {
        match self.peek() {
            Some(Tok::LimitLeft) => {
                self.bump();
                Some(LimitDirection::FromLeft)
            }
            Some(Tok::LimitRight) => {
                self.bump();
                Some(LimitDirection::FromRight)
            }
            _ => None,
        }
    }

    /// The approached point: a signed atom, so «минус бесконечность» is a
    /// target and not the start of the body.
    fn parse_limit_target(&mut self) -> Result<Math> {
        if self.eat(|t| matches!(t, Tok::Minus)) {
            return Ok(Math::UnaryMinus(Box::new(self.parse_limit_target()?)));
        }
        let _ = self.eat(|t| matches!(t, Tok::Plus));
        if !self.starts_atom() {
            return Err(Error::Parse {
                domain: "mathematics",
                reason: "limit without a target point".into(),
            });
        }
        self.parse_postfix()
    }

    fn parse_unit_expr(&mut self) -> Result<UnitExpr> {
        let mut factors = Vec::new();
        let mut divide = false;
        while let Some(Tok::Unit(sym)) = self.peek() {
            let symbol = sym.clone();
            self.bump();
            let mut power = 1i32;
            if self.eat(|t| matches!(t, Tok::Squared)) {
                power = 2;
            } else if self.eat(|t| matches!(t, Tok::Cubed)) {
                power = 3;
            }
            factors.push(UnitFactor {
                symbol,
                power,
                divide,
            });
            if self.eat(|t| matches!(t, Tok::Div)) {
                divide = true;
                continue;
            }
            break;
        }
        if factors.is_empty() {
            return Err(Error::Parse {
                domain: "physics",
                reason: "expected unit".into(),
            });
        }
        Ok(UnitExpr { factors })
    }
}

enum Nary {
    Sum,
    Product,
}

/// Turns a spoken total order into per-variable orders.
///
/// One variable carries the whole order (`d²y/dx²`). Several variables are
/// accepted only when the stated total equals their count, which is the one
/// unambiguous reading — «второго порядка по икс и по игрек» is `∂²T/∂x∂y`.
/// Any other split (an order of 3 over two variables) has more than one
/// meaning, so it is refused instead of guessed.
fn distribute_order(variables: Vec<Math>, order: Option<u32>) -> Result<Vec<DerivativeVariable>> {
    let refuse = |reason: &'static str| Error::Parse {
        domain: "mathematics",
        reason: reason.into(),
    };
    if variables.is_empty() {
        return Err(refuse("derivative without a variable after «по»"));
    }
    let count = u32::try_from(variables.len())
        .map_err(|_| refuse("too many variables of differentiation"))?;
    let orders: Vec<u32> = match order {
        None => vec![1; variables.len()],
        Some(0) => return Err(refuse("a derivative order of zero has no meaning")),
        Some(total) if total > MAX_DERIVATIVE_ORDER => {
            return Err(refuse("the derivative order is above the supported limit"))
        }
        Some(total) if variables.len() == 1 => vec![total],
        Some(total) if total == count => vec![1; variables.len()],
        Some(_) => {
            return Err(refuse(
                "the stated order does not split unambiguously over the variables",
            ))
        }
    };
    Ok(variables
        .into_iter()
        .zip(orders)
        .map(|(variable, order)| DerivativeVariable::new(variable, order))
        .collect())
}

fn tokenize(words: &[String], lex: &Lexicon, nums: &NumberLex, mode: MathMode) -> Result<Vec<Tok>> {
    let mut i = 0;
    let mut out = Vec::new();
    while i < words.len() {
        if words[i] == "," {
            out.push(Tok::Comma);
            i += 1;
            continue;
        }
        if i + 2 < words.len()
            && matches!(words[i].as_str(), "в" | "во")
            && words[i + 2] == "степени"
        {
            if let Some(power) = nums.ordinal(&words[i + 1]) {
                out.push(Tok::Degree);
                out.push(Tok::Num(power.to_string()));
                i += 3;
                continue;
            }
        }
        if let Some((tokens, n)) = match_keyword(&words[i..]) {
            i += n;
            out.extend(tokens);
            continue;
        }
        // «метра в секунду»: between two units a bare «в» is a division. The
        // rule is deliberately narrow — a unit must already be on the stack
        // and another must follow — so «в квадрате» and «в» as the letter v
        // are untouched.
        if mode == MathMode::Physics
            && matches!(words[i].as_str(), "в" | "во")
            && matches!(out.last(), Some(Tok::Unit(_)))
            && lex.longest_unit(words, i + 1).is_some()
        {
            out.push(Tok::Div);
            i += 1;
            continue;
        }
        // A bare ordinal («вторая», «третьего») is only meaningful as the
        // order of a derivative. Emitting it as its own token keeps the
        // grammar compositional; anywhere else the parser rejects it, so an
        // ordinary sentence still falls back to raw text.
        if let Some(order) = nums.ordinal(&words[i]) {
            out.push(Tok::Ordinal(order));
            i += 1;
            continue;
        }
        if let Some((num, n)) = nums.consume_number(words, i) {
            // Don't steal a lone number word that is also a letter command? numbers win.
            i += n;
            out.push(Tok::Num(num));
            continue;
        }
        if let Some((sym, n)) = consume_symbol(words, i, lex, mode) {
            i += n;
            out.push(Tok::Sym(sym));
            continue;
        }
        if mode == MathMode::Physics {
            if let Some((u, n)) = lex.longest_unit(words, i) {
                i += n;
                out.push(Tok::Unit(u.symbol));
                continue;
            }
        }
        return Err(Error::Parse {
            domain: "mathematics",
            reason: format!("unknown word '{}'", words[i]),
        });
    }
    Ok(out)
}

fn match_keyword(rest: &[String]) -> Option<(Vec<Tok>, usize)> {
    if let Some(token) = match rest.first().map(String::as_str) {
        Some("-") => Some(Tok::Minus),
        Some("+") => Some(Tok::Plus),
        Some("=") => Some(Tok::Eq),
        Some("/") => Some(Tok::Div),
        Some("*" | "·") => Some(Tok::Times),
        _ => None,
    } {
        return Some((vec![token], 1));
    }

    operator_patterns().iter().find_map(|pattern| {
        (rest.len() >= pattern.words.len()
            && pattern
                .words
                .iter()
                .enumerate()
                .all(|(index, word)| rest[index] == *word))
        .then(|| (pattern.tokens.clone(), pattern.words.len()))
    })
}

#[derive(Debug, Deserialize)]
struct OperatorGrammarYaml {
    schema_version: u32,
    binary: HashMap<String, Vec<String>>,
    unary: HashMap<String, Vec<String>>,
    postfix_power: HashMap<String, Vec<String>>,
    delimiters: HashMap<String, Vec<String>>,
    special: HashMap<String, Vec<String>>,
}

#[derive(Clone, Debug)]
struct OperatorPattern {
    words: Vec<String>,
    tokens: Vec<Tok>,
}

fn operator_patterns() -> &'static [OperatorPattern] {
    static PATTERNS: OnceLock<Vec<OperatorPattern>> = OnceLock::new();
    PATTERNS.get_or_init(load_operator_patterns)
}

fn load_operator_patterns() -> Vec<OperatorPattern> {
    let grammar: OperatorGrammarYaml =
        serde_yaml::from_str(OPERATORS_YAML).expect("embedded operator grammar must be valid");
    assert!(
        grammar.schema_version <= SUPPORTED_OPERATOR_SCHEMA,
        "embedded operator grammar schema is unsupported"
    );

    let mut patterns = Vec::new();
    add_operator_section(&mut patterns, "binary", grammar.binary);
    add_operator_section(&mut patterns, "unary", grammar.unary);
    add_operator_section(&mut patterns, "postfix_power", grammar.postfix_power);
    add_operator_section(&mut patterns, "delimiters", grammar.delimiters);
    add_operator_section(&mut patterns, "special", grammar.special);
    patterns.sort_by_key(|pattern| std::cmp::Reverse(pattern.words.len()));
    patterns
}

fn add_operator_section(
    target: &mut Vec<OperatorPattern>,
    section: &'static str,
    aliases: HashMap<String, Vec<String>>,
) {
    for (name, phrases) in aliases {
        let tokens = operator_tokens(section, &name)
            .unwrap_or_else(|| panic!("unknown operator grammar key {section}.{name}"));
        for phrase in phrases {
            let words = crate::normalize::words(&phrase);
            assert!(!words.is_empty(), "empty operator alias {section}.{name}");
            target.push(OperatorPattern {
                words,
                tokens: tokens.clone(),
            });
        }
    }
}

fn operator_tokens(section: &str, name: &str) -> Option<Vec<Tok>> {
    let token = match (section, name) {
        ("binary", "plus") | ("unary", "plus") => vec![Tok::Plus],
        ("binary", "minus") | ("unary", "minus") => vec![Tok::Minus],
        ("binary", "times") => vec![Tok::Times],
        ("binary", "divide") => vec![Tok::Div],
        ("binary", "equal") => vec![Tok::Eq],
        ("binary", "not_equal") => vec![Tok::Ne],
        ("binary", "lt") => vec![Tok::Lt],
        ("binary", "gt") => vec![Tok::Gt],
        ("binary", "le") => vec![Tok::Le],
        ("binary", "ge") => vec![Tok::Ge],
        ("binary", "plus_minus") => vec![Tok::PlusMinus],
        ("unary", "abs") => vec![Tok::AbsKw],
        ("unary", "factorial") => vec![Tok::Fact],
        ("unary", "sin") => vec![Tok::Function(FunctionKind::Sin)],
        ("unary", "cos") => vec![Tok::Function(FunctionKind::Cos)],
        ("unary", "tan") => vec![Tok::Function(FunctionKind::Tan)],
        ("unary", "cot") => vec![Tok::Function(FunctionKind::Cot)],
        ("unary", "ln") => vec![Tok::Function(FunctionKind::Ln)],
        ("unary", "log") => vec![Tok::Function(FunctionKind::Log)],
        ("unary", "exp") => vec![Tok::Function(FunctionKind::Exp)],
        ("postfix_power", "squared") => vec![Tok::Squared],
        ("postfix_power", "cubed") => vec![Tok::Cubed],
        ("postfix_power", "degree") => vec![Tok::Degree],
        ("postfix_power", "inverse") => vec![Tok::Degree, Tok::Num("-1".into())],
        ("delimiters", "fraction_start") => vec![Tok::FracStart],
        ("delimiters", "numerator") => vec![Tok::Numer],
        ("delimiters", "denominator") => vec![Tok::Denom],
        ("delimiters", "fraction_end") => vec![Tok::FracEnd],
        ("delimiters", "power_start") => vec![Tok::PowStart],
        ("delimiters", "power_end") => vec![Tok::PowEnd],
        ("delimiters", "paren_open") => vec![Tok::LParen],
        ("delimiters", "paren_close") => vec![Tok::RParen],
        ("delimiters", "bracket_open") => vec![Tok::LBrack],
        ("delimiters", "bracket_close") => vec![Tok::RBrack],
        ("delimiters", "brace_open") => vec![Tok::LBrace],
        ("delimiters", "brace_close") => vec![Tok::RBrace],
        ("delimiters", "root_start") => vec![Tok::RootStart],
        ("delimiters", "root_end") => vec![Tok::RootEnd],
        ("delimiters", "root") => vec![Tok::Root],
        ("delimiters", "sum") => vec![Tok::Sum],
        ("delimiters", "product") => vec![Tok::Product],
        ("delimiters", "integral") => vec![Tok::Integral],
        ("delimiters", "from") => vec![Tok::From],
        ("delimiters", "to") => vec![Tok::To],
        ("delimiters", "of_var") => vec![Tok::By],
        ("delimiters", "sum_end") => vec![Tok::SumEnd],
        ("delimiters", "product_end") => vec![Tok::ProdEnd],
        ("delimiters", "integral_end") => vec![Tok::IntEnd],
        ("delimiters", "subscript") => vec![Tok::SubKw],
        ("delimiters", "vector") => vec![Tok::VectorKw],
        ("delimiters", "infinity") => vec![Tok::Inf],
        ("delimiters", "ellipsis") => vec![Tok::Ellipsis],
        ("delimiters", "delta") => vec![Tok::Delta],
        ("delimiters", "comma") => vec![Tok::Comma],
        ("delimiters", "over") => vec![Tok::Div],
        ("delimiters", "derivative") => vec![Tok::Derivative],
        ("delimiters", "partial") => vec![Tok::Partial],
        ("delimiters", "order") => vec![Tok::OrderKw],
        ("delimiters", "and_by") => vec![Tok::AndBy],
        ("delimiters", "limit") => vec![Tok::Limit],
        ("delimiters", "limit_left") => vec![Tok::LimitLeft],
        ("delimiters", "limit_right") => vec![Tok::LimitRight],
        ("delimiters", "limit_var") => vec![Tok::LimitVar],
        ("delimiters", "tends_to") => vec![Tok::Tends],
        ("delimiters", "function_filler") => vec![Tok::FuncFiller],
        ("special", "zero_eq") => vec![Tok::Eq, Tok::Num("0".into())],
        _ => return None,
    };
    Some(token)
}

fn consume_symbol(
    words: &[String],
    i: usize,
    lex: &Lexicon,
    mode: MathMode,
) -> Option<(Symbol, usize)> {
    let w = words[i].as_str();
    let mut used = 1usize;
    let mut alphabet: Option<Alphabet> = None;
    let mut case: Option<Case> = None;
    while i + used < words.len() {
        match words[i + used].as_str() {
            "латинская" | "латинский" | "латинское" => {
                alphabet = Some(Alphabet::Latin);
                used += 1;
            }
            "греческая" | "греческий" | "греческое" => {
                alphabet = Some(Alphabet::Greek);
                used += 1;
            }
            "русская" | "русский" | "русское" | "кириллическая" =>
            {
                alphabet = Some(Alphabet::Cyrillic);
                used += 1;
            }
            "большое" | "большая" | "большой" | "заглавная" | "заглавный" =>
            {
                case = Some(Case::Upper);
                used += 1;
            }
            "малое" | "малая" | "малый" | "строчная" | "строчный" => {
                case = Some(Case::Lower);
                used += 1;
            }
            _ => break,
        }
    }

    let want_greek = alphabet == Some(Alphabet::Greek);
    if want_greek || alphabet.is_none() {
        if let Some(g) = lex.greek(w) {
            let case = case.unwrap_or(Case::Lower);
            let letter = if case == Case::Upper {
                g.upper.clone()
            } else {
                g.lower.clone()
            };
            return Some((Symbol::greek(letter, case), used));
        }
    }
    if alphabet == Some(Alphabet::Cyrillic) {
        if let Some(c) = lex.cyrillic.get(w) {
            let case = case.unwrap_or(Case::Lower);
            let letter = if case == Case::Upper {
                c.to_uppercase()
            } else {
                c.clone()
            };
            return Some((Symbol::cyrillic(letter, case), used));
        }
    }
    if w.chars().count() == 1 {
        let ch = w.chars().next().unwrap();
        if ch.is_ascii_alphabetic() {
            let mut case = case.unwrap_or(if ch.is_ascii_uppercase() {
                Case::Upper
            } else {
                Case::Lower
            });
            if mode == MathMode::Physics && matches!(ch.to_ascii_lowercase(), 'f' | 'e') {
                case = Case::Upper;
            }
            return Some((Symbol::latin(ch, case), used));
        }
    }
    if let Some(ch) = lex.latin(w) {
        let mut case = case.unwrap_or(Case::Lower);
        if mode == MathMode::Physics {
            // Physics defaults for Faraday / force / energy / EMF.
            if matches!(ch, 'f' | 'e') && case == Case::Lower && alphabet.is_none() {
                case = Case::Upper;
            }
            if w == "же" && case == Case::Lower {
                // standalone g; ΔG handled by Delta prefix + upper in render
            }
        }
        let mut sym = Symbol::latin(ch, case);
        if w == "же" && mode == MathMode::Physics {
            // keep as g unless explicitly upper
        }
        if alphabet == Some(Alphabet::Latin) {
            sym.alphabet = Alphabet::Latin;
        }
        return Some((sym, used));
    }
    None
}

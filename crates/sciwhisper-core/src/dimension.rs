//! Conservative SI dimensional analysis over the mathematical AST.
//!
//! The analyser proves dimensions where the units are actually present and
//! abstains everywhere else: a warning appears only when two *proven*
//! dimensions are incompatible. A bare symbol carries no dimension, so
//! `F = ma` stays silent instead of guessing that `F` is a force.
//!
//! Nothing here mutates the AST or the rendered payload — it only appends
//! warnings, exactly like the chemistry balance validator.
//!
//! Exponents are exact rationals (`M L T^-2`, `L^1/2`) with `checked_*`
//! arithmetic: overflow, an unsupported symbolic power or an unknown unit
//! all abstain rather than panic or guess.

use std::fmt;

use crate::ast::{BinOp, Math, UnitExpr, Warning};
use crate::lexicon::Lexicon;

/// Recursion guard. An artificially deep or hand-built AST abstains instead
/// of exhausting the stack.
const MAX_DEPTH: u32 = 128;

/// Base dimensions in fixed order: mass, length, time, current,
/// temperature, amount of substance, luminous intensity.
const BASE_SYMBOLS: [&str; 7] = ["M", "L", "T", "I", "Θ", "N", "J"];

/// An exact rational exponent, normalised so that `den > 0` and
/// `gcd(|num|, den) = 1`. Never a float: `√` needs `1/2`, not `0.5`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exponent {
    num: i32,
    den: i32,
}

impl Exponent {
    pub const ZERO: Exponent = Exponent { num: 0, den: 1 };

    /// `None` on a zero denominator or on overflow while normalising
    /// (`i32::MIN` has no positive negation).
    pub fn new(num: i32, den: i32) -> Option<Self> {
        if den == 0 {
            return None;
        }
        let (num, den) = if den < 0 {
            (num.checked_neg()?, den.checked_neg()?)
        } else {
            (num, den)
        };
        let divisor = i32::try_from(gcd(num.unsigned_abs(), den.unsigned_abs()))
            .ok()?
            .max(1);
        Some(Exponent {
            num: num / divisor,
            den: den / divisor,
        })
    }

    pub fn integer(value: i32) -> Self {
        Exponent { num: value, den: 1 }
    }

    pub fn is_zero(self) -> bool {
        self.num == 0
    }

    fn add(self, other: Self) -> Option<Self> {
        let num = self
            .num
            .checked_mul(other.den)?
            .checked_add(other.num.checked_mul(self.den)?)?;
        Self::new(num, self.den.checked_mul(other.den)?)
    }

    fn neg(self) -> Option<Self> {
        Some(Exponent {
            num: self.num.checked_neg()?,
            den: self.den,
        })
    }

    fn sub(self, other: Self) -> Option<Self> {
        self.add(other.neg()?)
    }

    fn mul(self, other: Self) -> Option<Self> {
        Self::new(
            self.num.checked_mul(other.num)?,
            self.den.checked_mul(other.den)?,
        )
    }

    fn div(self, other: Self) -> Option<Self> {
        if other.num == 0 {
            return None;
        }
        Self::new(
            self.num.checked_mul(other.den)?,
            self.den.checked_mul(other.num)?,
        )
    }
}

impl fmt::Display for Exponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a.max(1)
    } else {
        gcd(b, a % b)
    }
}

/// A point in the seven-dimensional SI exponent space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dimension {
    exponents: [Exponent; 7],
}

impl Dimension {
    pub const DIMENSIONLESS: Dimension = Dimension {
        exponents: [Exponent::ZERO; 7],
    };

    pub fn is_dimensionless(&self) -> bool {
        self.exponents.iter().all(|e| e.is_zero())
    }

    /// `d(xy) = d(x) + d(y)`
    pub fn mul(&self, other: &Self) -> Option<Self> {
        self.zip(other, Exponent::add)
    }

    /// `d(x/y) = d(x) − d(y)`
    pub fn div(&self, other: &Self) -> Option<Self> {
        self.zip(other, Exponent::sub)
    }

    /// `d(x^p) = p·d(x)`
    pub fn pow(&self, power: Exponent) -> Option<Self> {
        self.map(|e| e.mul(power))
    }

    /// `d(x^(1/p)) = d(x)/p`, so `d(√x) = d(x)/2`.
    pub fn root(&self, index: Exponent) -> Option<Self> {
        self.map(|e| e.div(index))
    }

    fn zip(&self, other: &Self, op: fn(Exponent, Exponent) -> Option<Exponent>) -> Option<Self> {
        let mut exponents = [Exponent::ZERO; 7];
        for (slot, (a, b)) in exponents
            .iter_mut()
            .zip(self.exponents.iter().zip(other.exponents.iter()))
        {
            *slot = op(*a, *b)?;
        }
        Some(Dimension { exponents })
    }

    fn map(&self, op: impl Fn(Exponent) -> Option<Exponent>) -> Option<Self> {
        let mut exponents = [Exponent::ZERO; 7];
        for (slot, e) in exponents.iter_mut().zip(self.exponents.iter()) {
            *slot = op(*e)?;
        }
        Some(Dimension { exponents })
    }

    /// Parses the human-readable form used in `units.yaml`: `"L"`,
    /// `"M L T^-2"`, `"T^-1"`, `"L^1/2"`, `"1"` for dimensionless.
    /// `Θ` may also be written `Th`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        if text == "1" {
            return Some(Dimension::DIMENSIONLESS);
        }
        let mut result = Dimension::DIMENSIONLESS;
        for token in text.split_whitespace() {
            let (symbol, power) = match token.split_once('^') {
                Some((symbol, power)) => (symbol, parse_exponent(power)?),
                None => (token, Exponent::integer(1)),
            };
            let index = base_index(symbol)?;
            let mut factor = Dimension::DIMENSIONLESS;
            factor.exponents[index] = power;
            result = result.mul(&factor)?;
        }
        Some(result)
    }
}

impl fmt::Display for Dimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_dimensionless() {
            return write!(f, "1");
        }
        let mut first = true;
        for (symbol, exponent) in BASE_SYMBOLS.iter().zip(self.exponents.iter()) {
            if exponent.is_zero() {
                continue;
            }
            if !first {
                write!(f, " ")?;
            }
            first = false;
            if *exponent == Exponent::integer(1) {
                write!(f, "{symbol}")?;
            } else {
                write!(f, "{symbol}^{exponent}")?;
            }
        }
        Ok(())
    }
}

fn base_index(symbol: &str) -> Option<usize> {
    match symbol {
        "M" => Some(0),
        "L" => Some(1),
        "T" => Some(2),
        "I" => Some(3),
        "Θ" | "Th" => Some(4),
        "N" => Some(5),
        "J" => Some(6),
        _ => None,
    }
}

fn parse_exponent(text: &str) -> Option<Exponent> {
    match text.split_once('/') {
        Some((num, den)) => Exponent::new(num.trim().parse().ok()?, den.trim().parse().ok()?),
        None => Some(Exponent::integer(text.trim().parse().ok()?)),
    }
}

/// Result of an inference step. `Unknown` means "not proven", never
/// "dimensionless": the two must not be confused, or a bare symbol would
/// start contradicting real units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inferred {
    Known(Dimension),
    Unknown,
}

impl Inferred {
    fn combine(self, other: Self, op: fn(&Dimension, &Dimension) -> Option<Dimension>) -> Self {
        match (self, other) {
            (Inferred::Known(a), Inferred::Known(b)) => {
                op(&a, &b).map_or(Inferred::Unknown, Inferred::Known)
            }
            _ => Inferred::Unknown,
        }
    }
}

/// Appends dimensional warnings for `math`. Never changes the AST.
pub fn check(math: &Math, warnings: &mut Vec<Warning>) {
    let _ = infer_with(math, Lexicon::builtin(), 0, warnings);
}

/// Infers the dimension of `math`, discarding diagnostics. Useful for tests
/// and for callers that only need the dimension itself.
pub fn infer(math: &Math) -> Inferred {
    let mut ignored = Vec::new();
    infer_with(math, Lexicon::builtin(), 0, &mut ignored)
}

fn infer_with(math: &Math, lex: &Lexicon, depth: u32, warnings: &mut Vec<Warning>) -> Inferred {
    if depth >= MAX_DEPTH {
        return Inferred::Unknown;
    }
    let depth = depth + 1;
    match math {
        // A pure number is dimensionless; a bare symbol is simply unknown.
        Math::Number(_) => Inferred::Known(Dimension::DIMENSIONLESS),
        Math::Symbol(_) | Math::Infinity | Math::Ellipsis => Inferred::Unknown,
        Math::Unit(expr) => unit_dimension(expr, lex),

        Math::Delta(inner)
        | Math::Vector(inner)
        | Math::UnaryMinus(inner)
        | Math::Abs(inner)
        | Math::Group { inner, .. } => infer_with(inner, lex, depth, warnings),

        Math::Juxt(items) => {
            let mut product = Some(Dimension::DIMENSIONLESS);
            for item in items {
                // Every branch is visited even once the product is unknown,
                // so a mismatch deeper inside is still reported.
                let item = infer_with(item, lex, depth, warnings);
                product = match (product, item) {
                    (Some(acc), Inferred::Known(d)) => acc.mul(&d),
                    _ => None,
                };
            }
            product.map_or(Inferred::Unknown, Inferred::Known)
        }

        Math::Binary { op, left, right } => {
            let l = infer_with(left, lex, depth, warnings);
            let r = infer_with(right, lex, depth, warnings);
            match op {
                BinOp::Mul => l.combine(r, Dimension::mul),
                BinOp::Div => l.combine(r, Dimension::div),
                BinOp::Add | BinOp::Sub | BinOp::PlusMinus => match (l, r) {
                    (Inferred::Known(a), Inferred::Known(b)) if a != b => {
                        warnings.push(mismatch(operation_name(*op), &a, &b));
                        Inferred::Unknown
                    }
                    (Inferred::Known(a), Inferred::Known(_)) => Inferred::Known(a),
                    _ => Inferred::Unknown,
                },
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                    if let (Inferred::Known(a), Inferred::Known(b)) = (l, r) {
                        if a != b {
                            warnings.push(mismatch(operation_name(*op), &a, &b));
                        }
                    }
                    // A relation is a statement, not a quantity.
                    Inferred::Unknown
                }
            }
        }

        Math::Fraction { num, den } => {
            let n = infer_with(num, lex, depth, warnings);
            let d = infer_with(den, lex, depth, warnings);
            n.combine(d, Dimension::div)
        }

        Math::Power { base, exp } => {
            let base_dimension = infer_with(base, lex, depth, warnings);
            // An exponent has to be a plain number: `2^{3 м}` is meaningless
            // whatever the base is, so a dimensionless base no longer
            // excuses an unproven or dimensioned power.
            if !require_dimensionless(
                "physics.dimensioned_exponent",
                "an exponent",
                exp,
                lex,
                depth,
                warnings,
            ) {
                return Inferred::Unknown;
            }
            let Some(power) = rational_value(exp, depth) else {
                return Inferred::Unknown;
            };
            match base_dimension {
                Inferred::Known(d) => d.pow(power).map_or(Inferred::Unknown, Inferred::Known),
                Inferred::Unknown => Inferred::Unknown,
            }
        }

        Math::Root { index, radicand } => {
            let radicand_dimension = infer_with(radicand, lex, depth, warnings);
            let degree = match index {
                None => Some(Exponent::integer(2)),
                Some(index) => {
                    if require_dimensionless(
                        "physics.dimensioned_exponent",
                        "a root index",
                        index,
                        lex,
                        depth,
                        warnings,
                    ) {
                        rational_value(index, depth)
                    } else {
                        None
                    }
                }
            };
            match (radicand_dimension, degree) {
                (Inferred::Known(d), Some(degree)) => {
                    d.root(degree).map_or(Inferred::Unknown, Inferred::Known)
                }
                _ => Inferred::Unknown,
            }
        }

        Math::Function { kind, arg } => {
            // sin, exp and friends yield a plain number — but only from a
            // well-formed application. A dimensioned argument makes the
            // expression wrong and an unproven one leaves it unproven, so
            // neither may be reported back as a proven dimension.
            if require_dimensionless(
                "physics.dimensioned_function_argument",
                &format!("the argument of {}", kind.name()),
                arg,
                lex,
                depth,
                warnings,
            ) {
                Inferred::Known(Dimension::DIMENSIONLESS)
            } else {
                Inferred::Unknown
            }
        }

        Math::Factorial(inner) => match infer_with(inner, lex, depth, warnings) {
            Inferred::Known(d) if d.is_dimensionless() => Inferred::Known(Dimension::DIMENSIONLESS),
            _ => Inferred::Unknown,
        },

        // A subscript identifies a quantity (v₀), it does not compose one.
        Math::Subscript { base, sub } => {
            let _ = infer_with(base, lex, depth, warnings);
            let _ = infer_with(sub, lex, depth, warnings);
            Inferred::Unknown
        }

        // Bounds and bodies are still visited so that a mismatch inside them
        // is reported; the aggregate itself is not proven in v1.
        Math::Sum {
            var,
            from,
            to,
            body,
        }
        | Math::Product {
            var,
            from,
            to,
            body,
        } => {
            for part in [var, from, to, body].into_iter().flatten() {
                let _ = infer_with(part, lex, depth, warnings);
            }
            Inferred::Unknown
        }

        Math::Integral {
            from,
            to,
            integrand,
            wrt,
        } => {
            for part in [from, to, integrand, wrt].into_iter().flatten() {
                let _ = infer_with(part, lex, depth, warnings);
            }
            Inferred::Unknown
        }
    }
}

fn unit_dimension(expr: &UnitExpr, lex: &Lexicon) -> Inferred {
    let mut product = Dimension::DIMENSIONLESS;
    for factor in &expr.factors {
        let Some(base) = lex.unit_dimension(&factor.symbol) else {
            return Inferred::Unknown;
        };
        let Some(power) = Exponent::new(factor.power, 1) else {
            return Inferred::Unknown;
        };
        let Some(scaled) = base.pow(power) else {
            return Inferred::Unknown;
        };
        let combined = if factor.divide {
            product.div(&scaled)
        } else {
            product.mul(&scaled)
        };
        let Some(combined) = combined else {
            return Inferred::Unknown;
        };
        product = combined;
    }
    Inferred::Known(product)
}

/// Checks a sub-expression that has to be a plain number: an exponent, a
/// root index, or a function argument. `true` only when dimensionlessness
/// is *proven*. A proven dimensioned value is reported under `code`; an
/// unproven one stays silent. In both of those cases `false` tells the
/// caller to abstain, so a diagnosed expression never comes back as a
/// proven dimension.
fn require_dimensionless(
    code: &str,
    role: &str,
    math: &Math,
    lex: &Lexicon,
    depth: u32,
    warnings: &mut Vec<Warning>,
) -> bool {
    match infer_with(math, lex, depth, warnings) {
        Inferred::Known(d) if d.is_dimensionless() => true,
        Inferred::Known(d) => {
            warnings.push(Warning {
                code: code.into(),
                message: format!("{role} must be dimensionless (got {d})"),
            });
            false
        }
        Inferred::Unknown => false,
    }
}

/// Exact rational value of an exponent expression. Anything symbolic or
/// unsupported returns `None`, which makes the surrounding power unknown
/// rather than guessed.
fn rational_value(math: &Math, depth: u32) -> Option<Exponent> {
    if depth >= MAX_DEPTH {
        return None;
    }
    let depth = depth + 1;
    match math {
        Math::Number(text) => parse_number(text),
        Math::UnaryMinus(inner) => rational_value(inner, depth)?.neg(),
        Math::Group { inner, .. } => rational_value(inner, depth),
        Math::Fraction { num, den } => rational_value(num, depth)?.div(rational_value(den, depth)?),
        _ => None,
    }
}

fn parse_number(text: &str) -> Option<Exponent> {
    let text = text.trim();
    // The tokenizer emits signed literals directly: «в минус первой» becomes
    // `Number("-1")`, not `UnaryMinus(Number("1"))`.
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (integer, fraction) = match unsigned.split_once([',', '.']) {
        Some((integer, fraction)) => (integer, fraction),
        None => (unsigned, ""),
    };
    if integer.is_empty() && fraction.is_empty() {
        return None;
    }
    if !integer.chars().all(|c| c.is_ascii_digit()) || !fraction.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    // Keep 10^k comfortably inside i32; longer decimals abstain.
    if fraction.len() > 9 {
        return None;
    }
    let digits = format!("{integer}{fraction}");
    let numerator: i32 = digits.parse().ok()?;
    let numerator = if negative {
        numerator.checked_neg()?
    } else {
        numerator
    };
    let denominator = 10i32.checked_pow(fraction.len() as u32)?;
    Exponent::new(numerator, denominator)
}

fn mismatch(operation: &str, left: &Dimension, right: &Dimension) -> Warning {
    Warning {
        code: "physics.dimension_mismatch".into(),
        message: format!("{operation} needs matching dimensions ({left} vs {right})"),
    }
}

fn operation_name(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "addition",
        BinOp::Sub => "subtraction",
        BinOp::PlusMinus => "±",
        BinOp::Eq => "equality",
        BinOp::Ne => "comparison",
        BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => "comparison",
        BinOp::Mul => "multiplication",
        BinOp::Div => "division",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Case, FunctionKind, GroupKind, Symbol, UnitFactor};

    fn dim(symbol: &str) -> Dimension {
        Lexicon::builtin()
            .unit_dimension(symbol)
            .unwrap_or_else(|| panic!("unit '{symbol}' must carry a dimension"))
    }

    fn unit(symbol: &str) -> Math {
        Math::Unit(UnitExpr {
            factors: vec![UnitFactor {
                symbol: symbol.into(),
                power: 1,
                divide: false,
            }],
        })
    }

    fn quantity(value: &str, symbol: &str) -> Math {
        Math::Juxt(vec![Math::Number(value.into()), unit(symbol)])
    }

    fn symbol(letter: &str) -> Math {
        Math::Symbol(Symbol::latin(letter.chars().next().unwrap(), Case::Lower))
    }

    fn warnings_for(math: &Math) -> Vec<Warning> {
        let mut warnings = Vec::new();
        check(math, &mut warnings);
        warnings
    }

    fn known(math: &Math) -> Dimension {
        match infer(math) {
            Inferred::Known(d) => d,
            Inferred::Unknown => panic!("expected a known dimension for {math:?}"),
        }
    }

    #[test]
    fn seven_si_base_dimensions_are_distinct_and_named() {
        let base = [
            ("м", "L"),
            ("кг", "M"),
            ("с", "T"),
            ("А", "I"),
            ("К", "Θ"),
            ("моль", "N"),
            ("кд", "J"),
        ];
        for (symbol, expected) in base {
            assert_eq!(dim(symbol).to_string(), expected, "unit {symbol}");
        }
        // All seven must differ from each other: no accidental duplicates.
        for (i, (a, _)) in base.iter().enumerate() {
            for (b, _) in base.iter().skip(i + 1) {
                assert_ne!(dim(a), dim(b), "{a} and {b} must differ");
            }
        }
    }

    #[test]
    fn derived_units_carry_their_si_dimensions() {
        for (symbol, expected) in [
            ("Гц", "T^-1"),
            ("Н", "M L T^-2"),
            ("Па", "M L^-1 T^-2"),
            ("Дж", "M L^2 T^-2"),
            ("Вт", "M L^2 T^-3"),
            ("Кл", "T I"),
            ("В", "M L^2 T^-3 I^-1"),
            ("Ом", "M L^2 T^-3 I^-2"),
        ] {
            assert_eq!(dim(symbol).to_string(), expected, "unit {symbol}");
        }
    }

    #[test]
    fn newton_equals_kilogram_metre_per_second_squared() {
        let expected = dim("кг")
            .mul(&dim("м"))
            .unwrap()
            .div(&dim("с").pow(Exponent::integer(2)).unwrap())
            .unwrap();
        assert_eq!(dim("Н"), expected);
    }

    #[test]
    fn joule_equals_newton_metre() {
        assert_eq!(dim("Дж"), dim("Н").mul(&dim("м")).unwrap());
    }

    #[test]
    fn metre_per_second_times_second_cancels_to_metre() {
        let speed = dim("м").div(&dim("с")).unwrap();
        assert_eq!(speed.mul(&dim("с")).unwrap(), dim("м"));
    }

    #[test]
    fn prefixes_change_scale_but_not_dimension() {
        for symbol in ["нм", "мм", "см", "км"] {
            assert_eq!(dim(symbol), dim("м"), "{symbol} must stay a length");
        }
        assert_eq!(dim("мс"), dim("с"));
        assert_eq!(dim("кГц"), dim("Гц"));
        assert_eq!(dim("МГц"), dim("Гц"));
    }

    #[test]
    fn compatible_addition_is_silent() {
        let math = Math::Binary {
            op: BinOp::Add,
            left: Box::new(quantity("3", "м")),
            right: Box::new(quantity("4", "см")),
        };
        assert!(warnings_for(&math).is_empty());
        assert_eq!(known(&math), dim("м"));
    }

    #[test]
    fn adding_length_to_time_is_reported() {
        let math = Math::Binary {
            op: BinOp::Add,
            left: Box::new(quantity("3", "м")),
            right: Box::new(quantity("4", "с")),
        };
        let warnings = warnings_for(&math);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "physics.dimension_mismatch");
        assert!(warnings[0].message.contains("L vs T"), "{warnings:?}");
    }

    #[test]
    fn adding_volts_to_amperes_is_reported() {
        let math = Math::Binary {
            op: BinOp::Add,
            left: Box::new(quantity("5", "В")),
            right: Box::new(quantity("3", "А")),
        };
        let warnings = warnings_for(&math);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "physics.dimension_mismatch");
    }

    #[test]
    fn bare_symbols_never_produce_a_verdict() {
        // F = ma: nothing here is a proven dimension, so nothing is claimed.
        let math = Math::Binary {
            op: BinOp::Eq,
            left: Box::new(Math::Vector(Box::new(symbol("F")))),
            right: Box::new(Math::Binary {
                op: BinOp::Mul,
                left: Box::new(symbol("m")),
                right: Box::new(Math::Vector(Box::new(symbol("a")))),
            }),
        };
        assert!(warnings_for(&math).is_empty());
        assert_eq!(infer(&math), Inferred::Unknown);
    }

    #[test]
    fn mismatched_equality_is_reported() {
        let math = Math::Binary {
            op: BinOp::Eq,
            left: Box::new(quantity("1", "Дж")),
            right: Box::new(quantity("1", "Н")),
        };
        let warnings = warnings_for(&math);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "physics.dimension_mismatch");
    }

    #[test]
    fn dimensionless_function_argument_is_accepted() {
        let math = Math::Function {
            kind: FunctionKind::Sin,
            arg: Box::new(Math::Number("2".into())),
        };
        assert!(warnings_for(&math).is_empty());
        assert_eq!(known(&math), Dimension::DIMENSIONLESS);
    }

    #[test]
    fn dimensioned_function_argument_is_reported_and_unproven() {
        for kind in [FunctionKind::Sin, FunctionKind::Log] {
            let math = Math::Function {
                kind,
                arg: Box::new(quantity("3", "м")),
            };
            let warnings = warnings_for(&math);
            assert_eq!(warnings.len(), 1, "{kind:?}");
            assert_eq!(warnings[0].code, "physics.dimensioned_function_argument");
            // A diagnosed application proves nothing: reporting it as a
            // known dimensionless value would contradict the warning.
            assert_eq!(infer(&math), Inferred::Unknown, "{kind:?}");
        }
    }

    #[test]
    fn unknown_function_argument_stays_silent_and_unproven() {
        let math = Math::Function {
            kind: FunctionKind::Sin,
            arg: Box::new(symbol("x")),
        };
        assert!(warnings_for(&math).is_empty());
        assert_eq!(infer(&math), Inferred::Unknown);
    }

    #[test]
    fn a_diagnosed_function_does_not_cascade_a_second_warning() {
        // sin(3 м) is already unknown, so adding a time to it must not
        // produce a mismatch on top of the argument diagnosis.
        let math = Math::Binary {
            op: BinOp::Add,
            left: Box::new(Math::Function {
                kind: FunctionKind::Sin,
                arg: Box::new(quantity("3", "м")),
            }),
            right: Box::new(quantity("1", "с")),
        };
        let warnings = warnings_for(&math);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].code, "physics.dimensioned_function_argument");
        assert_eq!(infer(&math), Inferred::Unknown);
    }

    #[test]
    fn powers_and_roots_use_exact_rational_exponents() {
        let squared = Math::Power {
            base: Box::new(unit("м")),
            exp: Box::new(Math::Number("2".into())),
        };
        assert_eq!(known(&squared).to_string(), "L^2");

        let sqrt_of_area = Math::Root {
            index: None,
            radicand: Box::new(squared.clone()),
        };
        assert_eq!(known(&sqrt_of_area), dim("м"));

        let cube_root_of_volume = Math::Root {
            index: Some(Box::new(Math::Number("3".into()))),
            radicand: Box::new(Math::Power {
                base: Box::new(unit("м")),
                exp: Box::new(Math::Number("3".into())),
            }),
        };
        assert_eq!(known(&cube_root_of_volume), dim("м"));

        // √L is a legitimate half-integer dimension, not a rounding artefact.
        let sqrt_of_length = Math::Root {
            index: None,
            radicand: Box::new(unit("м")),
        };
        assert_eq!(known(&sqrt_of_length).to_string(), "L^1/2");
    }

    #[test]
    fn negative_and_decimal_exponents_are_exact() {
        let inverse = Math::Power {
            base: Box::new(unit("с")),
            exp: Box::new(Math::UnaryMinus(Box::new(Math::Number("1".into())))),
        };
        assert_eq!(known(&inverse), dim("Гц"));

        let half = Math::Power {
            base: Box::new(unit("м")),
            exp: Box::new(Math::Number("0,5".into())),
        };
        assert_eq!(known(&half).to_string(), "L^1/2");
    }

    #[test]
    fn symbolic_exponent_abstains() {
        let math = Math::Power {
            base: Box::new(unit("м")),
            exp: Box::new(symbol("n")),
        };
        assert_eq!(infer(&math), Inferred::Unknown);
        assert!(warnings_for(&math).is_empty());
    }

    #[test]
    fn unproven_exponent_abstains_even_under_a_dimensionless_base() {
        // 2^n: nothing is wrong, but nothing is proven either.
        let math = Math::Power {
            base: Box::new(Math::Number("2".into())),
            exp: Box::new(symbol("n")),
        };
        assert_eq!(infer(&math), Inferred::Unknown);
        assert!(warnings_for(&math).is_empty());
    }

    #[test]
    fn dimensioned_exponent_is_reported_under_any_base() {
        // 2^{3 м} and x^{3 м} are both meaningless, dimensionless base or not.
        for base in [Math::Number("2".into()), symbol("x"), unit("м")] {
            let math = Math::Power {
                base: Box::new(base.clone()),
                exp: Box::new(quantity("3", "м")),
            };
            let warnings = warnings_for(&math);
            assert_eq!(warnings.len(), 1, "base {base:?}");
            assert_eq!(warnings[0].code, "physics.dimensioned_exponent");
            assert!(warnings[0].message.contains("exponent"), "{warnings:?}");
            assert_eq!(infer(&math), Inferred::Unknown);
        }
    }

    #[test]
    fn dimensioned_root_index_is_reported() {
        let math = Math::Root {
            index: Some(Box::new(quantity("3", "с"))),
            radicand: Box::new(quantity("4", "м")),
        };
        let warnings = warnings_for(&math);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "physics.dimensioned_exponent");
        assert!(warnings[0].message.contains("root index"), "{warnings:?}");
        assert_eq!(infer(&math), Inferred::Unknown);
    }

    #[test]
    fn unproven_root_index_abstains_silently() {
        let math = Math::Root {
            index: Some(Box::new(symbol("n"))),
            radicand: Box::new(quantity("4", "м")),
        };
        assert_eq!(infer(&math), Inferred::Unknown);
        assert!(warnings_for(&math).is_empty());
    }

    #[test]
    fn signed_number_exponents_are_parsed() {
        // The tokenizer emits Number("-1") for «в минус первой».
        let math = Math::Power {
            base: Box::new(quantity("1", "м")),
            exp: Box::new(Math::Number("-1".into())),
        };
        assert_eq!(known(&math).to_string(), "L^-1");
        assert!(warnings_for(&math).is_empty());

        let explicit_plus = Math::Power {
            base: Box::new(unit("м")),
            exp: Box::new(Math::Number("+2".into())),
        };
        assert_eq!(known(&explicit_plus).to_string(), "L^2");

        let signed_decimal = Math::Power {
            base: Box::new(unit("м")),
            exp: Box::new(Math::Number("-0,5".into())),
        };
        assert_eq!(known(&signed_decimal).to_string(), "L^-1/2");
    }

    #[test]
    fn exponent_overflow_abstains_instead_of_wrapping() {
        // (м^2000000000)^2000000000 cannot be represented exactly.
        let math = Math::Power {
            base: Box::new(Math::Power {
                base: Box::new(unit("м")),
                exp: Box::new(Math::Number("2000000000".into())),
            }),
            exp: Box::new(Math::Number("2000000000".into())),
        };
        assert_eq!(infer(&math), Inferred::Unknown);
        assert!(warnings_for(&math).is_empty());
    }

    #[test]
    fn oversized_numeric_exponent_abstains() {
        let math = Math::Power {
            base: Box::new(unit("м")),
            exp: Box::new(Math::Number("99999999999999999999".into())),
        };
        assert_eq!(infer(&math), Inferred::Unknown);
    }

    #[test]
    fn deep_artificial_ast_abstains_without_panicking() {
        let mut math = quantity("1", "м");
        for _ in 0..1_000 {
            math = Math::Group {
                kind: GroupKind::Paren,
                inner: Box::new(math),
            };
        }
        // Past the recursion guard the analyser abstains; it must not
        // recurse until the stack gives out.
        assert_eq!(infer(&math), Inferred::Unknown);
        assert!(warnings_for(&math).is_empty());
    }

    #[test]
    fn unknown_unit_symbol_abstains() {
        let math = Math::Binary {
            op: BinOp::Add,
            left: Box::new(Math::Juxt(vec![
                Math::Number("1".into()),
                Math::Unit(UnitExpr {
                    factors: vec![UnitFactor {
                        symbol: "парсек".into(),
                        power: 1,
                        divide: false,
                    }],
                }),
            ])),
            right: Box::new(quantity("1", "с")),
        };
        assert!(warnings_for(&math).is_empty());
        assert_eq!(infer(&math), Inferred::Unknown);
    }

    #[test]
    fn mismatch_inside_an_unproven_parent_is_still_reported() {
        // The sum itself is unknown (a bare symbol multiplies it), but the
        // inner metre + second contradiction is still proven.
        let math = Math::Binary {
            op: BinOp::Mul,
            left: Box::new(symbol("k")),
            right: Box::new(Math::Binary {
                op: BinOp::Add,
                left: Box::new(quantity("1", "м")),
                right: Box::new(quantity("1", "с")),
            }),
        };
        let warnings = warnings_for(&math);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "physics.dimension_mismatch");
    }

    #[test]
    fn dimension_text_round_trips() {
        for text in ["1", "L", "M L T^-2", "M L^2 T^-3 I^-1", "L^1/2"] {
            let parsed = Dimension::parse(text).unwrap();
            assert_eq!(parsed.to_string(), text);
        }
        assert_eq!(
            Dimension::parse("Th").unwrap(),
            Dimension::parse("Θ").unwrap()
        );
        for invalid in ["", "X", "L^", "L^1/0", "L^x", "^2"] {
            assert!(Dimension::parse(invalid).is_none(), "{invalid} must fail");
        }
    }

    #[test]
    fn exponent_arithmetic_is_checked() {
        assert!(Exponent::new(1, 0).is_none());
        assert!(Exponent::new(i32::MIN, -1).is_none());
        assert_eq!(Exponent::new(2, 4).unwrap(), Exponent::new(1, 2).unwrap());
        assert_eq!(Exponent::new(-1, -2).unwrap(), Exponent::new(1, 2).unwrap());
        assert!(Exponent::integer(i32::MAX)
            .mul(Exponent::integer(i32::MAX))
            .is_none());
        assert!(Exponent::integer(1).div(Exponent::ZERO).is_none());
    }
}

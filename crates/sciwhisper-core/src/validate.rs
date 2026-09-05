//! Semantic checks annotate a parsed AST without changing what the user said.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{derivative_defect, Chemical, Equation, Math, Node, Species, Warning};

/// Recursion guard for the structural walk, matching the one in the
/// dimensional analyser. A hand-built or deserialised AST may be deeper than
/// anything the parser produces, and the public API has to report that
/// rather than exhaust the stack.
const MAX_MATH_DEPTH: u32 = 128;

pub fn semantic_warnings(node: &Node) -> Vec<Warning> {
    let mut warnings = Vec::new();
    collect_warnings(node, &mut warnings);
    warnings
}

fn collect_warnings(node: &Node, warnings: &mut Vec<Warning>) {
    match node {
        Node::Document(nodes) => {
            for node in nodes {
                collect_warnings(node, warnings);
            }
        }
        Node::Chemical(Chemical::Equation(equation)) => validate_equation(equation, warnings),
        Node::Math(math) => {
            validate_math(math, 0, warnings);
            crate::dimension::check(math, warnings);
        }
        Node::Text(_) | Node::Chemical(Chemical::Species(_)) => {}
    }
}

fn validate_equation(equation: &Equation, warnings: &mut Vec<Warning>) {
    // `None` means atom counting itself overflowed (see `Formula::atom_counts`):
    // conservation can't be verified either way, so no unbalanced/balanced
    // claim is made rather than risking a wrong one.
    let atoms_unbalanced = match (side_atoms(&equation.left), side_atoms(&equation.right)) {
        (Some(left), Some(right)) => {
            let unbalanced = left != right;
            if unbalanced {
                let elements: BTreeSet<_> = left.keys().chain(right.keys()).cloned().collect();
                let differences = elements
                    .into_iter()
                    .filter_map(|element| {
                        let left_count = left.get(&element).copied().unwrap_or(0);
                        let right_count = right.get(&element).copied().unwrap_or(0);
                        (left_count != right_count)
                            .then(|| format!("{element}: {left_count} → {right_count}"))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                warnings.push(Warning {
                    code: "chemistry.unbalanced_atoms".into(),
                    message: format!("atom balance is not conserved ({differences})"),
                });
            }
            Some(unbalanced)
        }
        _ => None,
    };

    let left_charge = side_charge(&equation.left);
    let right_charge = side_charge(&equation.right);
    let charge_balanced = left_charge == right_charge;
    if !charge_balanced {
        warnings.push(Warning {
            code: "chemistry.unbalanced_charge".into(),
            message: format!("charge is not conserved ({left_charge} → {right_charge})"),
        });
    }

    // A suggestion is additive, never a silent rewrite: the dictated AST and
    // its coefficient stay untouched, only a warning proposes a fix.
    if atoms_unbalanced == Some(true) || !charge_balanced {
        if let Some(coeffs) = crate::balance::balance_equation(equation) {
            warnings.push(Warning {
                code: "chemistry.balance_suggestion".into(),
                message: format!(
                    "predicted balanced form: {}",
                    crate::balance::render_suggestion(equation, &coeffs)
                ),
            });
        }
    }
}

/// Structural checks on the mathematical AST. The parser cannot build a
/// malformed derivative — `Math::derivative` refuses to — but a node that
/// arrived by hand or over serde can be malformed, and the answer is a
/// warning, never a panic and never a silent repair.
fn validate_math(math: &Math, depth: u32, warnings: &mut Vec<Warning>) {
    if depth >= MAX_MATH_DEPTH {
        // One report per AST: a deep tree is one problem, not hundreds.
        if !warnings
            .iter()
            .any(|warning| warning.code == "math.ast_too_deep")
        {
            warnings.push(Warning {
                code: "math.ast_too_deep".into(),
                message: format!("expression nesting deeper than {MAX_MATH_DEPTH} is not analysed"),
            });
        }
        return;
    }
    let depth = depth + 1;
    match math {
        Math::Number(_) | Math::Symbol(_) | Math::Unit(_) | Math::Infinity | Math::Ellipsis => {}
        Math::Delta(inner)
        | Math::Vector(inner)
        | Math::UnaryMinus(inner)
        | Math::Abs(inner)
        | Math::Factorial(inner)
        | Math::Group { inner, .. } => validate_math(inner, depth, warnings),
        Math::Function { arg, .. } => validate_math(arg, depth, warnings),
        Math::Binary { left, right, .. } => {
            validate_math(left, depth, warnings);
            validate_math(right, depth, warnings);
        }
        Math::Fraction { num, den } => {
            validate_math(num, depth, warnings);
            validate_math(den, depth, warnings);
        }
        Math::Power { base, exp } => {
            validate_math(base, depth, warnings);
            validate_math(exp, depth, warnings);
        }
        Math::Subscript { base, sub } => {
            validate_math(base, depth, warnings);
            validate_math(sub, depth, warnings);
        }
        Math::Root { index, radicand } => {
            if let Some(index) = index {
                validate_math(index, depth, warnings);
            }
            validate_math(radicand, depth, warnings);
        }
        Math::Juxt(items) => {
            for item in items {
                validate_math(item, depth, warnings);
            }
        }
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
                validate_math(part, depth, warnings);
            }
        }
        Math::Integral {
            from,
            to,
            integrand,
            wrt,
        } => {
            for part in [from, to, integrand, wrt].into_iter().flatten() {
                validate_math(part, depth, warnings);
            }
        }
        Math::Derivative {
            expr,
            variables,
            kind: _,
        } => {
            if let Some(defect) = derivative_defect(variables) {
                warnings.push(Warning {
                    code: defect.code().into(),
                    message: defect.message(),
                });
            }
            validate_math(expr, depth, warnings);
            for variable in variables {
                validate_math(&variable.variable, depth, warnings);
            }
        }
        Math::Limit {
            variable,
            target,
            direction: _,
            body,
        } => {
            validate_math(variable, depth, warnings);
            validate_math(target, depth, warnings);
            validate_math(body, depth, warnings);
        }
    }
}

fn side_atoms(species: &[Species]) -> Option<BTreeMap<String, u64>> {
    let mut atoms = BTreeMap::new();
    for species in species {
        for (element, count) in species.formula.atom_counts()? {
            let contribution = u64::from(species.coefficient).checked_mul(count)?;
            let entry = atoms.entry(element).or_insert(0u64);
            *entry = entry.checked_add(contribution)?;
        }
    }
    Some(atoms)
}

fn side_charge(species: &[Species]) -> i64 {
    species
        .iter()
        .map(|species| {
            i64::from(species.coefficient) * i64::from(species.charge.unwrap_or_default())
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use crate::{interpret, Domain, InterpretOptions};

    fn parsed(text: &str) -> crate::ast::InterpretationResult {
        interpret(
            text,
            InterpretOptions {
                domain: Domain::Chemistry,
                allow_shortcuts: true,
            },
        )
    }

    #[test]
    fn balanced_reaction_has_no_balance_warning() {
        let result = parsed("гидроксид меди два превращается в оксид меди два плюс вода");
        assert!(!result
            .warnings
            .iter()
            .any(|warning| warning.code.starts_with("chemistry.unbalanced")));
    }

    #[test]
    fn unbalanced_reaction_is_preserved_and_annotated() {
        let result = parsed("уксусная кислота окисляется до аш два о");
        assert_eq!(result.confidence, 0.95);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.code == "chemistry.unbalanced_atoms"));
    }

    #[test]
    fn unbalanceable_reaction_gets_no_false_suggestion() {
        // Missing a carbon sink (CO2): no choice of coefficients over just
        // these two species conserves atoms, so the system must abstain
        // rather than invent a plausible-looking but wrong fix.
        let result = parsed("уксусная кислота окисляется до аш два о");
        assert!(!result
            .warnings
            .iter()
            .any(|warning| warning.code == "chemistry.balance_suggestion"));
    }

    #[test]
    fn suggests_balanced_coefficients_for_a_solvable_reaction() {
        let equation = crate::formula::parse_equation_str("H2 + O2 -> H2O").unwrap();
        let node = crate::ast::Node::Chemical(crate::ast::Chemical::Equation(equation));
        let warnings = super::semantic_warnings(&node);
        let suggestion = warnings
            .iter()
            .find(|warning| warning.code == "chemistry.balance_suggestion")
            .unwrap_or_else(|| panic!("no suggestion in {warnings:?}"));
        assert_eq!(
            suggestion.message,
            "predicted balanced form: 2H₂ + O₂ → 2H₂O"
        );
    }

    // ------------------------------------------- structural math validation

    use crate::ast::{
        Case, DerivativeKind, DerivativeVariable, LimitDirection, Math, Node, Symbol,
        MAX_DERIVATIVE_ORDER,
    };

    fn sym(letter: char) -> Math {
        Math::Symbol(Symbol::latin(letter, Case::Lower))
    }

    fn math_codes(math: Math) -> Vec<String> {
        super::semantic_warnings(&Node::Math(math))
            .into_iter()
            .filter(|warning| warning.code.starts_with("math."))
            .map(|warning| warning.code)
            .collect()
    }

    #[test]
    fn a_derivative_without_variables_is_reported_not_repaired() {
        let node = Math::Derivative {
            kind: DerivativeKind::Ordinary,
            expr: Box::new(sym('f')),
            variables: vec![],
        };
        assert_eq!(
            math_codes(node.clone()),
            ["math.derivative_without_variable"]
        );
        // The node itself is untouched: validation annotates, never rewrites.
        match node {
            Math::Derivative { variables, .. } => assert!(variables.is_empty()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_zero_order_derivative_is_reported() {
        assert_eq!(
            math_codes(Math::Derivative {
                kind: DerivativeKind::Ordinary,
                expr: Box::new(sym('f')),
                variables: vec![DerivativeVariable::new(sym('x'), 0)],
            }),
            ["math.derivative_zero_order"]
        );
    }

    #[test]
    fn an_order_above_the_cap_is_reported() {
        assert_eq!(
            math_codes(Math::Derivative {
                kind: DerivativeKind::Ordinary,
                expr: Box::new(sym('f')),
                variables: vec![DerivativeVariable::new(sym('x'), MAX_DERIVATIVE_ORDER + 1)],
            }),
            ["math.derivative_order_too_high"]
        );
    }

    #[test]
    fn a_total_order_above_the_cap_is_reported() {
        let variables = (0..(MAX_DERIVATIVE_ORDER + 1))
            .map(|_| DerivativeVariable::new(sym('x'), 1))
            .collect();
        assert_eq!(
            math_codes(Math::Derivative {
                kind: DerivativeKind::Partial,
                expr: Box::new(sym('f')),
                variables,
            }),
            ["math.derivative_total_order_too_high"]
        );
    }

    #[test]
    fn an_overflowing_total_order_is_reported_without_panicking() {
        assert_eq!(
            math_codes(Math::Derivative {
                kind: DerivativeKind::Ordinary,
                expr: Box::new(sym('f')),
                variables: vec![
                    DerivativeVariable::new(sym('x'), u32::MAX),
                    DerivativeVariable::new(sym('y'), 2),
                ],
            }),
            ["math.derivative_order_too_high"]
        );
    }

    #[test]
    fn a_well_formed_derivative_is_not_reported() {
        assert!(math_codes(Math::Derivative {
            kind: DerivativeKind::Partial,
            expr: Box::new(sym('f')),
            variables: vec![
                DerivativeVariable::new(sym('x'), 1),
                DerivativeVariable::new(sym('y'), 1),
            ],
        })
        .is_empty());
    }

    #[test]
    fn a_malformed_derivative_nested_deep_inside_is_still_found() {
        let mut node = Math::Derivative {
            kind: DerivativeKind::Ordinary,
            expr: Box::new(sym('f')),
            variables: vec![DerivativeVariable::new(sym('x'), 0)],
        };
        for _ in 0..8 {
            node = Math::limit(
                sym('x'),
                Math::Number("0".into()),
                LimitDirection::TwoSided,
                node,
            );
        }
        assert_eq!(math_codes(node), ["math.derivative_zero_order"]);
    }

    #[test]
    fn an_over_deep_ast_is_reported_once_and_never_panics() {
        let mut node = sym('f');
        for _ in 0..(super::MAX_MATH_DEPTH + 32) {
            node = Math::UnaryMinus(Box::new(node));
        }
        assert_eq!(math_codes(node), ["math.ast_too_deep"]);
    }
}

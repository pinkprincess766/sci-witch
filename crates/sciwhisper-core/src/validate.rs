//! Semantic checks annotate a parsed AST without changing what the user said.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{Chemical, Equation, Node, Species, Warning};

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
        Node::Math(math) => crate::dimension::check(math, warnings),
        Node::Text(_) | Node::Chemical(Chemical::Species(_)) => {}
    }
}

fn validate_equation(equation: &Equation, warnings: &mut Vec<Warning>) {
    let left = side_atoms(&equation.left);
    let right = side_atoms(&equation.right);
    let atoms_balanced = left == right;
    if !atoms_balanced {
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
    if !atoms_balanced || !charge_balanced {
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

fn side_atoms(species: &[Species]) -> BTreeMap<String, u64> {
    let mut atoms = BTreeMap::new();
    for species in species {
        for (element, count) in species.formula.atom_counts() {
            *atoms.entry(element).or_default() += u64::from(species.coefficient) * count;
        }
    }
    atoms
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
}

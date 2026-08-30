//! Semantic checks annotate a parsed AST without changing what the user said.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{Chemical, Equation, Formula, Node, Part, Species, Warning};

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
        Node::Text(_) | Node::Chemical(Chemical::Species(_)) | Node::Math(_) => {}
    }
}

fn validate_equation(equation: &Equation, warnings: &mut Vec<Warning>) {
    let left = side_atoms(&equation.left);
    let right = side_atoms(&equation.right);
    if left != right {
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
    if left_charge != right_charge {
        warnings.push(Warning {
            code: "chemistry.unbalanced_charge".into(),
            message: format!("charge is not conserved ({left_charge} → {right_charge})"),
        });
    }
}

fn side_atoms(species: &[Species]) -> BTreeMap<String, u64> {
    let mut atoms = BTreeMap::new();
    for species in species {
        collect_formula_atoms(&species.formula, u64::from(species.coefficient), &mut atoms);
    }
    atoms
}

fn collect_formula_atoms(formula: &Formula, multiplier: u64, atoms: &mut BTreeMap<String, u64>) {
    for part in &formula.parts {
        match part {
            Part::Atom { symbol, count } => {
                *atoms.entry(symbol.clone()).or_default() += multiplier * u64::from(*count);
            }
            Part::Group { inner, count } => {
                collect_formula_atoms(inner, multiplier * u64::from(*count), atoms);
            }
            Part::Hydrate { count } => {
                let waters = multiplier * u64::from(*count);
                *atoms.entry("H".into()).or_default() += waters * 2;
                *atoms.entry("O".into()).or_default() += waters;
            }
        }
    }
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
}

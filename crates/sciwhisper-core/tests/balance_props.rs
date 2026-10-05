//! Properties of [`sciwhisper_core::balance_equation`].
//!
//! These check the public post-condition, not the elimination steps.
//! A returned coefficient vector must conserve atoms and charge, be
//! strictly positive, primitive, and within the product cap. Reordering
//! species inside one side of the arrow must reorder those coefficients
//! the same way: when the kernel is one-dimensional the primitive
//! positive vector is unique, so it cannot depend on column order.

use std::collections::BTreeMap;

use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, RngSeed, TestCaseError};
use sciwhisper_core::ast::{Arrow, Equation, Formula, Part, Species};
use sciwhisper_core::balance_equation;

/// A reaction built so that one positive balance is known in advance.
///
/// Each left-hand species owns an element that appears in no other left-hand
/// species. The single right-hand formula is the sum of those species weighted
/// by the chosen coefficients, so those coefficients together with `1` on the
/// right are a strictly positive solution. The private elements keep the rows
/// independent, so the kernel is one-dimensional and that solution is the
/// primitive one.
#[derive(Clone, Debug)]
struct Constructed {
    equation: Equation,
    primitive: Vec<u32>,
}

const ELEMENTS: &[&str] = &["H", "O", "C", "N", "Fe", "Cl", "Na", "Cu"];

fn arb_element() -> impl Strategy<Value = String> {
    prop::sample::select(ELEMENTS.to_vec()).prop_map(str::to_string)
}

fn arb_formula() -> impl Strategy<Value = Formula> {
    let atom = (arb_element(), 1u32..=4).prop_map(|(symbol, count)| Part::Atom { symbol, count });
    prop_oneof![
        proptest::collection::vec(atom, 1..=3).prop_map(|parts| Formula { parts }),
        (arb_element(), 1u32..=3, 2u32..=3).prop_map(|(symbol, count, group)| Formula {
            parts: vec![Part::Group {
                inner: Formula::atom(symbol, count),
                count: group,
            }],
        }),
    ]
}

fn arb_species() -> impl Strategy<Value = Species> {
    (arb_formula(), prop::option::of(-2i32..=2)).prop_map(|(formula, charge)| {
        let mut species = Species::new(formula);
        species.charge = charge.filter(|value| *value != 0);
        species
    })
}

fn arb_side() -> impl Strategy<Value = Vec<Species>> {
    proptest::collection::vec(arb_species(), 1..=3)
}

fn arb_equation() -> impl Strategy<Value = Equation> {
    (arb_side(), arb_side(), any::<bool>()).prop_map(|(left, right, equilibrium)| Equation {
        left,
        arrow: if equilibrium {
            Arrow::Equilibrium
        } else {
            Arrow::Forward
        },
        right,
        condition: None,
    })
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn side_totals(species: &[Species], coeffs: &[u32]) -> Option<(BTreeMap<String, u64>, i64)> {
    let mut atoms: BTreeMap<String, u64> = BTreeMap::new();
    let mut charge = 0i64;
    for (species, &coeff) in species.iter().zip(coeffs) {
        let counts = species.formula.atom_counts()?;
        let factor = u64::from(coeff);
        for (element, count) in counts {
            let add = count.checked_mul(factor)?;
            let entry = atoms.entry(element).or_insert(0);
            *entry = (*entry).checked_add(add)?;
        }
        let species_charge = i64::from(species.charge.unwrap_or(0));
        charge = charge.checked_add(species_charge.checked_mul(i64::from(coeff))?)?;
    }
    Some((atoms, charge))
}

fn describe(equation: &Equation) -> String {
    let side = |species: &[Species]| {
        species
            .iter()
            .map(|species| {
                let atoms = species
                    .formula
                    .atom_counts()
                    .map(|counts| format!("{counts:?}"))
                    .unwrap_or_else(|| "overflow".to_string());
                format!("charge={:?} {atoms}", species.charge)
            })
            .collect::<Vec<_>>()
            .join(" + ")
    };
    format!("{} -> {}", side(&equation.left), side(&equation.right))
}

fn reorder(species: &[Species], keys: &[u32]) -> (Vec<usize>, Vec<Species>) {
    let mut order: Vec<usize> = (0..species.len()).collect();
    order.sort_by_key(|&index| (keys[index], index));
    let permuted = order.iter().map(|&index| species[index].clone()).collect();
    (order, permuted)
}

fn permute_parts(parts: &mut Vec<Part>, keys: &[u32]) {
    if parts.is_empty() {
        return;
    }
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by_key(|&index| (keys[index % keys.len()], index));
    let old = std::mem::take(parts);
    *parts = order.into_iter().map(|index| old[index].clone()).collect();
    for part in parts.iter_mut() {
        if let Part::Group { inner, .. } = part {
            permute_parts(&mut inner.parts, keys);
        }
    }
}

fn permute_formulas(equation: &mut Equation, keys: &[u32]) {
    for species in equation.left.iter_mut().chain(equation.right.iter_mut()) {
        permute_parts(&mut species.formula.parts, keys);
    }
}

/// `k` times a reaction whose formulas already balance: only the dictated
/// coefficients change. `balance_equation` does not read those coefficients.
fn scale_dictated(equation: &Equation, primitive: &[u32], k: u32) -> Equation {
    let mut scaled = equation.clone();
    for (species, &coeff) in scaled
        .left
        .iter_mut()
        .chain(scaled.right.iter_mut())
        .zip(primitive)
    {
        species.coefficient = coeff * k;
    }
    scaled
}

fn arb_constructed() -> impl Strategy<Value = Constructed> {
    (1usize..=3)
        .prop_flat_map(|n_left| {
            let coeffs = proptest::collection::vec(1u32..=6, n_left);
            let shared = proptest::collection::vec(proptest::collection::vec(0u32..=3, 2), n_left);
            (coeffs, shared)
        })
        .prop_map(|(coeffs, shared)| {
            let mut left = Vec::with_capacity(coeffs.len());
            let mut right_counts: BTreeMap<String, u32> = BTreeMap::new();
            for (index, (&coeff, shares)) in coeffs.iter().zip(shared).enumerate() {
                let private = format!("U{index}");
                let mut parts = vec![Part::Atom {
                    symbol: private.clone(),
                    count: 1,
                }];
                *right_counts.entry(private).or_insert(0) += coeff;
                for (share_index, count) in shares.into_iter().enumerate() {
                    if count == 0 {
                        continue;
                    }
                    let symbol = format!("S{share_index}");
                    parts.push(Part::Atom {
                        symbol: symbol.clone(),
                        count,
                    });
                    *right_counts.entry(symbol).or_insert(0) += coeff * count;
                }
                left.push(Species::new(Formula { parts }));
            }
            let right = Species::new(Formula {
                parts: right_counts
                    .into_iter()
                    .map(|(symbol, count)| Part::Atom { symbol, count })
                    .collect(),
            });
            let mut primitive = coeffs;
            primitive.push(1);
            Constructed {
                equation: Equation {
                    left,
                    arrow: Arrow::Forward,
                    right: vec![right],
                    condition: None,
                },
                primitive,
            }
        })
}

/// A fixed seed, and no failure file written into the source tree.
///
/// proptest's defaults are a fresh random seed on every run and, on failure,
/// a `*.proptest-regressions` file next to this one. The first makes two runs
/// of the suite disagree, so a release check could fail on CI for a reason
/// nobody can reproduce locally; the second leaves an artifact in the
/// repository. Everything else in this project is reproducible, and a fixed
/// seed makes a failure reproduce on every run, which is what the regression
/// file was for.
///
/// The cost is that the same 128 equations are tried every time. To search
/// further, change the seed or the case count here, in a reviewed edit —
/// not by leaving the search to chance.
fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 128,
        rng_seed: RngSeed::Fixed(20_260_928),
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(config())]

    /// If the balancer returns coefficients, they are a primitive positive
    /// integer solution of the conservation law, and none exceeds 9999.
    #[test]
    fn some_coefficients_conserve_atoms_charge_and_are_primitive(equation in arb_equation()) {
        let Some(coeffs) = balance_equation(&equation) else {
            return Ok(());
        };
        let left_len = equation.left.len();
        prop_assert_eq!(
            coeffs.len(),
            left_len + equation.right.len(),
            "coefficient count {}",
            describe(&equation)
        );
        prop_assert!(
            coeffs.iter().all(|coeff| (1..=9999).contains(coeff)),
            "coeffs {:?} are outside 1..=9999 for {}",
            coeffs,
            describe(&equation)
        );
        let divisor = coeffs.iter().fold(0u32, |acc, &coeff| gcd(acc, coeff));
        prop_assert_eq!(divisor, 1, "gcd {} of {:?}", divisor, coeffs);

        let (left_atoms, left_charge) =
            side_totals(&equation.left, &coeffs[..left_len]).expect("atom counts");
        let (right_atoms, right_charge) =
            side_totals(&equation.right, &coeffs[left_len..]).expect("atom counts");
        prop_assert_eq!(left_atoms, right_atoms, "{}", describe(&equation));
        prop_assert_eq!(left_charge, right_charge, "{}", describe(&equation));
    }

    /// Permuting species inside the left side and inside the right side
    /// permutes a successful coefficient vector the same way.
    #[test]
    fn coefficients_follow_a_permutation_of_each_side(
        equation in arb_equation(),
        left_keys in proptest::collection::vec(any::<u32>(), 3),
        right_keys in proptest::collection::vec(any::<u32>(), 3),
    ) {
        let Some(coeffs) = balance_equation(&equation) else {
            return Ok(());
        };
        let (left_order, left) = reorder(&equation.left, &left_keys);
        let (right_order, right) = reorder(&equation.right, &right_keys);
        let permuted = Equation {
            left,
            right,
            arrow: equation.arrow,
            condition: None,
        };
        let Some(permuted_coeffs) = balance_equation(&permuted) else {
            return Err(TestCaseError::fail(format!(
                "reordering turned a balance into abstention: {}",
                describe(&equation)
            )));
        };
        let mut expected = Vec::with_capacity(coeffs.len());
        for old in left_order {
            expected.push(coeffs[old]);
        }
        for old in right_order {
            expected.push(coeffs[equation.left.len() + old]);
        }
        prop_assert_eq!(permuted_coeffs, expected, "{}", describe(&equation));
    }

    /// A reaction assembled from chosen coefficients balances as exactly
    /// those coefficients, made primitive by the trailing 1 on the right.
    #[test]
    fn a_reaction_balanced_by_construction_returns_that_primitive_vector(
        built in arb_constructed(),
    ) {
        prop_assert_eq!(
            balance_equation(&built.equation),
            Some(built.primitive.clone()),
            "{}",
            describe(&built.equation)
        );
    }

    /// Multiplying an already balanced reaction by k does not change the
    /// primitive coefficients the balancer proposes.
    #[test]
    fn multiplying_a_balanced_reaction_by_k_keeps_the_primitive_coefficients(
        built in arb_constructed(),
        k in 2u32..=9,
    ) {
        let scaled = scale_dictated(&built.equation, &built.primitive, k);
        prop_assert_eq!(
            balance_equation(&scaled),
            Some(built.primitive),
            "k = {}; {}",
            k,
            describe(&scaled)
        );
    }

    /// The order of pieces inside a formula is not a chemical fact.
    #[test]
    fn permuting_elements_inside_formulas_keeps_the_coefficients(
        built in arb_constructed(),
        equation in arb_equation(),
        keys in proptest::collection::vec(any::<u32>(), 8),
    ) {
        let mut shuffled = built.equation.clone();
        permute_formulas(&mut shuffled, &keys);
        prop_assert_eq!(
            balance_equation(&shuffled),
            Some(built.primitive),
            "{}",
            describe(&shuffled)
        );

        let before = balance_equation(&equation);
        let mut shuffled_arbitrary = equation.clone();
        permute_formulas(&mut shuffled_arbitrary, &keys);
        prop_assert_eq!(
            balance_equation(&shuffled_arbitrary),
            before.clone(),
            "before {:?} for {}",
            before,
            describe(&equation)
        );
    }
}

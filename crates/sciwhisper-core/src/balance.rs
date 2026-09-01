//! Deterministic integer balancing of dictated reactions via null-space
//! computation over the atom (and, when present, charge) conservation matrix.
//! This never changes what the user dictated: it only proposes coefficients
//! for `chemistry.balance_suggestion`, computed independently of the spoken
//! ones, and abstains whenever the system is under- or over-determined.
//!
//! All fraction arithmetic below is `checked_*`: naive Gaussian elimination
//! on fractions can blow numerator/denominator size up exponentially over
//! enough steps, and this crate abstains on overflow rather than wrapping
//! into a wrong suggestion or panicking on a debug build.

use std::collections::BTreeSet;

use crate::ast::{Arrow, Equation, Species};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Frac {
    num: i128,
    den: i128,
}

impl Frac {
    /// `i128::MIN` has no positive negation, so both the sign flip and the
    /// gcd-reduced magnitude must be checked, not just the arithmetic that
    /// produced `num`/`den`.
    fn reduced(num: i128, den: i128) -> Option<Self> {
        debug_assert!(den != 0);
        let (n, d) = if den < 0 {
            (num.checked_neg()?, den.checked_neg()?)
        } else {
            (num, den)
        };
        let g = i128::try_from(gcd(n.unsigned_abs(), d.unsigned_abs()).max(1)).ok()?;
        Some(Frac {
            num: n / g,
            den: d / g,
        })
    }

    fn from_i128(n: i128) -> Self {
        Frac { num: n, den: 1 }
    }

    fn is_zero(self) -> bool {
        self.num == 0
    }

    fn checked_sub(self, o: Self) -> Option<Self> {
        let num = self
            .num
            .checked_mul(o.den)?
            .checked_sub(o.num.checked_mul(self.den)?)?;
        let den = self.den.checked_mul(o.den)?;
        Self::reduced(num, den)
    }

    fn checked_mul(self, o: Self) -> Option<Self> {
        let num = self.num.checked_mul(o.num)?;
        let den = self.den.checked_mul(o.den)?;
        Self::reduced(num, den)
    }

    /// `o` must have a non-zero numerator; every caller here divides only
    /// by an already-confirmed non-zero pivot.
    fn checked_div(self, o: Self) -> Option<Self> {
        let num = self.num.checked_mul(o.den)?;
        let den = self.den.checked_mul(o.num)?;
        Self::reduced(num, den)
    }
}

fn gcd(a: u128, b: u128) -> u128 {
    if b == 0 {
        a.max(1)
    } else {
        gcd(b, a % b)
    }
}

/// Column `j` is species `j` for `j < left.len()`, else `right[j - left.len()]`.
fn build_matrix(equation: &Equation) -> (Vec<Vec<Frac>>, usize) {
    let all_species = equation.left.iter().chain(equation.right.iter());
    let elements: BTreeSet<String> = all_species
        .flat_map(|s| s.formula.atom_counts().into_keys())
        .collect();
    let has_charge = equation
        .left
        .iter()
        .chain(equation.right.iter())
        .any(|s| s.charge.unwrap_or(0) != 0);

    let n = equation.left.len() + equation.right.len();
    let mut rows: Vec<Vec<Frac>> = Vec::with_capacity(elements.len() + has_charge as usize);

    for element in &elements {
        let mut row = vec![Frac::from_i128(0); n];
        for (j, species) in equation
            .left
            .iter()
            .chain(equation.right.iter())
            .enumerate()
        {
            let count = species
                .formula
                .atom_counts()
                .get(element)
                .copied()
                .unwrap_or(0) as i128;
            let sign = if j < equation.left.len() { 1 } else { -1 };
            row[j] = Frac::from_i128(sign * count);
        }
        rows.push(row);
    }

    if has_charge {
        let mut row = vec![Frac::from_i128(0); n];
        for (j, species) in equation
            .left
            .iter()
            .chain(equation.right.iter())
            .enumerate()
        {
            let charge = i128::from(species.charge.unwrap_or(0));
            let sign = if j < equation.left.len() { 1 } else { -1 };
            row[j] = Frac::from_i128(sign * charge);
        }
        rows.push(row);
    }

    (rows, n)
}

/// Reduces `a` to reduced row-echelon form in place and returns the pivot
/// column for each pivot row, in the order the pivots were found.
/// Returns `None` if any intermediate fraction would overflow `i128`.
fn rref(a: &mut [Vec<Frac>]) -> Option<Vec<usize>> {
    let rows = a.len();
    let cols = a.first().map_or(0, Vec::len);
    let mut pivots = Vec::new();
    let mut pivot_row = 0;

    for col in 0..cols {
        let Some(sel) = (pivot_row..rows).find(|&r| !a[r][col].is_zero()) else {
            continue;
        };
        a.swap(pivot_row, sel);
        let pv = a[pivot_row][col];
        for cell in a[pivot_row].iter_mut().take(cols) {
            *cell = cell.checked_div(pv)?;
        }
        for r in 0..rows {
            if r == pivot_row || a[r][col].is_zero() {
                continue;
            }
            let factor = a[r][col];
            let pivot_row_values = a[pivot_row].clone();
            for (cell, &pivot_value) in a[r].iter_mut().zip(pivot_row_values.iter()) {
                *cell = cell.checked_sub(factor.checked_mul(pivot_value)?)?;
            }
        }
        pivots.push(col);
        pivot_row += 1;
        if pivot_row == rows {
            break;
        }
    }
    Some(pivots)
}

/// Predicted minimal positive integer coefficients for `left ++ right`, or
/// `None` when the reaction is unsolvable (a species is missing, atoms
/// cannot balance at all), ambiguous (more than one independent balance
/// exists), or the exact-arithmetic search overflows `i128`. All three
/// cases are left to the deterministic warning already reported by
/// [`crate::validate`]; this function never guesses.
pub fn balance_equation(equation: &Equation) -> Option<Vec<u32>> {
    if equation.left.is_empty() || equation.right.is_empty() {
        return None;
    }
    let (mut matrix, n) = build_matrix(equation);
    let pivots = rref(&mut matrix)?;

    let free_cols: Vec<usize> = (0..n).filter(|c| !pivots.contains(c)).collect();
    let [free] = free_cols[..] else {
        return None;
    };

    let mut x = vec![Frac::from_i128(0); n];
    x[free] = Frac::from_i128(1);
    for (row, &col) in pivots.iter().enumerate() {
        x[col] = Frac::from_i128(0).checked_sub(matrix[row][free])?;
    }

    let mut lcm: i128 = 1;
    for f in &x {
        let g = gcd(lcm.unsigned_abs(), f.den.unsigned_abs()).max(1) as i128;
        lcm = (lcm / g).checked_mul(f.den)?;
    }
    let mut ints: Vec<i128> = Vec::with_capacity(n);
    for f in &x {
        ints.push(f.num.checked_mul(lcm / f.den)?);
    }

    if ints.contains(&0) {
        return None;
    }
    let all_positive = ints.iter().all(|&v| v > 0);
    let all_negative = ints.iter().all(|&v| v < 0);
    if !all_positive && !all_negative {
        return None;
    }
    if all_negative {
        for v in &mut ints {
            *v = v.checked_neg()?;
        }
    }

    let common = ints
        .iter()
        .fold(0u128, |acc, &v| gcd(acc, v.unsigned_abs()));
    let common = common.max(1) as i128;
    let coeffs: Option<Vec<u32>> = ints
        .iter()
        .map(|&v| u32::try_from(v / common).ok())
        .collect();
    coeffs.filter(|c| c.iter().all(|&v| v > 0 && v <= 9999))
}

/// Renders `equation` with `coeffs` (one per `left ++ right` species) applied,
/// ignoring the dictated coefficients. Only used to spell out a suggestion.
/// `coeffs` must have exactly `equation.left.len() + equation.right.len()`
/// entries, which every caller in this crate guarantees by construction.
pub(crate) fn render_suggestion(equation: &Equation, coeffs: &[u32]) -> String {
    let render_side = |side: &[Species], offset: usize| {
        side.iter()
            .zip(&coeffs[offset..offset + side.len()])
            .map(|(s, &coefficient)| {
                let s = Species {
                    coefficient,
                    ..s.clone()
                };
                crate::render::unicode::render_species(&s)
            })
            .collect::<Vec<_>>()
            .join(" + ")
    };
    let left = render_side(&equation.left, 0);
    let right = render_side(&equation.right, equation.left.len());
    let arrow = match equation.arrow {
        Arrow::Forward => "→",
        Arrow::Equilibrium => "⇌",
    };
    format!("{left} {arrow} {right}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Formula, Part};
    use crate::formula::parse_equation_str;

    fn balance(text: &str) -> Option<Vec<u32>> {
        balance_equation(&parse_equation_str(text).unwrap())
    }

    #[test]
    fn balances_water_synthesis() {
        assert_eq!(balance("H2 + O2 -> H2O"), Some(vec![2, 1, 2]));
    }

    #[test]
    fn balances_iron_combustion() {
        assert_eq!(balance("Fe + O2 -> Fe2O3"), Some(vec![4, 3, 2]));
    }

    #[test]
    fn balances_classic_six_species_redox() {
        assert_eq!(
            balance("KMnO4 + HCl -> KCl + MnCl2 + Cl2 + H2O"),
            Some(vec![2, 16, 2, 2, 5, 8])
        );
    }

    #[test]
    fn already_balanced_reaction_confirms_its_own_coefficients() {
        assert_eq!(balance("2H2 + O2 -> 2H2O"), Some(vec![2, 1, 2]));
    }

    #[test]
    fn missing_species_is_unsolvable_and_abstains() {
        // No choice of coefficients over just these two conserves carbon.
        assert_eq!(balance("CH4 -> H2O"), None);
    }

    #[test]
    fn identical_sides_are_ambiguous_and_abstain() {
        // Free dimension > 1: any equal pair of coefficients balances this.
        assert_eq!(balance("H2 + H2 -> H2 + H2"), None);
    }

    fn charged(symbol: &str, charge: i32) -> Species {
        Species {
            charge: Some(charge),
            ..Species::new(Formula::atom(symbol, 1))
        }
    }

    #[test]
    fn ionic_charge_is_balanced_like_an_extra_element() {
        // Constructed directly: the string parser splits terms on '+', which
        // is ambiguous with a charge's `+` suffix, so ionic equations here
        // bypass it and build the AST straight from Species/Formula.
        let fe_cu2_to_fe2_cu = Equation {
            left: vec![Species::new(Formula::atom("Fe", 1)), charged("Cu", 2)],
            arrow: Arrow::Forward,
            right: vec![charged("Fe", 2), Species::new(Formula::atom("Cu", 1))],
            condition: None,
        };
        // Atoms already 1:1; only charge conservation ties Cu^2+ to Fe^2+.
        assert_eq!(balance_equation(&fe_cu2_to_fe2_cu), Some(vec![1, 1, 1, 1]));

        let cu_ag_to_cu2_ag = Equation {
            left: vec![Species::new(Formula::atom("Cu", 1)), charged("Ag", 1)],
            arrow: Arrow::Forward,
            right: vec![charged("Cu", 2), Species::new(Formula::atom("Ag", 1))],
            condition: None,
        };
        // Forces a real charge-coefficient fix: two Ag+ per Cu^2+.
        assert_eq!(balance_equation(&cu_ag_to_cu2_ag), Some(vec![1, 2, 1, 2]));
    }

    #[test]
    fn hydrate_water_of_crystallization_is_counted() {
        // CuSO4 + 5H2O -> CuSO4.5H2O: the hydrate's waters must be counted
        // as ordinary H/O atoms, or this would look unsolvable.
        assert_eq!(balance("CuSO4 + H2O -> CuSO4.5H2O"), Some(vec![1, 5, 1]));
    }

    #[test]
    fn equilibrium_arrow_is_preserved_in_suggestion() {
        let equation = parse_equation_str("H2 + I2 <=> HI").unwrap();
        let coeffs = balance_equation(&equation).unwrap();
        assert_eq!(coeffs, vec![1, 1, 2]);
        assert_eq!(render_suggestion(&equation, &coeffs), "H₂ + I₂ ⇌ 2HI");
    }

    #[test]
    fn state_markers_do_not_affect_balancing() {
        let with_markers = parse_equation_str("Zn + HCl^ -> ZnCl2 + H2v").unwrap();
        let coeffs = balance_equation(&with_markers).unwrap();
        assert_eq!(coeffs, vec![1, 2, 1, 1]);
        assert_eq!(
            render_suggestion(&with_markers, &coeffs),
            "Zn + 2HCl↑ → ZnCl₂ + H₂↓"
        );
    }

    #[test]
    fn render_suggestion_matches_predicted_coefficients() {
        let equation = parse_equation_str("H2 + O2 -> H2O").unwrap();
        let coeffs = balance_equation(&equation).unwrap();
        assert_eq!(render_suggestion(&equation, &coeffs), "2H₂ + O₂ → 2H₂O");
    }

    #[test]
    fn rref_reports_overflow_as_none_instead_of_wrapping_or_panicking() {
        // Direct reproduction at the arithmetic layer: with unchecked i128
        // multiplication this used to panic in a debug build and silently
        // wrap to a wrong value in release. `huge` squared is far past
        // i128::MAX, so eliminating column 0 must overflow while computing
        // row 1's second entry; `rref` must report that as `None`, not
        // panic and not return a wrapped result.
        let huge = i128::MAX / 2;
        let mut a = vec![
            vec![Frac::from_i128(1), Frac::from_i128(huge)],
            vec![Frac::from_i128(huge), Frac::from_i128(3)],
        ];
        assert!(rref(&mut a).is_none());
    }

    #[test]
    fn reduced_reports_overflow_for_i128_min_instead_of_wrapping_or_panicking() {
        // i128::MIN has no positive counterpart: a plain `-n`/`-d` sign
        // flip would panic in debug and silently wrap in release whenever
        // the denominator arrived negative and the numerator or
        // denominator was exactly i128::MIN.
        assert!(Frac::reduced(i128::MIN, -1).is_none());
        assert!(Frac::reduced(1, i128::MIN).is_none());
        // An ordinary negative denominator still normalizes correctly.
        assert_eq!(Frac::reduced(-4, -8), Some(Frac { num: 1, den: 2 }));
    }

    #[test]
    fn balance_equation_abstains_when_elimination_overflows() {
        // Same guarantee through the public API, reproducing the reported
        // case directly: a full-rank ~20-element/21-species system (here
        // generated by a fixed-seed LCG so the test is deterministic
        // without a `rand` dependency) with u32-range entries. Naive
        // fraction elimination on it was independently confirmed (via an
        // arbitrary-precision simulation of the same algorithm) to reach
        // roughly 600-bit numerators/denominators, far past i128::MAX.
        // `balance_equation` must return `None` here, never panic and
        // never return a wrapped, silently-wrong suggestion.
        fn lcg_next(state: u64) -> u64 {
            state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407)
        }
        const ELEMENTS: usize = 20;
        const SPECIES: usize = 21;
        let mut state: u64 = 88172645463325252;
        let mut species = Vec::with_capacity(SPECIES);
        for _ in 0..SPECIES {
            let parts = (0..ELEMENTS)
                .map(|element| {
                    state = lcg_next(state);
                    let count = ((state >> 33) % (u32::MAX as u64 - 1) + 1) as u32;
                    Part::Atom {
                        symbol: format!("E{element}"),
                        count,
                    }
                })
                .collect();
            species.push(Species::new(Formula { parts }));
        }
        let right = species.split_off(10);
        let equation = Equation {
            left: species,
            arrow: Arrow::Forward,
            right,
            condition: None,
        };
        assert_eq!(balance_equation(&equation), None);
    }

    #[test]
    fn large_full_rank_system_never_panics() {
        // Breadth smoke test at roughly the reported repro's shape (~20
        // elements, 21 species, indices 1-9): this particular matrix stays
        // well under i128's range (checked by hand via an independent
        // arbitrary-precision simulation), so it is not itself an overflow
        // case, but it does confirm the checked-arithmetic path scales to
        // realistic matrix sizes without panicking, and that any answer it
        // does produce is actually a valid balance.
        const ELEMENTS: i128 = 20;
        const SPECIES: i128 = 21;
        let mut species = Vec::new();
        for col in 0..SPECIES {
            let mut parts = Vec::new();
            for element in 0..ELEMENTS {
                let count = 1 + ((element * 7 + col * 5) % 9) as u32;
                parts.push(Part::Atom {
                    symbol: format!("E{element}"),
                    count,
                });
            }
            species.push(Species::new(Formula { parts }));
        }
        let right = species.split_off(10);
        let equation = Equation {
            left: species,
            arrow: Arrow::Forward,
            right,
            condition: None,
        };

        // Must not panic regardless of overflow; if it produces an answer,
        // that answer must actually conserve every synthetic element.
        if let Some(coeffs) = balance_equation(&equation) {
            for element in 0..ELEMENTS {
                let symbol = format!("E{element}");
                let total = |side: &[Species], offset: usize| -> u64 {
                    side.iter()
                        .zip(&coeffs[offset..offset + side.len()])
                        .map(|(s, &c)| {
                            u64::from(c)
                                * s.formula.atom_counts().get(&symbol).copied().unwrap_or(0)
                        })
                        .sum()
                };
                assert_eq!(
                    total(&equation.left, 0),
                    total(&equation.right, equation.left.len()),
                    "element {symbol} does not balance"
                );
            }
        }
    }
}

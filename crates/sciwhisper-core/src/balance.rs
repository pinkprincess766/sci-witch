//! Deterministic integer balancing of dictated reactions via null-space
//! computation over the atom (and, when present, charge) conservation matrix.
//! This never changes what the user dictated: it only proposes coefficients
//! for `chemistry.balance_suggestion`, computed independently of the spoken
//! ones, and abstains whenever the system is under- or over-determined.
//!
//! All fraction arithmetic below is `checked_*`: naive Gaussian elimination
//! on fractions can blow numerator/denominator size up exponentially over
//! enough steps, and this crate abstains on overflow rather than wrapping
//! into a wrong suggestion or panicking on a debug build. For matrices whose
//! Hadamard bound is at most `HADAMARD_LIMIT` overflow is proven impossible
//! (`docs/compiler/BALANCE_KERNEL_RU.md`); larger ones may still balance.

use std::collections::{BTreeMap, BTreeSet};

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
/// `None` if any species' atom count overflows `u64` (see
/// [`crate::ast::Formula::atom_counts`]) rather than building a matrix from
/// a silently-wrapped count.
fn build_matrix(equation: &Equation) -> Option<(Vec<Vec<Frac>>, usize)> {
    let species: Vec<&Species> = equation.left.iter().chain(equation.right.iter()).collect();
    let counts: Vec<BTreeMap<String, u64>> = species
        .iter()
        .map(|s| s.formula.atom_counts())
        .collect::<Option<_>>()?;
    let elements: BTreeSet<&String> = counts.iter().flat_map(BTreeMap::keys).collect();
    let has_charge = species.iter().any(|s| s.charge.unwrap_or(0) != 0);

    let n = species.len();
    let mut rows: Vec<Vec<Frac>> = Vec::with_capacity(elements.len() + has_charge as usize);

    for element in &elements {
        let mut row = vec![Frac::from_i128(0); n];
        for (j, count) in counts.iter().enumerate() {
            let count = count.get(*element).copied().unwrap_or(0) as i128;
            let sign = if j < equation.left.len() { 1 } else { -1 };
            row[j] = Frac::from_i128(sign * count);
        }
        rows.push(row);
    }

    if has_charge {
        let mut row = vec![Frac::from_i128(0); n];
        for (j, s) in species.iter().enumerate() {
            let charge = i128::from(s.charge.unwrap_or(0));
            let sign = if j < equation.left.len() { 1 } else { -1 };
            row[j] = Frac::from_i128(sign * charge);
        }
        rows.push(row);
    }

    Some((rows, n))
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

/// Largest Hadamard bound `B` (see [`hadamard_bound_sq`]) for which `rref`
/// is proven not to overflow `i128`. The largest value `rref` forms is a
/// difference of two products of three stored values, at most `2 * B^3`, and
/// `2 * (2^42)^3 = 2^127 > i128::MAX`, so `B <= 2^42 - 1`. The derivation is
/// in `docs/compiler/BALANCE_KERNEL_RU.md`.
const HADAMARD_LIMIT: u128 = (1 << 42) - 1;

/// Exact sum of squares, `None` if it does not fit `u128`.
fn sum_of_squares(mut values: impl Iterator<Item = i128>) -> Option<u128> {
    values.try_fold(0u128, |acc, v| {
        acc.checked_add(v.unsigned_abs().checked_mul(v.unsigned_abs())?)
    })
}

/// Product of the `size` largest `max(1, s)` over the squared norms `sq`.
fn top_product(mut sq: Vec<u128>, size: usize) -> Option<u128> {
    sq.sort_unstable_by(|a, b| b.cmp(a));
    sq.iter()
        .take(size)
        .try_fold(1u128, |acc, &s| acc.checked_mul(s.max(1)))
}

/// Squared Hadamard bound `B^2` on every square minor of the integer matrix
/// `matrix` (all entries must have `den == 1`): `B` is the smaller of the
/// products of the `min(rows, cols)` largest values of `max(1, row norm)`
/// and of `max(1, column norm)`. Exact integer arithmetic; `None` when `B^2`
/// does not fit `u128`, which is far above [`HADAMARD_LIMIT`] squared.
fn hadamard_bound_sq(matrix: &[Vec<Frac>]) -> Option<u128> {
    debug_assert!(matrix.iter().flatten().all(|f| f.den == 1));
    let rows = matrix.len();
    let cols = matrix.first().map_or(0, Vec::len);
    let size = rows.min(cols);
    let by_rows = matrix
        .iter()
        .map(|row| sum_of_squares(row.iter().map(|f| f.num)))
        .collect::<Option<Vec<u128>>>()
        .and_then(|sq| top_product(sq, size));
    let by_cols = (0..cols)
        .map(|j| sum_of_squares(matrix.iter().map(|row| row[j].num)))
        .collect::<Option<Vec<u128>>>()
        .and_then(|sq| top_product(sq, size));
    match (by_rows, by_cols) {
        (Some(r), Some(c)) => Some(r.min(c)),
        (r, c) => r.or(c),
    }
}

/// `true` if `rref` on `matrix` is proven not to overflow `i128`. A `false`
/// answer proves nothing: the condition is sufficient, not necessary.
fn elimination_is_proven_safe(matrix: &[Vec<Frac>]) -> bool {
    hadamard_bound_sq(matrix).is_some_and(|b_sq| b_sq <= HADAMARD_LIMIT * HADAMARD_LIMIT)
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
    let (mut matrix, n) = build_matrix(equation)?;
    let proven_safe = cfg!(debug_assertions) && elimination_is_proven_safe(&matrix);
    let pivots = rref(&mut matrix);
    debug_assert!(
        pivots.is_some() || !proven_safe,
        "rref overflowed on a matrix inside the proven Hadamard region"
    );
    let pivots = pivots?;

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

    #[test]
    fn two_independent_reactions_in_one_equation_abstain() {
        // C + O2 -> CO2 and 2Na + Cl2 -> 2NaCl share no element, so any
        // positive mix of the two balances: rank 4 over 6 species leaves a
        // two-dimensional kernel, and no single answer is the right one.
        assert_eq!(balance("C + O2 + Na + Cl2 -> CO2 + NaCl"), None);
        // Each half on its own has a one-dimensional kernel and balances.
        assert_eq!(balance("C + O2 -> CO2"), Some(vec![1, 1, 1]));
        assert_eq!(balance("Na + Cl2 -> NaCl"), Some(vec![2, 1, 2]));
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

    /// A full-rank ~20-element/21-species system, generated by a fixed-seed
    /// LCG so the test is deterministic without a `rand` dependency, with
    /// u32-range entries.
    fn overflow_equation() -> Equation {
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
        Equation {
            left: species,
            arrow: Arrow::Forward,
            right,
            condition: None,
        }
    }

    #[test]
    fn balance_equation_abstains_when_elimination_overflows() {
        // Same guarantee through the public API, reproducing the reported
        // case directly (`overflow_equation`). Naive fraction elimination on
        // it was independently confirmed (via an arbitrary-precision
        // simulation of the same algorithm) to reach roughly 600-bit
        // numerators/denominators, far past i128::MAX. `balance_equation`
        // must return `None` here, never panic and never return a wrapped,
        // silently-wrong suggestion.
        assert_eq!(balance_equation(&overflow_equation()), None);
    }

    fn int_matrix(rows: &[&[i128]]) -> Vec<Vec<Frac>> {
        rows.iter()
            .map(|row| row.iter().map(|&v| Frac::from_i128(v)).collect())
            .collect()
    }

    /// Small deterministic generator, entries in `-spread..=spread`.
    struct Lcg(u64);

    impl Lcg {
        fn next_in(&mut self, spread: i128) -> i128 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let width = 2 * spread + 1;
            i128::from(self.0 >> 33) % width - spread
        }
    }

    fn random_matrix(rng: &mut Lcg, rows: usize, cols: usize, spread: i128) -> Vec<Vec<Frac>> {
        (0..rows)
            .map(|_| {
                (0..cols)
                    .map(|_| Frac::from_i128(rng.next_in(spread)))
                    .collect()
            })
            .collect()
    }

    /// Determinant by Laplace expansion along the first row; small sizes only.
    fn det(m: &[Vec<i128>]) -> i128 {
        if m.len() == 1 {
            return m[0][0];
        }
        (0..m.len())
            .map(|j| {
                let minor: Vec<Vec<i128>> = m[1..]
                    .iter()
                    .map(|row| {
                        row.iter()
                            .enumerate()
                            .filter(|&(c, _)| c != j)
                            .map(|(_, &v)| v)
                            .collect()
                    })
                    .collect();
                let sign = if j % 2 == 0 { 1 } else { -1 };
                sign * m[0][j] * det(&minor)
            })
            .sum()
    }

    /// Largest absolute value of any square minor of any size (brute force).
    fn max_minor(matrix: &[Vec<Frac>]) -> u128 {
        let rows = matrix.len();
        let cols = matrix[0].len();
        let mut best = 0u128;
        for row_mask in 1u32..(1 << rows) {
            for col_mask in 1u32..(1 << cols) {
                if row_mask.count_ones() != col_mask.count_ones() {
                    continue;
                }
                let sub: Vec<Vec<i128>> = (0..rows)
                    .filter(|r| (row_mask >> r) & 1 == 1)
                    .map(|r| {
                        (0..cols)
                            .filter(|c| (col_mask >> c) & 1 == 1)
                            .map(|c| matrix[r][c].num)
                            .collect()
                    })
                    .collect();
                best = best.max(det(&sub).unsigned_abs());
            }
        }
        best
    }

    #[test]
    fn hadamard_bound_matches_hand_computed_values() {
        // Rows: 1+4+9=14 and 16+25+36=77, product 1078. Columns: 17, 29, 45,
        // the two largest multiply to 1305. The smaller one wins.
        let m = int_matrix(&[&[1, 2, 3], &[4, 5, 6]]);
        assert_eq!(hadamard_bound_sq(&m), Some(1078));
        // A single column: only 1x1 minors exist, so only the largest row
        // norm counts (4), not the product of all three rows.
        let m = int_matrix(&[&[1], &[2], &[2]]);
        assert_eq!(hadamard_bound_sq(&m), Some(4));
        // A zero row contributes the factor 1, not 0: the bound stays valid.
        let m = int_matrix(&[&[0, 0], &[3, 4]]);
        assert_eq!(hadamard_bound_sq(&m), Some(25));
        // An entry too large to square: the bound does not fit u128.
        let m = int_matrix(&[&[i128::MAX / 2]]);
        assert_eq!(hadamard_bound_sq(&m), None);
    }

    #[test]
    fn hadamard_bound_dominates_every_minor() {
        let mut rng = Lcg(1);
        for (rows, cols) in [(2, 3), (3, 3), (3, 4), (4, 5), (5, 3)] {
            for _ in 0..40 {
                let m = random_matrix(&mut rng, rows, cols, 9);
                let minor = max_minor(&m);
                let bound_sq = hadamard_bound_sq(&m).unwrap();
                assert!(minor * minor <= bound_sq, "{rows}x{cols} {m:?}");
            }
        }
    }

    #[test]
    fn stored_fractions_never_exceed_the_largest_minor() {
        // The lemma behind the bound: after every pivot step each stored
        // fraction has |num| and den at most the largest minor. Elimination
        // on the first `t` columns is exactly the state after those columns,
        // so every prefix checks one intermediate state.
        let mut rng = Lcg(2);
        for (rows, cols) in [(2, 4), (3, 4), (3, 5), (4, 5), (4, 4)] {
            for _ in 0..40 {
                let full = random_matrix(&mut rng, rows, cols, 7);
                for t in 1..=cols {
                    let prefix: Vec<Vec<Frac>> = full.iter().map(|r| r[..t].to_vec()).collect();
                    let limit = max_minor(&prefix).max(1) as i128;
                    let mut state = prefix.clone();
                    assert!(rref(&mut state).is_some());
                    for f in state.iter().flatten() {
                        assert!(f.num.abs() <= limit && f.den <= limit, "{prefix:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn limit_is_the_exact_boundary_for_a_single_entry() {
        let limit = HADAMARD_LIMIT as i128;
        assert!(elimination_is_proven_safe(&int_matrix(&[&[limit]])));
        assert!(!elimination_is_proven_safe(&int_matrix(&[&[limit + 1]])));
    }

    #[test]
    fn random_matrices_below_the_limit_never_overflow() {
        // For each shape pick the largest entry spread whose worst-case row
        // norm `spread * sqrt(cols)` keeps the product over `min(rows, cols)`
        // rows near the limit, then require the check to accept and `rref`
        // to succeed on every sample.
        let mut rng = Lcg(3);
        for (rows, cols) in [(2, 3), (3, 4), (4, 5), (5, 6), (6, 7), (7, 8), (8, 9)] {
            let size = rows.min(cols) as f64;
            let spread = ((HADAMARD_LIMIT as f64).powf(1.0 / size) / (cols as f64).sqrt()) as i128;
            assert!(spread >= 3, "{rows}x{cols}");
            for _ in 0..200 {
                let m = random_matrix(&mut rng, rows, cols, spread);
                assert!(elimination_is_proven_safe(&m), "{rows}x{cols} {spread}");
                assert!(rref(&mut m.clone()).is_some(), "{rows}x{cols} {m:?}");
            }
        }
    }

    /// Two private elements, `L0 = (u, v)`, `L1 = (v, u)`, `R = 2 L0 + 3 L1`:
    /// the unique balance is `[2, 3, 1]`.
    fn big_count_equation(u: u32, v: u32) -> Equation {
        let two_species = |a: u32, b: u32| {
            Species::new(Formula {
                parts: vec![
                    Part::Atom {
                        symbol: "E0".to_string(),
                        count: a,
                    },
                    Part::Atom {
                        symbol: "E1".to_string(),
                        count: b,
                    },
                ],
            })
        };
        Equation {
            left: vec![two_species(u, v), two_species(v, u)],
            arrow: Arrow::Forward,
            right: vec![two_species(2 * u + 3 * v, 2 * v + 3 * u)],
            condition: None,
        }
    }

    #[test]
    fn matrix_just_under_the_limit_balances_without_a_checked_failure() {
        let equation = big_count_equation(1_103_680, 7);
        let (matrix, _) = build_matrix(&equation).unwrap();
        let bound_sq = hadamard_bound_sq(&matrix).unwrap();
        let limit_sq = HADAMARD_LIMIT * HADAMARD_LIMIT;
        // Within 1% of the limit, so the test sits at the boundary.
        assert!(bound_sq <= limit_sq && bound_sq > limit_sq / 100 * 99);
        assert!(rref(&mut matrix.clone()).is_some());
        assert_eq!(balance_equation(&equation), Some(vec![2, 3, 1]));
    }

    #[test]
    fn matrix_just_over_the_limit_is_not_proven_safe() {
        let (matrix, _) = build_matrix(&big_count_equation(1_104_442, 7)).unwrap();
        let bound_sq = hadamard_bound_sq(&matrix).unwrap();
        assert!(bound_sq > HADAMARD_LIMIT * HADAMARD_LIMIT);
        assert!(!elimination_is_proven_safe(&matrix));
    }

    #[test]
    fn the_overflowing_20_by_21_system_is_above_the_limit() {
        let (matrix, _) = build_matrix(&overflow_equation()).unwrap();
        assert!(!elimination_is_proven_safe(&matrix));
        // A bound using only the largest single row norm would call this
        // system safe: the product over rows is what rules it out.
        let largest_row_sq = matrix
            .iter()
            .map(|row| sum_of_squares(row.iter().map(|f| f.num)).unwrap())
            .max()
            .unwrap();
        assert!(largest_row_sq <= HADAMARD_LIMIT * HADAMARD_LIMIT);
    }

    #[test]
    fn a_bound_above_the_limit_that_fits_u128_is_still_rejected() {
        // By rows B^2 = (4e8)^3 = 6.4e25, by columns (3e8)^3 = 2.7e25;
        // both exceed (2^42)^2 = 1.9e25 and fit u128.
        let row: &[i128] = &[10_000, 10_000, 10_000, 10_000];
        let m = int_matrix(&[row; 3]);
        let bound_sq = hadamard_bound_sq(&m).unwrap();
        assert!(bound_sq > HADAMARD_LIMIT * HADAMARD_LIMIT);
        assert!(!elimination_is_proven_safe(&m));
    }

    #[test]
    fn balance_equation_abstains_when_atom_counting_overflows() {
        // `atom_counts` itself (not just the RREF/rational-arithmetic layer)
        // can overflow on an artificially deep or wide Formula. This must
        // surface as a clean `None` from `balance_equation`, not a panic.
        let huge = Formula {
            parts: vec![Part::Group {
                inner: Formula {
                    parts: vec![Part::Group {
                        inner: Formula::atom("X", u32::MAX),
                        count: u32::MAX,
                    }],
                },
                count: u32::MAX,
            }],
        };
        let equation = Equation {
            left: vec![Species::new(huge)],
            arrow: Arrow::Forward,
            right: vec![Species::new(Formula::atom("X", 1))],
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
                                * s.formula
                                    .atom_counts()
                                    .unwrap()
                                    .get(&symbol)
                                    .copied()
                                    .unwrap_or(0)
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

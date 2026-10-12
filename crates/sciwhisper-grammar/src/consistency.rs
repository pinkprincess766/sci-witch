//! Consistency check for a PCFG: do the probabilities of all finite trees of
//! the start symbol sum to 1?
//!
//! Booth & Thompson (1973, IEEE Trans. Computers C-22): with the mean-count
//! matrix `M[A][B] = sum over productions p of A of prob[p] * (occurrences of
//! B in rhs(p))`, the grammar is consistent if the spectral radius rho(M) < 1.
//! The check is a one-sided certificate: rho(M) <= ||M^k||^(1/k) for any k
//! (Gelfand), so ||M^k||_inf < 1 for some k proves rho(M) < 1. A critical
//! grammar (rho = 1) never gives a norm below 1, so it stays `NotProven`.
//!
//! The other side, [`rho_lower_bound`]: for a nonnegative n x n matrix,
//! tr(M^k) = sum of lambda_i^k over the eigenvalues, and
//! |sum of lambda_i^k| <= n * rho^k, so rho >= (tr(M^k) / n)^(1/k) for every
//! k >= 1. Booth & Thompson: a PCFG is inconsistent if rho(M) > 1, provided
//! every nonterminal reachable from the start symbol is productive (derives
//! some terminal string); [`unproductive_nonterminals`] checks that
//! condition. The bound is computed in f64 without interval arithmetic, so a
//! bound only just above 1 is not a proof; one far above 1 is.

use std::collections::{HashMap, HashSet};

use crate::earley::{Production, Symbol};

/// How many times M is squared at most: powers 1, 2, 4, …, 2^MAX_SQUARINGS.
/// 2^20 is far past where any ρ < 0.999 shows up; past this the check
/// gives up rather than loop.
pub const MAX_SQUARINGS: u32 = 20;

#[derive(Clone, Debug, PartialEq)]
pub enum Consistency {
    /// ‖M^power‖∞ = norm < 1, hence ρ(M) < 1.
    Proven { power: u64, norm: f64 },
    /// No power up to 2^MAX_SQUARINGS had norm < 1 (or the norm overflowed);
    /// `norm` is the last finite value seen.
    NotProven { norm: f64 },
}

/// Check that the PCFG `(productions, prob)` is consistent for `start`.
///
/// Only nonterminals reachable from `start` (through every rhs nonterminal,
/// whatever its probability) enter M.
///
/// Panics if `prob.len() != productions.len()` or `start` has no production.
pub fn check_consistency(productions: &[Production], prob: &[f64], start: &str) -> Consistency {
    let mut m = mean_count_matrix(productions, prob, start);

    let mut power: u64 = 1;
    let mut last_finite = f64::NAN;
    for squaring in 0..=MAX_SQUARINGS {
        let norm = norm_inf(&m);
        if !norm.is_finite() {
            break;
        }
        last_finite = norm;
        if norm < 1.0 {
            return Consistency::Proven { power, norm };
        }
        if squaring == MAX_SQUARINGS {
            break;
        }
        m = square(&m);
        power *= 2;
    }
    Consistency::NotProven { norm: last_finite }
}

/// Largest lower bound on rho(M) found, and the power k it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct RhoLowerBound {
    pub power: u64,
    pub bound: f64,
}

/// Largest (tr(M^k)/n)^(1/k) over k = 1, 2, 4, …, 2^MAX_SQUARINGS while M^k
/// stays finite.
///
/// For a nonnegative n x n matrix, tr(M^k) = sum of lambda_i^k and
/// |sum of lambda_i^k| <= n * rho^k, hence rho(M) >= (tr(M^k)/n)^(1/k) for
/// every k >= 1. The root is taken in log space, `(ln tr - ln n) / k`, so a
/// huge trace does not overflow; a trace of 0 gives 0 for that k. Stops at the
/// first power with a non-finite entry and keeps the best bound seen. Ties
/// keep the smaller power. `M` covers the same nonterminals as
/// [`check_consistency`].
///
/// Booth & Thompson (1973): if the bound exceeds 1 and
/// [`unproductive_nonterminals`] is empty, the PCFG is inconsistent. The
/// bound alone proves nothing about consistency.
///
/// Panics if `prob.len() != productions.len()` or `start` has no production.
pub fn rho_lower_bound(productions: &[Production], prob: &[f64], start: &str) -> RhoLowerBound {
    let mut m = mean_count_matrix(productions, prob, start);
    let ln_n = (m.len() as f64).ln();
    let mut power: u64 = 1;
    let mut best = RhoLowerBound { power, bound: 0.0 };
    for squaring in 0..=MAX_SQUARINGS {
        if m.iter().flatten().any(|x| !x.is_finite()) {
            break;
        }
        let trace: f64 = m.iter().enumerate().map(|(i, row)| row[i]).sum();
        let bound = if trace > 0.0 {
            ((trace.ln() - ln_n) / power as f64).exp()
        } else {
            0.0
        };
        if bound > best.bound {
            best = RhoLowerBound { power, bound };
        }
        if squaring == MAX_SQUARINGS {
            break;
        }
        m = square(&m);
        power *= 2;
    }
    best
}

/// Nonterminals reachable from `start` that derive no terminal string, sorted.
///
/// Least fixpoint: a nonterminal is productive if some production of it has
/// only productive nonterminals on the right (terminals and an empty rhs
/// count as productive: ε is a terminal string). A nonterminal with no
/// production is never productive.
pub fn unproductive_nonterminals(productions: &[Production], start: &str) -> Vec<String> {
    let mut productive: HashSet<&str> = HashSet::new();
    loop {
        let before = productive.len();
        for prod in productions {
            let all_productive = prod.rhs.iter().all(|sym| match sym {
                Symbol::Terminal(_) => true,
                Symbol::Nonterminal(b) => productive.contains(b.as_str()),
            });
            if all_productive {
                productive.insert(prod.lhs.as_str());
            }
        }
        if productive.len() == before {
            break;
        }
    }
    let mut out: Vec<String> = reachable(productions, start)
        .into_iter()
        .filter(|name| !productive.contains(name))
        .map(String::from)
        .collect();
    out.sort();
    out
}

/// Nonterminals reachable from `start` (through every rhs nonterminal,
/// whatever its probability), in discovery order.
fn reachable<'a>(productions: &'a [Production], start: &'a str) -> Vec<&'a str> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut order: Vec<&str> = vec![start];
    seen.insert(start, 0);
    let mut next = 0;
    while next < order.len() {
        let name = order[next];
        next += 1;
        for prod in productions.iter().filter(|p| p.lhs == name) {
            for sym in &prod.rhs {
                if let Symbol::Nonterminal(b) = sym {
                    if !seen.contains_key(b.as_str()) {
                        seen.insert(b.as_str(), order.len());
                        order.push(b.as_str());
                    }
                }
            }
        }
    }
    order
}

/// The mean-count matrix M over the nonterminals reachable from `start`.
///
/// Panics if `prob.len() != productions.len()` or `start` has no production.
fn mean_count_matrix(productions: &[Production], prob: &[f64], start: &str) -> Vec<Vec<f64>> {
    assert_eq!(
        prob.len(),
        productions.len(),
        "one probability per production"
    );
    assert!(
        productions.iter().any(|p| p.lhs == start),
        "start symbol {start:?} has no production"
    );

    let order = reachable(productions, start);
    let index: HashMap<&str, usize> = order.iter().enumerate().map(|(i, &x)| (x, i)).collect();
    let n = order.len();
    let mut m = vec![vec![0.0f64; n]; n];
    for (prod, &p) in productions.iter().zip(prob) {
        let Some(&a) = index.get(prod.lhs.as_str()) else {
            continue;
        };
        for sym in &prod.rhs {
            if let Symbol::Nonterminal(b) = sym {
                m[a][index[b.as_str()]] += p;
            }
        }
    }
    m
}

/// Max row sum of absolute values.
fn norm_inf(m: &[Vec<f64>]) -> f64 {
    m.iter()
        .map(|row| row.iter().map(|x| x.abs()).sum::<f64>())
        .fold(0.0, f64::max)
}

/// Naive O(n^3) product m * m.
fn square(m: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let n = m.len();
    let mut out = vec![vec![0.0f64; n]; n];
    for (row_in, row_out) in m.iter().zip(out.iter_mut()) {
        for (&mik, row_k) in row_in.iter().zip(m) {
            if mik == 0.0 {
                continue;
            }
            for (o, &mkj) in row_out.iter_mut().zip(row_k) {
                *o += mik * mkj;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earley::productions;
    use crate::ebnf::parse;

    fn prods(src: &str) -> Vec<Production> {
        productions(&parse(src).expect("toy grammar parses"))
    }

    const S_SS: &str = "s = s s | \"a\" ;";

    #[test]
    fn single_rule_binary_split() {
        let p = prods(S_SS);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].lhs, "s");
        assert_eq!(p[0].rhs.len(), 2); // s -> s s
        assert_eq!(p[1].rhs.len(), 1); // s -> a
                                       // M = [0.4 * 2] = [0.8]
        assert_eq!(
            check_consistency(&p, &[0.4, 0.6], "s"),
            Consistency::Proven {
                power: 1,
                norm: 0.8
            }
        );
        // M = [1.0]: critical, rho = 1.
        assert_eq!(
            check_consistency(&p, &[0.5, 0.5], "s"),
            Consistency::NotProven { norm: 1.0 }
        );
        // M = [1.2]: inconsistent. Squares to 1.44, 2.07, … and gives up.
        match check_consistency(&p, &[0.6, 0.4], "s") {
            Consistency::NotProven { norm } => assert!(norm.is_finite() && norm >= 1.0),
            other => panic!("expected NotProven, got {other:?}"),
        }
    }

    #[test]
    fn needs_squaring() {
        // ids: 0 a->b b, 1 b->a, 2 b->x
        let p = prods("a = b b ; b = a | \"x\" ;");
        assert_eq!(p.len(), 3);
        // States a, b. M = [[0, 2*1.0], [0.3, 0]]; ||M|| = 2.
        // M^2 = [[2*0.3, 0], [0, 0.3*2]] = [[0.6, 0], [0, 0.6]]; rho = sqrt(0.6).
        match check_consistency(&p, &[1.0, 0.3, 0.7], "a") {
            Consistency::Proven { power, norm } => {
                assert_eq!(power, 2);
                assert!((norm - 0.6).abs() < 1e-12, "norm = {norm}");
            }
            other => panic!("expected Proven, got {other:?}"),
        }
    }

    #[test]
    fn unreachable_rule_is_ignored() {
        // z is unreachable from s and would make M[z][z] = 2.
        let p = prods("s = s s | \"a\" ; z = z z ;");
        assert_eq!(p.len(), 3);
        assert_eq!(
            check_consistency(&p, &[0.4, 0.6, 1.0], "s"),
            Consistency::Proven {
                power: 1,
                norm: 0.8
            }
        );
    }

    #[test]
    fn cycle_proven_only_at_full_power() {
        // Cycle c0 -> c1 -> … -> c7 -> c0, rule i is `ci = c(i+1) c(i+1) | "x"`.
        // Edge weight is 2 * prob(pair production). Weights: 2 (prob 1.0),
        // 0.25 (prob 0.125), then six edges of 1 (prob 0.5). Product around
        // the cycle is 0.5. Every shorter window of 7 or fewer consecutive
        // edges that contains the 2 but not the 0.25 has product >= 1, so
        // M^1, M^2, M^4 have norm >= 1 and M^8 = 0.5 * I.
        let names = ["ca", "cb", "cc", "cd", "ce", "cf", "cg", "ch"];
        let pair_prob = [1.0, 0.125, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5];
        let mut src = String::new();
        let mut prob = Vec::new();
        for i in 0..8 {
            let next = names[(i + 1) % 8];
            src += &format!("{} = {next} {next} | \"x\" ;\n", names[i]);
            prob.push(pair_prob[i]);
            prob.push(1.0 - pair_prob[i]);
        }
        match check_consistency(&prods(&src), &prob, "ca") {
            Consistency::Proven { power, norm } => {
                assert_eq!(power, 8);
                assert!((norm - 0.5).abs() < 1e-12, "norm = {norm}");
            }
            other => panic!("expected Proven, got {other:?}"),
        }
    }

    #[test]
    fn overflow_reports_last_finite_norm() {
        let p = prods(S_SS);
        // M = [2e200]: squaring overflows at once; last finite norm is 2e200.
        assert_eq!(
            check_consistency(&p, &[1e200, 0.0], "s"),
            Consistency::NotProven { norm: 2e200 }
        );
    }

    #[test]
    #[should_panic(expected = "one probability per production")]
    fn wrong_prob_length_panics() {
        check_consistency(&prods(S_SS), &[1.0], "s");
    }

    #[test]
    #[should_panic(expected = "has no production")]
    fn missing_start_panics() {
        check_consistency(&prods(S_SS), &[0.4, 0.6], "t");
    }

    #[test]
    fn rho_bound_single_rule() {
        let p = prods(S_SS);
        // M = [1.2]; tr/n = 1.2 at power 1.
        let up = rho_lower_bound(&p, &[0.6, 0.4], "s");
        assert_eq!(up.power, 1);
        assert!((up.bound - 1.2).abs() < 1e-12, "bound = {}", up.bound);
        // M = [0.8].
        let down = rho_lower_bound(&p, &[0.4, 0.6], "s");
        assert!((down.bound - 0.8).abs() < 1e-12, "bound = {}", down.bound);
    }

    #[test]
    fn rho_bound_needs_the_division_by_n() {
        // M = [[0, 2], [0.3, 0]]: tr M = 0, tr M^2 = 1.2, n = 2, so the
        // bound is sqrt(0.6) = rho. Without /n it would be sqrt(1.2) > rho.
        let p = prods("a = b b ; b = a | \"x\" ;");
        let got = rho_lower_bound(&p, &[1.0, 0.3, 0.7], "a");
        assert_eq!(got.power, 2);
        let want = 0.6f64.sqrt();
        assert!((got.bound - want).abs() < 1e-12, "bound = {}", got.bound);
    }

    #[test]
    fn rho_bound_critical_runs_to_the_limit() {
        // M = [1.0]: every power gives exactly 1; all 2^MAX_SQUARINGS
        // squarings stay finite, the loop ends by the limit.
        let got = rho_lower_bound(&prods(S_SS), &[0.5, 0.5], "s");
        assert_eq!(got.power, 1);
        assert!((got.bound - 1.0).abs() < 1e-12, "bound = {}", got.bound);
    }

    #[test]
    fn rho_bound_stops_at_overflow_with_a_finite_bound() {
        // M = [2e200]: M^2 overflows, the bound stays that of power 1.
        let got = rho_lower_bound(&prods(S_SS), &[1e200, 0.0], "s");
        assert_eq!(got.power, 1);
        assert!(got.bound.is_finite() && (got.bound / 2e200 - 1.0).abs() < 1e-12);
        // M = [1.2]: M^4096 overflows; earlier powers still give ~1.2.
        let got = rho_lower_bound(&prods(S_SS), &[0.6, 0.4], "s");
        assert!(got.bound.is_finite() && (got.bound - 1.2).abs() < 1e-9);
    }

    #[test]
    fn rho_bound_ignores_unreachable_rules() {
        let p = prods("s = s s | \"a\" ; z = z z ;");
        let got = rho_lower_bound(&p, &[0.4, 0.6, 1.0], "s");
        assert!((got.bound - 0.8).abs() < 1e-12, "bound = {}", got.bound);
    }

    #[test]
    #[should_panic(expected = "one probability per production")]
    fn rho_bound_wrong_prob_length_panics() {
        rho_lower_bound(&prods(S_SS), &[1.0], "s");
    }

    #[test]
    fn unproductive_finds_the_loop_only() {
        assert!(unproductive_nonterminals(&prods(S_SS), "s").is_empty());
        let p = prods("s = \"a\" | z ; z = z \"b\" ;");
        assert_eq!(unproductive_nonterminals(&p, "s"), ["z"]);
    }

    #[test]
    fn unproductive_is_a_fixpoint_not_a_single_pass() {
        // c is productive only through b, b only through a: needs 3 rounds,
        // and rule order is against the flow.
        let p = prods("s = c ; c = b ; b = a ; a = \"x\" ;");
        assert!(unproductive_nonterminals(&p, "s").is_empty());
        // Mutual recursion with no way out: both unproductive, sorted.
        let p = prods("s = \"a\" | q ; q = r ; r = q ;");
        assert_eq!(unproductive_nonterminals(&p, "s"), ["q", "r"]);
    }

    #[test]
    fn unproductive_ignores_unreachable_and_counts_epsilon() {
        // w is unproductive but not reachable from s.
        let p = prods("s = \"a\" ; w = w ;");
        assert!(unproductive_nonterminals(&p, "s").is_empty());
        // e derives only the empty string, which is a terminal string.
        let p = prods("s = e ; e = { \"a\" } ;");
        assert!(unproductive_nonterminals(&p, "s").is_empty());
    }

    #[test]
    fn unproductive_start_without_way_out() {
        let p = prods("s = s \"a\" ;");
        assert_eq!(unproductive_nonterminals(&p, "s"), ["s"]);
    }

    #[test]
    fn uniform_pcfg_on_math_grammar() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/grammar/math.ebnf");
        let src = std::fs::read_to_string(path).expect("read math.ebnf");
        let grammar = parse(&src).expect("math.ebnf parses");
        let start = grammar.start().expect("start symbol").to_string();
        let p = productions(&grammar);
        let mut count: HashMap<&str, usize> = HashMap::new();
        for prod in &p {
            *count.entry(prod.lhs.as_str()).or_default() += 1;
        }
        let prob: Vec<f64> = p
            .iter()
            .map(|x| 1.0 / count[x.lhs.as_str()] as f64)
            .collect();
        let result = check_consistency(&p, &prob, &start);
        eprintln!(
            "math.ebnf: {} productions, {} lhs, start {start}: {result:?}",
            p.len(),
            count.len()
        );
        let bound = rho_lower_bound(&p, &prob, &start);
        let unproductive = unproductive_nonterminals(&p, &start);
        eprintln!("math.ebnf: rho lower bound {bound:?}, unproductive {unproductive:?}");
    }
}

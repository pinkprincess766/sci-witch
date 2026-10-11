//! Consistency check for a PCFG: do the probabilities of all finite trees of
//! the start symbol sum to 1?
//!
//! Booth & Thompson (1973, IEEE Trans. Computers C-22): with the mean-count
//! matrix `M[A][B] = sum over productions p of A of prob[p] * (occurrences of
//! B in rhs(p))`, the grammar is consistent if the spectral radius rho(M) < 1.
//! The check is a one-sided certificate: rho(M) <= ||M^k||^(1/k) for any k
//! (Gelfand), so ||M^k||_inf < 1 for some k proves rho(M) < 1. A critical
//! grammar (rho = 1) never gives a norm below 1, so it stays `NotProven`.

use std::collections::HashMap;

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
    assert_eq!(
        prob.len(),
        productions.len(),
        "one probability per production"
    );
    assert!(
        productions.iter().any(|p| p.lhs == start),
        "start symbol {start:?} has no production"
    );

    // Reachable nonterminals, in discovery order.
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut order: Vec<&str> = vec![start];
    index.insert(start, 0);
    let mut next = 0;
    while next < order.len() {
        let name = order[next];
        next += 1;
        for prod in productions.iter().filter(|p| p.lhs == name) {
            for sym in &prod.rhs {
                if let Symbol::Nonterminal(b) = sym {
                    if !index.contains_key(b.as_str()) {
                        index.insert(b.as_str(), order.len());
                        order.push(b.as_str());
                    }
                }
            }
        }
    }

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
    }
}

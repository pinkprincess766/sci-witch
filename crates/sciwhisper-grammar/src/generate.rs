//! Enumerate every short sentence of a grammar.
//!
//! [`sentences_up_to`] returns all terminal strings of length 1..=`max_len`
//! that a start symbol derives. It is the input for a bounded search for
//! ambiguous sentences: [`crate::count_parses`] counts the trees of a given
//! string, this module produces the strings.
//!
//! Method: `L(A, n)` is the set of strings of length `n` derivable from
//! `A`. Lengths are filled in order 0, 1, 2, …. Within one length the
//! nonterminals are taken dependencies first; a group that depends on
//! itself at the same length (an ε or unit cycle such as `s = t ; t = s`)
//! is repeated until no set grows. See `evaluation_groups`.

use std::collections::{BTreeSet, HashMap};

use crate::earley::{productions, Symbol};
use crate::ebnf::Grammar;

/// Longest string the generator will build. math.ebnf has 9, 229, 6182 and
/// 167368 strings of length 1 to 4 (173788 in all, under [`MAX_SENTENCES`]);
/// length 5 has millions and trips the cap.
pub const MAX_LEN: usize = 4;

/// Cap on distinct strings held for one (nonterminal, length) pair and in
/// the result. A partial right-hand side (a prefix of a production) is
/// capped the same way and reported under the production's left-hand side.
pub const MAX_SENTENCES: usize = 200_000;

#[derive(Debug, PartialEq, Eq)]
pub enum GenerateError {
    /// `max_len` was above [`MAX_LEN`].
    TooLong { max_len: usize },
    /// A set grew past [`MAX_SENTENCES`] while building length `len`.
    TooMany { nonterminal: String, len: usize },
    /// `start` is not the left-hand side of any production.
    NoStart,
}

// Terminal ids follow the sorted order of their texts, so comparing id
// strings is the same as comparing the token strings.
type Seq = Vec<u32>;
type Set = BTreeSet<Seq>;

enum Sym {
    T(u32),
    N(usize),
}

struct Prod {
    lhs: usize,
    rhs: Vec<Sym>,
}

/// Every distinct terminal string of length 1..=`max_len` derivable from
/// `start`, sorted by length, then lexicographically. Productions
/// containing a special-sequence terminal (a `Symbol::Terminal` whose text
/// starts with `?`) are skipped: such a terminal never matches a token.
pub fn sentences_up_to(
    grammar: &Grammar,
    start: &str,
    max_len: usize,
) -> Result<Vec<Vec<String>>, GenerateError> {
    if max_len > MAX_LEN {
        return Err(GenerateError::TooLong { max_len });
    }
    let all = productions(grammar);
    if !all.iter().any(|p| p.lhs == start) {
        return Err(GenerateError::NoStart);
    }
    let usable: Vec<_> = all
        .iter()
        .filter(|p| {
            !p.rhs
                .iter()
                .any(|s| matches!(s, Symbol::Terminal(t) if t.starts_with('?')))
        })
        .collect();

    let mut names: Vec<String> = Vec::new();
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut intern = |name: &str| -> usize {
        *ids.entry(name.to_string()).or_insert_with(|| {
            names.push(name.to_string());
            names.len() - 1
        })
    };
    let start_id = intern(start);
    for p in &usable {
        intern(&p.lhs);
        for s in &p.rhs {
            if let Symbol::Nonterminal(n) = s {
                intern(n);
            }
        }
    }
    let mut terminals: Vec<&str> = usable
        .iter()
        .flat_map(|p| p.rhs.iter())
        .filter_map(|s| match s {
            Symbol::Terminal(t) => Some(t.as_str()),
            Symbol::Nonterminal(_) => None,
        })
        .collect();
    terminals.sort_unstable();
    terminals.dedup();
    let prods: Vec<Prod> = usable
        .iter()
        .map(|p| Prod {
            lhs: ids[&p.lhs],
            rhs: p
                .rhs
                .iter()
                .map(|s| match s {
                    Symbol::Terminal(t) => {
                        Sym::T(terminals.binary_search(&t.as_str()).unwrap() as u32)
                    }
                    Symbol::Nonterminal(n) => Sym::N(ids[n]),
                })
                .collect(),
        })
        .collect();

    // Only productions reachable from `start` are evaluated, so an
    // unrelated exploding rule does not block the run.
    let mut reachable = vec![false; names.len()];
    let mut stack = vec![start_id];
    reachable[start_id] = true;
    while let Some(a) = stack.pop() {
        for p in prods.iter().filter(|p| p.lhs == a) {
            for s in &p.rhs {
                if let Sym::N(b) = s {
                    if !reachable[*b] {
                        reachable[*b] = true;
                        stack.push(*b);
                    }
                }
            }
        }
    }
    let active: Vec<&Prod> = prods.iter().filter(|p| reachable[p.lhs]).collect();
    let groups = evaluation_groups(&active, names.len());

    // sets[a][n] = L(a, n). prefix[p][i][n] = strings of length n that the
    // first i symbols of production p derive; prefix[p][k][n] is its yield.
    let mut sets: Vec<Vec<Set>> = vec![Vec::new(); names.len()];
    let mut prefix: Vec<Vec<Vec<Set>>> = active
        .iter()
        .map(|p| vec![Vec::new(); p.rhs.len() + 1])
        .collect();
    let mut total = 0;

    for n in 0..=max_len {
        for layers in &mut sets {
            layers.push(Set::new());
        }
        for (p, layers) in active.iter().zip(&mut prefix) {
            for layer in layers.iter_mut() {
                layer.push(Set::new());
            }
            if n == 0 {
                layers[0][0].insert(Vec::new());
            }
            debug_assert_eq!(layers.len(), p.rhs.len() + 1);
        }
        for (members, cyclic) in &groups {
            loop {
                let mut changed = false;
                for &pi in members {
                    let p = active[pi];
                    let layers = &mut prefix[pi];
                    let too_many = || GenerateError::TooMany {
                        nonterminal: names[p.lhs].clone(),
                        len: n,
                    };
                    for (i, sym) in p.rhs.iter().enumerate() {
                        let Some(acc) = extend(&layers[i], sym, &sets, n) else {
                            return Err(too_many());
                        };
                        layers[i + 1][n] = acc;
                    }
                    let yielded = &layers[p.rhs.len()][n];
                    for s in yielded {
                        if sets[p.lhs][n].insert(s.clone()) {
                            changed = true;
                        }
                    }
                    if sets[p.lhs][n].len() > MAX_SENTENCES {
                        return Err(too_many());
                    }
                }
                if !cyclic || !changed {
                    break;
                }
            }
        }
        // Prefix sets at length n were built before every set at length n
        // was final. Later lengths read them, so rebuild them now. The last
        // layer is never read as a prefix.
        for (p, layers) in active.iter().zip(&mut prefix) {
            for (i, sym) in p.rhs.iter().enumerate().take(p.rhs.len().saturating_sub(1)) {
                let Some(acc) = extend(&layers[i], sym, &sets, n) else {
                    return Err(GenerateError::TooMany {
                        nonterminal: names[p.lhs].clone(),
                        len: n,
                    });
                };
                layers[i + 1][n] = acc;
            }
        }
        if n > 0 {
            total += sets[start_id][n].len();
            if total > MAX_SENTENCES {
                return Err(GenerateError::TooMany {
                    nonterminal: start.to_string(),
                    len: n,
                });
            }
        }
    }

    Ok(sets[start_id][1..]
        .iter()
        .flatten()
        .map(|s| {
            s.iter()
                .map(|&t| terminals[t as usize].to_string())
                .collect()
        })
        .collect())
}

/// Strings of length `n` that `prefix` (indexed by length) followed by
/// `sym` derive. `None` if the result passes [`MAX_SENTENCES`].
fn extend(prefix: &[Set], sym: &Sym, sets: &[Vec<Set>], n: usize) -> Option<Set> {
    let mut acc = Set::new();
    for j in 0..=n {
        let left = &prefix[n - j];
        match sym {
            Sym::T(t) => {
                if j == 1 {
                    for s in left {
                        let mut next = s.clone();
                        next.push(*t);
                        acc.insert(next);
                    }
                }
            }
            Sym::N(b) => {
                for l in left {
                    for r in &sets[*b][j] {
                        let mut next = l.clone();
                        next.extend_from_slice(r);
                        acc.insert(next);
                    }
                    if acc.len() > MAX_SENTENCES {
                        return None;
                    }
                }
            }
        }
        if acc.len() > MAX_SENTENCES {
            return None;
        }
    }
    Some(acc)
}

/// Groups of productions in the order they must be evaluated within one
/// length, each with a flag saying whether it needs repeated passes.
///
/// A string of length `n` for `a` can use a string of the same length `n`
/// of `b` only if every other symbol of that production derives ε. That is
/// the only same-length dependency (`a -> b`), so the nonterminals are
/// grouped into strongly connected components of this graph and taken
/// dependencies first. A component with a cycle (including a self-loop)
/// is repeated until nothing grows; any other component needs one pass.
fn evaluation_groups(active: &[&Prod], n_nonterminals: usize) -> Vec<(Vec<usize>, bool)> {
    let mut nullable = vec![false; n_nonterminals];
    loop {
        let mut changed = false;
        for p in active {
            let all_null = p.rhs.iter().all(|s| matches!(s, Sym::N(c) if nullable[*c]));
            if all_null && !nullable[p.lhs] {
                nullable[p.lhs] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n_nonterminals];
    for p in active {
        for (i, sym) in p.rhs.iter().enumerate() {
            let Sym::N(b) = sym else { continue };
            let others_null = p
                .rhs
                .iter()
                .enumerate()
                .all(|(q, s)| q == i || matches!(s, Sym::N(c) if nullable[*c]));
            if others_null {
                edges[p.lhs].push(*b);
            }
        }
    }

    struct Tarjan<'a> {
        edges: &'a [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        components: Vec<Vec<usize>>,
    }
    impl Tarjan<'_> {
        fn visit(&mut self, v: usize) {
            self.index[v] = Some(self.next);
            self.low[v] = self.next;
            self.next += 1;
            self.stack.push(v);
            self.on_stack[v] = true;
            for k in 0..self.edges[v].len() {
                let w = self.edges[v][k];
                match self.index[w] {
                    None => {
                        self.visit(w);
                        self.low[v] = self.low[v].min(self.low[w]);
                    }
                    Some(iw) if self.on_stack[w] => self.low[v] = self.low[v].min(iw),
                    Some(_) => {}
                }
            }
            if Some(self.low[v]) == self.index[v] {
                let mut component = Vec::new();
                loop {
                    let w = self.stack.pop().unwrap();
                    self.on_stack[w] = false;
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                self.components.push(component);
            }
        }
    }
    let mut tarjan = Tarjan {
        edges: &edges,
        index: vec![None; n_nonterminals],
        low: vec![0; n_nonterminals],
        on_stack: vec![false; n_nonterminals],
        stack: Vec::new(),
        next: 0,
        components: Vec::new(),
    };
    for v in 0..n_nonterminals {
        if tarjan.index[v].is_none() {
            tarjan.visit(v);
        }
    }

    // Tarjan finishes a component after every component it depends on.
    tarjan
        .components
        .iter()
        .map(|component| {
            let members: Vec<usize> = (0..active.len())
                .filter(|&pi| component.contains(&active[pi].lhs))
                .collect();
            let cyclic = component.len() > 1 || edges[component[0]].contains(&component[0]);
            (members, cyclic)
        })
        .filter(|(members, _)| !members.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earley::{recognise, Recognition};
    use crate::ebnf::parse;

    fn toks(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    fn sentences(src: &str, max_len: usize) -> Vec<Vec<String>> {
        let grammar = parse(src).expect("toy grammar parses");
        let start = grammar.start().expect("start symbol").to_string();
        sentences_up_to(&grammar, &start, max_len).expect("generates")
    }

    fn accepted(grammar: &Grammar, start: &str, sentence: &[String]) -> bool {
        let tokens: Vec<&str> = sentence.iter().map(String::as_str).collect();
        recognise(grammar, start, &tokens) == Recognition::Accepted
    }

    const HAND: &str = r#"s = "a" [ "b" ] | s "c" ;"#;

    fn math() -> Grammar {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/grammar/math.ebnf");
        parse(&std::fs::read_to_string(path).expect("read math.ebnf")).expect("math.ebnf parses")
    }

    #[test]
    fn hand_set() {
        // s -> a | a b | s c.
        // Length 1: a.
        // Length 2: a b, and s c with s = a: a c.
        // Length 3: s c with |s| = 2, s in {a b, a c}: a b c, a c c.
        // Length 4 would add s c with |s| = 3, but the test stops at 3.
        // Sorted by length, then by token text.
        let want: Vec<Vec<String>> = ["a", "a b", "a c", "a b c", "a c c"]
            .iter()
            .map(|s| toks(s))
            .collect();
        assert_eq!(sentences(HAND, 3), want);
    }

    #[test]
    fn nothing_for_length_zero() {
        assert!(sentences(HAND, 0).is_empty());
    }

    #[test]
    fn hand_set_is_recognised() {
        let grammar = parse(HAND).unwrap();
        for s in sentences(HAND, MAX_LEN) {
            assert!(accepted(&grammar, "s", &s), "{s:?}");
        }
    }

    #[test]
    fn math_sentences_are_recognised() {
        let grammar = math();
        let start = grammar.start().unwrap().to_string();
        let all = sentences_up_to(&grammar, &start, 3).expect("generates");
        assert!(!all.is_empty());
        for s in &all {
            assert!(accepted(&grammar, &start, s), "{s:?}");
        }
    }

    #[test]
    fn output_is_sorted_by_length_then_text() {
        let grammar = math();
        let start = grammar.start().unwrap().to_string();
        let all = sentences_up_to(&grammar, &start, 3).unwrap();
        let key = |s: &Vec<String>| (s.len(), s.clone());
        assert!(all.windows(2).all(|w| key(&w[0]) < key(&w[1])));
    }

    /// All strings of length 1..=max_len over `alphabet` that `recognise`
    /// accepts. Naive and obviously complete for those lengths.
    fn brute_force(src: &str, alphabet: &[&str], max_len: usize) -> Vec<Vec<String>> {
        let grammar = parse(src).unwrap();
        let start = grammar.start().unwrap().to_string();
        let mut found = Vec::new();
        let mut layer: Vec<Vec<String>> = vec![Vec::new()];
        for _ in 0..max_len {
            layer = layer
                .iter()
                .flat_map(|s| {
                    alphabet.iter().map(move |a| {
                        let mut next = s.clone();
                        next.push(a.to_string());
                        next
                    })
                })
                .collect();
            found.extend(
                layer
                    .iter()
                    .filter(|s| accepted(&grammar, &start, s))
                    .cloned(),
            );
        }
        found.sort_by_key(|s| (s.len(), s.clone()));
        found
    }

    #[test]
    fn matches_brute_force() {
        let cases: [(&str, &[&str]); 3] = [
            // a^n b^n
            (r#"s = [ "a" s "b" ] ;"#, &["a", "b"]),
            // ambiguous binary grammar
            (r#"s = s s | "a" | "b" ;"#, &["a", "b"]),
            // unit cycle, nullable repeat, left recursion
            (
                r#"s = t "c" | s "a" ; t = { "a" | "b" } | s ;"#,
                &["a", "b", "c"],
            ),
        ];
        for (src, alphabet) in cases {
            let want = brute_force(src, alphabet, MAX_LEN);
            assert!(!want.is_empty(), "{src}");
            assert_eq!(sentences(src, MAX_LEN), want, "{src}");
        }
    }

    #[test]
    fn epsilon_and_unit_cycles_terminate() {
        // s = t | x ; t = s | [ y ].  t derives ε, y, and everything s does;
        // s derives x and everything t does. So L(s) = {ε, x, y}.
        let want: Vec<Vec<String>> = ["x", "y"].iter().map(|s| toks(s)).collect();
        assert_eq!(sentences(r#"s = t | "x" ; t = s | [ "y" ] ;"#, 4), want);
    }

    #[test]
    fn special_sequence_productions_are_skipped() {
        let src = r#"s = "a" | "b" ? word ? ;"#;
        assert_eq!(sentences(src, 3), vec![toks("a")]);
    }

    #[test]
    fn unknown_start_is_an_error() {
        let grammar = parse(HAND).unwrap();
        assert_eq!(
            sentences_up_to(&grammar, "nope", 2),
            Err(GenerateError::NoStart)
        );
    }

    #[test]
    fn length_limit_is_an_explicit_constant() {
        let grammar = parse(HAND).unwrap();
        assert_eq!(
            sentences_up_to(&grammar, "s", MAX_LEN + 1),
            Err(GenerateError::TooLong {
                max_len: MAX_LEN + 1
            })
        );
        assert!(sentences_up_to(&grammar, "s", MAX_LEN).is_ok());
    }

    /// `{ x1 | x2 | ... }` over `k` distinct letters.
    fn letters(k: usize) -> String {
        let alts: Vec<String> = (0..k).map(|i| format!("\"l{i:03}\"")).collect();
        format!("s = {{ {} }} ;", alts.join(" | "))
    }

    #[test]
    fn explosion_is_cut_by_the_cap() {
        // k letters give k^n strings of length n. Pick the smallest k with
        // k^MAX_LEN above the cap, so the cap trips at or before MAX_LEN.
        let k = (2..)
            .find(|k: &usize| k.pow(MAX_LEN as u32) > MAX_SENTENCES)
            .unwrap();
        let grammar = parse(&letters(k)).unwrap();
        match sentences_up_to(&grammar, "s", MAX_LEN) {
            Err(GenerateError::TooMany { nonterminal, len }) => {
                assert!(len <= MAX_LEN);
                assert!(!nonterminal.is_empty());
            }
            other => panic!("expected TooMany, got {other:?}"),
        }
    }

    /// `s = (x1|..|xa) (y1|..|yb) (z1|..|zc)` plus `extra`: exactly a*b*c
    /// strings of length 3 from the first alternative.
    fn product(a: usize, b: usize, c: usize, extra: &str) -> String {
        let alts = |name: &str, k: usize| {
            let items: Vec<String> = (0..k).map(|i| format!("\"{name}{i}\"")).collect();
            format!("( {} )", items.join(" | "))
        };
        format!(
            "s = {} {} {}{extra} ;",
            alts("x", a),
            alts("y", b),
            alts("z", c)
        )
    }

    #[test]
    fn cap_boundary_one_length() {
        // 50 * 50 * 80 = 200_000 = MAX_SENTENCES: allowed.
        assert_eq!(MAX_SENTENCES, 50 * 50 * 80);
        let grammar = parse(&product(50, 50, 80, "")).unwrap();
        let all = sentences_up_to(&grammar, "s", 3).expect("at the cap");
        assert_eq!(all.len(), MAX_SENTENCES);
        // One more string of the same length: refused, reported at len 3.
        let src = product(50, 50, 80, r#" | "q" "q" "q""#);
        let grammar = parse(&src).unwrap();
        assert_eq!(
            sentences_up_to(&grammar, "s", 3),
            Err(GenerateError::TooMany {
                nonterminal: "s".into(),
                len: 3
            })
        );
    }

    #[test]
    fn cap_boundary_in_the_result() {
        // 200_000 strings of length 3 plus one of length 1: each length is
        // under the cap, the total is not.
        let src = product(50, 50, 80, r#" | "q""#);
        let grammar = parse(&src).unwrap();
        assert_eq!(
            sentences_up_to(&grammar, "s", 3),
            Err(GenerateError::TooMany {
                nonterminal: "s".into(),
                len: 3
            })
        );
        // Stopping before the long length is fine.
        assert_eq!(sentences_up_to(&grammar, "s", 2).unwrap(), vec![toks("q")]);
    }

    #[test]
    fn math_ebnf_counts_per_length() {
        let grammar = math();
        let start = grammar.start().unwrap().to_string();
        let all = sentences_up_to(&grammar, &start, MAX_LEN).expect("math.ebnf up to MAX_LEN");
        for len in 1..=MAX_LEN {
            let at_len = all.iter().filter(|s| s.len() == len).count();
            eprintln!("math.ebnf length {len}: {at_len} strings");
        }
        eprintln!("math.ebnf up to {MAX_LEN}: {} strings", all.len());
        // One more length is too many: the cap, not a bug, is what stops it.
        assert!(matches!(
            sentences_up_to(&grammar, &start, MAX_LEN + 1),
            Err(GenerateError::TooLong { .. })
        ));
    }
}

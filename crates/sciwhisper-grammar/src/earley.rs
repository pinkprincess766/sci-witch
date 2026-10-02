//! Earley recogniser over a sequence of terminal names.
//!
//! The EBNF is lowered to BNF: `[A]` becomes a fresh nonterminal with an
//! empty alternative, `{A}` a right-recursive nonterminal with an empty
//! alternative, groups become fresh nonterminals.
//!
//! Empty productions are the usual Earley trap. This implementation uses the
//! Aycock–Horspool predictor: nullable nonterminals are precomputed, and a
//! dot sitting before a nullable nonterminal is advanced in the same set.

use std::collections::{HashMap, HashSet};

use crate::ebnf::{is_terminal, Expr, Grammar};

/// Longest token sequence the recogniser will look at.
///
/// A dictated formula is a handful of words; 256 tokens is already far
/// beyond any real utterance and any corpus record. Past this the
/// recogniser refuses instead of walking an ever-longer chart.
pub const MAX_TOKENS: usize = 256;

/// Cap on Earley items across the whole chart.
///
/// Ambiguous and highly nullable grammars fill the chart with items. One
/// million is enough for the mathematics grammar on corpus inputs and small
/// enough that a runaway grammar fails instead of hanging.
pub const MAX_ITEMS: usize = 1_000_000;

/// Outcome of [`recognise`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recognition {
    Accepted,
    Rejected {
        /// Index of the first token the chart could not consume, or
        /// `tokens.len()` if the tokens were consumed but the start symbol
        /// did not complete.
        furthest: usize,
    },
    TooLong,
    TooManyItems,
}

#[derive(Clone, Debug)]
enum Sym {
    T(String),
    N(usize),
}

#[derive(Clone, Debug)]
struct Prod {
    lhs: usize,
    rhs: Vec<Sym>,
}

struct Bnf {
    names: Vec<String>,
    id: HashMap<String, usize>,
    prods: Vec<Prod>,
    prods_of: Vec<Vec<usize>>,
}

impl Bnf {
    fn intern(&mut self, name: &str) -> usize {
        if let Some(&id) = self.id.get(name) {
            return id;
        }
        let id = self.names.len();
        self.names.push(name.to_string());
        self.id.insert(name.to_string(), id);
        self.prods_of.push(Vec::new());
        id
    }

    fn add_prod(&mut self, lhs: usize, rhs: Vec<Sym>) {
        let idx = self.prods.len();
        self.prods_of[lhs].push(idx);
        self.prods.push(Prod { lhs, rhs });
    }
}

struct Lowering<'a> {
    rule: &'a str,
    optional: usize,
    repeat: usize,
    group: usize,
    bnf: &'a mut Bnf,
}

impl Lowering<'_> {
    fn alternatives(expr: &Expr) -> Vec<&Expr> {
        match expr {
            Expr::Alt(alts) => alts.iter().collect(),
            other => vec![other],
        }
    }

    fn sequence(&mut self, expr: &Expr) -> Vec<Sym> {
        match expr {
            Expr::Seq(items) => items.iter().map(|i| self.item(i)).collect(),
            other => vec![self.item(other)],
        }
    }

    fn item(&mut self, expr: &Expr) -> Sym {
        match expr {
            Expr::Name(n) if is_terminal(n) => Sym::T(n.clone()),
            Expr::Name(n) => Sym::N(self.bnf.intern(n)),
            // A syntactic literal is its own terminal. It matches a token
            // only when the token class name equals the literal text.
            Expr::Lit(s) => Sym::T(s.clone()),
            // Special sequences are not checked; they become a terminal
            // that never equals a token-class name, so a syntactic special
            // would reject. math.ebnf keeps them in lexical rules only.
            Expr::Special(s) => Sym::T(format!("?{s}?")),
            Expr::Seq(_) | Expr::Alt(_) => {
                self.group += 1;
                let name = format!("{}~G{}", self.rule, self.group);
                let id = self.bnf.intern(&name);
                let prods: Vec<Vec<Sym>> = Self::alternatives(expr)
                    .into_iter()
                    .map(|a| self.sequence(a))
                    .collect();
                for rhs in prods {
                    self.bnf.add_prod(id, rhs);
                }
                Sym::N(id)
            }
            Expr::Opt(inner) => {
                self.optional += 1;
                let name = format!("{}~O{}", self.rule, self.optional);
                let id = self.bnf.intern(&name);
                let mut prods: Vec<Vec<Sym>> = Self::alternatives(inner)
                    .into_iter()
                    .map(|a| self.sequence(a))
                    .collect();
                prods.push(Vec::new());
                for rhs in prods {
                    self.bnf.add_prod(id, rhs);
                }
                Sym::N(id)
            }
            Expr::Rep(inner) => {
                self.repeat += 1;
                let name = format!("{}~R{}", self.rule, self.repeat);
                let id = self.bnf.intern(&name);
                let mut prods: Vec<Vec<Sym>> = Self::alternatives(inner)
                    .into_iter()
                    .map(|a| {
                        let mut seq = self.sequence(a);
                        seq.push(Sym::N(id));
                        seq
                    })
                    .collect();
                prods.push(Vec::new());
                for rhs in prods {
                    self.bnf.add_prod(id, rhs);
                }
                Sym::N(id)
            }
        }
    }
}

fn lower(grammar: &Grammar) -> Bnf {
    let mut bnf = Bnf {
        names: Vec::new(),
        id: HashMap::new(),
        prods: Vec::new(),
        prods_of: Vec::new(),
    };
    for rule in grammar.syntactic() {
        let lhs = bnf.intern(&rule.name);
        let mut lowering = Lowering {
            rule: &rule.name,
            optional: 0,
            repeat: 0,
            group: 0,
            bnf: &mut bnf,
        };
        let prods: Vec<Vec<Sym>> = Lowering::alternatives(&rule.body)
            .into_iter()
            .map(|a| lowering.sequence(a))
            .collect();
        for rhs in prods {
            bnf.add_prod(lhs, rhs);
        }
    }
    bnf
}

fn nullable_set(bnf: &Bnf) -> Vec<bool> {
    let n = bnf.names.len();
    let mut nullable = vec![false; n];
    loop {
        let mut changed = false;
        for prod in &bnf.prods {
            if nullable[prod.lhs] {
                continue;
            }
            let all_null = prod.rhs.iter().all(|s| match s {
                Sym::T(_) => false,
                Sym::N(id) => nullable[*id],
            });
            if all_null {
                nullable[prod.lhs] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    nullable
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Item {
    prod: u32,
    dot: u16,
    origin: u32,
}

struct Chart {
    sets: Vec<Vec<Item>>,
    seen: Vec<HashSet<Item>>,
    total: usize,
    max_items: usize,
}

impl Chart {
    fn new(n_sets: usize, max_items: usize) -> Self {
        Self {
            sets: vec![Vec::new(); n_sets],
            seen: vec![HashSet::new(); n_sets],
            total: 0,
            max_items,
        }
    }

    fn add(&mut self, set: usize, item: Item) -> Result<(), Recognition> {
        if !self.seen[set].insert(item) {
            return Ok(());
        }
        self.total += 1;
        if self.total > self.max_items {
            return Err(Recognition::TooManyItems);
        }
        self.sets[set].push(item);
        Ok(())
    }
}

fn recognise_inner(
    grammar: &Grammar,
    start: &str,
    tokens: &[&str],
    max_tokens: usize,
    max_items: usize,
    aycock: bool,
    strip_epsilon: bool,
) -> Recognition {
    if tokens.len() > max_tokens {
        return Recognition::TooLong;
    }
    let mut bnf = lower(grammar);
    let Some(&start_id) = bnf.id.get(start) else {
        return Recognition::Rejected { furthest: 0 };
    };
    if strip_epsilon {
        let kept: Vec<Prod> = bnf
            .prods
            .iter()
            .filter(|p| !p.rhs.is_empty())
            .cloned()
            .collect();
        bnf.prods_of = vec![Vec::new(); bnf.names.len()];
        bnf.prods.clear();
        for prod in kept {
            bnf.add_prod(prod.lhs, prod.rhs);
        }
    }
    let dummy = bnf.intern("~'");
    bnf.add_prod(dummy, vec![Sym::N(start_id)]);
    let nullable = nullable_set(&bnf);
    let dummy_prod = (bnf.prods.len() - 1) as u32;

    let n = tokens.len();
    let mut chart = Chart::new(n + 1, max_items);
    if let Err(r) = chart.add(
        0,
        Item {
            prod: dummy_prod,
            dot: 0,
            origin: 0,
        },
    ) {
        return r;
    }

    // `i` is the Earley position: it indexes the chart sets and the input
    // tokens together, which an iterator over either one would hide.
    #[allow(clippy::needless_range_loop)]
    for i in 0..=n {
        let mut p = 0;
        while p < chart.sets[i].len() {
            let item = chart.sets[i][p];
            p += 1;
            let prod = &bnf.prods[item.prod as usize];
            let lhs = prod.lhs;
            if item.dot as usize >= prod.rhs.len() {
                let origin = item.origin as usize;
                let mut pi = 0;
                while pi < chart.sets[origin].len() {
                    let parent = chart.sets[origin][pi];
                    pi += 1;
                    let advance = {
                        let parent_prod = &bnf.prods[parent.prod as usize];
                        match parent_prod.rhs.get(parent.dot as usize) {
                            Some(Sym::N(nt)) if *nt == lhs => Some(Item {
                                prod: parent.prod,
                                dot: parent.dot + 1,
                                origin: parent.origin,
                            }),
                            _ => None,
                        }
                    };
                    if let Some(next) = advance {
                        if let Err(r) = chart.add(i, next) {
                            return r;
                        }
                    }
                }
                continue;
            }
            match &prod.rhs[item.dot as usize] {
                Sym::N(nt) => {
                    let nt = *nt;
                    for &prod_id in &bnf.prods_of[nt] {
                        if let Err(r) = chart.add(
                            i,
                            Item {
                                prod: prod_id as u32,
                                dot: 0,
                                origin: i as u32,
                            },
                        ) {
                            return r;
                        }
                    }
                    if aycock && nullable[nt] {
                        if let Err(r) = chart.add(
                            i,
                            Item {
                                prod: item.prod,
                                dot: item.dot + 1,
                                origin: item.origin,
                            },
                        ) {
                            return r;
                        }
                    }
                }
                Sym::T(t) => {
                    if i < n && tokens[i] == t {
                        if let Err(r) = chart.add(
                            i + 1,
                            Item {
                                prod: item.prod,
                                dot: item.dot + 1,
                                origin: item.origin,
                            },
                        ) {
                            return r;
                        }
                    }
                }
            }
        }
    }

    let accepted = chart.sets[n].iter().any(|item| {
        item.prod == dummy_prod && item.origin == 0 && {
            let prod = &bnf.prods[item.prod as usize];
            item.dot as usize >= prod.rhs.len()
        }
    });
    if accepted {
        Recognition::Accepted
    } else {
        let furthest = chart
            .sets
            .iter()
            .enumerate()
            .rev()
            .find(|(_, set)| !set.is_empty())
            .map(|(i, _)| i)
            .unwrap_or(0);
        Recognition::Rejected { furthest }
    }
}

/// Recognise `tokens` as a complete derivation of `start` in `grammar`.
pub fn recognise(grammar: &Grammar, start: &str, tokens: &[&str]) -> Recognition {
    recognise_inner(grammar, start, tokens, MAX_TOKENS, MAX_ITEMS, true, false)
}

#[cfg(test)]
fn recognise_limited(
    grammar: &Grammar,
    start: &str,
    tokens: &[&str],
    max_tokens: usize,
    max_items: usize,
) -> Recognition {
    recognise_inner(grammar, start, tokens, max_tokens, max_items, true, false)
}

/// Naive Earley: empty productions are dropped and nullable nonterminals
/// are not skipped. A chain of nullable nonterminals then loses the parse.
#[cfg(test)]
fn recognise_naive(grammar: &Grammar, start: &str, tokens: &[&str]) -> Recognition {
    recognise_inner(grammar, start, tokens, MAX_TOKENS, MAX_ITEMS, false, true)
}

#[cfg(test)]
mod tests {
    use crate::ebnf::parse;

    use super::{
        recognise, recognise_limited, recognise_naive, Recognition, MAX_ITEMS, MAX_TOKENS,
    };

    fn g(src: &str) -> crate::ebnf::Grammar {
        parse(src).unwrap_or_else(|e| panic!("{e} in {src}"))
    }

    fn ok(src: &str, start: &str, tokens: &[&str]) {
        assert_eq!(
            recognise(&g(src), start, tokens),
            Recognition::Accepted,
            "should accept {tokens:?}"
        );
    }

    fn no(src: &str, start: &str, tokens: &[&str]) {
        assert!(
            matches!(
                recognise(&g(src), start, tokens),
                Recognition::Rejected { .. }
            ),
            "should reject {tokens:?}"
        );
    }

    #[test]
    fn arithmetic_with_precedence_is_accepted() {
        let src = r#"
            e = t { PLUS t } ;
            t = f { TIMES f } ;
            f = NUM | LP e RP ;
        "#;
        ok(src, "e", &["NUM"]);
        ok(src, "e", &["NUM", "PLUS", "NUM"]);
        ok(src, "e", &["NUM", "PLUS", "NUM", "TIMES", "NUM"]);
        ok(
            src,
            "e",
            &["LP", "NUM", "PLUS", "NUM", "RP", "TIMES", "NUM"],
        );
        no(src, "e", &["PLUS"]);
        no(src, "e", &["NUM", "PLUS"]);
        no(src, "e", &["LP", "NUM"]);
    }

    #[test]
    fn balanced_parentheses_are_accepted() {
        let src = "s = LP s RP | A ;";
        ok(src, "s", &["A"]);
        ok(src, "s", &["LP", "A", "RP"]);
        ok(src, "s", &["LP", "LP", "A", "RP", "RP"]);
        no(src, "s", &["LP", "A"]);
        no(src, "s", &["LP", "RP"]);
        no(src, "s", &["A", "RP"]);
    }

    #[test]
    fn an_ambiguous_grammar_is_still_accepted() {
        let src = "e = e PLUS e | A ;";
        ok(src, "e", &["A"]);
        ok(src, "e", &["A", "PLUS", "A"]);
        ok(src, "e", &["A", "PLUS", "A", "PLUS", "A"]);
        no(src, "e", &["PLUS", "A"]);
    }

    #[test]
    fn empty_input_depends_on_whether_the_start_symbol_is_nullable() {
        let with_eps = "s = [ A ] ;";
        assert_eq!(recognise(&g(with_eps), "s", &[]), Recognition::Accepted);
        ok(with_eps, "s", &["A"]);

        let without = "s = a ; a = X ;";
        assert_eq!(
            recognise(&g(without), "s", &[]),
            Recognition::Rejected { furthest: 0 }
        );
        ok(without, "s", &["X"]);
    }

    #[test]
    fn aycock_horspool_is_required_for_a_nullable_chain() {
        // A, B, C are nullable only through a chain of named nonterminals
        // ending in `[ Z ]`. Stripping ε and skipping the nullable
        // precomputation loses the parse of `X`; the Aycock–Horspool
        // predictor keeps it.
        let src = r#"
            s = a x ;
            a = b ;
            b = c ;
            c = [ z ] ;
            x = X ;
            z = Z ;
        "#;
        assert_eq!(
            recognise(&g(src), "s", &["X"]),
            Recognition::Accepted,
            "with Aycock–Horspool the chain of nullable names is skipped"
        );
        assert!(
            matches!(
                recognise_naive(&g(src), "s", &["X"]),
                Recognition::Rejected { .. }
            ),
            "without nullable precomputation the chain loses the parse"
        );
        ok(src, "s", &["Z", "X"]);
    }

    #[test]
    fn max_tokens_accepts_at_the_limit_and_refuses_one_past() {
        let src = r#"s = { X } ; x = X ;"#;
        let at: Vec<&str> = vec!["X"; MAX_TOKENS];
        let over: Vec<&str> = vec!["X"; MAX_TOKENS + 1];
        assert_eq!(recognise(&g(src), "s", &at), Recognition::Accepted);
        assert_eq!(recognise(&g(src), "s", &over), Recognition::TooLong);
        assert_eq!(
            recognise_limited(&g(src), "s", &at, MAX_TOKENS - 1, MAX_ITEMS),
            Recognition::TooLong
        );
    }

    #[test]
    fn max_items_accepts_at_the_limit_and_refuses_one_past() {
        let src = r#"s = X ;"#;
        // Dummy start S' → s, predict s → • X, scan X, complete s and S'.
        // That is four unique items.
        const FOUR: usize = 4;
        assert_eq!(
            recognise_limited(&g(src), "s", &["X"], MAX_TOKENS, FOUR),
            Recognition::Accepted
        );
        assert_eq!(
            recognise_limited(&g(src), "s", &["X"], MAX_TOKENS, FOUR - 1),
            Recognition::TooManyItems
        );
        assert_eq!(MAX_ITEMS, 1_000_000);
        assert_eq!(MAX_TOKENS, 256);
    }

    #[test]
    fn furthest_is_how_many_tokens_were_consumed() {
        let src = "s = A B C ;";
        match recognise(&g(src), "s", &["A", "A"]) {
            Recognition::Rejected { furthest } => assert_eq!(furthest, 1),
            other => panic!("{other:?}"),
        }
        match recognise(&g(src), "s", &["A", "B"]) {
            Recognition::Rejected { furthest } => assert_eq!(furthest, 2),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_syntactic_literal_matches_only_its_own_text() {
        // math.ebnf has no syntactic literals; this pins the decision for
        // when one appears: the literal text is a terminal of its own, not
        // rewritten to a token class.
        let src = r#"s = "hello" ;"#;
        ok(src, "s", &["hello"]);
        no(src, "s", &["HELLO"]);
        no(src, "s", &["PLUS"]);
    }

    #[test]
    fn a_missing_start_symbol_is_rejected() {
        assert_eq!(
            recognise(&g("s = A ;"), "nope", &["A"]),
            Recognition::Rejected { furthest: 0 }
        );
    }
}

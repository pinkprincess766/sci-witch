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

/// Cap on the number of derivation trees reported for one sentence.
///
/// [`ParseCount::Exact`] holds a count up to and including this value.
/// Anything larger, and any nullable cycle (`A → A`, `{ ε }`), is
/// [`ParseCount::AtLeastMax`]: the counter returns instead of walking an
/// infinite family of trees.
pub const MAX_PARSES: u32 = 1000;

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

/// How many derivation trees [`count_parses`] found for an accepted sentence.
///
/// `Exact(MAX_PARSES)` is exact. A larger finite total and a nullable cycle
/// are both `AtLeastMax`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseCount {
    Exact(u32),
    AtLeastMax,
}

impl ParseCount {
    fn add(self, other: Self) -> Self {
        match (self, other) {
            (Self::AtLeastMax, _) | (_, Self::AtLeastMax) => Self::AtLeastMax,
            (Self::Exact(a), Self::Exact(b)) => {
                let sum = u64::from(a) + u64::from(b);
                if sum > u64::from(MAX_PARSES) {
                    Self::AtLeastMax
                } else {
                    Self::Exact(sum as u32)
                }
            }
        }
    }

    /// `Exact(0)` times `AtLeastMax` is `Exact(0)`: an empty factor kills
    /// the product, including a cycle discovered on a split that cannot
    /// actually derive the span.
    fn mul(self, other: Self) -> Self {
        match (self, other) {
            (Self::Exact(0), _) | (_, Self::Exact(0)) => Self::Exact(0),
            (Self::AtLeastMax, _) | (_, Self::AtLeastMax) => Self::AtLeastMax,
            (Self::Exact(a), Self::Exact(b)) => {
                let prod = u64::from(a) * u64::from(b);
                if prod > u64::from(MAX_PARSES) {
                    Self::AtLeastMax
                } else {
                    Self::Exact(prod as u32)
                }
            }
        }
    }
}

/// Outcome of [`count_parses`].
///
/// Built from the same chart as [`recognise`]. A sentence the recogniser
/// accepts is [`CountOutcome::Counted`]; the refusal arms are the recogniser's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountOutcome {
    Counted(ParseCount),
    Rejected {
        /// Same index [`Recognition::Rejected`] reports.
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

struct ChartBuilt {
    bnf: Bnf,
    nullable: Vec<bool>,
    start_id: usize,
    chart: Chart,
    n: usize,
    dummy_prod: u32,
}

fn build_chart(
    grammar: &Grammar,
    start: &str,
    tokens: &[&str],
    max_tokens: usize,
    max_items: usize,
    aycock: bool,
    strip_epsilon: bool,
) -> Result<ChartBuilt, Recognition> {
    if tokens.len() > max_tokens {
        return Err(Recognition::TooLong);
    }
    let mut bnf = lower(grammar);
    let Some(&start_id) = bnf.id.get(start) else {
        return Err(Recognition::Rejected { furthest: 0 });
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
    chart.add(
        0,
        Item {
            prod: dummy_prod,
            dot: 0,
            origin: 0,
        },
    )?;

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
                        chart.add(i, next)?;
                    }
                }
                continue;
            }
            match &prod.rhs[item.dot as usize] {
                Sym::N(nt) => {
                    let nt = *nt;
                    for &prod_id in &bnf.prods_of[nt] {
                        chart.add(
                            i,
                            Item {
                                prod: prod_id as u32,
                                dot: 0,
                                origin: i as u32,
                            },
                        )?;
                    }
                    if aycock && nullable[nt] {
                        chart.add(
                            i,
                            Item {
                                prod: item.prod,
                                dot: item.dot + 1,
                                origin: item.origin,
                            },
                        )?;
                    }
                }
                Sym::T(t) => {
                    if i < n && tokens[i] == t {
                        chart.add(
                            i + 1,
                            Item {
                                prod: item.prod,
                                dot: item.dot + 1,
                                origin: item.origin,
                            },
                        )?;
                    }
                }
            }
        }
    }

    Ok(ChartBuilt {
        bnf,
        nullable,
        start_id,
        chart,
        n,
        dummy_prod,
    })
}

fn sentence_accepted(built: &ChartBuilt) -> bool {
    built.chart.sets[built.n].iter().any(|item| {
        item.prod == built.dummy_prod && item.origin == 0 && {
            let prod = &built.bnf.prods[item.prod as usize];
            item.dot as usize >= prod.rhs.len()
        }
    })
}

fn furthest_set(chart: &Chart) -> usize {
    chart
        .sets
        .iter()
        .enumerate()
        .rev()
        .find(|(_, set)| !set.is_empty())
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn judge(built: &ChartBuilt) -> Recognition {
    if sentence_accepted(built) {
        Recognition::Accepted
    } else {
        Recognition::Rejected {
            furthest: furthest_set(&built.chart),
        }
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
    match build_chart(
        grammar,
        start,
        tokens,
        max_tokens,
        max_items,
        aycock,
        strip_epsilon,
    ) {
        Ok(built) => judge(&built),
        Err(refusal) => refusal,
    }
}

/// Recognise `tokens` as a complete derivation of `start` in `grammar`.
pub fn recognise(grammar: &Grammar, start: &str, tokens: &[&str]) -> Recognition {
    recognise_inner(grammar, start, tokens, MAX_TOKENS, MAX_ITEMS, true, false)
}

/// Count derivation trees of `start` over `tokens`.
///
/// The chart is the one [`recognise`] builds. The count is dynamic
/// programming over completed productions: no packed forest is stored.
/// Re-entering a nonterminal on the same span is a nullable cycle and
/// yields [`ParseCount::AtLeastMax`] instead of diverging. An accepted
/// sentence whose count comes out `Exact(0)` means a completed item was
/// missed; that value is returned, not rewritten.
pub fn count_parses(grammar: &Grammar, start: &str, tokens: &[&str]) -> CountOutcome {
    let built = match build_chart(grammar, start, tokens, MAX_TOKENS, MAX_ITEMS, true, false) {
        Ok(built) => built,
        Err(Recognition::TooLong) => return CountOutcome::TooLong,
        Err(Recognition::TooManyItems) => return CountOutcome::TooManyItems,
        Err(Recognition::Rejected { furthest }) => return CountOutcome::Rejected { furthest },
        Err(Recognition::Accepted) => {
            panic!("internal: chart construction does not accept by itself")
        }
    };
    match judge(&built) {
        Recognition::Accepted => CountOutcome::Counted(count_chart(&built, tokens)),
        Recognition::Rejected { furthest } => CountOutcome::Rejected { furthest },
        Recognition::TooLong | Recognition::TooManyItems => {
            panic!("internal: a finished chart is accepted or rejected")
        }
    }
}

#[derive(Clone, Copy)]
enum Memo {
    InProgress,
    Done(ParseCount),
}

struct Counter<'a> {
    bnf: &'a Bnf,
    nullable: &'a [bool],
    /// `suffix[prod][k]` is true when `rhs[k..]` derives ε. The entry past
    /// the last symbol is true.
    suffix: Vec<Vec<bool>>,
    /// Completed `(production, origin, end)` items in the chart.
    completed: HashSet<(u32, u32, u32)>,
    tokens: &'a [&'a str],
    nt_memo: HashMap<(u32, u32, u32), Memo>,
    seq_memo: HashMap<(u32, u32, u32, u32), Memo>,
}

fn symbol_nullable(sym: &Sym, nullable: &[bool]) -> bool {
    match sym {
        Sym::T(_) => false,
        Sym::N(id) => nullable[*id],
    }
}

fn completed_items(bnf: &Bnf, chart: &Chart) -> HashSet<(u32, u32, u32)> {
    let mut done = HashSet::new();
    for (end, items) in chart.sets.iter().enumerate() {
        for item in items {
            let prod = &bnf.prods[item.prod as usize];
            if item.dot as usize >= prod.rhs.len() {
                done.insert((item.prod, item.origin, end as u32));
            }
        }
    }
    done
}

fn count_chart(built: &ChartBuilt, tokens: &[&str]) -> ParseCount {
    let suffix = built
        .bnf
        .prods
        .iter()
        .map(|prod| {
            let n = prod.rhs.len();
            let mut suf = vec![false; n + 1];
            suf[n] = true;
            for i in (0..n).rev() {
                suf[i] = symbol_nullable(&prod.rhs[i], &built.nullable) && suf[i + 1];
            }
            suf
        })
        .collect();
    let mut counter = Counter {
        bnf: &built.bnf,
        nullable: &built.nullable,
        suffix,
        completed: completed_items(&built.bnf, &built.chart),
        tokens,
        nt_memo: HashMap::new(),
        seq_memo: HashMap::new(),
    };
    counter.nt(built.start_id, 0, built.n as u32)
}

impl Counter<'_> {
    fn nt(&mut self, id: usize, from: u32, to: u32) -> ParseCount {
        let key = (id as u32, from, to);
        match self.nt_memo.get(&key) {
            Some(Memo::InProgress) => return ParseCount::AtLeastMax,
            Some(Memo::Done(count)) => return *count,
            None => {}
        }
        if from == to && !self.nullable[id] {
            self.nt_memo.insert(key, Memo::Done(ParseCount::Exact(0)));
            return ParseCount::Exact(0);
        }
        let prods: Vec<usize> = self.bnf.prods_of[id]
            .iter()
            .copied()
            .filter(|&prod| self.completed.contains(&(prod as u32, from, to)))
            .collect();
        if prods.is_empty() {
            self.nt_memo.insert(key, Memo::Done(ParseCount::Exact(0)));
            return ParseCount::Exact(0);
        }
        self.nt_memo.insert(key, Memo::InProgress);
        let mut acc = ParseCount::Exact(0);
        for prod in prods {
            acc = acc.add(self.seq(prod, 0, from, to));
            if acc == ParseCount::AtLeastMax {
                break;
            }
        }
        self.nt_memo.insert(key, Memo::Done(acc));
        acc
    }

    fn seq(&mut self, prod: usize, index: usize, from: u32, to: u32) -> ParseCount {
        let key = (prod as u32, index as u32, from, to);
        match self.seq_memo.get(&key) {
            Some(Memo::InProgress) => return ParseCount::AtLeastMax,
            Some(Memo::Done(count)) => return *count,
            None => {}
        }
        let rhs_len = self.bnf.prods[prod].rhs.len();
        if index == rhs_len {
            let count = if from == to {
                ParseCount::Exact(1)
            } else {
                ParseCount::Exact(0)
            };
            self.seq_memo.insert(key, Memo::Done(count));
            return count;
        }
        // Own the symbol before either recursive call. The RHS borrow must
        // not be held across `nt` / `seq`.
        let symbol = self.bnf.prods[prod].rhs[index].clone();
        let sym_nullable = symbol_nullable(&symbol, self.nullable);
        let rest_nullable = self.suffix[prod][index + 1];
        self.seq_memo.insert(key, Memo::InProgress);
        let mut acc = ParseCount::Exact(0);
        if from <= to {
            // A split that gives the symbol or the remainder an empty span
            // is legal only when that part derives ε. Letting a
            // non-nullable remainder take ε would treat the left-recursive
            // step of `E → E + E` as a cycle.
            for mid in from..=to {
                if mid == from && !sym_nullable {
                    continue;
                }
                if mid == to && !rest_nullable {
                    continue;
                }
                let left = match &symbol {
                    Sym::T(t) => self.terminal(t, from, mid),
                    Sym::N(nt) => self.nt(*nt, from, mid),
                };
                if left == ParseCount::Exact(0) {
                    continue;
                }
                let right = self.seq(prod, index + 1, mid, to);
                acc = acc.add(left.mul(right));
                if acc == ParseCount::AtLeastMax {
                    break;
                }
            }
        }
        self.seq_memo.insert(key, Memo::Done(acc));
        acc
    }

    fn terminal(&self, text: &str, from: u32, to: u32) -> ParseCount {
        let start = from as usize;
        if to == from + 1 && self.tokens.get(start) == Some(&text) {
            ParseCount::Exact(1)
        } else {
            ParseCount::Exact(0)
        }
    }
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
        count_parses, recognise, recognise_limited, recognise_naive, CountOutcome, ParseCount,
        Recognition, MAX_ITEMS, MAX_PARSES, MAX_TOKENS,
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

    fn counted(src: &str, start: &str, tokens: &[&str]) -> ParseCount {
        match count_parses(&g(src), start, tokens) {
            CountOutcome::Counted(count) => count,
            other => panic!("expected a count, got {other:?} for {tokens:?} in {src}"),
        }
    }

    fn plus_operands(operands: usize) -> Vec<&'static str> {
        let mut tokens = Vec::with_capacity(operands * 2);
        for i in 0..operands {
            if i > 0 {
                tokens.push("PLUS");
            }
            tokens.push("A");
        }
        tokens
    }

    #[test]
    fn ambiguous_addition_has_the_catalan_number_of_parses() {
        let src = "e = e PLUS e | A ;";
        // Ways to parenthesize k operands under E → E + E | a: C_{k-1}.
        let expected = [(1, 1u32), (2, 1), (3, 2), (4, 5), (5, 14), (6, 42)];
        for (operands, want) in expected {
            let tokens = plus_operands(operands);
            assert_eq!(
                counted(src, "e", &tokens),
                ParseCount::Exact(want),
                "{operands} operands ({tokens:?})"
            );
        }
    }

    #[test]
    fn a_right_recursive_sum_has_one_parse() {
        let src = "e = t { PLUS t } ; t = A ;";
        for operands in 1..=6 {
            let tokens = plus_operands(operands);
            assert_eq!(
                counted(src, "e", &tokens),
                ParseCount::Exact(1),
                "{operands} operands"
            );
        }
    }

    #[test]
    fn left_recursion_with_a_terminal_tail_stays_finite() {
        // The left-hand E of `E → E + t` must not be treated as a nullable
        // cycle: `+ t` does not derive ε, so each string has one tree.
        let src = "e = e PLUS t | t ; t = A ;";
        for operands in 1..=6 {
            let tokens = plus_operands(operands);
            assert_eq!(
                counted(src, "e", &tokens),
                ParseCount::Exact(1),
                "{operands} operands"
            );
        }
    }

    #[test]
    fn an_optional_symbol_has_one_parse_on_either_side() {
        let src = "s = [ A ] ;";
        assert_eq!(counted(src, "s", &[]), ParseCount::Exact(1));
        assert_eq!(counted(src, "s", &["A"]), ParseCount::Exact(1));
    }

    #[test]
    fn a_repeat_of_a_terminal_has_one_parse() {
        let src = "s = { A } ;";
        assert_eq!(counted(src, "s", &[]), ParseCount::Exact(1));
        assert_eq!(counted(src, "s", &["A", "A", "A"]), ParseCount::Exact(1));
    }

    #[test]
    fn a_direct_cycle_is_at_least_max_and_returns() {
        assert_eq!(counted("s = s | A ;", "s", &["A"]), ParseCount::AtLeastMax);
    }

    #[test]
    fn a_nullable_cycle_is_at_least_max_and_returns() {
        // `[ s ]` is ε | s. The empty derivation can be wrapped any number
        // of times, so the empty sentence has infinitely many trees.
        assert_eq!(counted("s = [ s ] ;", "s", &[]), ParseCount::AtLeastMax);
        // `{ [ A ] }` inserts any number of empty iterations around A.
        assert_eq!(counted("s = { [ A ] } ;", "s", &[]), ParseCount::AtLeastMax);
        assert_eq!(
            counted("s = { [ A ] } ;", "s", &["A"]),
            ParseCount::AtLeastMax
        );
    }

    #[test]
    fn a_nullable_chain_that_is_not_a_cycle_has_one_parse() {
        let src = r#"
            s = a x ;
            a = b ;
            b = c ;
            c = [ z ] ;
            x = X ;
            z = Z ;
        "#;
        assert_eq!(counted(src, "s", &["X"]), ParseCount::Exact(1));
        assert_eq!(counted(src, "s", &["Z", "X"]), ParseCount::Exact(1));
    }

    #[test]
    fn repeated_alternatives_stop_at_max_parses() {
        assert_eq!(MAX_PARSES, 1000);
        fn alternatives(n: usize) -> String {
            let mut body = String::new();
            for i in 0..n {
                if i > 0 {
                    body.push_str(" | ");
                }
                body.push_str("\"x\"");
            }
            format!("s = {body} ;")
        }
        assert_eq!(counted(&alternatives(1), "s", &["x"]), ParseCount::Exact(1));
        assert_eq!(counted(&alternatives(2), "s", &["x"]), ParseCount::Exact(2));
        assert_eq!(
            counted(&alternatives(MAX_PARSES as usize), "s", &["x"]),
            ParseCount::Exact(MAX_PARSES)
        );
        assert_eq!(
            counted(&alternatives(MAX_PARSES as usize + 1), "s", &["x"]),
            ParseCount::AtLeastMax
        );
    }

    #[test]
    fn the_count_agrees_with_the_recogniser_on_refusal() {
        let src = "s = A B ;";
        assert_eq!(
            count_parses(&g(src), "s", &["A"]),
            CountOutcome::Rejected { furthest: 1 }
        );
        assert_eq!(
            count_parses(&g("s = A ;"), "nope", &["A"]),
            CountOutcome::Rejected { furthest: 0 }
        );
        let over: Vec<&str> = vec!["X"; MAX_TOKENS + 1];
        assert_eq!(
            count_parses(&g("s = { X } ;"), "s", &over),
            CountOutcome::TooLong
        );
        assert_eq!(recognise(&g(src), "s", &["A", "B"]), Recognition::Accepted);
        assert_eq!(
            count_parses(&g(src), "s", &["A", "B"]),
            CountOutcome::Counted(ParseCount::Exact(1))
        );
    }
}

/// `count_parses` against a second counter that shares nothing with it but
/// the lowering: no chart, no cycle detection, no cap arithmetic. Trees are
/// counted by height bound. A finite count stops growing once the bound
/// passes the tallest tree; a nullable cycle keeps adding trees, so two
/// bounds disagree.
#[cfg(test)]
mod brute_force {
    use std::collections::HashMap;

    use super::{count_parses, lower, Bnf, CountOutcome, ParseCount, Sym, MAX_PARSES};
    use crate::ebnf::parse;

    struct Brute<'a> {
        bnf: &'a Bnf,
        tokens: &'a [&'a str],
        memo: HashMap<(usize, usize, usize, usize), u128>,
    }

    /// Saturation point. Far above `MAX_PARSES`, so a saturated count still
    /// compares as "more than the cap".
    const SATURATE: u128 = 1 << 100;

    impl Brute<'_> {
        /// Trees of nonterminal `id` over `tokens[from..to]` no taller than `height`.
        fn nt(&mut self, id: usize, from: usize, to: usize, height: usize) -> u128 {
            if height == 0 {
                return 0;
            }
            if let Some(&count) = self.memo.get(&(id, from, to, height)) {
                return count;
            }
            let mut acc = 0u128;
            for prod in self.bnf.prods_of[id].clone() {
                acc = (acc + self.seq(prod, 0, from, to, height - 1)).min(SATURATE);
            }
            self.memo.insert((id, from, to, height), acc);
            acc
        }

        fn seq(
            &mut self,
            prod: usize,
            index: usize,
            from: usize,
            to: usize,
            height: usize,
        ) -> u128 {
            let rhs = &self.bnf.prods[prod].rhs;
            if index == rhs.len() {
                return u128::from(from == to);
            }
            let symbol = rhs[index].clone();
            let mut acc = 0u128;
            for mid in from..=to {
                let left = match &symbol {
                    Sym::T(t) => u128::from(mid == from + 1 && self.tokens[from] == t),
                    Sym::N(nt) => self.nt(*nt, from, mid, height),
                };
                if left == 0 {
                    continue;
                }
                let right = self.seq(prod, index + 1, mid, to, height);
                acc = (acc + left.saturating_mul(right).min(SATURATE)).min(SATURATE);
            }
            acc
        }
    }

    /// `None` when the sentence has no tree at all.
    fn brute(src: &str, tokens: &[&str]) -> Option<ParseCount> {
        let bnf = lower(&parse(src).expect("generated grammar parses"));
        let start = bnf.id["s"];
        // Along a root-to-leaf path the span only shrinks, and a symbol
        // repeats on one span only through a cycle. Without cycles every
        // tree is at most |N| · (n + 1) tall.
        let bound = bnf.names.len() * (tokens.len() + 2) + 2;
        let mut brute = Brute {
            bnf: &bnf,
            tokens,
            memo: HashMap::new(),
        };
        let low = brute.nt(start, 0, tokens.len(), bound);
        let high = brute.nt(start, 0, tokens.len(), 2 * bound);
        if high == 0 {
            return None;
        }
        Some(if low != high || low > u128::from(MAX_PARSES) {
            ParseCount::AtLeastMax
        } else {
            ParseCount::Exact(low as u32)
        })
    }

    /// Fixed-seed generator: the test is deterministic and needs no crate.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }

    fn expr(rng: &mut Lcg, depth: u64) -> String {
        const ATOMS: [&str; 7] = ["s", "a", "b", "X", "Y", "X", "Y"];
        let atom = |rng: &mut Lcg| ATOMS[rng.below(ATOMS.len() as u64) as usize].to_string();
        if depth == 0 {
            return atom(rng);
        }
        match rng.below(6) {
            0 => format!("[ {} ]", expr(rng, depth - 1)),
            1 => format!("{{ {} }}", expr(rng, depth - 1)),
            2 => format!("( {} | {} )", expr(rng, depth - 1), expr(rng, depth - 1)),
            3 => format!("{} {}", expr(rng, depth - 1), expr(rng, depth - 1)),
            _ => atom(rng),
        }
    }

    fn rule_body(rng: &mut Lcg) -> String {
        let alternatives = 1 + rng.below(3);
        (0..alternatives)
            .map(|_| {
                let depth = 1 + rng.below(3);
                expr(rng, depth)
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// Random three-rule grammars with recursion, optionals, repeats and
    /// nullable cycles. Each defence in the counter (the empty-factor rule
    /// in `mul`, the nullable-remainder guard in `seq`) was removed in turn
    /// and this test failed both times; the hand-written tests above did not.
    #[test]
    fn count_parses_agrees_with_counting_by_height() {
        const CASES: usize = 1500;
        let mut rng = Lcg(0x5eed);
        let (mut accepted, mut ambiguous, mut cyclic) = (0, 0, 0);
        for case in 0..CASES {
            let src = format!(
                "s = {} ;\na = {} ;\nb = {} ;",
                rule_body(&mut rng),
                rule_body(&mut rng),
                rule_body(&mut rng)
            );
            let len = rng.below(6) as usize;
            let tokens: Vec<&str> = (0..len)
                .map(|_| if rng.below(2) == 0 { "X" } else { "Y" })
                .collect();
            let want = brute(&src, &tokens);
            let got = count_parses(
                &parse(&src).expect("generated grammar parses"),
                "s",
                &tokens,
            );
            match (want, got) {
                (None, CountOutcome::Rejected { .. }) => {}
                (Some(want), CountOutcome::Counted(got)) if want == got => {
                    accepted += 1;
                    match got {
                        ParseCount::Exact(1) => {}
                        ParseCount::Exact(_) => ambiguous += 1,
                        ParseCount::AtLeastMax => cyclic += 1,
                    }
                }
                (want, got) => {
                    panic!("case {case}: {src}\ntokens {tokens:?}: by height {want:?}, count_parses {got:?}")
                }
            }
        }
        // The generator must keep reaching every kind of answer, or the
        // agreement above says little.
        assert!(accepted > CASES / 4, "accepted {accepted} of {CASES}");
        assert!(ambiguous > 30, "finite counts above one: {ambiguous}");
        assert!(cyclic > 100, "cyclic counts: {cyclic}");
    }
}

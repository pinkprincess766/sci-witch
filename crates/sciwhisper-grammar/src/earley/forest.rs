//! Shared packed parse forest built from the Earley chart.
//!
//! This is a child module of `earley` and reuses its private items (chart,
//! `Counter`, nullable and suffix tables) without widening any visibility.

use std::collections::HashMap;

use super::{
    build_chart, judge, make_counter, Counter, ParseCount, Recognition, Sym, MAX_ITEMS, MAX_TOKENS,
};
use crate::ebnf::Grammar;

/// One node of a [`Forest`]: a nonterminal over the token span `from..to`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForestNode {
    /// Nonterminal name, as in [`crate::earley::productions`].
    pub symbol: String,
    pub from: usize,
    pub to: usize,
    /// One entry per way to derive this span: the production id and the child
    /// node ids for the nonterminals of its right-hand side, in order.
    /// Terminals carry no node.
    pub packed: Vec<(usize, Vec<usize>)>,
}

/// A shared packed parse forest of one accepted sentence.
///
/// There is one node per `(nonterminal, from, to)` that occurs in some
/// complete tree of the root, so the size is polynomial in the sentence
/// length even when the number of trees is exponential. Node ids follow
/// creation order, not depth; start from [`Forest::root`].
///
/// This is not Scott's binarised SPPF. There the right-hand side of every
/// production is split into pairs through intermediate nodes, which keeps
/// the forest cubic in the length for any grammar. Here a packed entry lists
/// all children of a production at once, so one node can carry up to
/// `O(n^(k-1))` entries for a right-hand side of `k` nonterminals. That is
/// enough for this crate: the right-hand sides in `math.ebnf` are short, and
/// the forest is used to count and enumerate trees and to measure forest
/// size, not to parse arbitrary grammars in cubic time.
///
/// The trees are exactly those [`crate::earley::count_parses`] counts: the
/// same chart, the same split rules (nullable symbol, nullable suffix) and
/// the same empty-factor pruning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forest {
    nodes: Vec<ForestNode>,
    root: usize,
    /// Node ids with every node after all of its children.
    order: Vec<usize>,
}

impl Forest {
    /// Id of the node for the start symbol over the whole sentence.
    pub fn root(&self) -> usize {
        self.root
    }

    pub fn nodes(&self) -> &[ForestNode] {
        &self.nodes
    }

    /// Number of derivation trees of the root, by dynamic programming over
    /// the DAG. Saturates at `u64::MAX`; the count is not capped at
    /// [`crate::earley::MAX_PARSES`].
    pub fn tree_count(&self) -> u64 {
        let mut count = vec![0u64; self.nodes.len()];
        for &id in &self.order {
            let mut total = 0u64;
            for (_, children) in &self.nodes[id].packed {
                let product = children
                    .iter()
                    .fold(1u64, |acc, &child| acc.saturating_mul(count[child]));
                total = total.saturating_add(product);
            }
            count[id] = total;
        }
        count[self.root]
    }

    /// Every derivation tree of the root as production ids in pre-order,
    /// sorted ascending. Same shape as
    /// [`crate::earley::DerivationOutcome::Derivations`].
    ///
    /// The list has [`Forest::tree_count`] entries, which can be
    /// astronomically many: check the count before calling.
    pub fn trees(&self) -> Vec<Vec<usize>> {
        let mut lists: Vec<Vec<Vec<usize>>> = vec![Vec::new(); self.nodes.len()];
        for &id in &self.order {
            let mut trees = Vec::new();
            for (prod, children) in &self.nodes[id].packed {
                let mut partial = vec![vec![*prod]];
                for &child in children {
                    let mut next = Vec::with_capacity(partial.len() * lists[child].len());
                    for head in &partial {
                        for tail in &lists[child] {
                            let mut tree = head.clone();
                            tree.extend_from_slice(tail);
                            next.push(tree);
                        }
                    }
                    partial = next;
                }
                trees.extend(partial);
            }
            lists[id] = trees;
        }
        let mut trees = std::mem::take(&mut lists[self.root]);
        trees.sort();
        trees
    }
}

/// Outcome of [`parse_forest`]. The refusal arms are the recogniser's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForestOutcome {
    Forest(Forest),
    /// The trees of the sentence are infinitely many: a nullable cycle
    /// (`A → A`, `{ [ A ] }`). No forest is built, since it would be cyclic.
    /// Unlike [`crate::earley::derivations`] this is not returned for a
    /// finite count above [`crate::earley::MAX_PARSES`]; the forest of such a
    /// sentence is built and [`Forest::tree_count`] gives the number.
    TooMany,
    Rejected {
        /// Same index [`Recognition::Rejected`] reports.
        furthest: usize,
    },
    TooLong,
    TooManyItems,
}

/// Build the packed parse forest of `start` over `tokens`.
///
/// The chart is the one [`crate::earley::recognise`] builds. A node exists
/// only if some complete tree of the root uses it.
pub fn parse_forest(grammar: &Grammar, start: &str, tokens: &[&str]) -> ForestOutcome {
    let built = match build_chart(grammar, start, tokens, MAX_TOKENS, MAX_ITEMS, true, false) {
        Ok(built) => built,
        Err(Recognition::TooLong) => return ForestOutcome::TooLong,
        Err(Recognition::TooManyItems) => return ForestOutcome::TooManyItems,
        Err(Recognition::Rejected { furthest }) => return ForestOutcome::Rejected { furthest },
        Err(Recognition::Accepted) => {
            panic!("internal: chart construction does not accept by itself")
        }
    };
    match judge(&built) {
        Recognition::Accepted => {}
        Recognition::Rejected { furthest } => return ForestOutcome::Rejected { furthest },
        Recognition::TooLong | Recognition::TooManyItems => {
            panic!("internal: a finished chart is accepted or rejected")
        }
    }
    let root_span = (built.start_id, 0, built.n as u32);
    let mut builder = Builder {
        counter: make_counter(&built, tokens),
        ids: HashMap::new(),
        spans: Vec::new(),
        nodes: Vec::new(),
    };
    assert!(
        builder.counter.nt(root_span.0, root_span.1, root_span.2) != ParseCount::Exact(0),
        "internal: an accepted sentence has no tree"
    );
    let root = builder.node_id(root_span);
    // `spans` doubles as the work list: nodes are expanded in creation order
    // and `node_id` appends, so no recursion follows the depth of the trees.
    let mut next = 0;
    while next < builder.spans.len() {
        builder.expand(next);
        next += 1;
    }
    let nodes = builder.nodes;
    let Some(order) = children_first_order(&nodes, root) else {
        return ForestOutcome::TooMany;
    };
    assert!(
        nodes.iter().all(|node| !node.packed.is_empty()),
        "internal: a forest node without a derivation"
    );
    ForestOutcome::Forest(Forest { nodes, root, order })
}

struct Builder<'a> {
    counter: Counter<'a>,
    /// `(nonterminal, from, to)` to node id.
    ids: HashMap<(usize, u32, u32), usize>,
    /// Inverse of `ids`, indexed by node id.
    spans: Vec<(usize, u32, u32)>,
    nodes: Vec<ForestNode>,
}

impl Builder<'_> {
    fn node_id(&mut self, span: (usize, u32, u32)) -> usize {
        if let Some(&id) = self.ids.get(&span) {
            return id;
        }
        let id = self.spans.len();
        self.ids.insert(span, id);
        self.spans.push(span);
        self.nodes.push(ForestNode {
            symbol: self.counter.bnf.names[span.0].clone(),
            from: span.1 as usize,
            to: span.2 as usize,
            packed: Vec::new(),
        });
        id
    }

    fn expand(&mut self, id: usize) {
        let (nt, from, to) = self.spans[id];
        let prods: Vec<usize> = self.counter.bnf.prods_of[nt]
            .iter()
            .copied()
            .filter(|&prod| self.counter.completed.contains(&(prod as u32, from, to)))
            .collect();
        let mut packed = Vec::new();
        for prod in prods {
            if self.counter.seq(prod, 0, from, to) == ParseCount::Exact(0) {
                continue;
            }
            self.splits(prod, 0, from, to, &mut Vec::new(), &mut packed);
        }
        self.nodes[id].packed = packed;
    }

    /// Every way to split `rhs[index..]` over `from..to`, with the same
    /// rules as `Counter::seq`: an empty span goes only to a nullable symbol
    /// or a nullable remainder, and a half with no tree is skipped.
    fn splits(
        &mut self,
        prod: usize,
        index: usize,
        from: u32,
        to: u32,
        path: &mut Vec<usize>,
        out: &mut Vec<(usize, Vec<usize>)>,
    ) {
        if index == self.counter.bnf.prods[prod].rhs.len() {
            if from == to {
                out.push((prod, path.clone()));
            }
            return;
        }
        let symbol = self.counter.bnf.prods[prod].rhs[index].clone();
        let sym_nullable = match &symbol {
            Sym::T(_) => false,
            Sym::N(id) => self.counter.nullable[*id],
        };
        let rest_nullable = self.counter.suffix[prod][index + 1];
        for mid in from..=to {
            if mid == from && !sym_nullable {
                continue;
            }
            if mid == to && !rest_nullable {
                continue;
            }
            let left = match &symbol {
                Sym::T(t) => self.counter.terminal(t, from, mid),
                Sym::N(nt) => self.counter.nt(*nt, from, mid),
            };
            if left == ParseCount::Exact(0)
                || self.counter.seq(prod, index + 1, mid, to) == ParseCount::Exact(0)
            {
                continue;
            }
            match &symbol {
                Sym::T(_) => self.splits(prod, index + 1, mid, to, path, out),
                Sym::N(nt) => {
                    let child = self.node_id((*nt, from, mid));
                    path.push(child);
                    self.splits(prod, index + 1, mid, to, path, out);
                    path.pop();
                }
            }
        }
    }
}

/// Nodes reachable from `root`, every node after all of its children, or
/// `None` when the reachable graph has a cycle. Iterative: the depth of a
/// forest follows the sentence length, not the thread's stack.
fn children_first_order(nodes: &[ForestNode], root: usize) -> Option<Vec<usize>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Open,
        Done,
    }
    let children: Vec<Vec<usize>> = nodes
        .iter()
        .map(|node| {
            node.packed
                .iter()
                .flat_map(|(_, children)| children.iter().copied())
                .collect()
        })
        .collect();
    let mut mark = vec![Mark::New; nodes.len()];
    let mut order = Vec::with_capacity(nodes.len());
    // (node, how many of its children, across all packed entries, are done).
    let mut stack = vec![(root, 0usize)];
    mark[root] = Mark::Open;
    while let Some((id, visited)) = stack.pop() {
        match children[id].get(visited) {
            None => {
                mark[id] = Mark::Done;
                order.push(id);
            }
            Some(&child) => {
                stack.push((id, visited + 1));
                match mark[child] {
                    Mark::Open => return None,
                    Mark::Done => {}
                    Mark::New => {
                        mark[child] = Mark::Open;
                        stack.push((child, 0));
                    }
                }
            }
        }
    }
    Some(order)
}

#[cfg(test)]
mod tests {
    use super::{parse_forest, Forest, ForestOutcome};
    use crate::earley::{
        count_parses, derivations, CountOutcome, DerivationOutcome, ParseCount, Recognition,
        MAX_TOKENS,
    };
    use crate::ebnf::parse;

    fn g(src: &str) -> crate::ebnf::Grammar {
        parse(src).unwrap_or_else(|e| panic!("{e} in {src}"))
    }

    fn forest(src: &str, start: &str, tokens: &[&str]) -> Forest {
        match parse_forest(&g(src), start, tokens) {
            ForestOutcome::Forest(forest) => forest,
            other => panic!("expected a forest, got {other:?} for {tokens:?} in {src}"),
        }
    }

    const PLUS: &str = r#"s = s "+" s | "x" ;"#;

    /// `x + x + ... + x` with `operands` operands.
    fn sum(operands: usize) -> Vec<&'static str> {
        let mut tokens = Vec::new();
        for i in 0..operands {
            if i > 0 {
                tokens.push("+");
            }
            tokens.push("x");
        }
        tokens
    }

    /// Catalan(m) = C(2m, m) / (m + 1), by the multiplicative recurrence
    /// C_{i+1} = C_i · 2(2i + 1) / (i + 2). Independent of the Earley code.
    fn catalan(m: u64) -> u64 {
        (0..m).fold(1u64, |c, i| c * 2 * (2 * i + 1) / (i + 2))
    }

    #[test]
    fn ambiguous_addition_forest_counts_catalan_and_stays_polynomial() {
        for operands in 1..=8usize {
            let tokens = sum(operands);
            let f = forest(PLUS, "s", &tokens);
            let want = catalan(operands as u64 - 1);
            assert_eq!(f.tree_count(), want, "{operands} operands");
            assert_eq!(
                count_parses(&g(PLUS), "s", &tokens),
                CountOutcome::Counted(ParseCount::Exact(want as u32)),
                "{operands} operands"
            );
            // Operands sit at the even token positions 0, 2, ..., 2(n-1). An
            // `s` node spans operands a..=b, that is tokens 2a..2b+1, and
            // there is one node per pair a <= b: n(n+1)/2 of them. Sharing
            // makes that the whole forest. n(n+1)/2 <= (2n-1)^2 for n >= 1.
            let n = operands;
            assert_eq!(f.nodes().len(), n * (n + 1) / 2, "{operands} operands");
            assert!(
                f.nodes().len() <= tokens.len() * tokens.len(),
                "{operands} operands: {} nodes for {} tokens",
                f.nodes().len(),
                tokens.len()
            );
        }
    }

    #[test]
    fn every_node_is_used_and_derivable() {
        // Pruning: a node with no packed entry is a dead span, and a node
        // that no packed entry (and not the root) points to is unused.
        for operands in 1..=6usize {
            let f = forest(PLUS, "s", &sum(operands));
            let mut referenced = vec![false; f.nodes().len()];
            referenced[f.root()] = true;
            for node in f.nodes() {
                assert!(!node.packed.is_empty(), "dead node {node:?}");
                for (_, children) in &node.packed {
                    for &child in children {
                        referenced[child] = true;
                    }
                }
            }
            assert!(referenced.iter().all(|&r| r), "unreferenced node");
            let root = &f.nodes()[f.root()];
            assert_eq!(
                (root.symbol.as_str(), root.from, root.to),
                ("s", 0, 2 * operands - 1)
            );
        }
    }

    #[test]
    fn a_shared_span_is_one_node_referenced_twice() {
        // x + x + x: tokens 0..5. (s,0,1) is the left operand of the root
        // entry [(s,0,1) + (s,2,5)] and of (s,0,3) = [(s,0,1) + (s,2,3)].
        let f = forest(PLUS, "s", &sum(3));
        let ids: Vec<usize> = f
            .nodes()
            .iter()
            .enumerate()
            .filter(|(_, node)| node.symbol == "s" && (node.from, node.to) == (0, 1))
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids.len(), 1, "one node for (s, 0, 1)");
        let references: usize = f
            .nodes()
            .iter()
            .flat_map(|node| node.packed.iter())
            .filter(|(_, children)| children.contains(&ids[0]))
            .count();
        assert!(
            references >= 2,
            "(s,0,1) referenced by {references} entries"
        );
        assert_eq!(f.nodes().len(), 6);
        assert_eq!(f.nodes()[f.root()].packed.len(), 2);
    }

    #[test]
    fn trees_equal_derivations_on_hand_grammars() {
        let cases: &[(&str, &[&str])] = &[
            (PLUS, &["x", "+", "x", "+", "x"]),
            (PLUS, &["x"]),
            ("s = A | A ;", &["A"]),
            ("s = [ A ] ;", &[]),
            ("s = [ A ] ;", &["A"]),
            ("s = { A } ;", &["A", "A", "A"]),
            ("s = x y ; x = [ Z ] ; y = X ;", &["X"]),
            ("s = x y ; x = [ Z ] ; y = X ;", &["Z", "X"]),
            (
                "s = a x ; a = b ; b = c ; c = [ z ] ; x = X ; z = Z ;",
                &["Z", "X"],
            ),
            (
                "s = t { PLUS t } ; t = A ;",
                &["A", "PLUS", "A", "PLUS", "A"],
            ),
            (
                "s = s PLUS t | t ; t = A ;",
                &["A", "PLUS", "A", "PLUS", "A"],
            ),
            ("s = { [ A ] } ;", &["A"]),
        ];
        for (src, tokens) in cases {
            let want = derivations(&g(src), "s", tokens);
            let outcome = parse_forest(&g(src), "s", tokens);
            match (want, outcome) {
                (DerivationOutcome::Derivations(want), ForestOutcome::Forest(f)) => {
                    assert_eq!(f.trees(), want, "{src} on {tokens:?}");
                    assert_eq!(f.tree_count(), want.len() as u64, "{src} on {tokens:?}");
                }
                (DerivationOutcome::TooMany, ForestOutcome::TooMany) => {}
                (want, got) => panic!("{src} on {tokens:?}: {want:?} vs {got:?}"),
            }
        }
    }

    #[test]
    fn refusals_match_derivations() {
        let over: Vec<&str> = vec!["X"; MAX_TOKENS + 1];
        let rep = g("s = { X } ;");
        assert_eq!(parse_forest(&rep, "s", &over), ForestOutcome::TooLong);
        let ab = g("s = A B ;");
        for tokens in [&["A"][..], &["A", "A"], &["B"], &[]] {
            let Recognition::Rejected { furthest } = crate::earley::recognise(&ab, "s", tokens)
            else {
                panic!("{tokens:?} should be rejected");
            };
            assert_eq!(
                parse_forest(&ab, "s", tokens),
                ForestOutcome::Rejected { furthest },
                "{tokens:?}"
            );
        }
        assert_eq!(
            parse_forest(&g("s = A ;"), "nope", &["A"]),
            ForestOutcome::Rejected { furthest: 0 }
        );
        for src in ["s = s | A ;", "s = [ s ] ;", "s = { [ A ] } ;"] {
            for tokens in [&["A"][..], &[]] {
                if derivations(&g(src), "s", tokens) == DerivationOutcome::TooMany {
                    assert_eq!(
                        parse_forest(&g(src), "s", tokens),
                        ForestOutcome::TooMany,
                        "{src} on {tokens:?}"
                    );
                }
            }
        }
        assert_eq!(
            parse_forest(&g("s = s | A ;"), "s", &["A"]),
            ForestOutcome::TooMany
        );
    }

    #[test]
    fn a_finite_count_above_max_parses_still_gives_a_forest() {
        // 12 operands: Catalan(11) = 58786 > MAX_PARSES, so `derivations`
        // says TooMany, but the forest is small and counts the trees.
        let tokens = sum(12);
        assert_eq!(
            derivations(&g(PLUS), "s", &tokens),
            DerivationOutcome::TooMany
        );
        let f = forest(PLUS, "s", &tokens);
        assert_eq!(f.tree_count(), catalan(11));
        assert_eq!(f.nodes().len(), 12 * 13 / 2);
    }

    #[test]
    fn the_forest_at_max_tokens_is_built_without_deep_recursion() {
        // 256 tokens through a right-recursive repeat; the literal is
        // deliberate, as in the counter's own limit test.
        let tokens = vec!["X"; 256];
        let f = forest("s = { X } ;", "s", &tokens);
        assert_eq!(f.tree_count(), 1);
        let big = forest(PLUS, "s", &sum(128));
        assert_eq!(big.nodes().len(), 128 * 129 / 2);
        // Catalan(127) overflows u64: the DP saturates instead of wrapping.
        assert_eq!(big.tree_count(), u64::MAX);
    }
}

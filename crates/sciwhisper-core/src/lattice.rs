//! Candidate Lattice v1 — every structural reading one utterance supports.
//!
//! [`interpret`](crate::interpret) answers with one reading, and
//! [`interpret_utterance`](crate::interpret_utterance) answers with one
//! document. Both are decisions. This module answers a different question:
//! **what else could this have been?** It returns a typed, ordered,
//! deduplicated set of readings, each carrying the reason it exists, so that a
//! ranker can later be trained on real data instead of on the parser's own
//! first guess.
//!
//! Four rules hold it together, and each of them is a test below.
//!
//! 1. **Nothing here rewrites a string in place.** A transformation produces
//!    a *candidate* whose [`Origin`] names exactly what was changed; the
//!    original words are still in [`Candidate::span`].
//! 2. **`RAW` is a candidate, not a filler.** Keeping the dictated words is
//!    the right answer for ordinary speech, and it is always in the set.
//! 3. **Offering is not deciding.** No shipped path calls this module. A
//!    wider lattice cannot, on its own, turn ordinary speech into notation,
//!    because nothing here inserts anything.
//! 4. **Every limit is explicit and tested.** Candidates, words, parse
//!    attempts, repairs and transformation depth all have named ceilings.
//!
//! [`Candidate::features`] is a vector of deterministic counts. None of them
//! is a probability, and [`Features::parse_level`] in particular is the
//! parser's own structural level — the same number `confidence` has always
//! been — not a calibrated estimate of anything.

use std::collections::BTreeSet;

use crate::ast::{Chemical, Domain, Math, Node};
use crate::interpret::{interpret, InterpretOptions};
use crate::lexicon::{ChemConnective, Lexicon};
use crate::normalize::words as split_words;
use crate::utterance::{
    bridge_phrases, interpret_utterance, is_filler, ScienceSpan, UtteranceMode, UtteranceOptions,
};

/// Largest lattice this module will ever return.
///
/// The generators below are a fixed list applied to the original utterance
/// only — none of them feeds another — so the real count is linear in the
/// number of generators. This ceiling is the guarantee that stays true even
/// if that ever stops being obvious.
pub const MAX_CANDIDATES: usize = 32;

/// Longest utterance the lattice will look at, in words. Mirrors
/// [`crate::utterance::MAX_UTTERANCE_WORDS`]: past it, the text is not one
/// dictated construct and only `RAW` is offered.
pub const MAX_WORDS: usize = 400;

/// Bound byte-sized work too: a single token may be arbitrarily long.
pub const MAX_INPUT_BYTES: usize = 16 * 1024;

/// Top-level parser calls allowed across generators. Mixed-text parsing has
/// its own bounded internal span search; those attempts are not counted here.
pub const MAX_ATTEMPTS: usize = 64;

/// Rotations tried when a product clause was spoken first.
pub const MAX_ROTATIONS: usize = 4;

/// Words one utterance may have repaired, and the shortest word a repair may
/// touch. A single vowel swap inside a long word is a recognisable Russian
/// ASR error; the same swap inside a three-letter word is a different word.
pub const MAX_REPAIRED_WORDS: usize = 2;
pub const MIN_REPAIR_WORD_CHARS: usize = 5;
pub const MAX_REPAIR_WORD_CHARS: usize = 64;

/// The vowel confusions this repair covers, in a fixed order.
///
/// This is a *rule*, not a dictionary: it is four ordered pairs, it applies
/// at most once per word, and the result has to already be in the lexicon to
/// survive. An open list of misspellings would grow without bound and would
/// start inventing substances; this cannot produce a word the project does
/// not already know.
pub const REPAIR_VOWELS: [(char, char); 4] = [('а', 'о'), ('о', 'а'), ('е', 'и'), ('и', 'е')];

/// A byte range in the original transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn whole(text: &str) -> Self {
        Span {
            start: 0,
            end: text.len(),
        }
    }

    pub fn slice<'a>(&self, text: &'a str) -> &'a str {
        text.get(self.start..self.end).unwrap_or("")
    }
}

/// What a candidate proposes be written.
#[derive(Clone, Debug, PartialEq)]
pub enum Reading {
    /// Keep the dictated words exactly as they are.
    Raw,
    /// Write this structure instead.
    Ast(Node),
}

impl Reading {
    pub fn is_raw(&self) -> bool {
        matches!(self, Reading::Raw)
    }

    pub fn ast(&self) -> Option<&Node> {
        match self {
            Reading::Raw => None,
            Reading::Ast(node) => Some(node),
        }
    }
}

/// Why a candidate exists.
///
/// A candidate may carry several: the same reading is often reached by more
/// than one route, and which routes agreed is itself a feature.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Origin {
    /// The words as dictated.
    Raw,
    /// The whole utterance read as one construct, by automatic routing.
    WholeUtterance,
    /// The whole utterance read as one construct, with the domain forced.
    ForcedDomain(Domain),
    /// A second reading the grammar itself offers, such as a radical whose
    /// end was never spoken.
    GrammarAlternative,
    /// The mixed document the shipped path builds: prose kept, proven spans
    /// replaced.
    MixedDocument,
    /// The longest proven scientific span, lifted out of its sentence.
    MaximalSpan,
    /// Bridge words and fillers were removed before parsing. The words are
    /// listed, so nothing disappears without a name attached.
    BridgeWordsDropped(Vec<String>),
    /// A product clause spoken first was rotated to the end.
    ProductClauseRotated,
    /// The speaker's own correction was applied before parsing.
    SelfCorrection,
    /// One vowel was swapped in the named words to reach a word the lexicon
    /// already knows.
    VowelRepair(Vec<VowelSwap>),
}

impl Origin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Origin::Raw => "raw",
            Origin::WholeUtterance => "whole_utterance",
            Origin::ForcedDomain(_) => "forced_domain",
            Origin::GrammarAlternative => "grammar_alternative",
            Origin::MixedDocument => "mixed_document",
            Origin::MaximalSpan => "maximal_span",
            Origin::BridgeWordsDropped(_) => "bridge_words_dropped",
            Origin::ProductClauseRotated => "product_clause_rotated",
            Origin::SelfCorrection => "self_correction",
            Origin::VowelRepair(_) => "vowel_repair",
        }
    }

    /// Whether this origin changed the words before parsing them. A ranker
    /// should be able to price "the parser read what was said" differently
    /// from "the parser read what it wished had been said".
    pub fn edits_the_words(&self) -> bool {
        matches!(
            self,
            Origin::BridgeWordsDropped(_)
                | Origin::ProductClauseRotated
                | Origin::SelfCorrection
                | Origin::VowelRepair(_)
        )
    }
}

/// Deterministic counts describing a candidate.
///
/// Counts saturate at u16::MAX; flags are computed from the AST and the
/// parse, with no arithmetic that could turn into a score. Weighing them
/// against each other is the ranker's job, and there is no ranker here.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Features {
    /// Words in the original utterance.
    pub words: u16,
    /// Words the candidate's own span covers.
    pub covered_words: u16,
    /// Words removed or moved; for corrections, only words outside the span.
    pub edited_words: u16,
    /// Warnings the parse raised.
    pub warnings: u16,
    /// Nodes and depth of the produced structure.
    pub ast_nodes: u16,
    pub ast_depth: u16,
    /// Atoms per formula, summed without reaction coefficients. Saturates
    /// at u16::MAX (also when exact formula counting overflows).
    pub atoms: u16,
    /// Text nodes left inside a document, i.e. prose that survived.
    pub text_nodes: u16,
    /// `Some(false)` where a dictated equation does not conserve atoms.
    /// `None` where balance is not a question that applies.
    pub balanced: Option<bool>,
    /// Passes the structural validator with no `math.*` defect.
    pub structurally_valid: bool,
    /// The parser's own structural level. **Not** a probability: it takes
    /// two values, 0.95 for a clean parse and 0.7 for one the grammar itself
    /// called ambiguous.
    pub parse_level: f32,
}

/// One reading of the utterance, with everything needed to rank it later.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub reading: Reading,
    /// The domain that produced the reading. `None` for `RAW`, which is not
    /// the product of any domain.
    pub domain: Option<Domain>,
    /// Where in the *original* transcript this reading came from.
    pub span: Span,
    /// The normalized form of that span, as the parser saw it.
    pub normalized: String,
    /// Every route that reached this reading, in generation order.
    pub origins: Vec<Origin>,
    /// Warning codes the parse raised, in the order the validator produced
    /// them. Kept as codes rather than messages: a message is for a person,
    /// a code is what a ranker can count.
    pub warning_codes: Vec<String>,
    pub features: Features,
    /// Whether this reading accounts for **everything the speaker said**,
    /// exactly as they said it.
    ///
    /// This is the line that keeps a wider lattice from becoming a riskier
    /// one. A mixed document keeps prose inside the structure; a maximal
    /// span throws the surrounding prose away; a bridge reduction removes
    /// words. All three are useful readings and none of them is "this whole
    /// utterance was a formula", so none of them may be preferred over
    /// keeping the words.
    pub whole_utterance: bool,
    /// Position in the generated order, counted from 1.
    pub rank: usize,
}

impl Candidate {
    pub fn is_raw(&self) -> bool {
        self.reading.is_raw()
    }

    /// Whether any route to this reading had to edit the words first.
    pub fn edits_the_words(&self) -> bool {
        self.origins.iter().any(Origin::edits_the_words)
    }
}

#[derive(Clone, Debug)]
pub struct LatticeOptions {
    /// Forced domain passes to run. `Domain::Auto` is always run first.
    pub domain: Domain,
    pub allow_shortcuts: bool,
}

impl Default for LatticeOptions {
    fn default() -> Self {
        Self {
            domain: Domain::Auto,
            allow_shortcuts: true,
        }
    }
}

/// The lattice, plus an honest account of what was left out.
#[derive(Clone, Debug)]
pub struct Lattice {
    pub candidates: Vec<Candidate>,
    /// Distinct structures offered, `RAW` excluded.
    pub distinct_asts: usize,
    /// A limit stopped generation before the generators ran out.
    pub truncated: bool,
    /// Parse attempts actually spent.
    pub attempts: usize,
}

impl Lattice {
    pub fn first(&self) -> Option<&Candidate> {
        self.candidates.first()
    }
}

/// Builds the candidate lattice for one utterance.
///
/// The order is fixed by the generator list and by nothing else: no hashing,
/// no scoring, no sorting. Two runs on the same string produce byte-identical
/// lattices, which is what makes a stored lattice usable as training data
/// later.
pub fn build(text: &str, options: LatticeOptions) -> Lattice {
    let mut builder = Builder::new(text, options);
    builder.run();
    builder.finish()
}

struct Builder<'a> {
    text: &'a str,
    options: LatticeOptions,
    lex: &'static Lexicon,
    words: Vec<String>,
    out: Vec<Candidate>,
    keys: Vec<String>,
    attempts: usize,
    truncated: bool,
}

impl<'a> Builder<'a> {
    fn new(text: &'a str, options: LatticeOptions) -> Self {
        Builder {
            text,
            options,
            lex: Lexicon::builtin(),
            words: if text.len() <= MAX_INPUT_BYTES {
                split_words(text)
            } else {
                Vec::new()
            },
            out: Vec::new(),
            keys: Vec::new(),
            attempts: 0,
            truncated: false,
        }
    }

    fn run(&mut self) {
        // `RAW` first, so that an utterance which produces nothing else still
        // has a complete lattice, and so that truncation can never remove it.
        self.push_raw();
        if self.text.len() > MAX_INPUT_BYTES {
            self.truncated = true;
            return;
        }
        if self.words.is_empty() {
            return;
        }
        if self.words.len() > MAX_WORDS {
            // The bound is a finding, not a failure: the lattice says it
            // stopped rather than pretending the utterance had one reading.
            self.truncated = true;
            return;
        }
        self.whole_utterance();
        self.forced_domains();
        self.mixed_document();
        self.maximal_span();
        self.bridge_reduction();
        self.product_rotation();
        self.self_correction();
        self.vowel_repair();
    }

    fn finish(self) -> Lattice {
        let mut candidates = self.out;
        for (index, candidate) in candidates.iter_mut().enumerate() {
            candidate.rank = index + 1;
        }
        let distinct: BTreeSet<String> = candidates
            .iter()
            .filter(|candidate| !candidate.is_raw())
            .filter_map(|candidate| key_of(&candidate.reading))
            .collect();
        Lattice {
            distinct_asts: distinct.len(),
            truncated: self.truncated,
            attempts: self.attempts,
            candidates,
        }
    }

    // ------------------------------------------------------------ budget

    /// Spends one parse attempt. `false` means the budget is gone and the
    /// caller must stop; the lattice records that it was cut short.
    fn spend(&mut self) -> bool {
        if self.attempts >= MAX_ATTEMPTS {
            self.truncated = true;
            return false;
        }
        self.attempts += 1;
        true
    }

    fn room(&mut self) -> bool {
        if self.out.len() >= MAX_CANDIDATES {
            self.truncated = true;
            return false;
        }
        true
    }

    // -------------------------------------------------------- generators

    fn push_raw(&mut self) {
        let candidate = Candidate {
            reading: Reading::Raw,
            domain: None,
            span: Span::whole(self.text),
            normalized: if self.text.len() <= MAX_INPUT_BYTES {
                crate::normalize::normalize(self.text)
            } else {
                self.text.to_owned()
            },
            origins: vec![Origin::Raw],
            warning_codes: Vec::new(),
            features: Features {
                words: clamp_u16(self.words.len()),
                covered_words: clamp_u16(self.words.len()),
                structurally_valid: true,
                ..Features::default()
            },
            whole_utterance: true,
            rank: 0,
        };
        self.push(candidate);
    }

    fn whole_utterance(&mut self) {
        if !self.spend() {
            return;
        }
        let result = interpret(
            self.text,
            InterpretOptions {
                domain: self.options.domain,
                allow_shortcuts: self.options.allow_shortcuts,
            },
        );
        let warnings = codes(&result.warnings);
        if let Some(node) = scientific(&result) {
            let candidate = self.candidate_for(
                node.clone(),
                Some(result.domain),
                Span::whole(self.text),
                result.normalized_transcript.clone(),
                Origin::WholeUtterance,
                warnings.clone(),
                result.confidence,
                0,
            );
            let mut candidate = candidate;
            candidate.whole_utterance = true;
            self.push(candidate);
        }
        for alternative in &result.alternatives {
            if matches!(alternative, Node::Text(_)) {
                continue;
            }
            if !self.room() {
                return;
            }
            let candidate = self.candidate_for(
                alternative.clone(),
                Some(result.domain),
                Span::whole(self.text),
                result.normalized_transcript.clone(),
                Origin::GrammarAlternative,
                warnings.clone(),
                result.confidence,
                0,
            );
            let mut candidate = candidate;
            candidate.whole_utterance = true;
            self.push(candidate);
        }
    }

    fn forced_domains(&mut self) {
        if self.options.domain != Domain::Auto {
            return;
        }
        for domain in [Domain::Chemistry, Domain::Mathematics, Domain::Physics] {
            if !self.room() || !self.spend() {
                return;
            }
            let result = interpret(
                self.text,
                InterpretOptions {
                    domain,
                    allow_shortcuts: self.options.allow_shortcuts,
                },
            );
            if let Some(node) = scientific(&result) {
                let candidate = self.candidate_for(
                    node.clone(),
                    Some(domain),
                    Span::whole(self.text),
                    result.normalized_transcript.clone(),
                    Origin::ForcedDomain(domain),
                    codes(&result.warnings),
                    result.confidence,
                    0,
                );
                let mut candidate = candidate;
                candidate.whole_utterance = true;
                self.push(candidate);
            }
        }
    }

    fn mixed_document(&mut self) {
        if !self.room() || !self.spend() {
            return;
        }
        let result = self.utterance(self.text);
        if result.spans.is_empty() {
            return;
        }
        let domain = result.spans.first().map(|span| span.domain);
        let candidate = self.candidate_for(
            result.document.clone(),
            domain,
            Span::whole(self.text),
            crate::normalize::normalize(self.text),
            Origin::MixedDocument,
            codes(&result.warnings),
            result.confidence,
            0,
        );
        self.push(candidate);
    }

    /// The longest span the shipped path proved, offered on its own.
    ///
    /// This is what makes «реакция идёт между X и Y с образованием Z» a
    /// candidate equation: the equation is already proven inside the
    /// sentence, and the only thing between it and being an answer is the
    /// framing prose in front of it.
    fn maximal_span(&mut self) {
        if !self.room() || !self.spend() {
            return;
        }
        let result = self.utterance(self.text);
        let Some(span) = longest_span(&result.spans) else {
            return;
        };
        let covered = clamp_u16(split_words(&span.source_text).len());
        let candidate = self.candidate_for(
            span.node.clone(),
            Some(span.domain),
            Span {
                start: span.start,
                end: span.end,
            },
            span.normalized.clone(),
            Origin::MaximalSpan,
            codes(&span.warnings),
            span.confidence,
            0,
        );
        let mut candidate = candidate;
        candidate.features.covered_words = covered;
        self.push(candidate);
        for alternative in &span.alternatives {
            if !self.room() {
                return;
            }
            let candidate = self.candidate_for(
                alternative.clone(),
                Some(span.domain),
                Span {
                    start: span.start,
                    end: span.end,
                },
                span.normalized.clone(),
                Origin::GrammarAlternative,
                codes(&span.warnings),
                span.confidence,
                0,
            );
            self.push(candidate);
        }
    }

    /// Reads the utterance again with the bridge words and fillers gone.
    ///
    /// «…карбонат кальция, **он** разлагается на…» is one reaction with a
    /// pronoun in the joint, and «ну значит карбонат кальция, **вот**,
    /// разлагается…» is the same reaction with a verbal tic in it. Dropping
    /// either is not a repair of the speech — the speaker said it, and the
    /// origin records that it went.
    fn bridge_reduction(&mut self) {
        let (reduced, dropped) = drop_bridges(&self.words);
        if dropped.is_empty() || !self.room() || !self.spend() {
            return;
        }
        let text = reduced.join(" ");
        let result = interpret(
            &text,
            InterpretOptions {
                domain: self.options.domain,
                allow_shortcuts: self.options.allow_shortcuts,
            },
        );
        let Some(node) = scientific(&result) else {
            return;
        };
        let edited = clamp_u16(
            dropped
                .iter()
                .map(|phrase| phrase.split_whitespace().count())
                .sum(),
        );
        let candidate = self.candidate_for(
            node.clone(),
            Some(result.domain),
            Span::whole(self.text),
            result.normalized_transcript.clone(),
            Origin::BridgeWordsDropped(dropped),
            codes(&result.warnings),
            result.confidence,
            edited,
        );
        self.push(candidate);
    }

    /// Reads the utterance again with a fronted product clause moved back.
    ///
    /// «с образованием Z идёт реакция между X и Y» states the product first.
    /// Rotating at a connective boundary is the whole transformation: no
    /// clause analysis, at most [`MAX_ROTATIONS`] tries, and the reagents
    /// keep the order the speaker gave them.
    fn product_rotation(&mut self) {
        if !starts_with_product_marker(self.lex, &self.words) {
            return;
        }
        let cuts = connective_starts(self.lex, &self.words);
        for cut in cuts.into_iter().take(MAX_ROTATIONS) {
            if !self.room() || !self.spend() {
                return;
            }
            let mut rotated: Vec<String> = self.words[cut..].to_vec();
            rotated.extend_from_slice(&self.words[..cut]);
            let text = rotated.join(" ");
            let result = interpret(
                &text,
                InterpretOptions {
                    domain: self.options.domain,
                    allow_shortcuts: self.options.allow_shortcuts,
                },
            );
            if let Some(node) = scientific(&result) {
                let candidate = self.candidate_for(
                    node.clone(),
                    Some(result.domain),
                    Span::whole(self.text),
                    result.normalized_transcript.clone(),
                    Origin::ProductClauseRotated,
                    codes(&result.warnings),
                    result.confidence,
                    clamp_u16(self.words.len()),
                );
                self.push(candidate);
                return;
            }
        }
    }

    /// The reading that survives the speaker's own correction.
    ///
    /// The shipped path already applies corrections inside a span. What it
    /// cannot do is notice that the correction leaves stray words outside the
    /// span — «два икс плюс один, нет, три икс плюс один» keeps a homeless
    /// «два». Reading the corrected span *as the whole answer* is the second
    /// candidate, and it is the one the corpus asks for.
    fn self_correction(&mut self) {
        if !self.room() || !self.spend() {
            return;
        }
        let result = self.utterance(self.text);
        let Some(span) = result
            .spans
            .iter()
            .find(|span| !span.corrections.is_empty())
            .cloned()
        else {
            return;
        };
        let outside = self
            .words
            .len()
            .saturating_sub(split_words(&span.source_text).len());
        let candidate = self.candidate_for(
            span.node.clone(),
            Some(span.domain),
            Span {
                start: span.start,
                end: span.end,
            },
            span.normalized.clone(),
            Origin::SelfCorrection,
            codes(&span.warnings),
            span.confidence,
            clamp_u16(outside),
        );
        self.push(candidate);
    }

    /// One vowel swap per word, at most [`MAX_REPAIRED_WORDS`] words, and
    /// only where the repaired word is already in the chemistry lexicon.
    ///
    /// «карбанат кальция» is a recognizer slip; «гырдксд жлз тр» is not
    /// something to guess at, and this rule cannot reach it.
    fn vowel_repair(&mut self) {
        let Some((repaired, swaps)) = repair_words(self.lex, &self.words) else {
            return;
        };
        if !self.room() || !self.spend() {
            return;
        }
        let text = repaired.join(" ");
        let result = interpret(
            &text,
            InterpretOptions {
                domain: self.options.domain,
                allow_shortcuts: self.options.allow_shortcuts,
            },
        );
        let Some(node) = scientific(&result) else {
            return;
        };
        let edited = clamp_u16(swaps.len());
        let candidate = self.candidate_for(
            node.clone(),
            Some(result.domain),
            Span::whole(self.text),
            result.normalized_transcript.clone(),
            Origin::VowelRepair(swaps),
            codes(&result.warnings),
            result.confidence,
            edited,
        );
        self.push(candidate);
    }

    // ----------------------------------------------------------- helpers

    fn utterance(&self, text: &str) -> crate::utterance::UtteranceResult {
        interpret_utterance(
            text,
            UtteranceOptions {
                domain: self.options.domain,
                mode: UtteranceMode::MixedText,
                allow_shortcuts: self.options.allow_shortcuts,
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn candidate_for(
        &self,
        node: Node,
        domain: Option<Domain>,
        span: Span,
        normalized: String,
        origin: Origin,
        warning_codes: Vec<String>,
        parse_level: f32,
        edited_words: u16,
    ) -> Candidate {
        let mut features = describe(&node);
        features.words = clamp_u16(self.words.len());
        features.covered_words = clamp_u16(split_words(span.slice(self.text)).len());
        features.warnings = clamp_u16(warning_codes.len());
        features.parse_level = parse_level;
        features.edited_words = edited_words;
        Candidate {
            reading: Reading::Ast(node),
            domain,
            span,
            normalized,
            origins: vec![origin],
            warning_codes,
            features,
            whole_utterance: false,
            rank: 0,
        }
    }

    /// Adds a candidate, or records another route to one already present.
    ///
    /// Merging rather than duplicating is the point: that two independent
    /// generators reached the same structure is evidence about that
    /// structure, and throwing the second route away would lose it.
    fn push(&mut self, candidate: Candidate) {
        let Some(key) = key_of(&candidate.reading) else {
            return;
        };
        if let Some(index) = self.keys.iter().position(|seen| seen == &key) {
            let existing = &mut self.out[index];
            // A reading reached by *any* whole-utterance route is one: the
            // narrower routes only add reasons, they cannot take the claim
            // away.
            existing.whole_utterance |= candidate.whole_utterance;
            for origin in candidate.origins {
                if !existing.origins.contains(&origin) {
                    existing.origins.push(origin);
                }
            }
            return;
        }
        if self.out.len() >= MAX_CANDIDATES {
            self.truncated = true;
            return;
        }
        self.keys.push(key);
        self.out.push(candidate);
    }
}

/// The structure the core actually built, or `None` where it declined to
/// build one. A zero level and a bare `Node::Text` are both the system saying
/// "no structure here", which is the `RAW` candidate, not a second reading.
fn scientific(result: &crate::ast::InterpretationResult) -> Option<&Node> {
    if result.confidence <= 0.0 {
        return None;
    }
    match &result.ast {
        Node::Text(_) => None,
        other => Some(other),
    }
}

/// A stable key for deduplication. `RAW` is its own key; a structure is keyed
/// by its serialization, which is exactly the equality the corpus uses.
fn key_of(reading: &Reading) -> Option<String> {
    match reading {
        Reading::Raw => Some("RAW".to_string()),
        Reading::Ast(node) => serde_json::to_string(node).ok(),
    }
}

fn longest_span(spans: &[ScienceSpan]) -> Option<&ScienceSpan> {
    spans
        .iter()
        // A tie keeps the earlier span: the order spans are found in is
        // already deterministic, and `max_by_key` would silently prefer the
        // later one.
        .fold(None::<&ScienceSpan>, |best, span| match best {
            Some(current) if current.end - current.start >= span.end - span.start => Some(current),
            _ => Some(span),
        })
}

fn drop_bridges(words: &[String]) -> (Vec<String>, Vec<String>) {
    let mut out = Vec::with_capacity(words.len());
    let mut dropped = Vec::new();
    let borrowed: Vec<&str> = words.iter().map(String::as_str).collect();
    let mut index = 0;
    while index < words.len() {
        let hit = bridge_phrases().iter().find(|phrase| {
            index + phrase.len() <= borrowed.len()
                && phrase
                    .iter()
                    .enumerate()
                    .all(|(offset, expected)| borrowed[index + offset] == expected)
        });
        match hit {
            Some(phrase) => {
                dropped.push(phrase.join(" "));
                index += phrase.len();
            }
            None if is_filler(&words[index]) => {
                dropped.push(words[index].clone());
                index += 1;
            }
            None => {
                out.push(words[index].clone());
                index += 1;
            }
        }
    }
    (out, dropped)
}

fn starts_with_product_marker(lex: &Lexicon, words: &[String]) -> bool {
    matches!(
        lex.chemistry_speech.connective_at(words, 0),
        Some((ChemConnective::ProductMarker, _))
    )
}

/// Every index at which a connective begins, after the first one.
fn connective_starts(lex: &Lexicon, words: &[String]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < words.len() {
        match lex.chemistry_speech.connective_at(words, index) {
            Some((_, used)) if used > 0 => {
                if index > 0 {
                    out.push(index);
                }
                index += used;
            }
            _ => index += 1,
        }
    }
    out
}

/// One repair: the word as heard, and the word it was repaired to.
pub type VowelSwap = (String, String);

/// The utterance with up to [`MAX_REPAIRED_WORDS`] single-vowel repairs, and
/// the swaps that were made. `None` where nothing could be repaired.
fn repair_words(lex: &Lexicon, words: &[String]) -> Option<(Vec<String>, Vec<VowelSwap>)> {
    let mut out = words.to_vec();
    let mut swaps = Vec::new();
    for (index, word) in words.iter().enumerate() {
        if swaps.len() >= MAX_REPAIRED_WORDS {
            break;
        }
        if word.chars().count() < MIN_REPAIR_WORD_CHARS || known_word(lex, word) {
            continue;
        }
        if let Some(repaired) = repair_one(lex, word) {
            swaps.push((word.clone(), repaired.clone()));
            out[index] = repaired;
        }
    }
    (!swaps.is_empty()).then_some((out, swaps))
}

/// Whether the lexicon already reads this word as chemistry. A word it knows
/// is never "repaired": the repair exists to reach the lexicon, not to move
/// around inside it.
fn known_word(lex: &Lexicon, word: &str) -> bool {
    lex.element(word).is_some()
        || lex.anion(word).is_some()
        || lex
            .substances
            .iter()
            .any(|item| item.names.iter().any(|name| name == word))
}

/// One vowel swapped, at a position that is not the first character, for the
/// first repair that lands on a word the lexicon knows.
fn repair_one(lex: &Lexicon, word: &str) -> Option<String> {
    if word.chars().take(MAX_REPAIR_WORD_CHARS + 1).count() > MAX_REPAIR_WORD_CHARS {
        return None;
    }
    let chars: Vec<char> = word.chars().collect();
    for (from, to) in REPAIR_VOWELS {
        for position in 1..chars.len() {
            if chars[position] != from {
                continue;
            }
            let mut attempt = chars.clone();
            attempt[position] = to;
            let attempt: String = attempt.into_iter().collect();
            if known_word(lex, &attempt) {
                return Some(attempt);
            }
        }
    }
    None
}

fn codes(warnings: &[crate::ast::Warning]) -> Vec<String> {
    warnings
        .iter()
        .map(|warning| warning.code.clone())
        .collect()
}

fn clamp_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

/// Counts that describe a structure. Pure, and bounded by the tree it walks.
fn describe(node: &Node) -> Features {
    let mut features = Features {
        structurally_valid: crate::validate::semantic_warnings(node)
            .iter()
            .all(|warning| !warning.code.starts_with("math.")),
        balanced: balance_of(node),
        ..Features::default()
    };
    walk(node, 1, &mut features);
    features
}

fn balance_of(node: &Node) -> Option<bool> {
    fn has_equation(node: &Node) -> bool {
        match node {
            Node::Chemical(Chemical::Equation(_)) => true,
            Node::Document(children) => children.iter().any(has_equation),
            _ => false,
        }
    }
    match node {
        Node::Chemical(Chemical::Equation(eq)) => {
            Some(crate::validate::side_atoms(&eq.left)? == crate::validate::side_atoms(&eq.right)?)
        }
        Node::Document(children) => {
            let mut found = false;
            let mut unknown = false;
            for child in children.iter().filter(|child| has_equation(child)) {
                found = true;
                match balance_of(child) {
                    Some(false) => return Some(false),
                    Some(true) => {}
                    None => unknown = true,
                }
            }
            (found && !unknown).then_some(true)
        }
        _ => None,
    }
}

fn walk(node: &Node, depth: u16, features: &mut Features) {
    features.ast_nodes = features.ast_nodes.saturating_add(1);
    features.ast_depth = features.ast_depth.max(depth);
    match node {
        Node::Document(children) => {
            for child in children {
                walk(child, depth + 1, features);
            }
        }
        Node::Text(_) => features.text_nodes = features.text_nodes.saturating_add(1),
        Node::Chemical(chemical) => {
            let species: Vec<_> = match chemical {
                Chemical::Species(species) => vec![species],
                Chemical::Equation(equation) => {
                    equation.left.iter().chain(equation.right.iter()).collect()
                }
            };
            for item in species {
                features.atoms = features.atoms.saturating_add(
                    item.formula
                        .atom_counts()
                        .map(|counts| {
                            counts.values().fold(0u16, |total, count| {
                                total.saturating_add(u16::try_from(*count).unwrap_or(u16::MAX))
                            })
                        })
                        .unwrap_or(u16::MAX),
                );
            }
        }
        Node::Math(math) => walk_math(math, depth, features),
    }
}

fn walk_math(math: &Math, depth: u16, features: &mut Features) {
    features.ast_depth = features.ast_depth.max(depth);
    // The count is of *nodes*, and a math tree is where most of them live.
    // Walking it through the serialized form would be shorter and would also
    // make the number depend on the serializer, so the shapes are listed.
    let mut children: Vec<&Math> = Vec::new();
    match math {
        Math::Number(_) | Math::Symbol(_) | Math::Infinity | Math::Ellipsis | Math::Unit(_) => {}
        Math::Delta(inner)
        | Math::Vector(inner)
        | Math::UnaryMinus(inner)
        | Math::Abs(inner)
        | Math::Factorial(inner)
        | Math::Group { inner, .. }
        | Math::Function { arg: inner, .. } => children.push(inner),
        Math::Binary { left, right, .. } => {
            children.push(left);
            children.push(right);
        }
        Math::Fraction { num, den } => {
            children.push(num);
            children.push(den);
        }
        Math::Power { base, exp } => {
            children.push(base);
            children.push(exp);
        }
        Math::Subscript { base, sub } => {
            children.push(base);
            children.push(sub);
        }
        Math::Root { index, radicand } => {
            children.extend(index.as_deref());
            children.push(radicand);
        }
        Math::Juxt(items) => children.extend(items.iter()),
        Math::Apply { name, args } => {
            children.push(name);
            children.extend(args.iter());
        }
        Math::Sum {
            var,
            from,
            to,
            body,
        }
        | Math::Product {
            var,
            from,
            to,
            body,
        } => {
            children.extend(
                [var, from, to, body]
                    .into_iter()
                    .filter_map(Option::as_deref),
            );
        }
        Math::Integral {
            from,
            to,
            integrand,
            wrt,
        } => {
            children.extend(
                [from, to, integrand, wrt]
                    .into_iter()
                    .filter_map(Option::as_deref),
            );
        }
        Math::Derivative { expr, .. } => children.push(expr),
        Math::Limit {
            variable,
            target,
            body,
            ..
        } => {
            children.push(variable);
            children.push(target);
            children.push(body);
        }
    }
    for child in children {
        features.ast_nodes = features.ast_nodes.saturating_add(1);
        walk_math(child, depth + 1, features);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lattice(text: &str) -> Lattice {
        build(text, LatticeOptions::default())
    }

    fn shows(lattice: &Lattice, expected: &str) -> bool {
        lattice.candidates.iter().any(|candidate| {
            candidate
                .reading
                .ast()
                .is_some_and(|node| crate::render(node, crate::Renderer::Unicode) == expected)
        })
    }

    #[test]
    fn raw_is_always_present_and_always_first() {
        for text in ["вода", "предел терпения", "", "икс плюс один"] {
            let lattice = lattice(text);
            assert!(
                lattice.candidates.first().is_some_and(Candidate::is_raw),
                "«{text}» lost RAW: {:?}",
                lattice.candidates
            );
        }
    }

    #[test]
    fn ordinary_speech_offers_nothing_but_raw() {
        for text in [
            "предел терпения не резиновый",
            "порядок величины имеет значение",
            "реакция была бурной",
            "интегральная оценка курса",
            "разговор между коллегами закончился ничем",
        ] {
            let lattice = lattice(text);
            assert_eq!(
                lattice.distinct_asts, 0,
                "«{text}» produced {:?}",
                lattice.candidates
            );
        }
    }

    #[test]
    fn generation_is_deterministic() {
        let text = "реакция идёт между натрием и хлором с образованием хлорида натрия";
        let describe = |lattice: &Lattice| {
            lattice
                .candidates
                .iter()
                .map(|candidate| {
                    (
                        key_of(&candidate.reading),
                        candidate
                            .origins
                            .iter()
                            .map(Origin::as_str)
                            .collect::<Vec<_>>(),
                        candidate.rank,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(describe(&lattice(text)), describe(&lattice(text)));
    }

    #[test]
    fn the_reaction_frames_all_reach_the_equation() {
        for text in [
            "реакция идёт между натрием и хлором с образованием хлорида натрия",
            "натрий взаимодействует с хлором, в результате получается хлорид натрия",
            "с образованием хлорида натрия идёт реакция между натрием и хлором",
        ] {
            let lattice = lattice(text);
            assert!(
                shows(&lattice, "Na + Cl₂ → NaCl"),
                "«{text}» offered {:?}",
                lattice
                    .candidates
                    .iter()
                    .filter_map(|c| c.reading.ast())
                    .map(|n| crate::render(n, crate::Renderer::Unicode))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn a_bridge_word_does_not_hide_a_reaction() {
        let lattice = lattice(
            "если нагреть карбонат кальция, он разлагается на оксид кальция и углекислый газ",
        );
        assert!(
            shows(&lattice, "CaCO₃ → CaO + CO₂"),
            "{:?}",
            lattice
                .candidates
                .iter()
                .filter_map(|c| c.reading.ast())
                .map(|n| crate::render(n, crate::Renderer::Unicode))
                .collect::<Vec<_>>()
        );
        let dropped = lattice
            .candidates
            .iter()
            .flat_map(|candidate| candidate.origins.iter())
            .find_map(|origin| match origin {
                Origin::BridgeWordsDropped(words) => Some(words.clone()),
                _ => None,
            });
        assert_eq!(
            dropped,
            Some(vec!["он".to_string()]),
            "the candidate must name the word it removed"
        );
    }

    #[test]
    fn the_ambiguous_radical_offers_both_readings() {
        let lattice = lattice("корень из икс плюс один");
        assert!(shows(&lattice, "√x + 1"), "narrow reading missing");
        assert!(shows(&lattice, "√(x + 1)"), "wide reading missing");
        assert!(lattice.distinct_asts >= 2);
    }

    #[test]
    fn an_explicit_boundary_leaves_nothing_to_choose() {
        let lattice = lattice("начало корня икс плюс один конец корня");
        assert_eq!(
            lattice.distinct_asts, 1,
            "an unambiguous phrase gained a second reading: {:?}",
            lattice.candidates
        );
    }

    #[test]
    fn a_vowel_repair_names_the_words_it_changed() {
        let lattice = lattice("карбанат кальция");
        assert!(shows(&lattice, "CaCO₃"), "{:?}", lattice.candidates);
        let swaps = lattice
            .candidates
            .iter()
            .flat_map(|candidate| candidate.origins.iter())
            .find_map(|origin| match origin {
                Origin::VowelRepair(swaps) => Some(swaps.clone()),
                _ => None,
            });
        assert_eq!(
            swaps,
            Some(vec![("карбанат".to_string(), "карбонат".to_string())])
        );
    }

    #[test]
    fn a_word_the_rule_cannot_reach_is_left_alone() {
        // One vowel swap cannot turn this into anything the lexicon knows,
        // and nothing here is allowed to guess further.
        let lattice = lattice("гырдксд жлз тр");
        assert_eq!(lattice.distinct_asts, 0, "{:?}", lattice.candidates);
    }

    #[test]
    fn a_candidate_that_edited_the_words_says_so() {
        let lattice = lattice("карбанат кальция");
        let edited: Vec<bool> = lattice
            .candidates
            .iter()
            .map(Candidate::edits_the_words)
            .collect();
        assert!(edited.iter().any(|flag| *flag));
        assert!(
            !lattice.candidates[0].edits_the_words(),
            "RAW never edits anything"
        );
    }

    #[test]
    fn a_partial_or_edited_reading_never_claims_the_whole_utterance() {
        for text in [
            // prose with a substance in it: the mixed document and the
            // maximal span both produce structure, and neither of them is
            // "the speaker dictated a formula"
            "медь, о которой я говорил, лежит в шкафу",
            "я взял пробу оксида",
            "гидроксид железа три — «ключевой» реагент",
            // reached only by editing the words
            "карбанат кальция",
            "если нагреть карбонат кальция, он разлагается на оксид кальция и углекислый газ",
        ] {
            let lattice = lattice(text);
            for candidate in &lattice.candidates {
                if candidate.is_raw() {
                    continue;
                }
                assert!(
                    !candidate.whole_utterance || !candidate.edits_the_words(),
                    "«{text}»: {:?} claims the whole utterance after editing it",
                    candidate.origins
                );
            }
        }
        // and the prose cases have no whole-utterance structural reading at
        // all, which is what keeps them behind RAW downstream
        for text in [
            "медь, о которой я говорил, лежит в шкафу",
            "я взял пробу оксида",
        ] {
            let lattice = lattice(text);
            assert!(
                lattice
                    .candidates
                    .iter()
                    .filter(|candidate| !candidate.is_raw())
                    .all(|candidate| !candidate.whole_utterance),
                "«{text}» produced a whole-utterance formula: {:?}",
                lattice.candidates
            );
        }
    }

    #[test]
    fn a_verbal_tic_does_not_hide_a_reaction() {
        let lattice = lattice(
            "ну значит карбонат кальция, вот, разлагается на оксид кальция и углекислый газ",
        );
        assert!(
            shows(&lattice, "CaCO₃ → CaO + CO₂"),
            "{:?}",
            lattice
                .candidates
                .iter()
                .filter_map(|c| c.reading.ast())
                .map(|n| crate::render(n, crate::Renderer::Unicode))
                .collect::<Vec<_>>()
        );
        let dropped = lattice
            .candidates
            .iter()
            .flat_map(|candidate| candidate.origins.iter())
            .find_map(|origin| match origin {
                Origin::BridgeWordsDropped(words) => Some(words.clone()),
                _ => None,
            })
            .expect("the filler candidate exists");
        assert!(dropped.contains(&"вот".to_string()), "{dropped:?}");
    }

    #[test]
    fn every_limit_holds_on_a_hostile_input() {
        // Long, deeply connective-laden, and full of repairable-looking
        // words. The lattice must come back bounded rather than large.
        let long = std::iter::repeat_n("карбанат кальция разлагается на оксид кальция и", 60)
            .collect::<Vec<_>>()
            .join(" ");
        let lattice = lattice(&long);
        assert!(lattice.candidates.len() <= MAX_CANDIDATES);
        assert!(lattice.attempts <= MAX_ATTEMPTS);
        assert!(lattice.candidates.first().is_some_and(Candidate::is_raw));
    }

    #[test]
    fn a_single_oversized_token_never_reaches_parsing_or_repair() {
        let text = "а".repeat(MAX_INPUT_BYTES);
        let result = lattice(&text);
        assert!(result.truncated);
        assert_eq!(result.attempts, 0);
        assert_eq!(result.candidates.len(), 1);
        assert!(result.candidates[0].is_raw());
        assert_eq!(result.candidates[0].normalized, text);
        assert!(repair_one(Lexicon::builtin(), &"а".repeat(MAX_REPAIR_WORD_CHARS + 1)).is_none());
    }

    #[test]
    fn document_atom_balance_checks_every_equation() {
        let balanced = interpret("вода превращается в воду", InterpretOptions::default()).ast;
        let unbalanced = interpret("вода превращается в кислород", InterpretOptions::default()).ast;
        assert_eq!(balance_of(&balanced), Some(true));
        assert_eq!(balance_of(&unbalanced), Some(false));
        assert_eq!(
            balance_of(&Node::Document(vec![balanced, unbalanced])),
            Some(false)
        );
    }

    #[test]
    fn an_utterance_past_the_word_limit_is_kept_verbatim() {
        let long = std::iter::repeat_n("икс плюс", MAX_WORDS)
            .collect::<Vec<_>>()
            .join(" ");
        let lattice = lattice(&long);
        assert_eq!(lattice.candidates.len(), 1);
        assert!(lattice.candidates[0].is_raw());
        assert!(lattice.truncated, "the bound must be reported, not hidden");
    }

    #[test]
    fn unicode_punctuation_does_not_break_the_span() {
        // A mathematical construction, not a substance name: a name mentioned
        // in prose is no longer substituted, and what this test is about is
        // dashes and guillemets not moving a byte range.
        let text = "корень из икс — «ключевой» множитель";
        let lattice = lattice(text);
        assert!(shows(&lattice, "√x"), "{:?}", lattice.candidates);
        for candidate in &lattice.candidates {
            // A span that is not on a character boundary would slice to "".
            assert!(
                candidate.span.end <= text.len(),
                "span out of range: {:?}",
                candidate.span
            );
            assert!(
                text.is_char_boundary(candidate.span.start)
                    && text.is_char_boundary(candidate.span.end),
                "span cuts a character: {:?}",
                candidate.span
            );
        }
    }

    #[test]
    fn features_count_the_structure_not_the_words() {
        let lattice = lattice("вода");
        let water = lattice
            .candidates
            .iter()
            .find(|candidate| !candidate.is_raw())
            .expect("«вода» has a structural reading");
        assert_eq!(water.features.atoms, 3, "two H atoms and one O atom");
        assert_eq!(water.features.text_nodes, 0);
        assert!(water.features.structurally_valid);
        assert_eq!(water.features.balanced, None, "a species has no balance");
    }

    #[test]
    fn a_dictated_reaction_reports_whether_it_balances() {
        let lattice =
            lattice("натрий взаимодействует с хлором, в результате получается хлорид натрия");
        let equation = lattice
            .candidates
            .iter()
            .find(|candidate| {
                matches!(
                    candidate.reading.ast(),
                    Some(Node::Chemical(Chemical::Equation(_)))
                )
            })
            .expect("the equation is in the lattice");
        assert_eq!(
            equation.features.balanced,
            Some(false),
            "the speaker dictated no coefficients, and the feature must say so"
        );
    }

    #[test]
    fn one_reading_reached_twice_is_one_candidate_with_two_reasons() {
        let lattice = lattice("вода");
        let water = lattice
            .candidates
            .iter()
            .find(|candidate| !candidate.is_raw())
            .expect("«вода» has a structural reading");
        assert!(
            water.origins.len() >= 2,
            "automatic routing and the forced chemistry pass both reach it: {:?}",
            water.origins
        );
        let keys: Vec<_> = lattice
            .candidates
            .iter()
            .filter_map(|candidate| key_of(&candidate.reading))
            .collect();
        let mut unique = keys.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(keys.len(), unique.len(), "a reading appears twice");
    }
}

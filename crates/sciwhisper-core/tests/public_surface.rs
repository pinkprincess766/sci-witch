//! The stable surface of the compiler, pinned by signature.
//!
//! `docs/development/COMPILER_CONTRACT_RU.md` promises that breaking these is
//! a release defect. A promise in a document is checked by nobody; this file
//! is checked by the compiler. Each function is assigned to a `fn` pointer of
//! its exact type, so a renamed item, a moved item, an extra parameter or a
//! different return type all stop this file from building — before any test
//! runs.
//!
//! What is *not* here is deliberately not stable: the parser, the lexicon,
//! normalization, number reading, the nomenclature helpers. They are `pub`
//! because the workspace's own crates reach into them, not because a caller
//! outside it may.
//!
//! The shape of the AST itself is pinned elsewhere, and more strictly: every
//! gold answer in every corpus under `research/data` is a serialized tree,
//! so a changed field name or variant fails the benchmark, not just a build.

use sciwhisper_core::ast::{Equation, Warning};
use sciwhisper_core::lattice::{self, Features, Span};
use sciwhisper_core::{
    balance_equation, choose_hypothesis, interpret, interpret_utterance, render, render_result,
    word_insert_xml, Candidate, Choice, ChoiceReason, Decision, Domain, InterpretOptions,
    InterpretationResult, Lattice, LatticeOptions, Node, Origin, Reading, Renderer, UtteranceMode,
    UtteranceOptions, UtteranceResult, MAX_HYPOTHESIS_EDITS,
};

#[test]
fn the_entry_points_keep_their_signatures() {
    let _: fn(&str, InterpretOptions) -> InterpretationResult = interpret;
    let _: fn(&str, UtteranceOptions) -> UtteranceResult = interpret_utterance;
    let _: fn(&Node, Renderer) -> String = render;
    let _: fn(&InterpretationResult, Renderer) -> String = render_result;
    let _: fn(&Node) -> String = word_insert_xml;
    let _: fn(&Equation) -> Option<Vec<u32>> = balance_equation;
    let _: fn(&str, LatticeOptions) -> Lattice = lattice::build;
    let _: fn(&[&str]) -> Choice = choose_hypothesis;
    let _: fn(&Node) -> Vec<Warning> = sciwhisper_core::validate::semantic_warnings;
}

#[test]
fn the_types_a_caller_matches_on_are_still_there() {
    // Constructed or named, not just imported, so a variant that disappears
    // is a build failure here and not a surprise downstream.
    let _ = [
        Domain::Auto,
        Domain::Chemistry,
        Domain::Mathematics,
        Domain::Physics,
        Domain::Plain,
    ];
    let _ = [Renderer::Unicode, Renderer::Latex, Renderer::Omml];
    let _ = [UtteranceMode::MixedText, UtteranceMode::ScientificOnly];
    let _ = [
        Decision::Accepted,
        Decision::Ambiguous,
        Decision::Rejected,
        Decision::Raw,
    ];
    let _ = [
        ChoiceReason::Empty,
        ChoiceReason::TopAlreadyParsed,
        ChoiceReason::NearHypothesisParsed,
        ChoiceReason::NothingNearParsed,
        ChoiceReason::NearHypothesesDisagree,
        ChoiceReason::NoScientificAnchor,
    ];
    let raw = Reading::Raw;
    assert!(raw.is_raw());
    let _: fn(&Candidate) -> bool = Candidate::edits_the_words;
    let _: fn(&Origin) -> &'static str = Origin::as_str;
    let _: fn(&str) -> Span = Span::whole;
    let _ = Features::default();
}

#[test]
fn the_documented_bounds_are_the_ones_in_the_code() {
    // The contract quotes these numbers. If one moves, the contract has to
    // move with it, and this is where that becomes impossible to forget.
    assert_eq!(MAX_HYPOTHESIS_EDITS, 2);
    assert_eq!(sciwhisper_core::nbest::MAX_HYPOTHESES, 16);
    assert_eq!(lattice::MAX_CANDIDATES, 32);
    assert_eq!(lattice::MAX_INPUT_BYTES, 16 * 1024);
    assert_eq!(sciwhisper_core::utterance::MAX_UTTERANCE_WORDS, 400);
    assert_eq!(sciwhisper_core::utterance::MAX_CORRECTIONS, 4);
    assert_eq!(sciwhisper_core::utterance::MIN_ENUMERATION_SPANS, 2);
    // Not re-exported at the crate root; same path as `MAX_FUNCTION_ARGS`.
    assert_eq!(sciwhisper_core::parser::math::MAX_PARSE_DEPTH, 64);
}

//! SciWhisper core: spoken scientific notation → AST → Unicode / LaTeX / OMML.
//!
//! # What is stable
//!
//! This crate is the compiler, and its contract is
//! `docs/development/COMPILER_CONTRACT_RU.md`. Two tiers, stated here because
//! Rust's visibility cannot say it on its own — every `pub mod` below is
//! reachable from outside:
//!
//! * **Stable**: everything re-exported at the root of this crate; the
//!   [`ast`] module, which *is* the output format; the [`lattice`] and
//!   [`nbest`] modules; [`validate::semantic_warnings`]. Signatures are
//!   pinned by `tests/public_surface.rs`, and the serialized shape of the
//!   AST by every gold answer in `research/data`.
//! * **Internal**: everything else — the parser, the lexicon, normalization,
//!   number reading, the nomenclature helpers. They are `pub` because the
//!   workspace's own crates use them, and they change without notice.

pub mod ast;
pub mod balance;
pub mod coordination;
pub mod dimension;
pub mod error;
pub mod formula;
pub mod interpret;
pub mod lattice;
pub mod lexicon;
pub mod nbest;
pub mod normalize;
pub mod numbers;
pub mod organic;
pub mod parser;
pub mod render;
pub mod units;
pub mod utterance;
pub mod validate;

pub use ast::{Domain, InterpretationResult, Node, Renderer, Species};
pub use balance::balance_equation;
pub use error::{Error, Result};
pub use interpret::{interpret, render_result, InterpretOptions};
pub use lattice::{Candidate, Lattice, LatticeOptions, Origin, Reading};
pub use nbest::{choose_hypothesis, Choice, ChoiceReason, MAX_HYPOTHESIS_EDITS};
pub use render::{render, word_insert_xml};
pub use utterance::{
    interpret_utterance, Decision, UtteranceMode, UtteranceOptions, UtteranceResult,
};

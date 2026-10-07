//! Independent Earley recogniser for the stage-3A spoken-mathematics grammar.
//!
//! The product parser is the recursive descent in `sciwhisper-core`. This
//! crate is a research oracle: it reads `docs/grammar/math.ebnf` and says
//! whether a sequence of token-class names belongs to that language. It is
//! not on the product path and has no runtime dependencies.
//!
//! Semantic constraints written in prose in the EBNF (comma skipping,
//! juxtaposition of weak letters, derivative-order agreement, and the rest
//! of the «семантические ограничения» section) are not checked. Special
//! sequences `? … ?` are not checked either. The recogniser therefore
//! accepts a superset of what the handwritten parser accepts.
//!
//! [`count_parses`] counts derivation trees of an accepted token string
//! from the Earley chart. It is the ambiguity measure for the grammar
//! report. It does not build a packed forest and it is not consulted by
//! the product parser.

pub mod earley;
pub mod ebnf;

pub use earley::{
    count_parses, recognise, CountOutcome, ParseCount, Recognition, MAX_ITEMS, MAX_PARSES,
    MAX_TOKENS,
};
pub use ebnf::{parse, Expr, Grammar, Rule};

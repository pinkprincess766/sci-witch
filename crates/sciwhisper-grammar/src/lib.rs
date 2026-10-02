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

pub mod earley;
pub mod ebnf;

pub use earley::{recognise, Recognition, MAX_ITEMS, MAX_TOKENS};
pub use ebnf::{parse, Expr, Grammar, Rule};

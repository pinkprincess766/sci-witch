//! SciWhisper core: spoken scientific notation → AST → Unicode / LaTeX / OMML.

pub mod ast;
pub mod balance;
pub mod dimension;
pub mod error;
pub mod formula;
pub mod interpret;
pub mod lexicon;
pub mod normalize;
pub mod numbers;
pub mod parser;
pub mod render;
pub mod utterance;
pub mod validate;

pub use ast::{Domain, InterpretationResult, Node, Renderer, Species};
pub use balance::balance_equation;
pub use error::{Error, Result};
pub use interpret::{interpret, render_result, InterpretOptions};
pub use render::{render, word_insert_xml};
pub use utterance::{
    interpret_utterance, Decision, UtteranceMode, UtteranceOptions, UtteranceResult,
};

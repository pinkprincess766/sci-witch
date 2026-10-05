pub mod chemistry;
pub mod math;

use crate::ast::{Domain, Node};
use crate::error::Result;
use crate::lexicon::Lexicon;
use crate::numbers::NumberLex;

use self::math::MathMode;

/// One domain's reading, including the warnings and alternatives that the
/// same `parse_math` call produced. A second call would rebuild them.
pub struct DomainParse {
    pub node: Node,
    pub warnings: Vec<String>,
    pub alternatives: Vec<Node>,
}

pub fn parse_domain(
    words: &[String],
    domain: Domain,
    lex: &Lexicon,
    nums: &NumberLex,
) -> Result<DomainParse> {
    match domain {
        Domain::Chemistry => bare(chemistry::parse_chemistry(words, lex, nums)?),
        Domain::Mathematics => from_math(words, lex, nums, MathMode::Math),
        Domain::Physics => from_math(words, lex, nums, MathMode::Physics),
        Domain::Plain => bare(Node::Text(words.join(" "))),
        Domain::Auto => unreachable!("auto must be resolved before parse_domain"),
    }
}

fn bare(node: Node) -> Result<DomainParse> {
    Ok(DomainParse {
        node,
        warnings: Vec::new(),
        alternatives: Vec::new(),
    })
}

fn from_math(
    words: &[String],
    lex: &Lexicon,
    nums: &NumberLex,
    mode: MathMode,
) -> Result<DomainParse> {
    let parsed = math::parse_math(words, lex, nums, mode)?;
    Ok(DomainParse {
        node: Node::Math(parsed.ast),
        warnings: parsed.warnings,
        alternatives: parsed.alternatives.into_iter().map(Node::Math).collect(),
    })
}

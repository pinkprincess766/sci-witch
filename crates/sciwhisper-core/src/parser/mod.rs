pub mod chemistry;
pub mod math;

use crate::ast::{Domain, Node};
use crate::error::Result;
use crate::lexicon::Lexicon;
use crate::numbers::NumberLex;

use self::math::MathMode;

pub fn parse_domain(
    words: &[String],
    domain: Domain,
    lex: &Lexicon,
    nums: &NumberLex,
) -> Result<Node> {
    match domain {
        Domain::Chemistry => chemistry::parse_chemistry(words, lex, nums),
        Domain::Mathematics => math::parse_math_node(words, lex, nums, MathMode::Math),
        Domain::Physics => math::parse_math_node(words, lex, nums, MathMode::Physics),
        Domain::Plain => Ok(Node::Text(words.join(" "))),
        Domain::Auto => unreachable!("auto must be resolved before parse_domain"),
    }
}

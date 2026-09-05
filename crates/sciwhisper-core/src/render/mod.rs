pub mod latex;
pub mod omml;
pub mod unicode;

use crate::ast::{Math, Node, Renderer};

pub fn render(node: &Node, renderer: Renderer) -> String {
    match renderer {
        Renderer::Unicode => unicode::render(node),
        Renderer::Latex => latex::render(node),
        Renderer::Omml => omml::render(node),
    }
}

pub fn word_insert_xml(node: &Node) -> String {
    omml::word_insert_xml(node)
}

/// Whether a derivative operand has to be bracketed. Shared by all three
/// renderers so that `d(x²)/dx` never becomes `dx²/dx` in one of them and
/// not the others: only a simple atom may sit next to the `d` unbracketed.
pub(crate) fn derivative_operand_needs_group(operand: &Math) -> bool {
    !matches!(
        operand,
        Math::Number(_)
            | Math::Symbol(_)
            | Math::Group { .. }
            | Math::Subscript { .. }
            | Math::Vector(_)
            | Math::Delta(_)
            | Math::Abs(_)
            | Math::Root { .. }
            | Math::Unit(_)
            | Math::Infinity
            | Math::Ellipsis
    )
}

pub mod latex;
pub mod omml;
pub mod unicode;

use crate::ast::{Node, Renderer};

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

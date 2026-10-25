//! Grammars the code editors highlight with beyond what gpui-kit ships
//! (the desktop's; a page has none), after zedis's `helpers/syntax.rs`:
//! registered by name with gpui-component's `LanguageRegistry`, so an
//! editor made with that language name picks them up.
//!
//! GraphQL (the GraphQL body's query): gpui-component knows the grammar's
//! name but has no highlight query for it, so its tokens would show
//! uncoloured. `graphql_highlights.scm` maps them onto the captures its
//! theme colours.

use gpui_kit::component::highlighter::{GrammarConfig, LanguageRegistry};

const GRAPHQL_HIGHLIGHTS: &str = include_str!("graphql_highlights.scm");

/// Registers every extra grammar; again replaces them, harmlessly.
pub fn register_languages() {
    let config = GrammarConfig::new(
        "graphql",
        tree_sitter::Language::new(tree_sitter_graphql::LANGUAGE),
        Vec::new(),
        GRAPHQL_HIGHLIGHTS,
        "",
        "",
    );
    LanguageRegistry::singleton().register("graphql", &config);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A query with a node or a capture the grammar does not have fails to
    /// compile, and the editor then shows the query uncoloured.
    #[test]
    fn the_graphql_highlights_compile_against_the_grammar() {
        let language = tree_sitter::Language::new(tree_sitter_graphql::LANGUAGE);
        tree_sitter::Query::new(&language, GRAPHQL_HIGHLIGHTS).expect("the highlights compile");
    }
}

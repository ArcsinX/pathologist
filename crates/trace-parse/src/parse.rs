use std::cell::RefCell;
use tree_sitter::{Node, Parser, Tree};

thread_local! {
    static PARSER: RefCell<Parser> = RefCell::new({
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .expect("failed to set C language");
        parser
    });
}

pub struct ParseResult {
    pub tree: Tree,
    pub source: String,
}

pub fn parse_c_source(source: &str) -> Result<ParseResult, String> {
    PARSER.with(|p| {
        let mut parser = p.borrow_mut();
        let tree = parser
            .parse(source, None)
            .ok_or_else(|| "tree-sitter returned no tree".to_string())?;
        Ok(ParseResult {
            tree,
            source: source.to_string(),
        })
    })
}

pub fn node_text<'a>(source: &'a str, node: &Node) -> &'a str {
    &source[node.start_byte()..node.end_byte()]
}

pub fn has_parse_errors(tree: &Tree) -> bool {
    tree.root_node().has_error()
}

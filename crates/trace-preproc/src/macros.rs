use crate::{Token, TokenKind};
use indexmap::IndexMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub enum MacroDef {
    Object {
        replacement: Vec<Token>,
    },
    Function {
        params: Vec<String>,
        replacement: Vec<Token>,
        variadic: bool,
    },
}

pub type MacroTable = IndexMap<String, MacroDef>;
pub type SharedMacroTable = Arc<RwLock<MacroTable>>;

pub fn new_shared_macro_table() -> SharedMacroTable {
    Arc::new(RwLock::new(MacroTable::new()))
}

pub fn macro_table_from_defines(defines: &indexmap::IndexMap<String, String>) -> MacroTable {
    use crate::Lexer;
    let mut table = MacroTable::new();
    for (name, val) in defines {
        let tokens = Lexer::new(val).tokenize();
        let filtered: Vec<Token> = tokens
            .into_iter()
            .filter(|t| !matches!(t.kind, TokenKind::Eof))
            .collect();
        table.insert(
            name.clone(),
            MacroDef::Object {
                replacement: filtered,
            },
        );
    }
    table
}

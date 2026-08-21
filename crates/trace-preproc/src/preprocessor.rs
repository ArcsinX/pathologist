use crate::macros::{MacroDef, MacroTable};
use crate::{Diagnostic, DiagnosticSeverity, Lexer, LineMap, PreprocessOptions, Token, TokenKind};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PreprocessError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{message}")]
    Message { message: String },
}

#[derive(Debug, Clone)]
pub struct PreprocessResult {
    pub output: String,
    pub line_map: LineMap,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug)]
struct PreprocessorState {
    opts: PreprocessOptions,
    macros: MacroTable,
    include_stack: Vec<PathBuf>,
    included_guard: HashSet<PathBuf>,
    conditional_stack: Vec<bool>,
    output: String,
    line_map: LineMap,
    diagnostics: Vec<Diagnostic>,
    current_file: PathBuf,
    current_line: u32,
}

impl PreprocessorState {
    fn new(opts: PreprocessOptions, file: PathBuf) -> Self {
        let mut state = Self {
            opts,
            macros: MacroTable::new(),
            include_stack: vec![file.clone()],
            included_guard: HashSet::new(),
            conditional_stack: vec![true],
            output: String::new(),
            line_map: LineMap::new(),
            diagnostics: Vec::new(),
            current_file: file,
            current_line: 1,
        };
        if let Some(shared) = &state.opts.shared_macros {
            if let Ok(guard) = shared.read() {
                state.macros = guard.clone();
            }
        } else {
            state.init_predefined_macros();
        }
        state
    }

    fn init_predefined_macros(&mut self) {
        if self.opts.shared_macros.is_some() {
            return;
        }
        let defines: Vec<_> = self
            .opts
            .defines
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (name, val) in defines {
            let tokens = Lexer::new(&val).tokenize();
            let filtered: Vec<Token> = tokens
                .into_iter()
                .filter(|t| !matches!(t.kind, TokenKind::Eof))
                .collect();
            self.insert_macro(
                name,
                MacroDef::Object {
                    replacement: filtered,
                },
            );
        }
    }

    fn insert_macro(&mut self, name: String, def: MacroDef) {
        self.macros.insert(name.clone(), def.clone());
        if self.opts.accumulate_macros {
            if let Some(shared) = &self.opts.shared_macros {
                if let Ok(mut guard) = shared.write() {
                    guard.insert(name, def);
                }
            }
        }
    }

    fn remove_macro(&mut self, name: &str) {
        self.macros.shift_remove(name);
        if self.opts.accumulate_macros {
            if let Some(shared) = &self.opts.shared_macros {
                if let Ok(mut guard) = shared.write() {
                    guard.shift_remove(name);
                }
            }
        }
    }

    fn is_active(&self) -> bool {
        self.conditional_stack.iter().all(|&b| b)
    }

    fn emit_token(&mut self, tok: &Token) {
        if matches!(tok.kind, TokenKind::Eof) {
            return;
        }
        if !matches!(tok.kind, TokenKind::Newline)
            && needs_leading_space(&self.output, &tok.kind)
        {
            self.output.push(' ');
        }
        let offset = self.output.len();
        let text = token_to_string(&tok.kind);
        self.output.push_str(&text);
        if self.opts.track_line_map {
            self.line_map
                .push(offset, self.current_file.clone(), tok.line, tok.col);
        }
        if matches!(tok.kind, TokenKind::Newline) {
            self.current_line += 1;
        }
    }

    fn emit_str(&mut self, s: &str, line: u32, col: u32) {
        let offset = self.output.len();
        self.output.push_str(s);
        if self.opts.track_line_map {
            self.line_map
                .push(offset, self.current_file.clone(), line, col);
        }
    }

    fn warn(&mut self, line: u32, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Warning,
            file: Some(self.current_file.clone()),
            line,
            message: message.into(),
        });
    }

    fn error(&mut self, line: u32, message: impl Into<String>) -> PreprocessError {
        let msg = message.into();
        self.diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            file: Some(self.current_file.clone()),
            line,
            message: msg.clone(),
        });
        PreprocessError::Message { message: msg }
    }

    fn process_file(&mut self, path: &Path) -> Result<(), PreprocessError> {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if self.included_guard.contains(&canonical) {
            return Ok(());
        }

        if let Some(cache) = &self.opts.include_expansion_cache {
            let cached = cache
                .read()
                .ok()
                .and_then(|guard| guard.get(&canonical).cloned());
            if let Some(entry) = cached {
                self.emit_str(&entry.text, 1, 1);
                self.included_guard.extend(entry.files.iter().cloned());
                return Ok(());
            }
        }

        let cache_header = self.opts.include_expansion_cache.is_some()
            && canonical
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("h"));

        let guard_snapshot = if cache_header {
            self.included_guard.clone()
        } else {
            HashSet::new()
        };
        self.included_guard.insert(canonical.clone());
        let output_start = self.output.len();

        let content = if let Some(cache) = &self.opts.source_cache {
            let key = canonical.clone();
            if let Some(s) = cache.get(&key) {
                s.clone()
            } else {
                fs::read_to_string(path).map_err(|source| PreprocessError::Io {
                    path: path.to_path_buf(),
                    source,
                })?
            }
        } else {
            fs::read_to_string(path).map_err(|source| PreprocessError::Io {
                path: path.to_path_buf(),
                source,
            })?
        };

        let prev_file = self.current_file.clone();
        self.current_file = path.to_path_buf();
        self.include_stack.push(path.to_path_buf());

        let tokens = Lexer::new(&content).tokenize();
        if let Err(e) = self.process_tokens(&tokens) {
            self.warn(1, format!("preprocess stopped: {e}"));
        }

        self.include_stack.pop();
        self.current_file = prev_file;

        if cache_header {
            if let Some(cache) = &self.opts.include_expansion_cache {
                let text: Arc<str> = self.output[output_start..].into();
                if !text.is_empty() {
                    let new_files: HashSet<PathBuf> = self
                        .included_guard
                        .difference(&guard_snapshot)
                        .cloned()
                        .collect();
                    if let Ok(mut guard) = cache.write() {
                        guard.entry(canonical).or_insert(crate::IncludeExpansion {
                            text,
                            files: Arc::new(new_files),
                        });
                    }
                }
            }
        }

        Ok(())
    }

    fn process_tokens(&mut self, tokens: &[Token]) -> Result<(), PreprocessError> {
        let mut i = 0;
        while i < tokens.len() {
            let tok = &tokens[i];
            if matches!(tok.kind, TokenKind::Eof) {
                break;
            }

            if matches!(tok.kind, TokenKind::Hash) {
                if at_beginning_of_line(tokens, i) {
                    i = self.handle_directive(tokens, i)?;
                    continue;
                }
                if let Some(TokenKind::Identifier(name)) = tokens.get(i + 1).map(|t| &t.kind) {
                    self.emit_str(
                        &format!("\"{name}\""),
                        tokens[i + 1].line,
                        tokens[i + 1].col,
                    );
                    i += 2;
                    continue;
                }
            }

            if self.is_active() {
                if let TokenKind::Identifier(name) = &tok.kind {
                    if name == "__FILE__" {
                        self.emit_str(
                            &format!("\"{}\"", self.current_file.display()),
                            tok.line,
                            tok.col,
                        );
                        i += 1;
                        continue;
                    }
                    if name == "__LINE__" {
                        self.emit_str(&tok.line.to_string(), tok.line, tok.col);
                        i += 1;
                        continue;
                    }
                    if let Some(macro_def) = self.macros.get(name).cloned() {
                        match macro_def {
                            MacroDef::Function {
                                params,
                                replacement,
                                variadic,
                            } => {
                                if self.next_non_newline_is(tokens, i + 1, "(") {
                                    i += 1;
                                    let args = self.parse_macro_args(tokens, &mut i)?;
                                    let expanded = apply_concatenation(substitute_macro(
                                        &replacement,
                                        &params,
                                        &args,
                                        variadic,
                                    ));
                                    self.process_tokens(&expanded)?;
                                    continue;
                                }
                                self.emit_token(tok);
                            }
                            MacroDef::Object { replacement } => {
                                self.expand_tokens_no_directives(&replacement)?;
                                i += 1;
                                continue;
                            }
                        }
                    } else {
                        self.emit_token(tok);
                    }
                } else {
                    self.emit_token(tok);
                }
            }
            i += 1;
        }
        Ok(())
    }

    /// Expand macro replacement tokens: no `#` directives; `#x` stringizes; recurse into object macros.
    fn expand_tokens_no_directives(&mut self, tokens: &[Token]) -> Result<(), PreprocessError> {
        let mut i = 0;
        while i < tokens.len() {
            let tok = &tokens[i];
            if matches!(tok.kind, TokenKind::Eof) {
                break;
            }
            if matches!(tok.kind, TokenKind::Hash) {
                if let Some(TokenKind::Identifier(name)) = tokens.get(i + 1).map(|t| &t.kind) {
                    self.emit_str(
                        &format!("\"{name}\""),
                        tokens[i + 1].line,
                        tokens[i + 1].col,
                    );
                    i += 2;
                    continue;
                }
                self.emit_token(tok);
                i += 1;
                continue;
            }
            if self.is_active() {
                if let TokenKind::Identifier(name) = &tok.kind {
                    if let Some(MacroDef::Object { replacement }) = self.macros.get(name).cloned() {
                        self.expand_tokens_no_directives(&replacement)?;
                        i += 1;
                        continue;
                    }
                }
                self.emit_token(tok);
            }
            i += 1;
        }
        Ok(())
    }

    fn handle_directive(
        &mut self,
        tokens: &[Token],
        start: usize,
    ) -> Result<usize, PreprocessError> {
        let mut i = start + 1;
        // skip to directive name (may be on next line)
        while i < tokens.len() && matches!(tokens[i].kind, TokenKind::Newline) {
            i += 1;
        }
        if i >= tokens.len() {
            return Ok(i);
        }

        let directive = match &tokens[i].kind {
            TokenKind::Identifier(s) => s.clone(),
            _ => {
                return Err(self.error(tokens[i].line, "expected directive name after #"));
            }
        };
        i += 1;

        match directive.as_str() {
            "include" if self.is_active() => {
                i = self.handle_include(tokens, i)?;
            }
            "define" if self.is_active() => {
                i = self.handle_define(tokens, i)?;
            }
            "include" | "define" if !self.is_active() => {}
            "ifdef" => {
                let name = self.read_directive_ident(tokens, &mut i)?;
                let defined = self.macros.contains_key(&name);
                self.conditional_stack.push(self.is_active() && defined);
            }
            "ifndef" => {
                let name = self.read_directive_ident(tokens, &mut i)?;
                let defined = self.macros.contains_key(&name);
                self.conditional_stack.push(self.is_active() && !defined);
            }
            "if" => {
                let cond = self.expand_and_eval_condition(tokens, &mut i);
                self.conditional_stack.push(self.is_active() && cond);
            }
            "elif" => {
                if self.conditional_stack.len() <= 1 {
                    return Err(self.error(tokens[i.saturating_sub(1)].line, "#elif without #if"));
                }
                let parent_active = self.conditional_stack[..self.conditional_stack.len() - 1]
                    .iter()
                    .all(|&b| b);
                let current = self.conditional_stack.pop().unwrap();
                if !parent_active || current {
                    self.conditional_stack.push(false);
                } else {
                    let cond = self.expand_and_eval_condition(tokens, &mut i);
                    self.conditional_stack.push(parent_active && cond);
                }
            }
            "else" => {
                if self.conditional_stack.len() <= 1 {
                    return Err(self.error(tokens[i.saturating_sub(1)].line, "#else without #if"));
                }
                let parent_active = self.conditional_stack[..self.conditional_stack.len() - 1]
                    .iter()
                    .all(|&b| b);
                let current = self.conditional_stack.pop().unwrap();
                self.conditional_stack.push(parent_active && !current);
            }
            "endif" => {
                if self.conditional_stack.len() <= 1 {
                    return Err(self.error(tokens[i.saturating_sub(1)].line, "#endif without #if"));
                }
                self.conditional_stack.pop();
            }
            "line" => {
                // #line N "file" — update location tracking
                i = self.skip_to_newline(tokens, i);
            }
            "undef" if self.is_active() => {
                let name = self.read_directive_ident(tokens, &mut i)?;
                self.remove_macro(&name);
            }
            "undef" if !self.is_active() => {}
            _ => {
                self.warn(
                    tokens[i.saturating_sub(1)].line,
                    format!("unknown directive #{directive}"),
                );
                i = self.skip_to_newline(tokens, i);
            }
        }
        i = self.skip_to_newline(tokens, i);
        Ok(i)
    }

    fn handle_include(&mut self, tokens: &[Token], mut i: usize) -> Result<usize, PreprocessError> {
        while i < tokens.len() && matches!(tokens[i].kind, TokenKind::Newline) {
            i += 1;
        }
        let path = match &tokens.get(i).map(|t| &t.kind) {
            Some(TokenKind::String(s)) => s.clone(),
            Some(TokenKind::Punct(s)) if s == "<" => {
                let mut header = String::new();
                i += 1;
                while i < tokens.len() {
                    match &tokens[i].kind {
                        TokenKind::Identifier(s) | TokenKind::Punct(s) if s != ">" => {
                            header.push_str(s);
                        }
                        TokenKind::Punct(s) if s == ">" => break,
                        _ => break,
                    }
                    i += 1;
                }
                header
            }
            _ => {
                return Err(self.error(
                    tokens.get(i).map(|t| t.line).unwrap_or(1),
                    "expected string or <...> after #include",
                ));
            }
        };

        let include_path = match self.resolve_include(&path) {
            Ok(p) => p,
            Err(_) => {
                self.warn(
                    tokens.get(i).map(|t| t.line).unwrap_or(1),
                    format!("include file not found, skipping: {path}"),
                );
                return Ok(i + 1);
            }
        };
        if let Err(e) = self.process_file(&include_path) {
            self.warn(
                tokens.get(i).map(|t| t.line).unwrap_or(1),
                format!("include preprocessing failed for {path}: {e}"),
            );
        }
        Ok(i + 1)
    }

    fn resolve_include(&self, path: &str) -> Result<PathBuf, PreprocessError> {
        let candidate = if path.starts_with('/') || path.contains('\\') {
            PathBuf::from(path)
        } else {
            self.current_file
                .parent()
                .unwrap_or(Path::new("."))
                .join(path)
        };
        if candidate.exists() {
            return Ok(candidate);
        }
        for inc in &self.opts.include_paths {
            let p = inc.join(path);
            if p.is_file() {
                return Ok(p);
            }
        }
        if let Some(index) = &self.opts.basename_index {
            if let Some(name) = Path::new(path).file_name().and_then(|n| n.to_str()) {
                if let Some(matches) = index.get(name) {
                    if matches.len() == 1 {
                        return Ok(matches[0].clone());
                    }
                }
            }
        }
        Err(PreprocessError::Message {
            message: format!("include file not found: {path}"),
        })
    }

    fn handle_define(&mut self, tokens: &[Token], mut i: usize) -> Result<usize, PreprocessError> {
        while i < tokens.len() && matches!(tokens[i].kind, TokenKind::Newline) {
            i += 1;
        }
        let name = self.read_directive_ident(tokens, &mut i)?;
        let paren_start = i;
        if matches!(tokens.get(i).map(|t| &t.kind), Some(TokenKind::Punct(s)) if s == "(")
            && self.looks_like_function_macro_params(tokens, i + 1)
        {
            i += 1;
            let (params, variadic) = self.parse_macro_param_list(tokens, &mut i)?;
            let mut replacement = Vec::new();
            while i < tokens.len() && !matches!(tokens[i].kind, TokenKind::Newline) {
                if matches!(&tokens[i].kind, TokenKind::Punct(s) if s == "\\")
                    && i + 1 < tokens.len()
                    && matches!(tokens[i + 1].kind, TokenKind::Newline)
                {
                    i += 2;
                    continue;
                }
                replacement.push(tokens[i].clone());
                i += 1;
            }
            self.insert_macro(
                name,
                MacroDef::Function {
                    params,
                    replacement,
                    variadic,
                },
            );
            return Ok(i);
        }

        if matches!(tokens.get(i).map(|t| &t.kind), Some(TokenKind::Punct(s)) if s == "(") {
            // `#define NAME (...)` — object macro, replacement starts with `(`.
            i = paren_start;
        }

        let mut replacement = Vec::new();
        while i < tokens.len() && !matches!(tokens[i].kind, TokenKind::Newline) {
            if matches!(&tokens[i].kind, TokenKind::Punct(s) if s == "\\")
                && i + 1 < tokens.len()
                && matches!(tokens[i + 1].kind, TokenKind::Newline)
            {
                i += 2;
                continue;
            }
            replacement.push(tokens[i].clone());
            i += 1;
        }
        self.insert_macro(name, MacroDef::Object { replacement });
        Ok(i)
    }

    fn looks_like_function_macro_params(&self, tokens: &[Token], mut i: usize) -> bool {
        while i < tokens.len() && matches!(tokens[i].kind, TokenKind::Newline) {
            i += 1;
        }
        match tokens.get(i).map(|t| &t.kind) {
            Some(TokenKind::Identifier(_)) => true,
            Some(TokenKind::Punct(s)) if s == ")" => true,
            Some(TokenKind::Punct(s)) if s == "..." => true,
            _ => false,
        }
    }

    fn parse_macro_param_list(
        &mut self,
        tokens: &[Token],
        i: &mut usize,
    ) -> Result<(Vec<String>, bool), PreprocessError> {
        let mut params = Vec::new();
        let mut variadic = false;
        loop {
            while *i < tokens.len() && matches!(tokens[*i].kind, TokenKind::Newline) {
                *i += 1;
            }
            if *i >= tokens.len() {
                return Err(self.error(1, "unterminated macro parameter list"));
            }
            if matches!(&tokens[*i].kind, TokenKind::Punct(s) if s == ")") {
                *i += 1;
                break;
            }
            if self.token_is_ellipsis(tokens, *i) {
                variadic = true;
                *i = self.skip_ellipsis(tokens, *i);
                while *i < tokens.len() && matches!(tokens[*i].kind, TokenKind::Newline) {
                    *i += 1;
                }
                if matches!(&tokens[*i].kind, TokenKind::Punct(s) if s == ")") {
                    *i += 1;
                }
                break;
            }
            let param = self.read_directive_ident(tokens, i)?;
            params.push(param);
            if self.token_is_ellipsis(tokens, *i) {
                variadic = true;
                *i = self.skip_ellipsis(tokens, *i);
                while *i < tokens.len() && matches!(tokens[*i].kind, TokenKind::Newline) {
                    *i += 1;
                }
                if matches!(&tokens[*i].kind, TokenKind::Punct(s) if s == ")") {
                    *i += 1;
                }
                break;
            }
            while *i < tokens.len() && matches!(tokens[*i].kind, TokenKind::Newline) {
                *i += 1;
            }
            if matches!(&tokens[*i].kind, TokenKind::Punct(s) if s == ")") {
                *i += 1;
                break;
            }
            if matches!(&tokens[*i].kind, TokenKind::Punct(s) if s == ",") {
                *i += 1;
                continue;
            }
            return Err(self.error(tokens[*i].line, "expected , or ) in macro parameters"));
        }
        Ok((params, variadic))
    }

    fn token_is_ellipsis(&self, tokens: &[Token], i: usize) -> bool {
        matches!(&tokens.get(i).map(|t| &t.kind), Some(TokenKind::Punct(s)) if s == "...")
            || (matches!(&tokens.get(i).map(|t| &t.kind), Some(TokenKind::Punct(s)) if s == ".")
                && matches!(&tokens.get(i + 1).map(|t| &t.kind), Some(TokenKind::Punct(s)) if s == ".")
                && matches!(&tokens.get(i + 2).map(|t| &t.kind), Some(TokenKind::Punct(s)) if s == "."))
    }

    fn skip_ellipsis(&self, tokens: &[Token], i: usize) -> usize {
        if matches!(&tokens.get(i).map(|t| &t.kind), Some(TokenKind::Punct(s)) if s == "...") {
            i + 1
        } else {
            i + 3
        }
    }

    fn next_non_newline_is(&self, tokens: &[Token], mut i: usize, punct: &str) -> bool {
        while i < tokens.len() && matches!(tokens[i].kind, TokenKind::Newline) {
            i += 1;
        }
        matches!(
            tokens.get(i).map(|t| &t.kind),
            Some(TokenKind::Punct(s)) if s == punct
        )
    }

    fn parse_macro_args(
        &mut self,
        tokens: &[Token],
        i: &mut usize,
    ) -> Result<Vec<Vec<Token>>, PreprocessError> {
        while *i < tokens.len() && matches!(tokens[*i].kind, TokenKind::Newline) {
            *i += 1;
        }
        if !matches!(tokens.get(*i).map(|t| &t.kind), Some(TokenKind::Punct(s)) if s == "(") {
            return Ok(Vec::new());
        }
        *i += 1;
        let mut args: Vec<Vec<Token>> = Vec::new();
        let mut current: Vec<Token> = Vec::new();
        let mut depth = 0u32;
        while *i < tokens.len() {
            if matches!(&tokens[*i].kind, TokenKind::Punct(s) if s == "\\")
                && *i + 1 < tokens.len()
                && matches!(tokens[*i + 1].kind, TokenKind::Newline)
            {
                *i += 2;
                continue;
            }
            match &tokens[*i].kind {
                TokenKind::Punct(s) if s == "(" => {
                    depth += 1;
                    current.push(tokens[*i].clone());
                    *i += 1;
                }
                TokenKind::Punct(s) if s == ")" && depth == 0 => {
                    args.push(current);
                    *i += 1;
                    break;
                }
                TokenKind::Punct(s) if s == ")" => {
                    depth -= 1;
                    current.push(tokens[*i].clone());
                    *i += 1;
                }
                TokenKind::Punct(s) if s == "," && depth == 0 => {
                    args.push(current);
                    current = Vec::new();
                    *i += 1;
                }
                TokenKind::Eof => {
                    return Err(self.error(tokens[*i].line, "unterminated macro argument list"));
                }
                _ => {
                    current.push(tokens[*i].clone());
                    *i += 1;
                }
            }
        }
        Ok(args)
    }

    fn read_directive_ident(
        &mut self,
        tokens: &[Token],
        i: &mut usize,
    ) -> Result<String, PreprocessError> {
        while *i < tokens.len() && matches!(tokens[*i].kind, TokenKind::Newline) {
            *i += 1;
        }
        match tokens.get(*i).map(|t| &t.kind) {
            Some(TokenKind::Identifier(s)) => {
                let name = s.clone();
                *i += 1;
                Ok(name)
            }
            _ => Err(self.error(
                tokens.get(*i).map(|t| t.line).unwrap_or(1),
                "expected identifier in directive",
            )),
        }
    }

    fn expand_and_eval_condition(&self, tokens: &[Token], i: &mut usize) -> bool {
        let mut expanded = String::new();
        while *i < tokens.len() && !matches!(tokens[*i].kind, TokenKind::Newline) {
            if !expanded.is_empty() {
                expanded.push(' ');
            }
            match &tokens[*i].kind {
                TokenKind::Identifier(name) => {
                    if let Some(MacroDef::Object { replacement }) = self.macros.get(name) {
                        for rt in replacement {
                            if !expanded.is_empty()
                                && !expanded.ends_with(' ')
                                && !matches!(rt.kind, TokenKind::Newline)
                            {
                                expanded.push(' ');
                            }
                            expanded.push_str(&token_to_string(&rt.kind));
                        }
                    } else {
                        expanded.push_str(name);
                    }
                }
                other => expanded.push_str(&token_to_string(other)),
            }
            *i += 1;
        }
        eval_pp_condition(&expanded, &self.macros)
    }

    fn skip_to_newline(&self, tokens: &[Token], mut i: usize) -> usize {
        while i < tokens.len() && !matches!(tokens[i].kind, TokenKind::Newline | TokenKind::Eof) {
            i += 1;
        }
        if i < tokens.len() && matches!(tokens[i].kind, TokenKind::Newline) {
            i += 1;
        }
        i
    }

    fn finish(self) -> PreprocessResult {
        PreprocessResult {
            output: self.output,
            line_map: self.line_map,
            diagnostics: self.diagnostics,
        }
    }
}

fn at_beginning_of_line(tokens: &[Token], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    matches!(tokens[i - 1].kind, TokenKind::Newline)
}

fn substitute_macro(
    body: &[Token],
    params: &[String],
    args: &[Vec<Token>],
    variadic: bool,
) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    let mut i = 0;
    while i < body.len() {
        if i + 1 < body.len() && matches!(&body[i].kind, TokenKind::Punct(s) if s == "##") {
            if let TokenKind::Identifier(name) = &body[i + 1].kind {
                if let Some(idx) = params.iter().position(|p| p == name) {
                    let arg = if variadic && idx + 1 == params.len() && idx < args.len() {
                        args[idx..].concat()
                    } else {
                        args.get(idx).cloned().unwrap_or_default()
                    };
                    if arg.is_empty() {
                        if let Some(last) = out.last() {
                            if matches!(&last.kind, TokenKind::Punct(s) if s == ",") {
                                out.pop();
                            }
                        }
                        i += 2;
                        continue;
                    }
                }
            }
        }
        if let TokenKind::Identifier(name) = &body[i].kind {
            if name == "__VA_ARGS__" && variadic {
                let start = params.len().saturating_sub(1);
                for (ai, arg) in args.iter().enumerate().skip(start) {
                    if ai > start {
                        out.push(Token {
                            kind: TokenKind::Punct(",".into()),
                            line: body[i].line,
                            col: body[i].col,
                        });
                    }
                    out.extend(arg.iter().cloned());
                }
                i += 1;
                continue;
            }
            if let Some(idx) = params.iter().position(|p| p == name) {
                if variadic && idx + 1 == params.len() {
                    for (ai, arg) in args.iter().enumerate().skip(idx) {
                        if ai > idx {
                            out.push(Token {
                                kind: TokenKind::Punct(",".into()),
                                line: body[i].line,
                                col: body[i].col,
                            });
                        }
                        out.extend(arg.iter().cloned());
                    }
                } else if let Some(arg) = args.get(idx) {
                    out.extend(arg.iter().cloned());
                }
                i += 1;
                continue;
            }
        }
        out.push(body[i].clone());
        i += 1;
    }
    out
}

/// Apply `##` token pasting after parameter substitution.
fn apply_concatenation(mut tokens: Vec<Token>) -> Vec<Token> {
    loop {
        let mut next = Vec::new();
        let mut changed = false;
        let mut i = 0;
        while i < tokens.len() {
            if i + 2 < tokens.len() && concat_width_at(&tokens, i + 1) > 0 {
                next.push(paste_two_tokens(&tokens[i], &tokens[i + 2]));
                i += 3;
                changed = true;
            } else if concat_width_at(&tokens, i) > 0 {
                i += concat_width_at(&tokens, i);
                changed = true;
            } else {
                next.push(tokens[i].clone());
                i += 1;
            }
        }
        if !changed {
            return next;
        }
        tokens = next;
    }
}

fn concat_width_at(tokens: &[Token], i: usize) -> usize {
    if matches!(&tokens[i].kind, TokenKind::Punct(s) if s == "##") {
        return 1;
    }
    if matches!(&tokens[i].kind, TokenKind::Hash)
        && i + 1 < tokens.len()
        && matches!(tokens[i + 1].kind, TokenKind::Hash)
    {
        return 2;
    }
    0
}

fn paste_two_tokens(left: &Token, right: &Token) -> Token {
    let text = format!(
        "{}{}",
        token_paste_fragment(&left.kind),
        token_paste_fragment(&right.kind)
    );
    Token {
        kind: TokenKind::Identifier(text),
        line: left.line,
        col: left.col,
    }
}

fn token_paste_fragment(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Identifier(s) => s.clone(),
        TokenKind::Number(s) => s.clone(),
        TokenKind::Punct(s) if s != "##" => s.clone(),
        _ => String::new(),
    }
}

fn needs_leading_space(output: &str, kind: &TokenKind) -> bool {
    if output.is_empty() {
        return false;
    }
    if output.ends_with('\n') {
        return false;
    }
    let last = output.chars().last().unwrap();
    if last == ' ' {
        return false;
    }
    match kind {
        TokenKind::Punct(s) => matches!(s.as_str(), ";" | "," | ")" | "]" | "}" | "::" | "."),
        TokenKind::Newline => false,
        _ => !matches!(last, '(' | '[' | '{' | '.' | ';'),
    }
}

fn token_to_string(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Identifier(s) => s.clone(),
        TokenKind::Number(s) => s.clone(),
        TokenKind::String(s) => format!("\"{s}\""),
        TokenKind::Char(s) => format!("'{s}'"),
        TokenKind::Punct(s) => s.clone(),
        TokenKind::Hash => "#".to_string(),
        TokenKind::Newline => "\n".to_string(),
        TokenKind::Eof => String::new(),
    }
}

fn eval_pp_condition(cond: &str, macros: &MacroTable) -> bool {
    let cond = cond.trim();
    if cond.is_empty() {
        return false;
    }
    if let Some(rest) = cond.strip_prefix('!') {
        return !eval_pp_condition(rest.trim(), macros);
    }
    if cond.starts_with("defined(") || cond.starts_with("defined (") {
        let inner = cond
            .trim_start_matches("defined")
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim();
        return macros.contains_key(inner);
    }
    if let Some((lhs, rhs)) = cond.split_once("&&") {
        return eval_pp_condition(lhs, macros) && eval_pp_condition(rhs, macros);
    }
    if let Some((lhs, rhs)) = cond.split_once("||") {
        return eval_pp_condition(lhs, macros) || eval_pp_condition(rhs, macros);
    }
    if let Some((lhs, rhs)) = cond.split_once("==") {
        return eval_pp_atom(lhs) == eval_pp_atom(rhs);
    }
    if let Some((lhs, rhs)) = cond.split_once("!=") {
        return eval_pp_atom(lhs) != eval_pp_atom(rhs);
    }
    eval_pp_atom(cond) != 0
}

fn eval_pp_atom(atom: &str) -> i64 {
    let atom = atom.trim();
    if atom == "0" || atom.eq_ignore_ascii_case("false") {
        return 0;
    }
    if atom == "1" || atom.eq_ignore_ascii_case("true") {
        return 1;
    }
    if let Ok(v) = atom.parse::<i64>() {
        return v;
    }
    if atom.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !atom.is_empty() {
        return 1;
    }
    0
}

pub fn preprocess_file(
    path: &Path,
    opts: &PreprocessOptions,
) -> Result<PreprocessResult, PreprocessError> {
    let mut state = PreprocessorState::new(opts.clone(), path.to_path_buf());
    state.process_file(path)?;
    Ok(state.finish())
}

pub fn preprocess_string(source: &str, file: &Path, opts: &PreprocessOptions) -> PreprocessResult {
    let mut state = PreprocessorState::new(opts.clone(), file.to_path_buf());
    let tokens = Lexer::new(source).tokenize();
    if let Err(e) = state.process_tokens(&tokens) {
        state.warn(1, format!("preprocess stopped: {e}"));
    }
    state.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_function_like_macro() {
        let src = "#define SQUARE(x) ((x) * (x))\nint y = SQUARE(n);\n";
        let result = preprocess_string(src, Path::new("t.c"), &PreprocessOptions::new());
        assert!(
            result.output.contains("((n") && result.output.contains("*"),
            "{}",
            result.output
        );
        assert!(!result.output.contains("SQUARE"));
    }

    #[test]
    fn expands_function_like_field_macro() {
        let src = "#define FIELD_P(o) ((o)->inner.p)\nFIELD_P(obj);\n";
        let result = preprocess_string(src, Path::new("t.c"), &PreprocessOptions::new());
        assert!(
            result.output.contains("inner") && result.output.contains("obj"),
            "{}",
            result.output
        );
        assert!(!result.output.contains("FIELD_P"));
    }

    #[test]
    fn expands_token_paste_concat() {
        let src = "#define CAT(a,b) a ## b\nint CAT(x, y);\n";
        let result = preprocess_string(src, Path::new("t.c"), &PreprocessOptions::new());
        assert!(
            result.output.contains("xy") || result.output.contains("x y"),
            "{}",
            result.output
        );
        assert!(!result.output.contains("CAT"));
    }

    #[test]
    fn expands_object_macro() {
        let opts = PreprocessOptions::new().with_define("NULL", "0");
        let result = preprocess_string("int *p = NULL;", Path::new("test.c"), &opts);
        assert!(result.output.contains("int") && result.output.contains("0"));
        assert!(!result.output.contains("NULL"));
    }

    #[test]
    fn preproc_if0_skips_define_in_dead_branch() {
        let src = "#if 0\n#define HIDDEN 42\n#endif\nint x = 1;\n";
        let result = preprocess_string(src, Path::new("t.c"), &PreprocessOptions::new());
        assert!(!result.output.contains("42"));
        assert!(result.output.contains("x = 1") || result.output.contains("int x"));
    }

    #[test]
    fn handles_ifdef() {
        let opts = PreprocessOptions::new().with_define("FEATURE", "1");
        let src = "#ifdef FEATURE\nint x;\n#else\nint y;\n#endif\n";
        let result = preprocess_string(src, Path::new("test.c"), &opts);
        assert!(result.output.contains("int x") || result.output.contains("int  x"));
        assert!(!result.output.contains("int y"));
    }

    #[test]
    fn handles_ifdef_file() {
        use std::path::PathBuf;
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/preproc/ifdef.c");
        let opts = PreprocessOptions::new().with_define("FEATURE", "1");
        let result = preprocess_file(&path, &opts).unwrap();
        assert!(
            result.output.contains("enabled") && result.output.contains("1"),
            "output was: {}",
            result.output
        );
    }

    #[test]
    fn if_else_selects_active_branch_only() {
        use std::path::PathBuf;
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/preproc/if_else.c");
        let result = preprocess_file(&path, &PreprocessOptions::new()).unwrap();
        assert!(
            result.output.contains("active"),
            "expected #if FEATURE branch, got: {}",
            result.output
        );
        assert!(
            !result.output.contains("dead"),
            "dead branch must not appear: {}",
            result.output
        );
        assert!(
            !result.output.contains("also_dead"),
            "inverse branch must not appear: {}",
            result.output
        );
        assert!(
            result.output.contains("also_active"),
            "expected #else after !FEATURE, got: {}",
            result.output
        );
    }

    #[test]
    fn if_macro_value_expands_in_condition() {
        let src = "#define OUTER 1\n#if OUTER\nint on;\n#else\nint off;\n#endif\n";
        let result = preprocess_string(src, Path::new("t.c"), &PreprocessOptions::new());
        assert!(result.output.contains("on"), "{}", result.output);
        assert!(!result.output.contains("off"), "{}", result.output);
    }

    #[test]
    fn nested_if_respects_inner_else() {
        use std::path::PathBuf;
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/preproc/nested_if.c");
        let result = preprocess_file(&path, &PreprocessOptions::new()).unwrap();
        assert!(result.output.contains("outer_on"), "{}", result.output);
        assert!(!result.output.contains("inner_on"), "{}", result.output);
        assert!(result.output.contains("inner_off"), "{}", result.output);
        assert!(!result.output.contains("outer_off"), "{}", result.output);
    }

    #[test]
    fn ifndef_and_else_inverse() {
        use std::path::PathBuf;
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/preproc/ifndef_else.c");
        let result = preprocess_file(&path, &PreprocessOptions::new()).unwrap();
        assert!(result.output.contains("guarded"), "{}", result.output);
        assert!(!result.output.contains("unguarded"), "{}", result.output);
        assert!(result.output.contains("present"), "{}", result.output);
        assert!(!result.output.contains("missing"), "{}", result.output);
    }

    #[test]
    fn object_macro_with_parenthesized_value() {
        let src = "#define START (-100)\nint x = START;\n";
        let result = preprocess_string(src, Path::new("t.c"), &PreprocessOptions::new());
        assert!(
            result.output.contains("(-100)")
                || result.output.contains("-100")
                || result.output.contains("- 100"),
            "{}",
            result.output
        );
        assert!(!result.output.contains("START"));
    }

    #[test]
    fn variadic_macro_empty_args_strips_hash_hash() {
        let src = "#define WRAP(fmt, arg...) BASE(fmt, ##arg)\nWRAP(\"x\");\n";
        let result = preprocess_string(src, Path::new("t.c"), &PreprocessOptions::new());
        assert!(
            result.output.contains("BASE") && result.output.contains("\"x\""),
            "{}",
            result.output
        );
        assert!(
            !result.output.contains(", ,"),
            "should not leave dangling comma: {}",
            result.output
        );
    }

    #[test]
    fn enum_body_define_does_not_break_preproc() {
        let src = "typedef enum {\n    A = 1,\n#define OFF (-100)\n    B = OFF,\n} E;\n";
        let result = preprocess_string(src, Path::new("t.c"), &PreprocessOptions::new());
        assert!(!result
            .diagnostics
            .iter()
            .any(|d| { d.message.contains("expected identifier in directive") }));
    }
}

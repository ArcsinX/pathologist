use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct PreprocessOptions {
    pub include_paths: Vec<PathBuf>,
    pub defines: indexmap::IndexMap<String, String>,
    /// Canonical path → raw file contents (skips disk reads during `#include` expansion).
    pub source_cache: Option<std::sync::Arc<HashMap<PathBuf, String>>>,
}

impl PreprocessOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_include(mut self, path: PathBuf) -> Self {
        self.include_paths.push(path);
        self
    }

    pub fn with_define(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.defines.insert(name.into(), value.into());
        self
    }
}

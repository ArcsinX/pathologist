use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Cached preprocessed body for a `#include`d file (shared across translation units).
#[derive(Debug, Clone)]
pub struct IncludeExpansion {
    pub text: Arc<str>,
    pub files: Arc<HashSet<PathBuf>>,
}

#[derive(Debug, Clone, Default)]
pub struct PreprocessOptions {
    pub include_paths: Vec<PathBuf>,
    pub defines: indexmap::IndexMap<String, String>,
    /// Canonical path → raw file contents (skips disk reads during `#include` expansion).
    pub source_cache: Option<std::sync::Arc<HashMap<PathBuf, String>>>,
    /// Shared cache of expanded `#include` bodies keyed by canonical path.
    pub include_expansion_cache: Option<Arc<RwLock<HashMap<PathBuf, IncludeExpansion>>>>,
    /// Basename → project paths for fast include resolution.
    pub basename_index: Option<Arc<HashMap<String, Vec<PathBuf>>>>,
    /// Shared macro table populated during header warm-up; inherited by translation units.
    pub shared_macros: Option<crate::SharedMacroTable>,
    /// When true, `#define` / `#undef` update [`Self::shared_macros`].
    pub accumulate_macros: bool,
    /// When false, skip `LineMap` updates (faster indexing; spans are not remapped yet).
    pub track_line_map: bool,
}

impl PreprocessOptions {
    pub fn new() -> Self {
        Self {
            track_line_map: true,
            ..Self::default()
        }
    }

    pub fn for_indexing(mut self) -> Self {
        self.track_line_map = false;
        self
    }

    pub fn with_include_expansion_cache(
        mut self,
        cache: Arc<RwLock<HashMap<PathBuf, IncludeExpansion>>>,
    ) -> Self {
        self.include_expansion_cache = Some(cache);
        self
    }

    pub fn with_basename_index(mut self, index: Arc<HashMap<String, Vec<PathBuf>>>) -> Self {
        self.basename_index = Some(index);
        self
    }

    pub fn with_shared_macros(mut self, table: crate::SharedMacroTable) -> Self {
        self.shared_macros = Some(table);
        self
    }

    pub fn with_accumulate_macros(mut self, accumulate: bool) -> Self {
        self.accumulate_macros = accumulate;
        self
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

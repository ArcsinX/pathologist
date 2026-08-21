use crate::deps::IncludeGraph;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use trace_preproc::{preprocess_file, PreprocessOptions};

/// Preprocessed source text for indexing (one entry per canonical file path).
#[derive(Debug, Clone, Default)]
pub struct IndexSourceCache {
    inner: Arc<RwLock<HashMap<PathBuf, Arc<str>>>>,
}

impl IndexSourceCache {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn get_or_preprocess(
        &self,
        path: &Path,
        graph: &IncludeGraph,
        eff_opts: &PreprocessOptions,
    ) -> Result<Arc<str>, String> {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if let Ok(guard) = self.inner.read() {
            if let Some(text) = guard.get(&canonical) {
                return Ok(Arc::clone(text));
            }
        }

        let text = read_index_source(path, graph, eff_opts)?;
        let arc: Arc<str> = text.into();
        if let Ok(mut guard) = self.inner.write() {
            guard
                .entry(canonical)
                .or_insert_with(|| Arc::clone(&arc));
        }
        Ok(arc)
    }
}

fn read_index_source(
    path: &Path,
    graph: &IncludeGraph,
    eff_opts: &PreprocessOptions,
) -> Result<String, String> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !should_preprocess(path, eff_opts, graph) {
        if let Some(s) = graph.source_cache.get(&canonical) {
            return Ok(s.clone());
        }
        return std::fs::read_to_string(path).map_err(|e| e.to_string());
    }
    let preproc_result = preprocess_file(path, eff_opts).map_err(|e| e.to_string())?;
    let preproc_failed = preproc_result.diagnostics.iter().any(|d| {
        matches!(d.severity, trace_preproc::DiagnosticSeverity::Error)
            || d.message.contains("preprocess stopped")
    });
    if preproc_failed {
        if let Some(s) = graph.source_cache.get(&canonical) {
            return Ok(s.clone());
        }
        std::fs::read_to_string(path).map_err(|e| e.to_string())
    } else {
        Ok(preproc_result.output)
    }
}

fn should_preprocess(path: &Path, opts: &PreprocessOptions, graph: &IncludeGraph) -> bool {
    if !opts.defines.is_empty() || !opts.include_paths.is_empty() {
        return true;
    }
    graph.needs_preprocess.contains(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn should_preprocess_uses_effective_include_paths() {
        let path = PathBuf::from("/proj/main.c");
        let mut graph = IncludeGraph {
            root: PathBuf::from("/proj"),
            ..Default::default()
        };
        graph.needs_preprocess.insert(path.clone());

        let empty = PreprocessOptions::default();
        assert!(should_preprocess(&path, &empty, &graph));

        let with_include = PreprocessOptions::default().with_include(PathBuf::from("/proj/include"));
        assert!(should_preprocess(&path, &with_include, &graph));
    }

    #[test]
    fn get_or_preprocess_falls_back_when_file_missing() {
        let cache = IndexSourceCache::new();
        let graph = IncludeGraph {
            root: PathBuf::from("/nonexistent"),
            ..Default::default()
        };
        let opts = PreprocessOptions::default();
        let missing = PathBuf::from("/nonexistent/definitely_missing_trace_file.c");
        assert!(cache
            .get_or_preprocess(&missing, &graph, &opts)
            .is_err());
    }
}

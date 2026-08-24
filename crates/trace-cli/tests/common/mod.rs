//! Shared helpers for trace integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use trace_analysis::{AnalysisResult, ResolutionKind};
use trace_ir::Program;
use trace_preproc::PreprocessOptions;

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

pub fn default_opts(root: &Path) -> PreprocessOptions {
    let include_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/include");
    PreprocessOptions::new()
        .with_include(root.to_path_buf())
        .with_include(include_dir)
}

pub fn fn_name(program: &Program, id: trace_ir::FnId) -> String {
    program.symbols.function(id).name.clone()
}

pub fn has_edge(
    program: &Program,
    analysis: &AnalysisResult,
    caller: &str,
    callee: &str,
    resolution: ResolutionKind,
) -> bool {
    analysis.call_edges.iter().any(|e| {
        fn_name(program, e.caller) == caller
            && fn_name(program, e.callee) == callee
            && e.resolution == resolution
    })
}

pub fn must_not_have_edge(
    program: &Program,
    analysis: &AnalysisResult,
    caller: &str,
    callee: &str,
) -> bool {
    !analysis
        .call_edges
        .iter()
        .any(|e| fn_name(program, e.caller) == caller && fn_name(program, e.callee) == callee)
}

pub fn callees_of(
    program: &Program,
    analysis: &AnalysisResult,
    caller: &str,
) -> Vec<(String, ResolutionKind)> {
    analysis
        .call_edges
        .iter()
        .filter(|e| fn_name(program, e.caller) == caller)
        .map(|e| (fn_name(program, e.callee), e.resolution))
        .collect()
}

pub fn export_program(
    program: &Program,
    pag: &trace_analysis::Pag,
    analysis: &AnalysisResult,
) -> PathBuf {
    export_program_with_options(program, pag, analysis, false)
}

pub fn export_program_full(
    program: &Program,
    pag: &trace_analysis::Pag,
    analysis: &AnalysisResult,
) -> PathBuf {
    export_program_with_options(program, pag, analysis, true)
}

fn export_program_with_options(
    program: &Program,
    pag: &trace_analysis::Pag,
    analysis: &AnalysisResult,
    full_detail: bool,
) -> PathBuf {
    let out = std::env::temp_dir().join(format!("trace_export_{}.db", uuid_simple()));
    let _ = std::fs::remove_file(&out);
    trace_db::export_to_sqlite(
        program,
        pag,
        analysis,
        &trace_db::ExportOptions {
            output: out.clone(),
            include_points_to: false,
            full_detail,
        },
    )
    .expect("export");
    out
}

fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    format!(
        "{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

pub fn arg_flow_count(analysis: &AnalysisResult) -> usize {
    analysis.arg_flow_edges.len()
}

pub fn has_fn_arg_flow(
    program: &Program,
    analysis: &AnalysisResult,
    caller: &str,
    callee: &str,
    arg_index: u32,
    actual_fn: &str,
) -> bool {
    let caller_id = program
        .symbols
        .functions
        .iter()
        .find(|f| f.name == caller)
        .map(|f| f.id);
    let actual_id = program
        .symbols
        .functions
        .iter()
        .find(|f| f.name == actual_fn)
        .map(|f| f.id);
    let (Some(caller_id), Some(actual_id)) = (caller_id, actual_id) else {
        return false;
    };
    let call_site_ids: std::collections::HashSet<_> = analysis
        .call_edges
        .iter()
        .filter(|e| e.caller == caller_id && fn_name(program, e.callee) == callee)
        .map(|e| e.call_site)
        .collect();
    analysis.arg_flow_edges.iter().any(|e| {
        call_site_ids.contains(&e.call_site)
            && e.arg_index == arg_index
            && e.actual_fn == Some(actual_id)
    })
}

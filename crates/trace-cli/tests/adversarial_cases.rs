//! Adversarial C patterns: macros, memcpy, casts, tables.
//! Tests document both expected behavior and known soundness gaps.

mod common;

use common::*;
use trace_analysis::{analyze, AnalysisResult, ResolutionKind};
use trace_ir::{FlowConstraint, Program};
use trace_parse::build_program;
use trace_preproc::preprocess_file;

fn has_any_edge(program: &Program, analysis: &AnalysisResult, caller: &str, callee: &str) -> bool {
    !must_not_have_edge(program, analysis, caller, callee)
}

// --- Macros (preprocessor must expand before parse) ---

#[test]
fn macro_field_access_expands_to_store_flow() {
    let root = fixture("macro_field");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    assert!(
        program.flow.iter().any(|f| {
            matches!(
                f,
                FlowConstraint::Store { .. } | FlowConstraint::GepField { .. }
            )
        }),
        "FIELD_P macro should expand to field store in macro_assign"
    );
    let (_pag, analysis) = analyze(&program);
    assert!(
        has_edge(
            &program,
            &analysis,
            "macro_user",
            "sink",
            // `sink` is prototype-only in this fixture: resolved statically,
            // but classified external because no definition exists here.
            ResolutionKind::External
        ),
        "macro_user -> sink"
    );
}

#[test]
fn nested_field_macros_emit_gep_chain() {
    let root = fixture("macro_nested_field");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let geps = program
        .flow
        .iter()
        .filter(|f| matches!(f, FlowConstraint::GepField { .. }))
        .count();
    assert!(
        geps >= 2,
        "WRAP_SLOT should expand to nested field path, got {geps} GEPs"
    );
}

#[test]
fn macro_indirect_call_resolves_target() {
    let root = fixture("macro_indirect");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(
        has_any_edge(&program, &analysis, "via_macro_indirect", "target"),
        "INVOKE(fp) should resolve to target"
    );
    assert!(
        has_edge(
            &program,
            &analysis,
            "via_macro_direct_name",
            "decoy",
            ResolutionKind::Direct
        ),
        "INVOKE(decoy) is a direct call"
    );
    assert!(
        must_not_have_edge(&program, &analysis, "via_macro_indirect", "decoy"),
        "via_macro_indirect must not reach decoy"
    );
}

#[test]
fn union_field_macro_store_flow() {
    let root = fixture("union_macro");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    assert!(
        program
            .flow
            .iter()
            .any(|f| matches!(f, FlowConstraint::Store { .. })),
        "UNION_P macro should produce store flow"
    );
}

#[test]
fn preproc_expands_field_macro_before_parse() {
    let path = fixture("macro_field").join("main.c");
    let pre = preprocess_file(&path, &default_opts(&fixture("macro_field"))).unwrap();
    assert!(
        !pre.output.contains("FIELD_P"),
        "macro must be expanded: {}",
        pre.output
    );
    assert!(
        pre.output.contains("inner") && pre.output.contains("p"),
        "expanded field access should remain: {}",
        pre.output
    );
}

/// Function-like `#define FIELD_P(o) ...` expands and produces field store flow.
#[test]
fn function_like_field_macro_produces_flow() {
    let root = fixture("macro_fnlike");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    assert!(
        program.flow.iter().any(|f| {
            matches!(
                f,
                FlowConstraint::Store { .. } | FlowConstraint::GepField { .. }
            )
        }),
        "FIELD_P(o) macro should expand to field store flow"
    );
}

// --- Comma operator and casts ---

#[test]
fn comma_operator_indirect_call() {
    let root = fixture("comma_fnptr");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(
        has_any_edge(&program, &analysis, "comma_indirect", "alpha"),
        "comma assignment then call should reach alpha"
    );
}

#[test]
fn cast_chain_preserves_indirect_target() {
    let root = fixture("cast_fnptr");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(
        has_any_edge(&program, &analysis, "cast_indirect", "through_cast"),
        "opaque void* cast round-trip should still call through_cast"
    );
}

// --- May-analysis: branch merge ---

#[test]
fn branch_merge_reports_both_callees() {
    let root = fixture("merge_branches");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(
        has_any_edge(&program, &analysis, "ambiguous_branch", "path_a"),
        "may-analysis: path_a reachable"
    );
    assert!(
        has_any_edge(&program, &analysis, "ambiguous_branch", "path_b"),
        "may-analysis: path_b reachable"
    );
}

// --- False-positive guards (soundness) ---

#[test]
fn memcpy_into_side_buffer_does_not_widen_fn_ptr() {
    let root = fixture("memcpy_false_pos");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(
        has_any_edge(&program, &analysis, "memcpy_side_buffer", "fn_a"),
        "fp still calls fn_a"
    );
    assert!(
        must_not_have_edge(&program, &analysis, "memcpy_side_buffer", "fn_b"),
        "memcpy to side buffer must not connect fp to fn_b"
    );
}

#[test]
fn comma_without_assignment_keeps_first_fn_ptr() {
    let root = fixture("comma_fnptr");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(
        has_any_edge(&program, &analysis, "comma_still_alpha", "alpha"),
        "(void)0, fp() should still call alpha"
    );
    assert!(
        must_not_have_edge(&program, &analysis, "comma_still_alpha", "beta"),
        "comma must not widen to beta"
    );
}

// --- Known limitations (document imprecision / missing libc models) ---

/// memcpy of fn-ptr bytes is not modeled; indirect call through post-memcpy fp is missed.
#[test]
fn limitation_memcpy_fnptr_indirect_unresolved() {
    let root = fixture("memcpy_fnptr");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(
        !has_any_edge(&program, &analysis, "memcpy_indirect", "real_target"),
        "memcpy-modeled fn-ptr would make this pass — update test if intentional"
    );
    assert!(
        has_any_edge(&program, &analysis, "memcpy_no_fn_edge", "real_target"),
        "plain fp() path still works"
    );
    assert!(
        must_not_have_edge(&program, &analysis, "memcpy_no_fn_edge", "ghost"),
        "memset/memcpy on blob must not invent ghost edge"
    );
}

/// memmove staging of fn-ptr — same gap as memcpy unless summarized.
#[test]
fn limitation_memmove_staged_fnptr_unresolved() {
    let root = fixture("memmove_fnptr");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(
        !has_any_edge(&program, &analysis, "memmove_indirect", "mover_target"),
        "memmove-modeled fn-ptr would make this pass — update test if intentional"
    );
}

/// Subscript on fn-ptr table: may-analysis resolves to all initializer targets.
#[test]
fn fn_ptr_table_over_approximates_all_entries() {
    let root = fixture("fn_ptr_table");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    assert!(
        program
            .flow
            .iter()
            .any(|f| matches!(f, FlowConstraint::ArrayFnMember { .. })),
        "expected ArrayFnMember for each table initializer"
    );
    let (_pag, analysis) = analyze(&program);
    assert!(
        has_any_edge(&program, &analysis, "dispatch_table", "row0"),
        "table[0]() may reach row0"
    );
    assert!(
        has_any_edge(&program, &analysis, "dispatch_table", "row1"),
        "unknown index: may also reach row1 (over-approx)"
    );
}

/// memcpy of a struct that embeds a fn-ptr — field initializer now wires setConfig.
#[test]
fn limitation_memcpy_struct_with_fnptr_unresolved() {
    let root = fixture("memcpy_struct_fn");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(
        has_any_edge(&program, &analysis, "struct_holder_memcpy", "embedded"),
        "struct fn-ptr field should resolve after designated-init store fix"
    );
}

//! Regression tests for cross-TU resolution bugs found on a real HDF-scale
//! codebase: pointer-returning prototypes shadowed by phantom variables,
//! direct calls whose definition lives in another TU, and file-`static`
//! definitions that must shadow same-name external functions.

mod common;

use common::*;
use trace_analysis::{analyze, ResolutionKind};
use trace_ir::Linkage;
use trace_parse::build_program;

/// `struct Widget *WidgetGet(void);` is declared in a header and defined in
/// another TU. Lowering used to register a *variable* named `WidgetGet` for
/// the pointer-returning prototype, turning every call into an indirect call
/// through a variable that never receives function addresses (no edge).
#[test]
fn ptr_return_prototype_resolves_direct_edge() {
    let root = fixture("ptr_return_proto");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);

    assert!(
        has_edge(
            &program,
            &analysis,
            "CheckReady",
            "WidgetGet",
            ResolutionKind::Direct
        ),
        "cross-TU call to pointer-returning function must produce a direct edge"
    );

    // The prototype must not leak into the variable table.
    assert!(
        !program
            .symbols
            .variables
            .iter()
            .any(|v| v.name == "WidgetGet"),
        "prototype registered as phantom variable"
    );
}

/// A plain call to a function defined in another TU (no fn-ptr var) must
/// still yield a Direct edge after merge, even though lowering could not see
/// the callee in the calling TU.
#[test]
fn cross_tu_direct_call_recovers_edge() {
    let root = fixture("ptr_return_proto");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);

    let edges = callees_of(&program, &analysis, "CheckReady");
    assert_eq!(edges.len(), 1, "exactly one edge from CheckReady");
    assert_eq!(edges[0].0, "WidgetGet");
    assert_eq!(edges[0].1, ResolutionKind::Direct);
}

/// Within a.c, the internal-linkage `helper` shadows b.c's external `helper`.
#[test]
fn static_definition_shadows_external_same_name() {
    let root = fixture("static_shadow");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);

    let caller_a_id = program
        .symbols
        .functions
        .iter()
        .find(|f| f.name == "caller_a")
        .expect("caller_a exists")
        .id;
    let edges_to_helper: Vec<_> = analysis
        .call_edges
        .iter()
        .filter(|e| e.caller == caller_a_id)
        .map(|e| program.symbols.function(e.callee))
        .filter(|f| f.name == "helper")
        .collect();

    assert_eq!(edges_to_helper.len(), 1, "one helper edge from caller_a");
    assert_eq!(
        edges_to_helper[0].linkage,
        Linkage::Internal,
        "caller_a must bind to its own file-static helper"
    );

    // caller_b still binds to the external helper in b.c.
    let caller_b_id = program
        .symbols
        .functions
        .iter()
        .find(|f| f.name == "caller_b")
        .expect("caller_b exists")
        .id;
    let b_edges: Vec<_> = analysis
        .call_edges
        .iter()
        .filter(|e| e.caller == caller_b_id && fn_name(&program, e.callee) == "helper")
        .collect();
    assert_eq!(b_edges.len(), 1, "one helper edge from caller_b");
    assert_eq!(
        program.symbols.function(b_edges[0].callee).linkage,
        Linkage::External,
        "caller_b binds to the external helper"
    );
}

/// Arrays of structs with fn-ptr members, initialized with nested positional
/// initializer lists (`{ { FnA }, { FnB } }`), must feed ArrayFnMember facts
/// into the table var; an element field call resolves to every listed fn.
#[test]
fn nested_positional_init_table_resolves_members() {
    let root = fixture("fn_ptr_nested_table");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);

    for callee in ["FnA", "FnB"] {
        assert!(
            has_edge(
                &program,
                &analysis,
                "CallTbl",
                callee,
                ResolutionKind::Indirect
            ),
            "tbl[i].fn call must resolve to {callee}"
        );
    }
}

/// Same table shape with designated initializers inside the nested lists
/// (`{ .name = "..", .init = Fn }`), invoked through a pointer to an element.
#[test]
fn nested_designated_init_table_resolves_members() {
    let root = fixture("fn_ptr_nested_table");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);

    for callee in ["InitNet", "InitFs"] {
        assert!(
            has_edge(
                &program,
                &analysis,
                "CallMod",
                callee,
                ResolutionKind::Indirect
            ),
            "m->init call through &g_modules[i] must resolve to {callee}"
        );
    }
}

/// `&outer.member` must yield the member subobject location (typed by the
/// member's own struct), not the flattened outer instance. A Dispatch load
/// through `dev.service` must not pick up functions stored only in other
/// fields of the outer struct (HDF RegulatorTest.TestEntry vs
/// IDeviceIoService.Dispatch shared positional index 2).
#[test]
fn member_address_of_preserves_field_identity() {
    let root = fixture("fn_ptr_nested_table");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);

    assert!(
        has_edge(
            &program,
            &analysis,
            "InvokeTest",
            "EntryFn",
            ResolutionKind::Indirect
        ),
        "inst.TestEntry call must resolve to EntryFn"
    );
    assert!(
        has_edge(
            &program,
            &analysis,
            "CoreRun",
            "RealDispatch",
            ResolutionKind::Indirect
        ),
        "dev.service Dispatch call must resolve to RealDispatch"
    );
    assert!(
        !has_edge(&program, &analysis, "CoreRun", "EntryFn", ResolutionKind::Indirect),
        "Dispatch load must not see fns stored in sibling fields of the outer struct"
    );
}

/// A `static inline` defined in a header and called from several TUs must
/// appear once, attributed to the header file (not once per including TU),
/// and its internal call sites must be deduplicated with header-origin
/// spans. Direct edges into/out of the canonical copy must survive.
#[test]
fn header_inline_calls_deduplicate_to_header_attribution() {
    let root = fixture("header_dedup");
    let program = build_program(&root, &default_opts(&root)).expect("build");

    let file_path = |program: &trace_ir::Program, id: trace_ir::FileId| -> String {
        program
            .symbols
            .files
            .iter()
            .find(|f| f.id == id)
            .map(|f| f.path.display().to_string())
            .unwrap_or_default()
    };

    let hdr_adds: Vec<_> = program
        .symbols
        .functions
        .iter()
        .filter(|f| f.name == "hdr_add")
        .collect();
    assert_eq!(hdr_adds.len(), 1, "hdr_add must collapse to one row");
    let hdr_add = hdr_adds[0];
    assert!(
        file_path(&program, hdr_add.span.file).ends_with("shared.h"),
        "hdr_add span must attribute to shared.h, got {}",
        file_path(&program, hdr_add.span.file)
    );
    assert_eq!(hdr_add.file, hdr_add.span.file);

    let helpers: Vec<_> = program
        .symbols
        .functions
        .iter()
        .filter(|f| f.name == "hdr_helper")
        .collect();
    assert_eq!(helpers.len(), 1, "hdr_helper must collapse to one row");
    let helper_id = helpers[0].id;

    // One deduplicated call site inside hdr_add, attributed to the header.
    let sites: Vec<_> = program
        .symbols
        .call_sites
        .iter()
        .filter(|cs| cs.caller == hdr_add.id && cs.callee_name == "hdr_helper")
        .collect();
    assert_eq!(sites.len(), 1, "duplicate hdr_helper call sites must merge");
    assert!(
        file_path(&program, sites[0].span.file).ends_with("shared.h"),
        "call site span must attribute to shared.h"
    );

    let (_pag, analysis) = analyze(&program);
    for (caller, callee) in [("use_a", "hdr_add"), ("use_b", "hdr_add"), ("hdr_add", "hdr_helper")] {
        assert!(
            has_edge(&program, &analysis, caller, callee, ResolutionKind::Direct),
            "{caller} -> {callee} direct edge must survive dedup"
        );
    }
    assert_eq!(helpers[0].id, helper_id);

    // TU-local functions stay distinct.
    assert_eq!(
        program
            .symbols
            .functions
            .iter()
            .filter(|f| f.name == "use_a")
            .count(),
        1
    );
}

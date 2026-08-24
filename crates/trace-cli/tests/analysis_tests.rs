mod common;

use common::*;
use trace_analysis::{analyze, ResolutionKind};
use trace_db::open_db;
use trace_parse::build_program;
use trace_preproc::PreprocessOptions;

#[test]
fn direct_call_exact_edge() {
    let root = fixture("direct_call");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(has_edge(
        &program,
        &analysis,
        "main",
        "helper",
        ResolutionKind::Direct
    ));
}

#[test]
fn false_positive_narrowed_fn_ptr() {
    let root = fixture("false_positive");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);

    assert!(
        has_edge(
            &program,
            &analysis,
            "narrowed",
            "a",
            ResolutionKind::Indirect
        ) || has_edge(&program, &analysis, "narrowed", "a", ResolutionKind::Direct),
        "expected narrowed -> a"
    );
    assert!(
        must_not_have_edge(&program, &analysis, "narrowed", "b"),
        "false positive: narrowed -> b"
    );
    assert!(
        must_not_have_edge(&program, &analysis, "narrowed", "c"),
        "false positive: narrowed -> c"
    );
}

#[test]
fn fn_ptr_init_resolves_target() {
    let root = fixture("fn_ptr_init");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);

    assert!(
        has_edge(
            &program,
            &analysis,
            "caller",
            "target",
            ResolutionKind::Indirect
        ) || has_edge(
            &program,
            &analysis,
            "caller",
            "target",
            ResolutionKind::Direct
        ),
        "caller should reach target via function pointer"
    );
    assert!(
        !program.flow.is_empty(),
        "expected flow constraints from initializer"
    );
}

#[test]
fn fn_ptr_field_assign_resolves_target() {
    let root = fixture("fn_ptr_field");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(
        has_edge(
            &program,
            &analysis,
            "caller",
            "target",
            ResolutionKind::Indirect
        ),
        "field assign then call should resolve"
    );
}

#[test]
fn fn_ptr_designated_init_resolves_target() {
    let root = fixture("fn_ptr_designated");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(
        has_edge(
            &program,
            &analysis,
            "caller",
            "target",
            ResolutionKind::Indirect
        ),
        "designated .handler = target should resolve indirect call"
    );
}

#[test]
fn fn_ptr_vtable_multi_hop_resolves_target() {
    let root = fixture("fn_ptr_vtable");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(
        has_edge(
            &program,
            &analysis,
            "dispatch",
            "target",
            ResolutionKind::Indirect
        ),
        "multi-hop interFace->handler should resolve"
    );
}

#[test]
fn camera_subdev_ops_setconfig_resolves_via_call_return() {
    let root = fixture("camera_subdev_ops");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(
        has_edge(
            &program,
            &analysis,
            "CommonDeviceSetConfig",
            "CameraCmdSensorSetConfig",
            ResolutionKind::Indirect
        ),
        "subDevOps->setConfig should resolve via GetSensorDeviceOps return"
    );
}

#[test]
fn in_out_ptr_has_store_flow() {
    let root = fixture("in_out_ptr");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    assert!(
        program
            .flow
            .iter()
            .any(|f| matches!(f, trace_ir::FlowConstraint::Store { .. })),
        "expected Store constraint from *pp = &global_x"
    );
    let (_pag, analysis) = analyze(&program);
    assert!(has_edge(
        &program,
        &analysis,
        "caller",
        "init",
        ResolutionKind::Direct
    ));
}

#[test]
fn arg_flow_pointer_param() {
    let root = fixture("arg_flow");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);

    assert!(has_edge(
        &program,
        &analysis,
        "provider",
        // `consume` is prototype-only: statically resolved, but classified
        // external because no definition exists under the fixture root.
        "consume",
        ResolutionKind::External
    ));
    assert!(
        arg_flow_count(&analysis) >= 1,
        "expected arg-flow from provider to consume"
    );
}

#[test]
fn sub_struct_field_assignment_flow() {
    let root = fixture("sub_struct");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    assert!(
        program.flow.iter().any(|f| matches!(
            f,
            trace_ir::FlowConstraint::Store { .. } | trace_ir::FlowConstraint::GepField { .. }
        )),
        "expected field/store flow from o->inner.p = v"
    );
    let (_pag, analysis) = analyze(&program);
    assert!(has_edge(
        &program,
        &analysis,
        "user",
        "assign_field",
        ResolutionKind::Direct
    ));
}

#[test]
fn multi_tu_unique_ids_and_export() {
    let root = fixture("indirect_call");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    assert!(
        program.symbols.function_ids_unique(),
        "function ids must be unique across translation units"
    );
    let (pag, analysis) = analyze(&program);
    let _ = pag;
    let db = export_program(&program, &analysis);
    let conn = open_db(&db).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM functions", [], |r| r.get(0))
        .unwrap();
    assert!(count >= 4, "expected functions from both TUs");
    let _ = std::fs::remove_file(db);
}

#[test]
fn indirect_call_via_param() {
    let root = fixture("indirect_param");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);

    let edges = callees_of(&program, &analysis, "via_param");
    assert!(
        edges.iter().any(|(name, _)| name == "callee"),
        "via_param should call callee indirectly, got {:?}",
        edges
    );
    assert!(
        !edges
            .iter()
            .any(|(name, res)| name == "cb" && *res == ResolutionKind::Direct),
        "must not treat param cb as direct function name"
    );
}

#[test]
fn indirect_call_fixture_precise() {
    let root = fixture("indirect_call");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);

    assert!(
        has_edge(
            &program,
            &analysis,
            "run",
            "target",
            ResolutionKind::Indirect
        ) || has_edge(&program, &analysis, "run", "target", ResolutionKind::Direct)
    );

    let dispatcher_edges = callees_of(&program, &analysis, "dispatcher");
    assert!(
        !dispatcher_edges
            .iter()
            .any(|(n, r)| n == "use_fn_ptr" && *r == ResolutionKind::Direct),
        "false positive dispatcher -> use_fn_ptr: {:?}",
        dispatcher_edges
    );
}

#[test]
fn preproc_if0_skips_dead_branch() {
    let path = fixture("preproc/if0.c");
    let result = trace_preproc::preprocess_file(&path, &PreprocessOptions::new()).unwrap();
    assert!(
        !result.output.contains("42"),
        "dead branch must not define or emit HIDDEN=42"
    );
    assert!(
        result.output.contains("visible = 1")
            || result.output.contains("visible =1")
            || result.output.contains("int visible")
    );
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("missing_header")),
        "must not attempt include from #if 0 branch"
    );
}

#[test]
fn export_sqlite_has_call_and_arg_tables() {
    let root = fixture("arg_flow");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (pag, analysis) = analyze(&program);
    let _ = pag;
    let db = export_program(&program, &analysis);
    let conn = open_db(&db).unwrap();
    let calls: i64 = conn
        .query_row("SELECT COUNT(*) FROM call_edges", [], |r| r.get(0))
        .unwrap();
    assert!(calls >= 1);
    let _ = std::fs::remove_file(db);
}

#[test]
fn static_direct_call_resolves() {
    let root = fixture("static_direct_call");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(has_edge(
        &program,
        &analysis,
        "caller",
        "helper",
        ResolutionKind::Direct
    ));
}

#[test]
fn fn_arg_flow_exported() {
    let root = fixture("fn_arg_flow");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(has_edge(
        &program,
        &analysis,
        "user",
        "register_cb",
        ResolutionKind::Direct
    ));
    assert!(
        has_fn_arg_flow(&program, &analysis, "user", "register_cb", 0, "handler"),
        "expected fn pointer actual handler wired to register_cb formal"
    );

    let db = export_program(&program, &analysis);
    let conn = open_db(&db).unwrap();
    let fn_flow: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM arg_flow_edges WHERE actual_fn_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        fn_flow >= 1,
        "expected function-pointer arg flow in SQLite export"
    );
    let _ = std::fs::remove_file(db);
}

#[test]
fn static_call_return_expands() {
    let root = fixture("static_call_return");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);
    assert!(has_edge(
        &program,
        &analysis,
        "user",
        "GetOps",
        ResolutionKind::Direct
    ));
    assert!(
        program
            .flow
            .iter()
            .any(|f| matches!(f, trace_ir::FlowConstraint::CallReturn { .. })),
        "expected CallReturn constraint from GetOps() assignment"
    );
}

#[test]
fn fn_static_local_variable() {
    let root = fixture("fn_static_local");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let handler = program
        .symbols
        .variables
        .iter()
        .find(|v| v.name == "handler")
        .expect("handler variable");
    assert_eq!(
        handler.storage,
        trace_ir::StorageClass::FnStatic,
        "function-local static must be FnStatic, not Local"
    );

    let (_pag, analysis) = analyze(&program);
    assert!(has_edge(
        &program,
        &analysis,
        "user",
        "target",
        ResolutionKind::Indirect
    ));

    let db = export_program_full(&program, &analysis);
    let conn = open_db(&db).unwrap();
    let kind: String = conn
        .query_row(
            "SELECT kind FROM variables WHERE name = 'handler'",
            [],
            |r| r.get(0),
        )
        .expect("handler exported in full export");
    assert_eq!(kind, "fn_static");
    let _ = std::fs::remove_file(db);
}

#[test]
fn header_inline_call_indexed_from_header_unit() {
    let root = fixture("header_inline_call");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);

    let header_only = program.symbols.functions.iter().find(|f| {
        f.name == "HeaderOnlyCaller"
            && program
                .symbols
                .files
                .get(f.file.0 as usize)
                .is_some_and(|fi| fi.path.ends_with("orphan_call.h"))
    });
    assert!(
        header_only.is_some(),
        "orphan_call.h must be indexed as its own unit (not included by any .c)"
    );
    assert!(
        has_edge(
            &program,
            &analysis,
            "HeaderOnlyCaller",
            "ExternalTarget",
            ResolutionKind::Direct
        ) || has_edge(
            &program,
            &analysis,
            "HeaderOnlyCaller",
            "ExternalTarget",
            ResolutionKind::Indirect
        ),
        "call inside header-only inline function should resolve"
    );
    assert!(
        program
            .symbols
            .files
            .iter()
            .any(|f| f.path.ends_with("helper.h")),
        "helper.h is included by main.c and must appear as an attributed origin file"
    );
}

#[test]
fn header_chain_reachable_from_c_attributed_to_headers() {
    let root = fixture("header_chain");
    let program = build_program(&root, &default_opts(&root)).expect("build");

    // Headers reachable from a .c are no longer separate indexing units,
    // but they must appear as origin files for their lowered entities.
    assert!(
        program
            .symbols
            .files
            .iter()
            .any(|f| f.path.ends_with("chain_b.h")),
        "chain_b.h must be an attributed origin file"
    );
    let b_caller = program
        .symbols
        .functions
        .iter()
        .find(|f| f.name == "BCaller")
        .expect("BCaller from chain_b.h should appear via main.c TU expansion");
    assert!(
        program
            .symbols
            .files
            .get(b_caller.span.file.0 as usize)
            .is_some_and(|fi| fi.path.ends_with("chain_b.h")),
        "BCaller should be attributed to its defining header, not the translation unit"
    );
}

#[test]
#[cfg(unix)]
fn macro_warm_preprocess_failure_is_nonfatal() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("trace_warm_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("main.c"),
        "#include \"good.h\"\nvoid main_fn(void) {}\n",
    )
    .unwrap();
    std::fs::write(root.join("good.h"), "void helper(void);\n").unwrap();
    std::fs::write(root.join("bad.h"), "void bad_helper(void);\n").unwrap();
    std::fs::write(
        root.join("also.c"),
        "#include \"bad.h\"\nvoid also_fn(void) {}\n",
    )
    .unwrap();
    let bad = root.join("bad.h");
    let mut perms = std::fs::metadata(&bad).unwrap().permissions();
    perms.set_mode(0o000);
    std::fs::set_permissions(&bad, perms).unwrap();

    let program = build_program(&root, &PreprocessOptions::new()).expect("build continues");
    let _ = std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o644));
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        program.diagnostics.iter().any(|d| {
            d.stage == "preprocess" && d.message.contains("macro warm preprocess failed")
        }),
        "expected macro warm warning for unreadable reachable header: {:?}",
        program.diagnostics
    );
    assert!(
        program
            .symbols
            .functions
            .iter()
            .any(|f| f.name == "main_fn"),
        "main.c should still be indexed after macro warm failure"
    );
}

#[test]
fn array_table_designated_init_resolves_targets() {
    let root = fixture("array_table_designated");
    let program = build_program(&root, &default_opts(&root)).expect("build");
    let (_pag, analysis) = analyze(&program);

    // Designated-init global table via helper-returned element pointer.
    assert!(
        has_edge(
            &program,
            &analysis,
            "caller_helper_ptr",
            "raw_obtain",
            ResolutionKind::Indirect
        ),
        "helper-ptr designated init: raw_obtain missing"
    );
    assert!(
        has_edge(
            &program,
            &analysis,
            "caller_helper_ptr",
            "ipc_obtain",
            ResolutionKind::Indirect
        ),
        "helper-ptr designated init: ipc_obtain missing"
    );

    // Direct subscript access on the same table.
    assert!(
        has_edge(
            &program,
            &analysis,
            "caller_direct",
            "raw_obtain",
            ResolutionKind::Indirect
        ) && has_edge(
            &program,
            &analysis,
            "caller_direct",
            "ipc_obtain",
            ResolutionKind::Indirect
        ),
        "direct subscript designated init targets missing"
    );

    // Tentative (initializer-less) array + runtime stores into elements.
    assert!(
        has_edge(
            &program,
            &analysis,
            "run",
            "impl_a",
            ResolutionKind::Indirect
        ) && has_edge(
            &program,
            &analysis,
            "run",
            "impl_b",
            ResolutionKind::Indirect
        ),
        "runtime store into tentative array element: impl_a/impl_b missing"
    );

    // Local array with designated initializers.
    assert!(
        has_edge(
            &program,
            &analysis,
            "caller_local",
            "loc_a",
            ResolutionKind::Indirect
        ) && has_edge(
            &program,
            &analysis,
            "caller_local",
            "loc_b",
            ResolutionKind::Indirect
        ),
        "local designated-init array targets missing"
    );
}

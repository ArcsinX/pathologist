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
        "consume",
        ResolutionKind::Direct
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

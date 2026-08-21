use crate::deps::IncludeGraph;
use crate::merge::{merge_unit_index, UnitIndex};
use crate::parse::{node_text, parse_c_source};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use trace_ir::{
    CallSite, Diagnostic, DiagnosticSeverity, FieldId, FlowConstraint, FnId, Function, Linkage,
    Program, ReturnFlow, Span, StorageClass, TypeDesc, VarId, Variable,
};
use trace_preproc::{preprocess_file, PreprocessOptions};
use tree_sitter::Node;

struct LowerContext {
    current_fn: Option<FnId>,
    current_file: trace_ir::FileId,
    locals: HashMap<String, VarId>,
}

fn register_local(ctx: &mut LowerContext, name: String, id: VarId) {
    if ctx.current_fn.is_some() {
        ctx.locals.insert(name, id);
    }
}

pub fn build_program(root: &Path, opts: &PreprocessOptions) -> Result<Program, String> {
    let jobs = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1);
    build_program_with_jobs(root, opts, jobs)
}

pub fn build_program_with_jobs(
    root: &Path,
    opts: &PreprocessOptions,
    jobs: usize,
) -> Result<Program, String> {
    let jobs = jobs.max(1);
    let mut program = Program::new(root.to_path_buf());
    program.include_paths = opts.include_paths.clone();
    program.defines = opts
        .defines
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let headers = normalize_discovered_paths(crate::discover::discover_header_files(root));
    let files = normalize_discovered_paths(crate::discover::discover_c_files(root));
    if files.is_empty() && headers.is_empty() {
        return Err(format!("no .c or .h files found under {}", root.display()));
    }

    let include_graph = IncludeGraph::build(root, &files, &headers);
    // Headers expanded into preprocessed `.c` TUs do not need a separate index pass.
    // Orphan headers (never `#include`d by any translation unit) carry no reachable code.
    let headers_to_index: Vec<PathBuf> = Vec::new();
    let header_order = include_graph.index_order(&headers_to_index);
    let file_order = include_graph.index_order(&files);

    let mut to_precompute: HashSet<PathBuf> = HashSet::new();
    for p in file_order.iter().chain(header_order.iter()) {
        if should_preprocess_file(p, opts, &include_graph) {
            to_precompute.insert(p.clone());
        }
    }
    let eff_opts = project_preprocess_opts(root, opts, &include_graph);
    let preprocessed_cache: Arc<HashMap<PathBuf, String>> = Arc::new(if to_precompute.is_empty() {
        HashMap::new()
    } else if jobs == 1 {
        to_precompute
            .iter()
            .filter_map(|path| {
                preprocess_file(path, &eff_opts)
                    .ok()
                    .map(|r| (path.clone(), r.output))
            })
            .collect()
    } else {
        to_precompute
            .par_iter()
            .filter_map(|path| {
                preprocess_file(path, &eff_opts)
                    .ok()
                    .map(|r| (path.clone(), r.output))
            })
            .collect()
    });

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .map_err(|e| e.to_string())?;

    if jobs == 1 {
        for path in &header_order {
            let _ = lower_header_unit(
                &mut program,
                path,
                opts,
                &include_graph,
                &preprocessed_cache,
            );
        }
        for path in &file_order {
            if let Err(e) = process_translation_unit(
                &mut program,
                path,
                opts,
                &include_graph,
                &preprocessed_cache,
            ) {
                program.add_diagnostic(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    file: None,
                    line: 0,
                    message: e,
                    stage: "parse".into(),
                });
            }
        }
    } else {
        pool.install(|| {
            let mut header_units: std::collections::HashMap<PathBuf, Result<UnitIndex, String>> =
                headers_to_index
                    .par_iter()
                    .map(|path| {
                        let unit = index_header_unit(
                            path,
                            opts,
                            root,
                            &include_graph,
                            &preprocessed_cache,
                        );
                        (path.clone(), unit)
                    })
                    .collect();
            for path in &header_order {
                if let Some(unit) = header_units.remove(path) {
                    match unit {
                        Ok(u) => merge_unit_index(&mut program, u),
                        Err(e) => program.add_diagnostic(Diagnostic {
                            severity: DiagnosticSeverity::Error,
                            file: None,
                            line: 0,
                            message: e.clone(),
                            stage: "parse".into(),
                        }),
                    }
                }
            }

            let mut units: std::collections::HashMap<PathBuf, UnitIndex> = files
                .par_iter()
                .map(|path| {
                    let unit = index_translation_unit(
                        path,
                        opts,
                        root,
                        &include_graph,
                        &preprocessed_cache,
                    );
                    (path.clone(), unit)
                })
                .collect();
            for path in &file_order {
                if let Some(unit) = units.remove(path) {
                    merge_unit_index(&mut program, unit);
                }
            }
        });
    }

    program.include_deps = include_graph.edge_list();
    for dir in &include_graph.include_dirs {
        if !program.include_paths.iter().any(|p| p == dir) {
            program.include_paths.push(dir.clone());
        }
    }

    Ok(program)
}

fn normalize_discovered_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .map(|p| p.canonicalize().unwrap_or(p))
        .collect()
}

fn should_preprocess_file(path: &Path, opts: &PreprocessOptions, graph: &IncludeGraph) -> bool {
    if !opts.defines.is_empty() || !opts.include_paths.is_empty() {
        return true;
    }
    graph.needs_preprocess.contains(path)
}

fn project_preprocess_opts(
    root: &Path,
    opts: &PreprocessOptions,
    graph: &IncludeGraph,
) -> PreprocessOptions {
    let mut eff = opts.clone();
    for dir in &graph.include_dirs {
        if !eff.include_paths.iter().any(|p| p == dir) {
            eff.include_paths.push(dir.clone());
        }
    }
    if eff.source_cache.is_none() && !graph.source_cache.is_empty() {
        eff.source_cache = Some(Arc::new(graph.source_cache.clone()));
    }
    let _ = root;
    eff
}

fn index_header_unit(
    path: &Path,
    opts: &PreprocessOptions,
    root: &Path,
    graph: &IncludeGraph,
    preprocessed: &Arc<HashMap<PathBuf, String>>,
) -> Result<UnitIndex, String> {
    let mut program = Program::new(root.to_path_buf());
    lower_header_unit(&mut program, path, opts, graph, preprocessed)?;
    Ok(program_into_unit(path.to_path_buf(), program))
}

fn index_translation_unit(
    path: &Path,
    opts: &PreprocessOptions,
    root: &Path,
    graph: &IncludeGraph,
    preprocessed: &Arc<HashMap<PathBuf, String>>,
) -> UnitIndex {
    let mut program = Program::new(root.to_path_buf());
    match process_translation_unit(&mut program, path, opts, graph, preprocessed) {
        Ok(()) => program_into_unit(path.to_path_buf(), program),
        Err(e) => UnitIndex {
            path: path.to_path_buf(),
            diagnostics: vec![Diagnostic {
                severity: DiagnosticSeverity::Error,
                file: None,
                line: 0,
                message: e,
                stage: "parse".into(),
            }],
            ..Default::default()
        },
    }
}

fn program_into_unit(path: PathBuf, program: Program) -> UnitIndex {
    UnitIndex {
        path,
        types: program.types,
        functions: program.symbols.functions,
        variables: program.symbols.variables,
        call_sites: program.symbols.call_sites,
        flow: program.flow,
        fn_returns: program.fn_returns.into_iter().collect(),
        diagnostics: program.diagnostics,
        anon_type_counter: program.anon_type_counter,
    }
}

fn read_source_file(
    path: &Path,
    root: &Path,
    opts: &PreprocessOptions,
    graph: &IncludeGraph,
    preprocessed: &Arc<HashMap<PathBuf, String>>,
) -> Result<String, String> {
    if !should_preprocess_file(path, opts, graph) {
        return fs::read_to_string(path).map_err(|e| e.to_string());
    }
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Some(out) = preprocessed.get(&canonical) {
        return Ok(out.clone());
    }
    let eff = project_preprocess_opts(root, opts, graph);
    let preproc_result = preprocess_file(path, &eff).map_err(|e| e.to_string())?;
    let preproc_failed = preproc_result.diagnostics.iter().any(|d| {
        matches!(d.severity, trace_preproc::DiagnosticSeverity::Error)
            || d.message.contains("preprocess stopped")
    });
    if preproc_failed {
        fs::read_to_string(path).map_err(|e| e.to_string())
    } else {
        Ok(preproc_result.output)
    }
}

fn lower_header_unit(
    program: &mut Program,
    path: &Path,
    opts: &PreprocessOptions,
    graph: &IncludeGraph,
    preprocessed: &Arc<HashMap<PathBuf, String>>,
) -> Result<(), String> {
    let source = read_source_file(path, &program.root, opts, graph, preprocessed)?;
    let parsed = parse_c_source(&source)?;
    let file_id = program.symbols.add_file(path.to_path_buf());
    let mut ctx = LowerContext {
        current_fn: None,
        current_file: file_id,
        locals: HashMap::new(),
    };
    lower_type_declarations(program, &mut ctx, &parsed.source, parsed.tree.root_node());
    Ok(())
}

fn lower_typedef(program: &mut Program, source: &str, node: Node) {
    if let Some(decl) = node.child_by_field_name("declarator") {
        let (alias, _) = parse_declarator_name(source, decl);
        if let Some(type_node) = node.child_by_field_name("type") {
            if type_node.kind() == "struct_specifier" || type_node.kind() == "union_specifier" {
                let tag = lower_struct_specifier(program, source, type_node);
                if !alias.is_empty() && !tag.is_empty() && alias != tag {
                    let kind = if type_node.kind() == "union_specifier" {
                        TypeDesc::Union {
                            name: tag.clone(),
                            fields: Vec::new(),
                        }
                    } else {
                        TypeDesc::Struct {
                            name: tag.clone(),
                            fields: Vec::new(),
                        }
                    };
                    program.types.intern(kind);
                }
            }
        }
    }
}

fn lower_type_declarations(
    program: &mut Program,
    ctx: &mut LowerContext,
    source: &str,
    node: Node,
) {
    match node.kind() {
        "declaration" => lower_declaration(program, ctx, source, node, None),
        "struct_specifier" | "union_specifier" => {
            lower_struct_specifier(program, source, node);
        }
        "type_definition" => lower_typedef(program, source, node),
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                lower_type_declarations(program, ctx, source, child);
            }
        }
    }
}

fn process_translation_unit(
    program: &mut Program,
    path: &Path,
    opts: &PreprocessOptions,
    graph: &IncludeGraph,
    preprocessed: &Arc<HashMap<PathBuf, String>>,
) -> Result<(), String> {
    let source = read_source_file(path, &program.root, opts, graph, preprocessed)?;
    let parsed = parse_c_source(&source)?;
    if crate::parse::has_parse_errors(&parsed.tree) {
        program.add_diagnostic(Diagnostic {
            severity: DiagnosticSeverity::Warning,
            file: None,
            line: 0,
            message: format!("parse errors in {}", path.display()),
            stage: "parse".into(),
        });
    }

    let file_id = program.symbols.add_file(path.to_path_buf());
    let mut ctx = LowerContext {
        current_fn: None,
        current_file: file_id,
        locals: HashMap::new(),
    };
    lower_tree(program, &mut ctx, &parsed.source, parsed.tree.root_node());
    Ok(())
}

fn lower_tree(program: &mut Program, ctx: &mut LowerContext, source: &str, node: Node) {
    match node.kind() {
        "function_definition" => lower_function(program, ctx, source, node),
        "declaration" => lower_declaration(program, ctx, source, node, None),
        "struct_specifier" | "union_specifier" => {
            lower_struct_specifier(program, source, node);
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                lower_tree(program, ctx, source, child);
            }
        }
    }
}

fn lower_struct_specifier(program: &mut Program, source: &str, node: Node) -> String {
    let is_union = node.kind() == "union_specifier";
    let mut name = node
        .child_by_field_name("name")
        .map(|n| node_text(source, &n).to_string())
        .unwrap_or_default();

    if name.is_empty() {
        program.anon_type_counter += 1;
        name = format!("anon_{}", program.anon_type_counter);
    }

    let mut fields = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "field_declaration" {
                if let Some((fname, field_type)) =
                    type_desc_from_field_declaration(program, source, child)
                {
                    if !fname.is_empty() {
                        fields.push((fname, field_type));
                    }
                }
            }
        }
    }

    if !fields.is_empty() {
        if is_union {
            program.types.compute_union_layout(name.clone(), fields);
        } else {
            program.types.compute_struct_layout(name.clone(), fields);
        }
    }
    name
}

fn lower_function(program: &mut Program, ctx: &mut LowerContext, source: &str, node: Node) {
    let Some(decl) = node
        .child_by_field_name("declarator")
        .or_else(|| find_function_declarator(node))
    else {
        return;
    };
    let (name, _) = parse_declarator_name(source, decl);
    if name.is_empty() {
        return;
    }
    let ret_type = node
        .child_by_field_name("type")
        .map(|t| parse_type_node(program, source, t))
        .unwrap_or_else(|| program.types.int());
    let provisional_id = program.symbols.alloc_fn_id();
    let mut params = Vec::new();
    if let Some(params_node) = find_params(decl) {
        for param in params_node.children(&mut params_node.walk()) {
            if param.kind() == "parameter_declaration" {
                if let Some(var) = lower_parameter(
                    program,
                    ctx,
                    source,
                    param,
                    provisional_id,
                    params.len() as u32,
                ) {
                    params.push(var);
                }
            }
        }
    }

    let is_static = declaration_is_static(source, node);

    let fn_id = program.symbols.add_function(Function {
        id: provisional_id,
        name: name.clone(),
        linkage: if is_static {
            Linkage::Internal
        } else {
            Linkage::External
        },
        return_type: ret_type,
        params: params.clone(),
        locals: Vec::new(),
        span: node_span(ctx, node),
        file: ctx.current_file,
        is_defined: true,
    });
    reassign_fn_id(program, provisional_id, fn_id);
    ctx.current_fn = Some(fn_id);
    ctx.locals.clear();
    for &param in &params {
        if let Some(v) = program.symbols.variable_by_id(param) {
            ctx.locals.insert(v.name.clone(), param);
        }
    }

    if let Some(body_node) = node.child_by_field_name("body") {
        walk_function_body(program, ctx, source, body_node, fn_id);
    }

    ctx.current_fn = None;
    ctx.locals.clear();
}

fn lower_parameter(
    program: &mut Program,
    ctx: &mut LowerContext,
    source: &str,
    node: Node,
    fn_id: FnId,
    index: u32,
) -> Option<VarId> {
    let decl = node.child_by_field_name("declarator")?;
    let (name, is_ptr) = parse_declarator_name(source, decl);
    if name.is_empty() {
        return None;
    }
    let base_desc = node
        .child_by_field_name("type")
        .map(|t| type_desc_from_node(program, source, t))
        .unwrap_or(TypeDesc::Int);
    let type_desc = if is_ptr {
        TypeDesc::Ptr(Box::new(base_desc))
    } else {
        base_desc
    };
    let type_id = program.types.intern(type_desc);
    let var_id = program.symbols.alloc_var_id();
    program.symbols.add_variable(Variable {
        id: var_id,
        name: name.clone(),
        type_id,
        storage: StorageClass::Param,
        fn_id: Some(fn_id),
        param_index: Some(index),
        span: node_span(ctx, node),
        is_pointer: is_ptr,
    });
    register_local(ctx, name, var_id);
    Some(var_id)
}

fn lower_declaration(
    program: &mut Program,
    ctx: &mut LowerContext,
    source: &str,
    node: Node,
    storage_override: Option<StorageClass>,
) {
    let type_node = match node.child_by_field_name("type") {
        Some(t) => t,
        None => return,
    };
    let type_id = parse_type_node(program, source, type_node);
    let is_static = declaration_is_static(source, node);

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "init_declarator" => {
                let decl = child.child_by_field_name("declarator").unwrap_or(child);
                lower_one_declarator(
                    program,
                    ctx,
                    source,
                    child,
                    decl,
                    type_id,
                    is_static,
                    storage_override,
                    child.child_by_field_name("value"),
                );
            }
            "declarator" | "pointer_declarator" | "function_declarator" => {
                lower_one_declarator(
                    program,
                    ctx,
                    source,
                    child,
                    child,
                    type_id,
                    is_static,
                    storage_override,
                    None,
                );
            }
            "identifier" => {
                let name = node_text(source, &child).to_string();
                if name.is_empty() {
                    continue;
                }
                let var_id = program.symbols.alloc_var_id();
                program.symbols.add_variable(Variable {
                    id: var_id,
                    name: name.clone(),
                    type_id,
                    storage: storage_override.unwrap_or_else(|| storage_for(ctx, is_static)),
                    fn_id: ctx.current_fn,
                    param_index: None,
                    span: node_span(ctx, child),
                    is_pointer: false,
                });
                register_local(ctx, name, var_id);
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_one_declarator(
    program: &mut Program,
    ctx: &mut LowerContext,
    source: &str,
    span_node: Node,
    decl: Node,
    type_id: trace_ir::TypeId,
    is_static: bool,
    storage_override: Option<StorageClass>,
    init_expr: Option<Node>,
) {
    if is_function_pointer_declarator(decl) {
        let (name, _is_ptr) = parse_declarator_name(source, decl);
        if name.is_empty() {
            return;
        }
        let var_id = program.symbols.alloc_var_id();
        program.symbols.add_variable(Variable {
            id: var_id,
            name: name.clone(),
            type_id,
            storage: storage_override.unwrap_or_else(|| storage_for(ctx, is_static)),
            fn_id: ctx.current_fn,
            param_index: None,
            span: node_span(ctx, span_node),
            is_pointer: true,
        });
        register_local(ctx, name, var_id);
        if let Some(init) = init_expr {
            if init.kind() == "initializer_list" {
                lower_fn_ptr_array_init(program, ctx, source, var_id, init);
            }
            extract_flow_from_expr(program, ctx, source, init, Some(var_id));
        }
        return;
    }

    if decl.kind() == "function_declarator" && !is_function_pointer_declarator(decl) {
        lower_function_decl(program, ctx, source, decl, type_id, is_static);
        return;
    }

    let (name, is_ptr) = parse_declarator_name(source, decl);
    if name.is_empty() {
        return;
    }
    let var_id = program.symbols.alloc_var_id();
    program.symbols.add_variable(Variable {
        id: var_id,
        name: name.clone(),
        type_id,
        storage: storage_override.unwrap_or_else(|| storage_for(ctx, is_static)),
        fn_id: ctx.current_fn,
        param_index: None,
        span: node_span(ctx, span_node),
        is_pointer: is_ptr,
    });
    register_local(ctx, name, var_id);
    if let Some(init) = init_expr {
        if init.kind() == "initializer_list" {
            lower_fn_ptr_array_init(program, ctx, source, var_id, init);
        }
        extract_flow_from_expr(program, ctx, source, init, Some(var_id));
    }
}

fn lower_fn_ptr_array_init(
    program: &mut Program,
    ctx: &LowerContext,
    source: &str,
    array: VarId,
    init: Node,
) {
    let mut cursor = init.walk();
    for child in init.children(&mut cursor) {
        if matches!(child.kind(), "(" | ")" | ",") {
            continue;
        }
        if let Some(callee) = resolve_call_fn_arg(program, ctx, source, child) {
            program
                .flow
                .push(FlowConstraint::ArrayFnMember { array, callee });
        }
    }
}

fn lower_function_decl(
    program: &mut Program,
    ctx: &mut LowerContext,
    source: &str,
    decl: Node,
    ret_type: trace_ir::TypeId,
    is_static: bool,
) {
    let (name, _) = parse_declarator_name(source, decl);
    if name.is_empty() {
        return;
    }
    let provisional_id = program.symbols.alloc_fn_id();
    let mut params = Vec::new();
    if let Some(params_node) = find_params(decl) {
        for param in params_node.children(&mut params_node.walk()) {
            if param.kind() == "parameter_declaration" {
                if let Some(var) = lower_parameter(
                    program,
                    ctx,
                    source,
                    param,
                    provisional_id,
                    params.len() as u32,
                ) {
                    params.push(var);
                }
            }
        }
    }
    let fn_id = program.symbols.add_function(Function {
        id: provisional_id,
        name,
        linkage: if is_static {
            Linkage::Internal
        } else {
            Linkage::External
        },
        return_type: ret_type,
        params,
        locals: Vec::new(),
        span: node_span(ctx, decl),
        file: ctx.current_file,
        is_defined: false,
    });
    reassign_fn_id(program, provisional_id, fn_id);
}

fn reassign_fn_id(program: &mut Program, from: FnId, to: FnId) {
    if from == to {
        return;
    }
    for var in &mut program.symbols.variables {
        if var.fn_id == Some(from) {
            var.fn_id = Some(to);
        }
    }
    for cs in &mut program.symbols.call_sites {
        if cs.caller == from {
            cs.caller = to;
        }
    }
    if let Some(func) = program.symbols.functions.iter_mut().find(|f| f.id == to) {
        func.params = program
            .symbols
            .variables
            .iter()
            .filter(|v| v.fn_id == Some(to) && v.storage == StorageClass::Param)
            .map(|v| v.id)
            .collect();
    }
}

fn walk_function_body(
    program: &mut Program,
    ctx: &mut LowerContext,
    source: &str,
    node: Node,
    caller: FnId,
) {
    match node.kind() {
        "declaration" => lower_declaration(program, ctx, source, node, None),
        "assignment_expression" => {
            extract_flow_from_expr(program, ctx, source, node, None);
        }
        "call_expression" => collect_call_at_node(program, ctx, source, node, caller),
        "return_statement" => collect_return_statement(program, ctx, source, node, caller),
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_function_body(program, ctx, source, child, caller);
    }
}

fn collect_call_at_node(
    program: &mut Program,
    ctx: &mut LowerContext,
    source: &str,
    node: Node,
    caller: FnId,
) {
    let func = match node.child_by_field_name("function") {
        Some(f) => f,
        None => return,
    };
    let (callee_name, mut is_direct, callee_var) =
        resolve_callee_with_loads(program, ctx, source, func);
    if !is_direct && callee_var.is_none() {
        is_direct = resolve_function_named(program, ctx, &callee_name).is_some();
    }
    if !is_direct && is_likely_macro_callee(&callee_name) {
        return;
    }
    let mut var_args = Vec::new();
    let mut fn_args = Vec::new();
    let mut arg_index = 0u32;
    if let Some(args_node) = node.child_by_field_name("arguments") {
        for arg in args_node.children(&mut args_node.walk()) {
            if arg.kind() != "(" && arg.kind() != ")" && arg.kind() != "," {
                if let Some(v) = resolve_expr_var(program, ctx, source, arg) {
                    var_args.push((arg_index, v));
                    arg_index += 1;
                } else if let Some(fn_id) = resolve_call_fn_arg(program, ctx, source, arg) {
                    fn_args.push((arg_index, fn_id));
                    arg_index += 1;
                }
            }
        }
    }
    let call_id = program.symbols.alloc_call_id();
    program.symbols.call_sites.push(CallSite {
        id: call_id,
        caller,
        callee_name,
        callee_var,
        var_args,
        fn_args,
        span: node_span(ctx, node),
        is_direct,
    });
}

fn extract_flow_from_expr(
    program: &mut Program,
    ctx: &mut LowerContext,
    source: &str,
    node: Node,
    assign_target: Option<VarId>,
) {
    if node.kind() == "assignment_expression" {
        let lhs = peel_expression(
            node.child_by_field_name("left")
                .or_else(|| node.named_child(0))
                .unwrap(),
        );
        let rhs = node
            .child_by_field_name("right")
            .or_else(|| node.named_child(1))
            .unwrap();
        if is_deref_lhs(source, lhs) {
            if let Some(arg) = deref_operand(lhs) {
                if let Some(ptr) = resolve_lvalue_var(program, ctx, source, arg) {
                    if let Some(src) = expr_to_store_src(program, ctx, source, rhs) {
                        program.flow.push(FlowConstraint::Store { dst: ptr, src });
                    } else if rhs.kind() == "call_expression" {
                        if let Some(callee_name) = resolve_direct_call(program, ctx, source, rhs) {
                            let ret_temp = alloc_ret_temp(program, ctx, node);
                            program.flow.push(FlowConstraint::CallReturn {
                                dst: ret_temp,
                                callee_name,
                            });
                            program.flow.push(FlowConstraint::Store {
                                dst: ptr,
                                src: ret_temp,
                            });
                        }
                    }
                }
            }
        } else if lhs.kind() == "field_expression" {
            emit_field_store(program, ctx, source, lhs, rhs);
        } else if let Some(dst) = resolve_lvalue_var(program, ctx, source, lhs) {
            if let Some(flow) = expr_to_rhs_flow(program, ctx, source, rhs, dst) {
                program.flow.push(flow);
            }
        }
        return;
    }

    if node.kind() == "initializer_list" {
        if let Some(base) = assign_target {
            lower_initializer_list(program, ctx, source, node, base);
            return;
        }
    }

    if let Some(dst) = assign_target {
        if let Some(flow) = expr_to_rhs_flow(program, ctx, source, node, dst) {
            program.flow.push(flow);
        }
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_flow_from_expr(program, ctx, source, child, None);
    }
}

fn lower_initializer_list(
    program: &mut Program,
    ctx: &mut LowerContext,
    source: &str,
    node: Node,
    base: VarId,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "designated_initializer" | "initializer_pair" => {
                lower_designated_initializer(program, ctx, source, child, base);
            }
            _ => {}
        }
    }
}

fn lower_designated_initializer(
    program: &mut Program,
    ctx: &mut LowerContext,
    source: &str,
    node: Node,
    base: VarId,
) {
    let mut field_names = Vec::new();
    let mut value = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "field_designator" => {
                let mut inner = child.walk();
                for c in child.children(&mut inner) {
                    if c.kind() == "field_identifier" {
                        field_names.push(node_text(source, &c).to_string());
                    }
                }
            }
            "=" => {}
            _ if value.is_none() && child.is_named() && child.kind() != "field_designator" => {
                value = Some(child)
            }
            _ => {}
        }
    }
    let Some(value_node) = value else {
        return;
    };
    let mut type_id = match struct_type_for_var(program, base) {
        Some(t) => t,
        None => return,
    };
    let mut current = base;
    for (i, fname) in field_names.iter().enumerate() {
        let Some(fid) = program.types.field_id_by_name(type_id, fname) else {
            return;
        };
        if i + 1 == field_names.len() {
            emit_field_value_store(
                program,
                ctx,
                source,
                node,
                current,
                std::slice::from_ref(&fid),
                value_node,
            );
        } else {
            current = alloc_gep_temp(program, ctx, node, current, fid);
            type_id = program.types.get(type_id).layout.fields[&fid].type_id;
        }
    }
}

fn peel_expression(mut node: Node) -> Node {
    while node.kind() == "parenthesized_expression" {
        node = node.named_child(0).unwrap_or(node);
    }
    node
}

fn emit_field_store(
    program: &mut Program,
    ctx: &mut LowerContext,
    source: &str,
    lhs: Node,
    rhs: Node,
) {
    let Some((base, field_ids)) = decompose_field_path(program, ctx, source, lhs) else {
        return;
    };
    if field_ids.is_empty() {
        return;
    }
    emit_field_value_store(program, ctx, source, lhs, base, &field_ids, rhs);
}

fn emit_field_value_store(
    program: &mut Program,
    ctx: &mut LowerContext,
    source: &str,
    span_node: Node,
    base: VarId,
    field_ids: &[FieldId],
    value_node: Node,
) {
    let mut current = base;
    for (i, fid) in field_ids.iter().enumerate() {
        if i + 1 == field_ids.len() {
            let gep = alloc_gep_temp(program, ctx, span_node, current, *fid);
            if let Some(src) = expr_to_store_src(program, ctx, source, value_node) {
                program.flow.push(FlowConstraint::Store { dst: gep, src });
            } else if value_node.kind() == "identifier" {
                let name = node_text(source, &value_node);
                if let Some(callee) = resolve_function_named(program, ctx, name) {
                    let src_temp = alloc_ret_temp(program, ctx, span_node);
                    program.flow.push(FlowConstraint::AddrOfFn {
                        dst: src_temp,
                        callee,
                    });
                    program.flow.push(FlowConstraint::Store {
                        dst: gep,
                        src: src_temp,
                    });
                }
            } else {
                let ret_temp = alloc_ret_temp(program, ctx, span_node);
                let emitted = if value_node.kind() == "call_expression" {
                    if let Some(callee_name) = resolve_direct_call(program, ctx, source, value_node)
                    {
                        program.flow.push(FlowConstraint::CallReturn {
                            dst: ret_temp,
                            callee_name,
                        });
                        true
                    } else {
                        false
                    }
                } else {
                    expr_to_rhs_flow(program, ctx, source, value_node, ret_temp)
                        .map(|flow| {
                            program.flow.push(flow);
                        })
                        .is_some()
                };
                if emitted {
                    program.flow.push(FlowConstraint::Store {
                        dst: gep,
                        src: ret_temp,
                    });
                }
            }
        } else {
            current = alloc_gep_temp(program, ctx, span_node, current, *fid);
        }
    }
}

fn alloc_gep_temp(
    program: &mut Program,
    ctx: &LowerContext,
    span_node: Node,
    base: VarId,
    field: FieldId,
) -> VarId {
    let var_id = program.symbols.alloc_var_id();
    program.symbols.add_variable(Variable {
        id: var_id,
        name: format!("_gep{}", var_id.0),
        type_id: program.types.int(),
        storage: StorageClass::Local,
        fn_id: ctx.current_fn,
        param_index: None,
        span: node_span(ctx, span_node),
        is_pointer: true,
    });
    program.flow.push(FlowConstraint::GepField {
        dst: var_id,
        base,
        field,
    });
    var_id
}

fn field_name_from_node(source: &str, node: Node) -> Option<String> {
    node.child_by_field_name("field")
        .map(|n| node_text(source, &n).to_string())
}

fn decompose_field_path(
    program: &mut Program,
    ctx: &LowerContext,
    source: &str,
    node: Node,
) -> Option<(VarId, Vec<FieldId>)> {
    let mut field_names = Vec::new();
    let mut cur = peel_expression(node);
    while cur.kind() == "field_expression" {
        field_names.push(field_name_from_node(source, cur)?);
        cur = cur.child_by_field_name("argument")?;
    }
    let base = resolve_lvalue_var(program, ctx, source, cur)?;
    field_names.reverse();

    let mut type_id = struct_type_for_var(program, base)?;
    let mut field_ids = Vec::new();
    for fname in &field_names {
        let fid = program.types.field_id_by_name(type_id, fname)?;
        field_ids.push(fid);
        let layout = program.types.get(type_id);
        type_id = layout.layout.fields.get(&fid)?.type_id;
        type_id = peel_ptr_to_struct(program, type_id);
    }
    Some((base, field_ids))
}

fn peel_ptr_to_struct(program: &mut Program, type_id: trace_ir::TypeId) -> trace_ir::TypeId {
    let inner = match &program.types.get(type_id).desc {
        TypeDesc::Ptr(inner) => Some((**inner).clone()),
        _ => None,
    };
    inner.map_or(type_id, |desc| program.types.intern(desc))
}

fn struct_type_for_var(program: &Program, var: VarId) -> Option<trace_ir::TypeId> {
    let mut type_id = variable_type_id(program, var)?;
    for _ in 0..4 {
        match &program.types.get(type_id).desc {
            TypeDesc::Ptr(inner) => {
                type_id = program.types.resolve_type_id(inner);
            }
            TypeDesc::Struct { .. } | TypeDesc::Union { .. } => return Some(type_id),
            _ => return Some(type_id),
        }
    }
    Some(type_id)
}

fn variable_type_id(program: &Program, var: VarId) -> Option<trace_ir::TypeId> {
    program.symbols.variable_by_id(var).map(|v| v.type_id)
}

fn pointer_op(source: &str, node: Node) -> Option<String> {
    if node.kind() != "pointer_expression" {
        return None;
    }
    node.child_by_field_name("operator")
        .map(|n| node_text(source, &n).to_string())
        .or_else(|| node.child(0).map(|n| node_text(source, &n).to_string()))
}

fn pointer_arg(node: Node) -> Option<Node> {
    if node.kind() != "pointer_expression" {
        return None;
    }
    node.child_by_field_name("argument")
        .or_else(|| node.named_child(0))
}

fn is_deref_lhs(source: &str, node: Node) -> bool {
    pointer_op(source, node).as_deref() == Some("*")
}

fn deref_operand(node: Node) -> Option<Node> {
    pointer_arg(node)
}

fn expr_to_store_src(
    program: &Program,
    ctx: &LowerContext,
    source: &str,
    node: Node,
) -> Option<VarId> {
    match node.kind() {
        "pointer_expression" => {
            let op = pointer_op(source, node);
            let arg = pointer_arg(node)?;
            if op.as_deref() == Some("&") {
                return resolve_lvalue_var(program, ctx, source, arg);
            }
            None
        }
        "identifier" => resolve_lvalue_var(program, ctx, source, node),
        _ => resolve_expr_var(program, ctx, source, node),
    }
}

fn expr_to_rhs_flow(
    program: &mut Program,
    ctx: &LowerContext,
    source: &str,
    node: Node,
    dst: VarId,
) -> Option<FlowConstraint> {
    match node.kind() {
        "identifier" => {
            let name = node_text(source, &node);
            if let Some(callee) = program
                .symbols
                .resolve_function_in_scope(name, Some(ctx.current_file))
            {
                Some(FlowConstraint::AddrOfFn { dst, callee })
            } else {
                lookup_var(ctx, program, name).map(|src| FlowConstraint::Copy { dst, src })
            }
        }
        "pointer_expression" => {
            let op = pointer_op(source, node);
            let arg = pointer_arg(node)?;
            if op.as_deref() == Some("&") {
                if let Some(callee) = resolve_fn_ref(program, ctx, source, arg) {
                    Some(FlowConstraint::AddrOfFn { dst, callee })
                } else {
                    resolve_lvalue_var(program, ctx, source, arg)
                        .map(|src| FlowConstraint::AddrOfVar { dst, src })
                }
            } else if op.as_deref() == Some("*") {
                let ptr = resolve_lvalue_var(program, ctx, source, arg)?;
                Some(FlowConstraint::Load { dst, src: ptr })
            } else {
                None
            }
        }
        "cast_expression" => node
            .child_by_field_name("expression")
            .or_else(|| node.named_child(1))
            .and_then(|inner| expr_to_rhs_flow(program, ctx, source, inner, dst)),
        "parenthesized_expression" => node
            .named_child(0)
            .and_then(|inner| expr_to_rhs_flow(program, ctx, source, inner, dst)),
        "call_expression" => {
            if let Some(callee_name) = resolve_direct_call(program, ctx, source, node) {
                program
                    .flow
                    .push(FlowConstraint::CallReturn { dst, callee_name });
            }
            None
        }
        "field_expression" => {
            let (base, field_ids) = decompose_field_path(program, ctx, source, node)?;
            let mut current = base;
            for (i, fid) in field_ids.iter().enumerate() {
                if i + 1 == field_ids.len() {
                    let tmp = alloc_gep_temp(program, ctx, node, current, *fid);
                    return Some(FlowConstraint::Load { dst, src: tmp });
                }
                current = alloc_gep_temp(program, ctx, node, current, *fid);
            }
            None
        }
        _ => resolve_expr_var(program, ctx, source, node)
            .map(|src| FlowConstraint::Copy { dst, src }),
    }
}

fn collect_return_statement(
    program: &mut Program,
    ctx: &LowerContext,
    source: &str,
    node: Node,
    fn_id: FnId,
) {
    let value = node
        .child_by_field_name("value")
        .or_else(|| node.named_child(0));
    let Some(value) = value else {
        return;
    };
    if value.kind() == ";" {
        return;
    }
    collect_return_flow(program, ctx, source, value, fn_id);
}

fn collect_return_flow(
    program: &mut Program,
    ctx: &LowerContext,
    source: &str,
    node: Node,
    fn_id: FnId,
) {
    if let Some(flow) = return_flow_from_expr(program, ctx, source, node) {
        program.fn_returns.entry(fn_id).or_default().push(flow);
    }
}

fn return_flow_from_expr(
    program: &Program,
    ctx: &LowerContext,
    source: &str,
    node: Node,
) -> Option<ReturnFlow> {
    let node = peel_expression(node);
    match node.kind() {
        "pointer_expression" => {
            let op = pointer_op(source, node);
            let arg = pointer_arg(node)?;
            if op.as_deref() == Some("&") {
                if let Some(callee) = resolve_fn_ref(program, ctx, source, arg) {
                    Some(ReturnFlow::AddrOfFn { callee })
                } else {
                    resolve_lvalue_var(program, ctx, source, arg).map(|src| ReturnFlow::AddrOfVar { src })
                }
            } else {
                None
            }
        }
        "identifier" => {
            let name = node_text(source, &node);
            if resolve_function_named(program, ctx, name).is_some() {
                None
            } else {
                lookup_var(ctx, program, name).map(|src| ReturnFlow::Copy { src })
            }
        }
        "call_expression" => resolve_direct_call_name(source, node)
            .map(|callee_name| ReturnFlow::Call { callee_name }),
        "cast_expression" => node
            .child_by_field_name("expression")
            .or_else(|| node.named_child(1))
            .and_then(|inner| return_flow_from_expr(program, ctx, source, inner)),
        "parenthesized_expression" => node
            .named_child(0)
            .and_then(|inner| return_flow_from_expr(program, ctx, source, inner)),
        _ => None,
    }
}

fn resolve_direct_call_name(source: &str, node: Node) -> Option<String> {
    let func = node.child_by_field_name("function")?;
    let func = peel_expression(func);
    match func.kind() {
        "identifier" => Some(node_text(source, &func).to_string()),
        "pointer_expression" | "parenthesized_expression" => func
            .named_child(0)
            .and_then(|inner| resolve_direct_call_name(source, inner)),
        _ => None,
    }
}

fn resolve_direct_call(
    _program: &Program,
    _ctx: &LowerContext,
    source: &str,
    node: Node,
) -> Option<String> {
    resolve_direct_call_name(source, node)
}

fn alloc_ret_temp(program: &mut Program, ctx: &LowerContext, span_node: Node) -> VarId {
    let var_id = program.symbols.alloc_var_id();
    program.symbols.add_variable(Variable {
        id: var_id,
        name: format!("_ret{}", var_id.0),
        type_id: program.types.int(),
        storage: StorageClass::Local,
        fn_id: ctx.current_fn,
        param_index: None,
        span: node_span(ctx, span_node),
        is_pointer: true,
    });
    var_id
}

fn resolve_lvalue_var(
    program: &Program,
    ctx: &LowerContext,
    source: &str,
    node: Node,
) -> Option<VarId> {
    match node.kind() {
        "identifier" => {
            let name = node_text(source, &node);
            lookup_var(ctx, program, name)
        }
        "pointer_expression" => {
            let op = pointer_op(source, node);
            let arg = pointer_arg(node)?;
            if op.as_deref() == Some("*") {
                return resolve_lvalue_var(program, ctx, source, arg);
            }
            resolve_lvalue_var(program, ctx, source, arg)
        }
        "field_expression" | "subscript_expression" => node
            .child_by_field_name("argument")
            .and_then(|n| resolve_lvalue_var(program, ctx, source, n)),
        "parenthesized_expression" => node
            .named_child(0)
            .and_then(|n| resolve_lvalue_var(program, ctx, source, n)),
        "cast_expression" => node
            .child_by_field_name("expression")
            .or_else(|| node.named_child(1))
            .and_then(|n| resolve_lvalue_var(program, ctx, source, n)),
        _ => None,
    }
}

fn find_function_declarator(node: Node) -> Option<Node> {
    if node.kind() == "function_declarator" {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_function_declarator(child) {
            return Some(found);
        }
    }
    None
}

fn type_desc_from_field_declaration(
    program: &mut Program,
    source: &str,
    node: Node,
) -> Option<(String, TypeDesc)> {
    let decl = node.child_by_field_name("declarator")?;
    let (fname, _) = parse_declarator_name(source, decl);
    if fname.is_empty() {
        return None;
    }
    let base = node
        .child_by_field_name("type")
        .map(|t| type_desc_from_node(program, source, t))
        .unwrap_or(TypeDesc::Int);
    let desc = if is_function_pointer_declarator(decl) {
        TypeDesc::FnPtr {
            ret: Box::new(base),
            params: Vec::new(),
        }
    } else if declarator_is_pointer(decl) {
        TypeDesc::Ptr(Box::new(base))
    } else {
        base
    };
    Some((fname, desc))
}

fn declarator_is_pointer(decl: Node) -> bool {
    match decl.kind() {
        "pointer_declarator" => true,
        "function_declarator" | "parenthesized_declarator" | "array_declarator" => decl
            .child_by_field_name("declarator")
            .is_some_and(declarator_is_pointer),
        _ => false,
    }
}

fn resolve_call_fn_arg(
    program: &Program,
    ctx: &LowerContext,
    source: &str,
    node: Node,
) -> Option<FnId> {
    if let Some(fn_id) = resolve_fn_ref(program, ctx, source, node) {
        return Some(fn_id);
    }
    if node.kind() == "pointer_expression" {
        if let Some(inner) = pointer_arg(node) {
            return resolve_call_fn_arg(program, ctx, source, inner);
        }
    }
    None
}

fn resolve_function_named(program: &Program, ctx: &LowerContext, name: &str) -> Option<FnId> {
    program
        .symbols
        .resolve_function_in_scope(name, Some(ctx.current_file))
        .or_else(|| program.symbols.resolve_function(name))
}

fn resolve_fn_ref(program: &Program, ctx: &LowerContext, source: &str, node: Node) -> Option<FnId> {
    if node.kind() == "identifier" {
        return resolve_function_named(program, ctx, node_text(source, &node));
    }
    None
}

fn resolve_callee_with_loads(
    program: &mut Program,
    ctx: &mut LowerContext,
    source: &str,
    node: Node,
) -> (String, bool, Option<VarId>) {
    let node = peel_expression(node);
    if node.kind() == "field_expression" {
        if let Some((base, field_ids)) = decompose_field_path(program, ctx, source, node) {
            let text = field_callee_text(source, node);
            if let Some(load_var) =
                emit_field_fn_ptr_load(program, ctx, source, node, base, &field_ids)
            {
                return (text, false, Some(load_var));
            }
        }
    }
    resolve_callee(program, ctx, source, node)
}

fn field_callee_text(source: &str, node: Node) -> String {
    let mut parts = Vec::new();
    let mut cur = peel_expression(node);
    while cur.kind() == "field_expression" {
        if let Some(field) = cur.child_by_field_name("field") {
            parts.push(node_text(source, &field).to_string());
        }
        cur = cur.child_by_field_name("argument").unwrap_or(cur);
    }
    parts.reverse();
    let base = node_text(source, &cur);
    if parts.is_empty() {
        base.to_string()
    } else {
        format!("{}->{}", base, parts.join("->"))
    }
}

fn emit_field_fn_ptr_load(
    program: &mut Program,
    ctx: &LowerContext,
    _source: &str,
    span_node: Node,
    base: VarId,
    field_ids: &[FieldId],
) -> Option<VarId> {
    if field_ids.is_empty() {
        return None;
    }
    let mut type_id = struct_type_for_var(program, base)?;
    let mut current = base;
    for (i, fid) in field_ids.iter().enumerate() {
        let gep = alloc_gep_temp(program, ctx, span_node, current, *fid);
        let field_type_id = program.types.get(type_id).layout.fields.get(fid)?.type_id;
        type_id = field_type_id;
        if i + 1 == field_ids.len() {
            let load_var = program.symbols.alloc_var_id();
            program.symbols.add_variable(Variable {
                id: load_var,
                name: format!("_load{}", load_var.0),
                type_id: program.types.int(),
                storage: StorageClass::Local,
                fn_id: ctx.current_fn,
                param_index: None,
                span: node_span(ctx, span_node),
                is_pointer: true,
            });
            program.flow.push(FlowConstraint::Load {
                dst: load_var,
                src: gep,
            });
            return Some(load_var);
        }
        if matches!(program.types.get(field_type_id).desc, TypeDesc::Ptr(_)) {
            let load_var = program.symbols.alloc_var_id();
            program.symbols.add_variable(Variable {
                id: load_var,
                name: format!("_load{}", load_var.0),
                type_id: field_type_id,
                storage: StorageClass::Local,
                fn_id: ctx.current_fn,
                param_index: None,
                span: node_span(ctx, span_node),
                is_pointer: true,
            });
            program.flow.push(FlowConstraint::Load {
                dst: load_var,
                src: gep,
            });
            current = load_var;
            type_id = program
                .types
                .resolve_type_id(match &program.types.get(field_type_id).desc {
                    TypeDesc::Ptr(inner) => inner,
                    _ => unreachable!(),
                });
        } else {
            current = gep;
        }
    }
    None
}

fn is_likely_macro_callee(name: &str) -> bool {
    if name.contains("->") || name.contains('.') || name.contains('(') {
        return false;
    }
    name.len() > 2
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn resolve_callee(
    program: &Program,
    ctx: &LowerContext,
    source: &str,
    node: Node,
) -> (String, bool, Option<VarId>) {
    let node = peel_expression(node);
    match node.kind() {
        "identifier" => {
            let name = node_text(source, &node).to_string();
            if let Some(v) = lookup_var(ctx, program, &name) {
                return (name, false, Some(v));
            }
            if resolve_function_named(program, ctx, &name).is_some() {
                return (name, true, None);
            }
            (name, false, None)
        }
        "pointer_expression" | "parenthesized_expression" => node
            .named_child(0)
            .map(|inner| resolve_callee(program, ctx, source, inner))
            .unwrap_or(("<indirect>".into(), false, None)),
        "field_expression" => {
            let field = node
                .child_by_field_name("field")
                .map(|n| node_text(source, &n).to_string())
                .unwrap_or_else(|| "field".into());
            let arg = node.child_by_field_name("argument").unwrap();
            if let Some(v) = resolve_lvalue_var(program, ctx, source, arg) {
                return (
                    format!("{}->{}", node_text(source, &arg), field),
                    false,
                    Some(v),
                );
            }
            (field, false, None)
        }
        "subscript_expression" => {
            let arr = node.child_by_field_name("argument").unwrap();
            if let Some(v) = resolve_lvalue_var(program, ctx, source, arr) {
                return (format!("{}[...]", node_text(source, &arr)), false, Some(v));
            }
            ("<indirect>".into(), false, None)
        }
        _ => (node_text(source, &node).to_string(), false, None),
    }
}

fn resolve_expr_var(
    program: &Program,
    ctx: &LowerContext,
    source: &str,
    node: Node,
) -> Option<VarId> {
    match node.kind() {
        "identifier" => {
            let name = node_text(source, &node);
            lookup_var(ctx, program, name)
        }
        "pointer_expression" => {
            let op = pointer_op(source, node);
            let arg = pointer_arg(node)?;
            if op.as_deref() == Some("&") {
                return resolve_lvalue_var(program, ctx, source, arg);
            }
            resolve_expr_var(program, ctx, source, arg)
        }
        "field_expression" | "subscript_expression" => node
            .child_by_field_name("argument")
            .and_then(|n| resolve_expr_var(program, ctx, source, n)),
        "parenthesized_expression" => node
            .named_child(0)
            .and_then(|n| resolve_expr_var(program, ctx, source, n)),
        _ => None,
    }
}

fn lookup_var(ctx: &LowerContext, program: &Program, name: &str) -> Option<VarId> {
    if ctx.current_fn.is_some() {
        if let Some(&id) = ctx.locals.get(name) {
            return Some(id);
        }
    }
    if let Some(&id) = program.symbols.global_by_name.get(name) {
        return Some(id);
    }
    program
        .symbols
        .variables
        .iter()
        .find(|v| {
            v.name == name
                && match v.storage {
                    StorageClass::FileStatic => v.span.file == ctx.current_file,
                    StorageClass::FnStatic => v.fn_id == ctx.current_fn,
                    _ => false,
                }
        })
        .map(|v| v.id)
}

fn declaration_is_static(_source: &str, node: Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "storage_class_specifier" {
            continue;
        }
        let mut inner = child.walk();
        for token in child.children(&mut inner) {
            if token.kind() == "static" {
                return true;
            }
        }
    }
    false
}

fn is_function_pointer_declarator(decl: Node) -> bool {
    if decl.kind() != "function_declarator" {
        return false;
    }
    matches!(
        decl.child_by_field_name("declarator").map(|n| n.kind()),
        Some("parenthesized_declarator") | Some("pointer_declarator")
    )
}

fn storage_for(ctx: &LowerContext, is_static: bool) -> StorageClass {
    if ctx.current_fn.is_some() {
        if is_static {
            StorageClass::FnStatic
        } else {
            StorageClass::Local
        }
    } else if is_static {
        StorageClass::FileStatic
    } else {
        StorageClass::Global
    }
}

fn type_desc_from_node(program: &mut Program, source: &str, node: Node) -> TypeDesc {
    if node.kind() == "struct_specifier" || node.kind() == "union_specifier" {
        let name = lower_struct_specifier(program, source, node);
        if node.kind() == "union_specifier" {
            return TypeDesc::Union {
                name,
                fields: Vec::new(),
            };
        }
        return TypeDesc::Struct {
            name,
            fields: Vec::new(),
        };
    }
    let text = node_text(source, &node);
    if text.contains("union") {
        TypeDesc::Union {
            name: extract_tag_name(source, &node, "union"),
            fields: Vec::new(),
        }
    } else if text.contains("struct") {
        TypeDesc::Struct {
            name: extract_tag_name(source, &node, "struct"),
            fields: Vec::new(),
        }
    } else if text.contains("char") {
        TypeDesc::Char
    } else if text.contains("void") {
        TypeDesc::Void
    } else {
        TypeDesc::Int
    }
}

fn parse_type_node(program: &mut Program, source: &str, node: Node) -> trace_ir::TypeId {
    let desc = type_desc_from_node(program, source, node);
    program.types.intern(desc)
}

fn extract_tag_name(source: &str, node: &Node, keyword: &str) -> String {
    let text = node_text(source, node);
    if let Some(rest) = text.split(keyword).nth(1) {
        rest.trim()
            .trim_start_matches(" {")
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .next()
            .unwrap_or("anon")
            .to_string()
    } else {
        "anon".into()
    }
}

fn parse_declarator_name(source: &str, node: Node) -> (String, bool) {
    match node.kind() {
        "identifier" => (node_text(source, &node).to_string(), false),
        "pointer_declarator" => {
            if let Some(inner) = node.child_by_field_name("declarator") {
                let (name, _) = parse_declarator_name(source, inner);
                (name, true)
            } else {
                (String::new(), true)
            }
        }
        "function_declarator" => {
            if let Some(inner) = node.child_by_field_name("declarator") {
                parse_declarator_name(source, inner)
            } else {
                (String::new(), false)
            }
        }
        "parenthesized_declarator" => node
            .named_child(0)
            .map(|n| parse_declarator_name(source, n))
            .unwrap_or((String::new(), false)),
        "array_declarator" => node
            .child_by_field_name("declarator")
            .map(|n| parse_declarator_name(source, n))
            .unwrap_or((String::new(), false)),
        _ => (node_text(source, &node).to_string(), false),
    }
}

fn find_params(decl: Node) -> Option<Node> {
    if decl.kind() == "function_declarator" {
        return decl.child_by_field_name("parameters");
    }
    for i in 0..decl.child_count() {
        if let Some(child) = decl.child(i) {
            if let Some(p) = find_params(child) {
                return Some(p);
            }
        }
    }
    None
}

fn node_span(ctx: &LowerContext, node: Node) -> Span {
    Span::new(
        ctx.current_file,
        node.start_position().row as u32 + 1,
        node.start_position().column as u32 + 1,
    )
}

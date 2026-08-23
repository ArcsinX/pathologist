use crate::schema::SCHEMA_V1;
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use rustc_hash::FxHashSet;
use std::fs;
use std::path::{Path, PathBuf};
use trace_analysis::{AnalysisResult, Pag};
use trace_ir::{Linkage, Program, StorageClass, TypeDesc, VarId, TRACE_VERSION};

pub struct ExportOptions {
    pub output: PathBuf,
    /// Export points-to debug table (requires analysis with `retain_points_to`).
    pub include_points_to: bool,
    /// Export types, all variables, and PAG locations (slower, larger DB).
    pub full_detail: bool,
}

impl ExportOptions {
    pub fn minimal(output: PathBuf) -> Self {
        Self {
            output,
            include_points_to: false,
            full_detail: false,
        }
    }
}

pub fn export_to_sqlite(
    program: &Program,
    pag: Option<&Pag>,
    analysis: &AnalysisResult,
    opts: &ExportOptions,
) -> Result<()> {
    let temp = opts.output.with_extension("db.tmp");
    if let Some(parent) = temp.parent() {
        fs::create_dir_all(parent)?;
    }
    if temp.exists() {
        fs::remove_file(&temp)?;
    }
    {
        let conn = Connection::open(&temp)?;
        conn.execute_batch(
            "PRAGMA foreign_keys = OFF; PRAGMA synchronous = OFF; PRAGMA journal_mode = MEMORY;",
        )?;
        conn.execute_batch("BEGIN IMMEDIATE;")?;
        conn.execute_batch(SCHEMA_V1)?;

        let options_json = serde_json::json!({
            "include_paths": program.include_paths,
            "defines": program.defines,
            "include_points_to": opts.include_points_to,
            "full_detail": opts.full_detail,
        })
        .to_string();

        conn.execute(
            "INSERT INTO analysis_run (trace_version, target_root, created_at, options_json) VALUES (?1, ?2, ?3, ?4)",
            params![
                TRACE_VERSION,
                program.root.display().to_string(),
                chrono_lite_now(),
                options_json
            ],
        )?;

        export_files(&conn, program)?;
        export_functions(&conn, program)?;
        export_call_sites_filtered(&conn, program, analysis)?;
        export_call_edges(&conn, analysis)?;
        if opts.full_detail {
            export_types(&conn, program)?;
            export_variables(&conn, program)?;
            if let Some(pag) = pag {
                export_locations(&conn, pag)?;
            }
        } else {
            export_arg_flow_vars(&conn, program, analysis)?;
        }
        export_arg_flow(&conn, analysis)?;
        if opts.include_points_to {
            let pag = pag.context("points-to export requires PAG")?;
            export_points_to(&conn, pag, analysis)?;
        }
        export_diagnostics(&conn, program)?;
        conn.execute_batch("COMMIT;")?;
    }

    if opts.output.exists() {
        fs::remove_file(&opts.output)?;
    }
    fs::rename(&temp, &opts.output)?;
    Ok(())
}

fn chrono_lite_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", dur.as_secs())
}

fn export_files(conn: &Connection, program: &Program) -> Result<()> {
    let mut stmt =
        conn.prepare_cached("INSERT INTO files (id, path, sha256) VALUES (?1, ?2, ?3)")?;
    for file in &program.symbols.files {
        stmt.execute(params![file.id.0, file.path.display().to_string(), ""])?;
    }
    Ok(())
}

fn export_types(conn: &Connection, program: &Program) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO types (id, kind, name, size, layout_json) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for ty in program.types.all() {
        let kind = match &ty.desc {
            TypeDesc::Void => "void",
            TypeDesc::Char => "char",
            TypeDesc::Int => "int",
            TypeDesc::Long => "long",
            TypeDesc::SizeT => "size_t",
            TypeDesc::Unknown => "unknown",
            TypeDesc::Ptr(_) => "ptr",
            TypeDesc::Array { .. } => "array",
            TypeDesc::Struct { .. } => "struct",
            TypeDesc::Union { .. } => "union",
            TypeDesc::FnPtr { .. } => "fn_ptr",
        };
        let name = type_name(&ty.desc);
        let layout_json = serde_json::to_string(&ty.layout)?;
        stmt.execute(params![ty.id.0, kind, name, ty.size, layout_json])?;
    }
    Ok(())
}

fn type_name(desc: &TypeDesc) -> String {
    match desc {
        TypeDesc::Struct { name, .. } | TypeDesc::Union { name, .. } => name.clone(),
        TypeDesc::Ptr(inner) => format!("{}*", type_name(inner)),
        TypeDesc::Array { elem, size } => {
            format!(
                "{}[{}]",
                type_name(elem),
                size.map(|s| s.to_string()).unwrap_or_default()
            )
        }
        TypeDesc::FnPtr { .. } => "fn_ptr".into(),
        TypeDesc::Void => "void".into(),
        TypeDesc::Char => "char".into(),
        TypeDesc::Int => "int".into(),
        TypeDesc::Long => "long".into(),
        TypeDesc::SizeT => "size_t".into(),
        TypeDesc::Unknown => "unknown".into(),
    }
}

fn export_variables(conn: &Connection, program: &Program) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO variables (id, name, kind, fn_id, type_id, file_id, line) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for var in &program.symbols.variables {
        export_one_variable(&mut stmt, var)?;
    }
    Ok(())
}

fn export_arg_flow_vars(
    conn: &Connection,
    program: &Program,
    analysis: &AnalysisResult,
) -> Result<()> {
    if analysis.arg_flow_edges.is_empty() {
        return Ok(());
    }
    let mut needed: FxHashSet<VarId> = FxHashSet::default();
    for edge in &analysis.arg_flow_edges {
        if let Some(v) = edge.actual_var {
            needed.insert(v);
        }
        needed.insert(edge.formal);
    }
    let mut stmt = conn.prepare_cached(
        "INSERT INTO variables (id, name, kind, fn_id, type_id, file_id, line) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for var in &program.symbols.variables {
        if needed.contains(&var.id) {
            export_one_variable(&mut stmt, var)?;
        }
    }
    Ok(())
}

fn export_one_variable(stmt: &mut rusqlite::Statement<'_>, var: &trace_ir::Variable) -> Result<()> {
    let kind = match var.storage {
        StorageClass::Global => "global",
        StorageClass::FileStatic => "file_static",
        StorageClass::FnStatic => "fn_static",
        StorageClass::Param => "param",
        StorageClass::Local => "local",
    };
    stmt.execute(params![
        var.id.0,
        var.name,
        kind,
        var.fn_id.map(|f| f.0),
        var.type_id.0,
        var.span.file.0,
        var.span.line
    ])?;
    Ok(())
}

fn export_functions(conn: &Connection, program: &Program) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO functions (id, name, file_id, line_start, line_end, linkage, signature, is_defined) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for func in &program.symbols.functions {
        let linkage = match func.linkage {
            Linkage::External => "external",
            Linkage::Internal => "internal",
            Linkage::None => "none",
        };
        stmt.execute(params![
            func.id.0,
            func.name,
            func.file.0,
            func.span.line,
            func.span.line,
            linkage,
            format!("fn_{}", func.name),
            func.is_defined as i32
        ])?;
    }
    Ok(())
}

fn export_call_sites_filtered(
    conn: &Connection,
    program: &Program,
    analysis: &AnalysisResult,
) -> Result<()> {
    let mut with_edge = FxHashSet::default();
    for edge in &analysis.call_edges {
        with_edge.insert(edge.call_site);
    }
    let mut with_arg_flow = FxHashSet::default();
    for edge in &analysis.arg_flow_edges {
        with_arg_flow.insert(edge.call_site);
    }

    let mut stmt = conn.prepare_cached(
        "INSERT INTO call_sites (id, caller_fn_id, file_id, line, col, callee_text, is_direct) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for cs in &program.symbols.call_sites {
        let export = with_edge.contains(&cs.id) || with_arg_flow.contains(&cs.id) || !cs.is_direct;
        if !export {
            continue;
        }
        stmt.execute(params![
            cs.id.0,
            cs.caller.0,
            cs.span.file.0,
            cs.span.line,
            cs.span.col,
            cs.callee_name,
            cs.is_direct as i32
        ])?;
    }
    Ok(())
}

fn export_call_edges(conn: &Connection, analysis: &AnalysisResult) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO call_edges (id, call_site_id, callee_fn_id, resolution) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (i, edge) in analysis.call_edges.iter().enumerate() {
        let resolution = match edge.resolution {
            trace_analysis::ResolutionKind::Direct => "direct",
            trace_analysis::ResolutionKind::Indirect => "indirect",
            trace_analysis::ResolutionKind::Ambiguous => "ambiguous",
            trace_analysis::ResolutionKind::External => "external",
        };
        stmt.execute(params![
            i as i64 + 1,
            edge.call_site.0,
            edge.callee.0,
            resolution
        ])?;
    }
    Ok(())
}

fn export_arg_flow(conn: &Connection, analysis: &AnalysisResult) -> Result<()> {
    if analysis.arg_flow_edges.is_empty() {
        return Ok(());
    }
    let mut stmt = conn.prepare_cached(
        "INSERT INTO arg_flow_edges (id, call_site_id, arg_index, actual_var_id, actual_fn_id, formal_var_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for (i, edge) in analysis.arg_flow_edges.iter().enumerate() {
        stmt.execute(params![
            i as i64 + 1,
            edge.call_site.0,
            edge.arg_index,
            edge.actual_var.map(|v| v.0),
            edge.actual_fn.map(|f| f.0),
            edge.formal.0
        ])?;
    }
    Ok(())
}

fn export_locations(conn: &Connection, pag: &Pag) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO locations (id, kind, desc, type_id) VALUES (?1, ?2, ?3, NULL)",
    )?;
    for loc in &pag.locations {
        let kind = format!("{:?}", loc.kind);
        stmt.execute(params![loc.id.0, kind, loc.desc])?;
    }
    Ok(())
}

fn export_points_to(conn: &Connection, _pag: &Pag, analysis: &AnalysisResult) -> Result<()> {
    let mut stmt = conn
        .prepare_cached("INSERT OR IGNORE INTO points_to (var_node_id, loc_id) VALUES (?1, ?2)")?;
    for (node, locs) in &analysis.points_to {
        for loc in locs {
            stmt.execute(params![node.0, loc.0])?;
        }
    }
    Ok(())
}

fn export_diagnostics(conn: &Connection, program: &Program) -> Result<()> {
    if program.diagnostics.is_empty() {
        return Ok(());
    }
    let mut stmt = conn.prepare_cached(
        "INSERT INTO diagnostics (id, severity, file_id, line, message, stage) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for (i, d) in program.diagnostics.iter().enumerate() {
        let severity = match d.severity {
            trace_ir::DiagnosticSeverity::Error => "error",
            trace_ir::DiagnosticSeverity::Warning => "warning",
            trace_ir::DiagnosticSeverity::Info => "info",
        };
        stmt.execute(params![
            i as i64 + 1,
            severity,
            d.file.map(|f| f.0),
            d.line,
            d.message,
            d.stage
        ])?;
    }
    Ok(())
}

pub fn open_db(path: &Path) -> Result<Connection> {
    Connection::open(path).with_context(|| format!("failed to open db at {}", path.display()))
}

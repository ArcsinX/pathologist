use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Instant;
use trace_analysis::{analyze_with_options, AnalyzeOptions, ResolutionKind};
use trace_db::{export_to_sqlite, open_db, ExportOptions};
use trace_parse::build_program_with_jobs;
use trace_preproc::PreprocessOptions;

#[derive(Parser)]
#[command(
    name = "trace",
    version,
    about = "C call graph and pointer analysis tool"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze a C project directory and write results to SQLite.
    Analyze {
        /// Target project directory containing .c files.
        target: PathBuf,
        /// Output SQLite database path.
        #[arg(short, long, default_value = "trace.db")]
        output: PathBuf,
        /// Add include search path (repeatable).
        #[arg(long = "include")]
        includes: Vec<PathBuf>,
        /// Define preprocessor macro NAME or NAME=VALUE (repeatable).
        #[arg(short = 'D')]
        defines: Vec<String>,
        /// Number of parallel jobs for indexing (parse/lower).
        #[arg(long)]
        jobs: Option<usize>,
        /// Include points-to debug table in output (also retains points-to in memory during analysis).
        #[arg(long)]
        debug_points_to: bool,
        /// Export full IR detail (types, all variables, PAG locations). Default: call graph + arg-flow only.
        #[arg(long)]
        full_export: bool,
    },
    /// Inspect an existing analysis database.
    Inspect {
        /// Path to SQLite database.
        db: PathBuf,
        #[command(subcommand)]
        command: InspectCommands,
    },
}

#[derive(Subcommand)]
enum InspectCommands {
    /// List call graph edges.
    Calls {
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Analyze {
            target,
            output,
            includes,
            defines,
            jobs,
            debug_points_to,
            full_export,
        } => run_analyze(
            target,
            output,
            includes,
            defines,
            jobs,
            debug_points_to,
            full_export,
        ),
        Commands::Inspect { db, command } => run_inspect(db, command),
    }
}

fn run_analyze(
    target: PathBuf,
    output: PathBuf,
    includes: Vec<PathBuf>,
    defines: Vec<String>,
    jobs: Option<usize>,
    debug_points_to: bool,
    full_export: bool,
) -> Result<()> {
    let jobs = jobs.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .max(1)
    });
    let mut opts = PreprocessOptions::new();
    for inc in includes {
        opts.include_paths.push(inc);
    }
    for def in defines {
        if let Some((name, value)) = def.split_once('=') {
            opts = opts.with_define(name, value);
        } else {
            opts = opts.with_define(def, "1");
        }
    }

    // Include paths pointing outside the analyzed tree make twin headers
    // (same basename, different tree) resolve to the wrong copy, which
    // silently starves translation units. Warn loudly — this misconfiguration
    // previously produced silent false negatives.
    let root_canon = target.canonicalize().unwrap_or_else(|_| target.clone());
    let outside: Vec<PathBuf> = opts
        .include_paths
        .iter()
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
        .filter(|c| !(c.starts_with(&root_canon) || root_canon.starts_with(c)))
        .collect();
    if !outside.is_empty() {
        eprintln!(
            "warning: {} include path(s) lie outside the analysis tree {};",
            outside.len(),
            root_canon.display()
        );
        eprintln!("         headers may resolve to twins in another tree and lose definitions:");
        for p in outside.iter().take(5) {
            eprintln!("           {}", p.display());
        }
        if outside.len() > 5 {
            eprintln!("           ... and {} more", outside.len() - 5);
        }
    }

    let t0 = Instant::now();
    let program = build_program_with_jobs(&target, &opts, jobs).map_err(|e| anyhow::anyhow!(e))?;
    eprintln!(
        "index: {:.1}s ({} files, {} functions, {} flow)",
        t0.elapsed().as_secs_f64(),
        program.symbols.files.len(),
        program.symbols.functions.len(),
        program.flow.len(),
    );

    let t1 = Instant::now();
    let (pag, analysis) = analyze_with_options(
        &program,
        AnalyzeOptions {
            retain_points_to: debug_points_to,
        },
    );
    let indirect = analysis
        .call_edges
        .iter()
        .filter(|e| e.resolution == ResolutionKind::Indirect)
        .count();
    eprintln!(
        "analyze: {:.1}s ({} edges, {} indirect)",
        t1.elapsed().as_secs_f64(),
        analysis.call_edges.len(),
        indirect,
    );

    let t2 = Instant::now();
    let pag_ref = if full_export || debug_points_to {
        Some(&pag)
    } else {
        None
    };
    export_to_sqlite(
        &program,
        pag_ref,
        &analysis,
        &ExportOptions {
            output: output.clone(),
            include_points_to: debug_points_to,
            full_detail: full_export,
        },
    )
    .with_context(|| format!("failed to export to {}", output.display()))?;
    eprintln!("export: {:.1}s", t2.elapsed().as_secs_f64());

    let mut direct_edges = 0usize;
    let mut indirect_edges = 0usize;
    let mut external_edges = 0usize;
    for e in &analysis.call_edges {
        match e.resolution {
            // Ambiguous groups with direct in the summary: both are
            // statically-name-resolved; ambiguity only means several
            // same-name candidates, not pointer indirection.
            trace_analysis::ResolutionKind::Direct | trace_analysis::ResolutionKind::Ambiguous => {
                direct_edges += 1
            }
            trace_analysis::ResolutionKind::Indirect => indirect_edges += 1,
            trace_analysis::ResolutionKind::External => external_edges += 1,
        }
    }
    eprintln!(
        "analysis complete: {} functions ({} external), {} call edges ({} direct, {} indirect, {} external), {} arg-flow edges -> {}",
        program.symbols.functions.len(),
        program
            .symbols
            .functions
            .iter()
            .filter(|f| !f.is_defined)
            .count(),
        analysis.call_edges.len(),
        direct_edges,
        indirect_edges,
        external_edges,
        analysis.arg_flow_edges.len(),
        output.display()
    );
    Ok(())
}

fn run_inspect(db: PathBuf, command: InspectCommands) -> Result<()> {
    let conn = open_db(&db)?;
    match command {
        InspectCommands::Calls { from, to } => {
            let mut sql = String::from(
                "SELECT caller.name, callee.name, ce.resolution, cs.line \
                 FROM call_edges ce \
                 JOIN call_sites cs ON cs.id = ce.call_site_id \
                 JOIN functions caller ON caller.id = cs.caller_fn_id \
                 JOIN functions callee ON callee.id = ce.callee_fn_id WHERE 1=1",
            );
            if from.is_some() {
                sql.push_str(" AND caller.name = ?1");
            }
            if to.is_some() {
                sql.push_str(if from.is_some() {
                    " AND callee.name = ?2"
                } else {
                    " AND callee.name = ?1"
                });
            }
            let mut stmt = conn.prepare(&sql)?;
            match (from.as_deref(), to.as_deref()) {
                (Some(f), Some(t)) => {
                    let rows = stmt.query_map(rusqlite::params![f, t], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    })?;
                    for row in rows {
                        let (caller, callee, res, line) = row?;
                        println!("{caller} -> {callee} ({res}) at line {line}");
                    }
                }
                (Some(f), None) => {
                    let rows = stmt.query_map(rusqlite::params![f], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    })?;
                    for row in rows {
                        let (caller, callee, res, line) = row?;
                        println!("{caller} -> {callee} ({res}) at line {line}");
                    }
                }
                (None, Some(t)) => {
                    let rows = stmt.query_map(rusqlite::params![t], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    })?;
                    for row in rows {
                        let (caller, callee, res, line) = row?;
                        println!("{caller} -> {callee} ({res}) at line {line}");
                    }
                }
                (None, None) => {
                    let rows = stmt.query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    })?;
                    for row in rows {
                        let (caller, callee, res, line) = row?;
                        println!("{caller} -> {callee} ({res}) at line {line}");
                    }
                }
            }
        }
    }
    Ok(())
}

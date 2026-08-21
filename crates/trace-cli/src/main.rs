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

    eprintln!(
        "analysis complete: {} functions, {} call edges, {} arg-flow edges -> {}",
        program.symbols.functions.len(),
        analysis.call_edges.len(),
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

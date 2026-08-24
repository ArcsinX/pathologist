use std::path::Path;
use trace_analysis::analyze;
use trace_parse::build_program;
use trace_preproc::PreprocessOptions;

fn main() {
    let root = Path::new("tests/fixtures/memcpy_struct_fn");
    let inc = Path::new("tests/fixtures/include");
    let opts = PreprocessOptions::new()
        .with_include(root.to_path_buf())
        .with_include(inc.to_path_buf());
    let program = build_program(root, &opts).unwrap();
    let (_pag, analysis) = analyze(&program);
    for e in &analysis.call_edges {
        println!(
            "EDGE {:?} -> {:?}",
            e.resolution,
            program.symbols.function(e.callee).name.clone()
        );
    }
}

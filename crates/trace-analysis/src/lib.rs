//! Andersen-style pointer analysis and call graph construction.

mod callgraph;
mod constraints;
mod pag;
mod solver;
mod summaries;

pub use callgraph::*;
pub use constraints::{
    AbstractLocation, ArgFlowEdge, CallGraphEdge, Constraint, ConstraintKind, LocKind,
    ResolutionKind,
};
pub use pag::*;
pub use solver::{analyze, analyze_with_options, AnalysisResult, AnalyzeOptions};
pub use summaries::*;

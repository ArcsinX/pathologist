//! Shared intermediate representation for trace.

mod flow;
mod ids;
mod program;
mod span;
mod symbol;
mod types;

pub use flow::*;
pub use ids::*;
pub use program::*;
pub use span::*;
pub use symbol::*;
pub use types::*;

pub const TRACE_VERSION: &str = env!("CARGO_PKG_VERSION");

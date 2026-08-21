use trace_ir::{CallSiteId, FnId};

pub use crate::constraints::CallGraphEdge;

#[derive(Debug, Default)]
pub struct CallGraph {
    pub edges: Vec<CallGraphEdge>,
}

impl CallGraph {
    pub fn callees_of(&self, caller: FnId) -> Vec<FnId> {
        self.edges
            .iter()
            .filter(|e| e.caller == caller)
            .map(|e| e.callee)
            .collect()
    }

    pub fn callers_of(&self, callee: FnId) -> Vec<(FnId, CallSiteId)> {
        self.edges
            .iter()
            .filter(|e| e.callee == callee)
            .map(|e| (e.caller, e.call_site))
            .collect()
    }
}

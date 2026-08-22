use crate::constraints::{ArgFlowEdge, CallGraphEdge, LocKind, ResolutionKind};
use crate::pag::{Pag, PagNodeKind};
use crate::summaries::apply_call_summary;
use indexmap::IndexMap;
use rustc_hash::{FxHashMap, FxHashSet};
use trace_ir::{CallSiteId, FnId, LocId, PagNodeId, Program, StorageClass};

#[derive(Debug, Clone, Copy, Default)]
pub struct AnalyzeOptions {
    /// Retain full points-to sets on the result (for `--debug-points-to` export).
    pub retain_points_to: bool,
}

#[derive(Debug, Default)]
pub struct AnalysisResult {
    pub points_to: IndexMap<PagNodeId, FxHashSet<LocId>>,
    pub call_edges: Vec<CallGraphEdge>,
    pub arg_flow_edges: Vec<ArgFlowEdge>,
    pub wired_arg_flow: FxHashSet<(CallSiteId, u32, FnId)>,
}

pub fn analyze(program: &Program) -> (Pag, AnalysisResult) {
    analyze_with_options(program, AnalyzeOptions::default())
}

pub fn analyze_with_options(program: &Program, opts: AnalyzeOptions) -> (Pag, AnalysisResult) {
    let mut pag = Pag::build(program);
    let mut result = solve(&mut pag, program, opts.retain_points_to);
    let call_edges = result.call_edges.clone();
    let wired = result.wired_arg_flow.clone();
    extract_arg_flow(program, &call_edges, &wired, &mut result);
    (pag, result)
}

struct SolverState {
    pts: FxHashMap<PagNodeId, FxHashSet<LocId>>,
    memory_pts: FxHashMap<LocId, FxHashSet<LocId>>,
    loc_nodes: FxHashMap<LocId, FxHashSet<PagNodeId>>,
    worklist: Vec<PagNodeId>,
    queued: FxHashSet<PagNodeId>,
}

impl SolverState {
    fn push(&mut self, node: PagNodeId) {
        if self.queued.insert(node) {
            self.worklist.push(node);
        }
    }

    fn requeue_loc(&mut self, loc: LocId) {
        if let Some(nodes) = self.loc_nodes.get(&loc).cloned() {
            for n in nodes {
                self.push(n);
            }
        }
    }

    /// Merge `memory_pts[mem_loc]` into `pts[dst]` without cloning. Field-
    /// disjoint borrows keep this allocation-free apart from a tiny staging
    /// vector of newly added locations.
    fn merge_memory_into(&mut self, dst: PagNodeId, mem_loc: LocId) {
        let Some(mem) = self.memory_pts.get(&mem_loc) else {
            return;
        };
        let mut new_locs: Vec<LocId> = Vec::new();
        {
            let entry = self.pts.entry(dst).or_default();
            for &loc in mem.iter() {
                if !entry.contains(&loc) {
                    new_locs.push(loc);
                }
            }
        }
        if new_locs.is_empty() {
            return;
        }
        {
            let entry = self.pts.get_mut(&dst).expect("pts entry exists");
            for loc in &new_locs {
                entry.insert(*loc);
            }
        }
        for loc in new_locs {
            self.loc_nodes.entry(loc).or_default().insert(dst);
        }
        self.push(dst);
    }
}

/// A call site denotes a recoverable direct call when lowering recorded no
/// callee variable (`callee_var`) and the callee text is a plain identifier.
/// Cross-TU calls satisfy this: lowering marks them indirect only because the
/// definition was not visible in the translation unit.
fn direct_by_name(cs: &trace_ir::CallSite) -> bool {
    cs.callee_var.is_none() && !cs.callee_name.contains("->") && !cs.callee_name.contains('.')
}

fn solve(pag: &mut Pag, program: &Program, retain_points_to: bool) -> AnalysisResult {
    let mut st = SolverState {
        pts: FxHashMap::default(),
        memory_pts: FxHashMap::default(),
        loc_nodes: FxHashMap::default(),
        worklist: Vec::new(),
        queued: FxHashSet::default(),
    };

    for c in &pag.constraints {
        st.push(c.dst);
        st.push(c.src);
    }

    let mut call_edges: Vec<CallGraphEdge> = Vec::new();
    let mut resolved_indirect: FxHashMap<CallSiteId, Vec<FnId>> = FxHashMap::default();
    let mut wired_arg_flow: FxHashSet<(CallSiteId, u32, FnId)> = FxHashSet::default();

    for var in &program.symbols.variables {
        if !matches!(
            var.storage,
            StorageClass::Global | StorageClass::FileStatic | StorageClass::FnStatic
        ) {
            continue;
        }
        if let Some(&loc) = pag.var_location.get(&var.id) {
            let node = pag.var_node[&var.id];
            add_pts(&mut st, node, loc);
        }
    }

    for cs in &program.symbols.call_sites {
        // Direct sites: lowering saw the TU-local binding, so scope-first
        // resolution is exact per C visibility rules (file-`static` shadows
        // same-name externals inside its own TU).
        //
        // Recovered cross-TU sites: no local binding existed at lowering, and
        // merge erases which TU's view a name reflects — a name matching both
        // a `static` def and an external def is genuinely ambiguous. May-
        // approximation: consider every candidate.
        let callees: Vec<FnId> = if cs.is_direct {
            program
                .symbols
                .resolve_function_in_scope(&cs.callee_name, Some(cs.span.file))
                .into_iter()
                .collect()
        } else if direct_by_name(cs) {
            program
                .symbols
                .resolve_function_candidates(&cs.callee_name, Some(cs.span.file))
        } else {
            Vec::new()
        };
        for callee in callees {
            call_edges.push(CallGraphEdge {
                call_site: cs.id,
                caller: cs.caller,
                callee,
                resolution: ResolutionKind::Direct,
            });
            wire_params(pag, program, cs, callee, &mut st, &mut wired_arg_flow);
        }
    }

    while let Some(node) = st.worklist.pop() {
        st.queued.remove(&node);

        if let Some(idxs) = pag.indices.copy_src.get(&node) {
            let node_pts = st.pts.get(&node).cloned().unwrap_or_default();
            if !node_pts.is_empty() {
                for &idx in idxs {
                    let dst = pag.constraints[idx].dst;
                    propagate_pts(&mut st, dst, &node_pts);
                }
            }
        }

        if let Some(idxs) = pag.indices.addr_of_dst.get(&node) {
            for &idx in idxs {
                let c = &pag.constraints[idx];
                if let PagNodeKind::Loc(loc) = pag.nodes[c.src.0 as usize].kind {
                    add_pts(&mut st, node, loc);
                }
            }
        }

        if let Some(idxs) = pag.indices.load_src.get(&node) {
            let node_pts = st.pts.get(&node).cloned().unwrap_or_default();
            if !node_pts.is_empty() {
                for &idx in idxs {
                    let dst = pag.constraints[idx].dst;
                    for &loc in &node_pts {
                        if fn_for_loc(pag, loc).is_some() {
                            add_pts(&mut st, dst, loc);
                        } else {
                            st.merge_memory_into(dst, loc);
                        }
                    }
                }
            }
        }

        if let Some(idxs) = pag.indices.store_dst.get(&node) {
            for &idx in idxs {
                apply_store(pag, idx, &mut st);
            }
        }

        if let Some(idxs) = pag.indices.store_src.get(&node) {
            if pts_has_values_or_self_loc(pag, &st.pts, node) {
                for &idx in idxs {
                    apply_store(pag, idx, &mut st);
                }
            }
        }

        if let Some(idxs) = pag.indices.gep_src.get(&node) {
            let node_pts = st.pts.get(&node).cloned().unwrap_or_default();
            let idxs = idxs.clone();
            for idx in idxs {
                let (dst, src, field) = {
                    let c = &pag.constraints[idx];
                    (c.dst, c.src, c.field)
                };
                let Some(field) = field else {
                    continue;
                };
                if !node_pts.is_empty() {
                    for loc in &node_pts {
                        // Fn values parked in the base's points-to (e.g.
                        // ArrayFnMember tables of structs with fn members)
                        // flow through field accesses unchanged.
                        if fn_for_loc(pag, *loc).is_some() {
                            add_pts(&mut st, dst, *loc);
                            continue;
                        }
                        if let Some(field_loc) = pag.ensure_field_loc(program, *loc, field) {
                            // Field loc plus its instance-insensitive summary.
                            let targets = [Some(field_loc), pag.summary_for_field_loc(field_loc)];
                            propagate_locs(
                                &mut st,
                                dst,
                                targets.iter().filter_map(|t| t.as_ref().copied()),
                            );
                            for fl in targets.into_iter().flatten() {
                                st.merge_memory_into(dst, fl);
                            }
                            // ArrayFnMember element fns: reachable through
                            // the array itself or any pointer to an element.
                            if let Some(owner) = pag.locations[loc.0 as usize].var {
                                if let Some(fn_locs) = pag.array_fn_members.get(&owner) {
                                    for fl in fn_locs.iter().copied() {
                                        add_pts(&mut st, dst, fl);
                                    }
                                }
                            }
                        }
                    }
                } else if let PagNodeKind::Var(base_var) = pag.nodes[src.0 as usize].kind {
                    if let Some(summary) =
                        pag.ensure_field_summary_for_var(program, base_var, field)
                    {
                        propagate_locs(&mut st, dst, [summary]);
                        st.merge_memory_into(dst, summary);
                    }
                }
            }
        }

        if let Some(call_sites) = pag.indices.indirect_by_target.get(&node) {
            let target_pts = st.pts.get(&node).cloned().unwrap_or_default();
            if target_pts.is_empty() {
                continue;
            }
            for &cs_id in call_sites {
                let cs = program
                    .symbols
                    .call_sites
                    .get(cs_id.0 as usize)
                    .filter(|c| c.id == cs_id)
                    .expect("call site id in index");
                let mut new_callees = Vec::new();
                for &loc in &target_pts {
                    if let Some(fn_id) = fn_for_loc(pag, loc) {
                        new_callees.push(fn_id);
                    }
                }
                let prev = resolved_indirect.entry(cs_id).or_default();
                for callee in new_callees {
                    if !prev.contains(&callee) {
                        prev.push(callee);
                        call_edges.push(CallGraphEdge {
                            call_site: cs.id,
                            caller: cs.caller,
                            callee,
                            resolution: ResolutionKind::Indirect,
                        });
                        wire_params(pag, program, cs, callee, &mut st, &mut wired_arg_flow);
                        apply_call_summary(cs, callee, program);
                    }
                }
            }
        }
    }

    let points_to = if retain_points_to {
        st.pts.into_iter().collect()
    } else {
        IndexMap::new()
    };

    AnalysisResult {
        points_to,
        call_edges,
        arg_flow_edges: Vec::new(),
        wired_arg_flow,
    }
}

fn propagate_locs(st: &mut SolverState, dst: PagNodeId, locs: impl IntoIterator<Item = LocId>) {
    let mut new_locs: Vec<LocId> = Vec::new();
    {
        let entry = st.pts.entry(dst).or_default();
        for loc in locs {
            if !entry.contains(&loc) {
                new_locs.push(loc);
            }
        }
    }
    if new_locs.is_empty() {
        return;
    }
    let entry = st.pts.get_mut(&dst).expect("entry just created");
    for loc in new_locs {
        entry.insert(loc);
        st.loc_nodes.entry(loc).or_default().insert(dst);
    }
    st.push(dst);
}

fn apply_store(pag: &Pag, idx: usize, st: &mut SolverState) {
    let c = &pag.constraints[idx];
    // Clone-free store: `pts` and `memory_pts` are disjoint fields, so the
    // destination set can be iterated by reference while memory is mutated.
    let Some(dst_pts) = st.pts.get(&c.dst) else {
        return;
    };
    if dst_pts.is_empty() {
        return;
    }
    let src_set = st.pts.get(&c.src);
    let self_loc = match pag.nodes[c.src.0 as usize].kind {
        PagNodeKind::Var(v) => pag.var_location.get(&v).copied(),
        _ => None,
    };
    if src_set.map(|s| s.is_empty()).unwrap_or(true) && self_loc.is_none() {
        return;
    }
    let mut requeues: Vec<LocId> = Vec::new();
    for &loc in dst_pts.iter() {
        if fn_for_loc(pag, loc).is_some() {
            continue;
        }
        let mut changed = false;
        {
            let entry = st.memory_pts.entry(loc).or_default();
            let before = entry.len();
            if let Some(s) = src_set {
                for l in s {
                    entry.insert(*l);
                }
            }
            if let Some(sl) = self_loc {
                entry.insert(sl);
            }
            changed |= entry.len() > before;
        }
        let mut summary_loc = None;
        if let Some(summary) = pag.summary_for_field_loc(loc) {
            summary_loc = Some(summary);
            let summary_entry = st.memory_pts.entry(summary).or_default();
            let before_summary = summary_entry.len();
            if let Some(s) = src_set {
                for l in s {
                    summary_entry.insert(*l);
                }
            }
            if let Some(sl) = self_loc {
                summary_entry.insert(sl);
            }
            changed |= summary_entry.len() > before_summary;
        }
        if changed {
            requeues.push(loc);
            if let Some(summary) = summary_loc {
                requeues.push(summary);
            }
        }
    }
    for loc in requeues {
        st.requeue_loc(loc);
    }
}

/// Cheap emptiness probe used by the store-src trigger: does the node hold any
/// pointee, or (for variables) at least its own abstract storage location?
fn pts_has_values_or_self_loc(
    pag: &Pag,
    pts: &FxHashMap<PagNodeId, FxHashSet<LocId>>,
    node: PagNodeId,
) -> bool {
    if pts.get(&node).map(|s| !s.is_empty()).unwrap_or(false) {
        return true;
    }
    match pag.nodes[node.0 as usize].kind {
        PagNodeKind::Var(v) => pag.var_location.contains_key(&v),
        _ => false,
    }
}

fn wire_params(
    pag: &Pag,
    program: &Program,
    cs: &trace_ir::CallSite,
    callee: FnId,
    st: &mut SolverState,
    wired: &mut FxHashSet<(CallSiteId, u32, FnId)>,
) {
    let callee_fn = program.symbols.function(callee);
    for (i, formal) in callee_fn.params.iter().enumerate() {
        let idx = i as u32;
        if let Some(actual) = cs.var_args.iter().find(|(j, _)| *j == idx).map(|(_, v)| *v) {
            let formal_node = pag.var_node.get(formal).copied().expect("formal var node");
            let actual_node = pag.var_node.get(&actual).copied().expect("actual var node");
            if let Some(actual_pts) = st.pts.get(&actual_node).cloned() {
                propagate_pts(st, formal_node, &actual_pts);
            }
            wired.insert((cs.id, idx, callee));
        } else if let Some(fn_id) = cs.fn_args.iter().find(|(j, _)| *j == idx).map(|(_, f)| *f) {
            let formal_node = pag.var_node.get(formal).copied().expect("formal var node");
            if let Some(&fn_loc) = pag.fn_locations.get(&fn_id) {
                add_pts(st, formal_node, fn_loc);
            }
            wired.insert((cs.id, idx, callee));
        }
    }
}

fn propagate_pts(st: &mut SolverState, dst: PagNodeId, src_pts: &FxHashSet<LocId>) {
    let mut new_locs: Vec<LocId> = Vec::new();
    {
        let entry = st.pts.entry(dst).or_default();
        for &loc in src_pts {
            if !entry.contains(&loc) {
                new_locs.push(loc);
            }
        }
    }
    if new_locs.is_empty() {
        return;
    }
    let entry = st.pts.get_mut(&dst).expect("entry just created");
    for loc in new_locs {
        entry.insert(loc);
        st.loc_nodes.entry(loc).or_default().insert(dst);
    }
    st.push(dst);
}

fn add_pts(st: &mut SolverState, node: PagNodeId, loc: LocId) {
    let inserted = {
        let entry = st.pts.entry(node).or_default();
        entry.insert(loc)
    };
    if inserted {
        st.loc_nodes.entry(loc).or_default().insert(node);
        st.push(node);
    }
}

fn fn_for_loc(pag: &Pag, loc: LocId) -> Option<FnId> {
    let abstract_loc = &pag.locations[loc.0 as usize];
    if abstract_loc.kind == LocKind::Function {
        abstract_loc.fn_id
    } else {
        None
    }
}

fn extract_arg_flow(
    program: &Program,
    call_edges: &[CallGraphEdge],
    wired: &FxHashSet<(CallSiteId, u32, FnId)>,
    result: &mut AnalysisResult,
) {
    for edge in call_edges {
        let cs = program
            .symbols
            .call_sites
            .get(edge.call_site.0 as usize)
            .filter(|c| c.id == edge.call_site)
            .expect("call site for edge");
        let callee = program.symbols.function(edge.callee);
        for (i, formal) in callee.params.iter().enumerate() {
            let idx = i as u32;
            if wired.contains(&(edge.call_site, idx, edge.callee)) {
                if let Some(actual) = cs.var_args.iter().find(|(j, _)| *j == idx).map(|(_, v)| *v) {
                    result.arg_flow_edges.push(ArgFlowEdge {
                        call_site: edge.call_site,
                        arg_index: idx,
                        actual_var: Some(actual),
                        actual_fn: None,
                        formal: *formal,
                    });
                } else if let Some(fn_id) =
                    cs.fn_args.iter().find(|(j, _)| *j == idx).map(|(_, f)| *f)
                {
                    result.arg_flow_edges.push(ArgFlowEdge {
                        call_site: edge.call_site,
                        arg_index: idx,
                        actual_var: None,
                        actual_fn: Some(fn_id),
                        formal: *formal,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_by_name_classifies_plain_identifiers() {
        let mk = |callee_name: &str, callee_var: Option<u32>, is_direct: bool| trace_ir::CallSite {
            id: trace_ir::CallSiteId(0),
            caller: trace_ir::FnId(0),
            callee_name: callee_name.into(),
            callee_var: callee_var.map(trace_ir::VarId),
            var_args: Vec::new(),
            fn_args: Vec::new(),
            span: trace_ir::Span {
                file: trace_ir::FileId(0),
                line: 1,
                col: 1,
            },
            is_direct,
        };
        assert!(direct_by_name(&mk("OsalMemCalloc", None, false)));
        assert!(direct_by_name(&mk("f", None, true)));
        assert!(!direct_by_name(&mk("ops->Dispatch", None, false)));
        assert!(!direct_by_name(&mk("obj.fn", None, false)));
        assert!(!direct_by_name(&mk("fp", Some(3), false)));
    }
}

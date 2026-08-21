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
    pub wired_arg_flow: FxHashSet<(CallSiteId, u32)>,
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
    let mut wired_arg_flow: FxHashSet<(CallSiteId, u32)> = FxHashSet::default();

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
        if cs.is_direct {
            if let Some(callee) = program
                .symbols
                .resolve_function_in_scope(&cs.callee_name, Some(cs.span.file))
            {
                call_edges.push(CallGraphEdge {
                    call_site: cs.id,
                    caller: cs.caller,
                    callee,
                    resolution: ResolutionKind::Direct,
                });
                wire_params(pag, program, cs, callee, &mut st, &mut wired_arg_flow);
            }
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
                        } else if let Some(mem) = st.memory_pts.get(&loc).cloned() {
                            propagate_pts(&mut st, dst, &mem);
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
            let src_pts = pts_for_var_or_loc(pag, &st.pts, node);
            if !src_pts.is_empty() {
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
                        if let Some(field_loc) = pag.ensure_field_loc(program, *loc, field) {
                            let mut targets = FxHashSet::default();
                            targets.insert(field_loc);
                            if let Some(summary) = pag.summary_for_field_loc(field_loc) {
                                targets.insert(summary);
                            }
                            propagate_pts(&mut st, dst, &targets);
                            for fl in targets {
                                if let Some(mem) = st.memory_pts.get(&fl).cloned() {
                                    propagate_pts(&mut st, dst, &mem);
                                }
                            }
                        }
                    }
                } else if let PagNodeKind::Var(base_var) = pag.nodes[src.0 as usize].kind {
                    if let Some(summary) =
                        pag.ensure_field_summary_for_var(program, base_var, field)
                    {
                        propagate_pts(&mut st, dst, &FxHashSet::from_iter([summary]));
                        if let Some(mem) = st.memory_pts.get(&summary).cloned() {
                            propagate_pts(&mut st, dst, &mem);
                        }
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

fn apply_store(pag: &Pag, idx: usize, st: &mut SolverState) {
    let c = &pag.constraints[idx];
    let dst_pts = st.pts.get(&c.dst).cloned().unwrap_or_default();
    if dst_pts.is_empty() {
        return;
    }
    let src_pts = pts_for_var_or_loc(pag, &st.pts, c.src);
    if src_pts.is_empty() {
        return;
    }
    for loc in dst_pts {
        if fn_for_loc(pag, loc).is_some() {
            continue;
        }
        let mut changed = false;
        let entry = st.memory_pts.entry(loc).or_default();
        let before = entry.len();
        for l in &src_pts {
            entry.insert(*l);
        }
        if entry.len() > before {
            changed = true;
        }
        let mut summary_loc = None;
        if let Some(summary) = pag.summary_for_field_loc(loc) {
            summary_loc = Some(summary);
            let summary_entry = st.memory_pts.entry(summary).or_default();
            let before_summary = summary_entry.len();
            for l in &src_pts {
                summary_entry.insert(*l);
            }
            if summary_entry.len() > before_summary {
                changed = true;
            }
        }
        if changed {
            st.requeue_loc(loc);
            if let Some(summary) = summary_loc {
                st.requeue_loc(summary);
            }
        }
    }
}

fn pts_for_var_or_loc(
    pag: &Pag,
    pts: &FxHashMap<PagNodeId, FxHashSet<LocId>>,
    node: PagNodeId,
) -> FxHashSet<LocId> {
    let mut set = pts.get(&node).cloned().unwrap_or_default();
    if let PagNodeKind::Var(v) = pag.nodes[node.0 as usize].kind {
        if let Some(&loc) = pag.var_location.get(&v) {
            set.insert(loc);
        }
    }
    set
}

fn wire_params(
    pag: &Pag,
    program: &Program,
    cs: &trace_ir::CallSite,
    callee: FnId,
    st: &mut SolverState,
    wired: &mut FxHashSet<(CallSiteId, u32)>,
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
            wired.insert((cs.id, idx));
        } else if let Some(fn_id) = cs.fn_args.iter().find(|(j, _)| *j == idx).map(|(_, f)| *f) {
            let formal_node = pag.var_node.get(formal).copied().expect("formal var node");
            if let Some(&fn_loc) = pag.fn_locations.get(&fn_id) {
                add_pts(st, formal_node, fn_loc);
            }
            wired.insert((cs.id, idx));
        }
    }
}

fn propagate_pts(st: &mut SolverState, dst: PagNodeId, src_pts: &FxHashSet<LocId>) {
    let mut changed = false;
    let entry = st.pts.entry(dst).or_default();
    for &loc in src_pts {
        if entry.insert(loc) {
            st.loc_nodes.entry(loc).or_default().insert(dst);
            changed = true;
        }
    }
    if changed {
        st.push(dst);
    }
}

fn add_pts(st: &mut SolverState, node: PagNodeId, loc: LocId) {
    let entry = st.pts.entry(node).or_default();
    if entry.insert(loc) {
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
    wired: &FxHashSet<(CallSiteId, u32)>,
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
            if wired.contains(&(edge.call_site, idx)) {
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

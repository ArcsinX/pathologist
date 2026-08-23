use crate::constraints::{AbstractLocation, Constraint, ConstraintKind, LocKind};
use indexmap::IndexMap;
use rustc_hash::{FxHashMap, FxHashSet};
use trace_ir::{
    FieldId, FlowConstraint, FnId, LocId, PagNodeId, Program, ReturnFlow, StorageClass, VarId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PagNodeKind {
    Var(VarId),
    Loc(LocId),
    CallTarget(trace_ir::CallSiteId),
}

#[derive(Debug, Clone)]
pub struct PagNode {
    pub id: PagNodeId,
    pub kind: PagNodeKind,
}

/// Adjacency lists for constraint propagation (built once after PAG construction).
#[derive(Debug, Default)]
pub struct SolverIndices {
    pub copy_src: FxHashMap<PagNodeId, Vec<usize>>,
    pub addr_of_dst: FxHashMap<PagNodeId, Vec<usize>>,
    pub load_src: FxHashMap<PagNodeId, Vec<usize>>,
    pub store_dst: FxHashMap<PagNodeId, Vec<usize>>,
    pub store_src: FxHashMap<PagNodeId, Vec<usize>>,
    pub gep_src: FxHashMap<PagNodeId, Vec<usize>>,
    pub indirect_by_target: FxHashMap<PagNodeId, Vec<trace_ir::CallSiteId>>,
}

/// Maximum nesting depth for instance-sensitive field locations. Deeper
/// accesses fold into instance-insensitive summaries (see `ensure_field_loc`).
const FIELD_LOC_DEPTH_CAP: u8 = 4;

#[derive(Debug, Default)]
pub struct Pag {
    pub nodes: Vec<PagNode>,
    pub constraints: Vec<Constraint>,
    pub locations: Vec<AbstractLocation>,
    pub var_node: IndexMap<VarId, PagNodeId>,
    pub loc_node: IndexMap<LocId, PagNodeId>,
    pub call_targets: IndexMap<trace_ir::CallSiteId, PagNodeId>,
    pub fn_locations: IndexMap<FnId, LocId>,
    pub var_location: IndexMap<VarId, LocId>,
    /// Field abstract locations keyed by (parent object location, field id).
    pub field_loc: IndexMap<(LocId, FieldId), LocId>,
    /// Nesting depth of each synthesized field location (var-rooted = 0
    /// children start at 1). Bounds recursive `obj->next->next->...`
    /// location synthesis once interprocedural flow reaches chained structs.
    pub field_depth: FxHashMap<LocId, u8>,
    /// Per-(struct type, field) summary location for instance-insensitive field flow.
    pub field_summary: IndexMap<(trace_ir::TypeId, FieldId), LocId>,
    pub field_loc_to_summary: IndexMap<LocId, LocId>,
    /// Fn locations parked into an array var by `ArrayFnMember` inits
    /// (`{ {.., Fn}, .. }`); reachable through any element field load.
    pub array_fn_members: FxHashMap<VarId, Vec<LocId>>,
    pub indices: SolverIndices,
}

impl Pag {
    pub fn build(program: &Program) -> Self {
        let mut pag = Self::default();
        pag.build_variables(program);
        pag.build_function_locations(program);
        pag.build_flow_constraints(program);
        pag.build_call_constraints(program);
        pag.build_indices(program);
        pag
    }

    fn alloc_node(&mut self, kind: PagNodeKind) -> PagNodeId {
        let id = PagNodeId(self.nodes.len() as u32);
        self.nodes.push(PagNode { id, kind });
        id
    }

    fn alloc_loc(&mut self, loc: AbstractLocation) -> LocId {
        let id = loc.id;
        let node_id = self.alloc_node(PagNodeKind::Loc(id));
        self.loc_node.insert(id, node_id);
        self.locations.push(loc);
        id
    }

    pub fn var_node_id(&mut self, var: VarId) -> PagNodeId {
        if let Some(&id) = self.var_node.get(&var) {
            return id;
        }
        let id = self.alloc_node(PagNodeKind::Var(var));
        self.var_node.insert(var, id);
        id
    }

    fn build_variables(&mut self, program: &Program) {
        for var in &program.symbols.variables {
            self.var_node_id(var.id);
            if !matches!(
                var.storage,
                StorageClass::Global | StorageClass::FileStatic | StorageClass::FnStatic
            ) {
                continue;
            }
            let kind = match var.storage {
                StorageClass::Global => LocKind::Global,
                StorageClass::FileStatic => LocKind::FileStatic,
                StorageClass::FnStatic => LocKind::FnStatic,
                StorageClass::Local | StorageClass::Param => LocKind::Local,
            };
            let loc_id = LocId(self.locations.len() as u32);
            self.alloc_loc(AbstractLocation {
                id: loc_id,
                kind,
                var: Some(var.id),
                fn_id: var.fn_id,
                field: None,
                type_id: var.type_id,
                desc: var.name.clone(),
            });
            self.var_location.insert(var.id, loc_id);
        }
    }

    pub fn ensure_var_loc(&mut self, program: &Program, var: VarId) -> Option<LocId> {
        if let Some(&loc) = self.var_location.get(&var) {
            return Some(loc);
        }
        let v = program.symbols.variable_by_id(var)?;
        let kind = match v.storage {
            StorageClass::Global => LocKind::Global,
            StorageClass::FileStatic => LocKind::FileStatic,
            StorageClass::FnStatic => LocKind::FnStatic,
            StorageClass::Local | StorageClass::Param => LocKind::Local,
        };
        let loc_id = LocId(self.locations.len() as u32);
        self.alloc_loc(AbstractLocation {
            id: loc_id,
            kind,
            var: Some(var),
            fn_id: v.fn_id,
            field: None,
            type_id: v.type_id,
            desc: v.name.clone(),
        });
        self.var_location.insert(var, loc_id);
        Some(loc_id)
    }

    fn build_indices(&mut self, program: &Program) {
        for (i, c) in self.constraints.iter().enumerate() {
            match c.kind {
                ConstraintKind::Copy => {
                    self.indices.copy_src.entry(c.src).or_default().push(i);
                }
                ConstraintKind::AddrOf => {
                    self.indices.addr_of_dst.entry(c.dst).or_default().push(i);
                }
                ConstraintKind::Load => {
                    self.indices.load_src.entry(c.src).or_default().push(i);
                }
                ConstraintKind::Store => {
                    self.indices.store_dst.entry(c.dst).or_default().push(i);
                    self.indices.store_src.entry(c.src).or_default().push(i);
                }
                ConstraintKind::Gep => {
                    self.indices.gep_src.entry(c.src).or_default().push(i);
                }
            }
        }
        for cs in &program.symbols.call_sites {
            if cs.is_direct {
                continue;
            }
            if let Some(&target) = self.call_targets.get(&cs.id) {
                self.indices
                    .indirect_by_target
                    .entry(target)
                    .or_default()
                    .push(cs.id);
            }
        }
    }

    fn alloc_field_loc(
        &mut self,
        parent_loc: LocId,
        field: FieldId,
        field_type: trace_ir::TypeId,
        name: &str,
    ) -> LocId {
        if let Some(&loc) = self.field_loc.get(&(parent_loc, field)) {
            return loc;
        }
        let base_var = self.locations[parent_loc.0 as usize].var;
        let loc_id = LocId(self.locations.len() as u32);
        self.alloc_loc(AbstractLocation {
            id: loc_id,
            kind: LocKind::Field,
            var: base_var,
            fn_id: None,
            field: Some(field),
            type_id: field_type,
            desc: name.to_string(),
        });
        let depth = self.field_depth.get(&parent_loc).copied().unwrap_or(0) + 1;
        self.field_depth.insert(loc_id, depth);
        self.field_loc.insert((parent_loc, field), loc_id);
        loc_id
    }

    /// Instance-sensitive child location for `parent.field`. Past
    /// [`FIELD_LOC_DEPTH_CAP`], further nesting is folded into the
    /// instance-insensitive summary: unbounded recursive synthesis (linked
    /// structures reached through interprocedural flow) would otherwise
    /// diverge, while summaries stay bounded by (type, field).
    pub fn ensure_field_loc(
        &mut self,
        program: &Program,
        parent_loc: LocId,
        field: FieldId,
    ) -> Option<LocId> {
        if let Some(&loc) = self.field_loc.get(&(parent_loc, field)) {
            return Some(loc);
        }
        let parent_type = struct_type_for_loc(self, program, parent_loc)?;
        let field_layout = program.types.get(parent_type).layout.fields.get(&field)?;
        if self.field_depth.get(&parent_loc).copied().unwrap_or(0) >= FIELD_LOC_DEPTH_CAP {
            return Some(self.ensure_field_summary_loc(
                program,
                parent_type,
                field,
                field_layout.type_id,
                &field_layout.name,
            ));
        }
        let field_loc =
            self.alloc_field_loc(parent_loc, field, field_layout.type_id, &field_layout.name);
        let summary = self.ensure_field_summary_loc(
            program,
            parent_type,
            field,
            field_layout.type_id,
            &field_layout.name,
        );
        self.field_loc_to_summary.insert(field_loc, summary);
        Some(field_loc)
    }

    fn ensure_field_summary_loc(
        &mut self,
        program: &Program,
        struct_type: trace_ir::TypeId,
        field: FieldId,
        field_type: trace_ir::TypeId,
        name: &str,
    ) -> LocId {
        if let Some(&loc) = self.field_summary.get(&(struct_type, field)) {
            return loc;
        }
        let struct_name = match &program.types.get(struct_type).desc {
            trace_ir::TypeDesc::Struct { name, .. } => name.clone(),
            trace_ir::TypeDesc::Union { name, .. } => name.clone(),
            _ => format!("type{}", struct_type.0),
        };
        let loc_id = LocId(self.locations.len() as u32);
        self.alloc_loc(AbstractLocation {
            id: loc_id,
            kind: LocKind::FieldSummary,
            var: None,
            fn_id: None,
            field: Some(field),
            type_id: field_type,
            desc: format!("summary:{struct_name}.{name}"),
        });
        self.field_summary.insert((struct_type, field), loc_id);
        loc_id
    }

    pub fn summary_for_field_loc(&self, field_loc: LocId) -> Option<LocId> {
        self.field_loc_to_summary.get(&field_loc).copied()
    }

    /// Instance-insensitive summary location for `(struct type of var, field)`.
    pub fn ensure_field_summary_for_var(
        &mut self,
        program: &Program,
        var: trace_ir::VarId,
        field: FieldId,
    ) -> Option<LocId> {
        let v = program.symbols.variable_by_id(var)?;
        let struct_type = struct_type_from_type_id(program, v.type_id)?;
        let field_layout = program.types.get(struct_type).layout.fields.get(&field)?;
        Some(self.ensure_field_summary_loc(
            program,
            struct_type,
            field,
            field_layout.type_id,
            &field_layout.name,
        ))
    }

    pub fn field_loc_for_parent(&self, parent_loc: LocId, field: FieldId) -> Option<LocId> {
        self.field_loc.get(&(parent_loc, field)).copied()
    }

    fn build_function_locations(&mut self, program: &Program) {
        for func in &program.symbols.functions {
            if self.fn_locations.contains_key(&func.id) {
                continue;
            }
            let loc_id = LocId(self.locations.len() as u32);
            self.alloc_loc(AbstractLocation {
                id: loc_id,
                kind: LocKind::Function,
                var: None,
                fn_id: Some(func.id),
                field: None,
                type_id: func.return_type,
                desc: func.name.clone(),
            });
            self.fn_locations.insert(func.id, loc_id);
        }
    }

    fn build_flow_constraints(&mut self, program: &Program) {
        for flow in &program.flow {
            match flow {
                FlowConstraint::Copy { dst, src } => {
                    let dst_n = self.var_node_id(*dst);
                    let src_n = self.var_node_id(*src);
                    self.add_copy(dst_n, src_n);
                }
                FlowConstraint::AddrOfVar { dst, src } => {
                    let dst_n = self.var_node_id(*dst);
                    if let Some(loc) = self.ensure_var_loc(program, *src) {
                        let loc_n = self.loc_node[&loc];
                        self.add_addr_of(dst_n, loc_n);
                    }
                }
                FlowConstraint::AddrOfFn { dst, callee } => {
                    let dst_n = self.var_node_id(*dst);
                    // `callee` was resolved in TU scope during lowering and
                    // remapped at merge; a global name lookup here could bind
                    // an unrelated same-name function (e.g. file-`static`s).
                    if let Some(&fn_loc) = self.fn_locations.get(callee) {
                        let loc_n = self.loc_node[&fn_loc];
                        self.add_addr_of(dst_n, loc_n);
                    }
                }
                FlowConstraint::Load { dst, src } => {
                    let dst_n = self.var_node_id(*dst);
                    let src_n = self.var_node_id(*src);
                    self.add_load(dst_n, src_n);
                }
                FlowConstraint::Store { dst, src } => {
                    let dst_n = self.var_node_id(*dst);
                    let src_n = self.var_node_id(*src);
                    self.add_store(dst_n, src_n);
                }
                FlowConstraint::GepField { dst, base, field } => {
                    let dst_n = self.var_node_id(*dst);
                    let base_n = self.var_node_id(*base);
                    self.add_gep(dst_n, base_n, *field);
                }
                FlowConstraint::ArrayFnMember { array, callee } => {
                    let array_n = self.var_node_id(*array);
                    // Trust the merge-remapped FnId (see AddrOfFn above).
                    if let Some(&fn_loc) = self.fn_locations.get(callee) {
                        let loc_n = self.loc_node[&fn_loc];
                        self.add_addr_of(array_n, loc_n);
                        // Also record for element-field loads through
                        // pointers to the array (order-independent).
                        self.array_fn_members.entry(*array).or_default().push(fn_loc);
                    }
                }
                FlowConstraint::CallReturn { dst, callee_name } => {
                    let dst_n = self.var_node_id(*dst);
                    let file = program
                        .symbols
                        .variable(*dst)
                        .fn_id
                        .map(|f| program.symbols.function(f).file);
                    // May-approximation: a merged name may bind to the
                    // query file's `static` def, the external def, or both.
                    let mut visited = FxHashSet::default();
                    for callee in program
                        .symbols
                        .resolve_function_candidates(callee_name, file)
                    {
                        self.expand_return_flows(program, dst_n, callee, &mut visited);
                    }
                }
            }
        }
    }

    fn expand_return_flows(
        &mut self,
        program: &Program,
        dst: PagNodeId,
        callee: FnId,
        visited: &mut FxHashSet<FnId>,
    ) {
        if !visited.insert(callee) {
            return;
        }
        let Some(flows) = program.fn_returns.get(&callee) else {
            return;
        };
        for flow in flows.clone() {
            match flow {
                ReturnFlow::AddrOfVar { src } => {
                    if let Some(loc) = self.ensure_var_loc(program, src) {
                        let loc_n = self.loc_node[&loc];
                        self.add_addr_of(dst, loc_n);
                    }
                }
                ReturnFlow::AddrOfFn { callee: fn_id } => {
                    // Trust the merge-remapped FnId (see AddrOfFn above).
                    if let Some(&fn_loc) = self.fn_locations.get(&fn_id) {
                        let loc_n = self.loc_node[&fn_loc];
                        self.add_addr_of(dst, loc_n);
                    }
                }
                ReturnFlow::Copy { src } => {
                    let src_n = self.var_node_id(src);
                    self.add_copy(dst, src_n);
                }
                ReturnFlow::Call { callee_name } => {
                    let file = program.symbols.function(callee).file;
                    for inner in program
                        .symbols
                        .resolve_function_candidates(&callee_name, Some(file))
                    {
                        self.expand_return_flows(program, dst, inner, visited);
                    }
                }
            }
        }
    }

    fn build_call_constraints(&mut self, program: &Program) {
        let mut fn_vars: FxHashMap<FnId, FxHashMap<String, VarId>> = FxHashMap::default();
        for var in &program.symbols.variables {
            if let Some(fn_id) = var.fn_id {
                fn_vars
                    .entry(fn_id)
                    .or_default()
                    .insert(var.name.clone(), var.id);
            }
        }
        for cs in &program.symbols.call_sites {
            if cs.is_direct {
                continue;
            }
            if let Some(var) = cs.callee_var {
                let call_target = self.call_target_node(cs.id);
                let var_node = self.var_node_id(var);
                if cs.callee_name.contains("->") || cs.callee_name.contains('.') {
                    self.add_copy(call_target, var_node);
                } else {
                    self.add_load(call_target, var_node);
                }
            } else if program.symbols.resolve_function(&cs.callee_name).is_none() {
                let call_target = self.call_target_node(cs.id);
                if let Some(v) = lookup_var_in_fn(&fn_vars, program, &cs.callee_name, cs.caller) {
                    let var_node = self.var_node_id(v);
                    self.add_load(call_target, var_node);
                }
            }
        }
    }

    pub fn call_target_node(&mut self, cs: trace_ir::CallSiteId) -> PagNodeId {
        if let Some(&id) = self.call_targets.get(&cs) {
            return id;
        }
        let id = self.alloc_node(PagNodeKind::CallTarget(cs));
        self.call_targets.insert(cs, id);
        id
    }

    pub fn add_copy(&mut self, dst: PagNodeId, src: PagNodeId) {
        self.constraints.push(Constraint {
            kind: crate::constraints::ConstraintKind::Copy,
            dst,
            src,
            field: None,
        });
    }

    fn add_addr_of(&mut self, dst: PagNodeId, loc_node: PagNodeId) {
        self.constraints.push(Constraint {
            kind: crate::constraints::ConstraintKind::AddrOf,
            dst,
            src: loc_node,
            field: None,
        });
    }

    fn add_load(&mut self, dst: PagNodeId, src: PagNodeId) {
        self.constraints.push(Constraint {
            kind: crate::constraints::ConstraintKind::Load,
            dst,
            src,
            field: None,
        });
    }

    fn add_store(&mut self, dst: PagNodeId, src: PagNodeId) {
        self.constraints.push(Constraint {
            kind: crate::constraints::ConstraintKind::Store,
            dst,
            src,
            field: None,
        });
    }

    pub fn add_gep(&mut self, dst: PagNodeId, base: PagNodeId, field: FieldId) {
        self.constraints.push(Constraint {
            kind: crate::constraints::ConstraintKind::Gep,
            dst,
            src: base,
            field: Some(field),
        });
    }
}

fn struct_type_for_loc(pag: &Pag, program: &Program, loc: LocId) -> Option<trace_ir::TypeId> {
    if let Some(var) = pag.locations[loc.0 as usize].var {
        let mut type_id = program.symbols.variable_by_id(var)?.type_id;
        for _ in 0..4 {
            match &program.types.get(type_id).desc {
                trace_ir::TypeDesc::Ptr(inner) => {
                    type_id = match inner.as_ref() {
                        trace_ir::TypeDesc::Struct { name, .. } => program
                            .types
                            .type_id_by_tag(name, trace_ir::TypeKind::Struct)
                            .unwrap_or_else(|| program.types.resolve_type_id(inner)),
                        trace_ir::TypeDesc::Union { name, .. } => program
                            .types
                            .type_id_by_tag(name, trace_ir::TypeKind::Union)
                            .unwrap_or_else(|| program.types.resolve_type_id(inner)),
                        _ => program.types.resolve_type_id(inner),
                    };
                }
                // Arrays of structs: resolve fields against the element type.
                trace_ir::TypeDesc::Array { elem, .. } => {
                    type_id = program.types.resolve_type_id(inner_or_elem(elem));
                }
                trace_ir::TypeDesc::Struct { .. } | trace_ir::TypeDesc::Union { .. } => {
                    return Some(type_id);
                }
                _ => return Some(type_id),
            }
        }
        return Some(type_id);
    }
    type_id_of_loc(pag, program, loc)
}

fn inner_or_elem(desc: &trace_ir::TypeDesc) -> &trace_ir::TypeDesc {
    match desc {
        trace_ir::TypeDesc::Ptr(inner) | trace_ir::TypeDesc::Array { elem: inner, .. } => inner,
        other => other,
    }
}

fn type_id_of_loc(pag: &Pag, program: &Program, loc: LocId) -> Option<trace_ir::TypeId> {
    let mut type_id = pag.locations[loc.0 as usize].type_id;
    for _ in 0..4 {
        match &program.types.get(type_id).desc {
            trace_ir::TypeDesc::Ptr(inner) => {
                type_id = program.types.resolve_type_id(inner);
            }
            _ => break,
        }
    }
    Some(type_id)
}

fn struct_type_from_type_id(
    program: &Program,
    mut type_id: trace_ir::TypeId,
) -> Option<trace_ir::TypeId> {
    for _ in 0..6 {
        match &program.types.get(type_id).desc {
            trace_ir::TypeDesc::Ptr(inner) => {
                type_id = match inner.as_ref() {
                    trace_ir::TypeDesc::Struct { name, .. } => program
                        .types
                        .type_id_by_tag(name, trace_ir::TypeKind::Struct)
                        .unwrap_or_else(|| program.types.resolve_type_id(inner)),
                    trace_ir::TypeDesc::Union { name, .. } => program
                        .types
                        .type_id_by_tag(name, trace_ir::TypeKind::Union)
                        .unwrap_or_else(|| program.types.resolve_type_id(inner)),
                    _ => program.types.resolve_type_id(inner),
                };
            }
            // Arrays of structs: resolve fields against the element type.
            trace_ir::TypeDesc::Array { elem, .. } => {
                type_id = program.types.resolve_type_id(inner_or_elem(elem));
            }
            trace_ir::TypeDesc::Struct { .. } | trace_ir::TypeDesc::Union { .. } => {
                return Some(type_id);
            }
            _ => return None,
        }
    }
    None
}

fn lookup_var_in_fn(
    fn_vars: &FxHashMap<FnId, FxHashMap<String, VarId>>,
    program: &Program,
    name: &str,
    caller: FnId,
) -> Option<VarId> {
    fn_vars
        .get(&caller)
        .and_then(|m| m.get(name).copied())
        .or_else(|| program.symbols.global_by_name.get(name).copied())
}

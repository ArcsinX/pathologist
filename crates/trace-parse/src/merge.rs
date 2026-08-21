use std::collections::HashMap;
use std::path::PathBuf;
use trace_ir::{
    CallSite, CallSiteId, FlowConstraint, FnId, Function, Program, ReturnFlow, TypeDesc, TypeId,
    VarId, Variable,
};

/// Per-file indexing result merged into a single [`Program`].
#[derive(Debug, Clone, Default)]
pub struct UnitIndex {
    pub path: PathBuf,
    pub types: trace_ir::TypeTable,
    pub functions: Vec<Function>,
    pub variables: Vec<Variable>,
    pub call_sites: Vec<CallSite>,
    pub flow: Vec<FlowConstraint>,
    pub fn_returns: std::collections::HashMap<FnId, Vec<ReturnFlow>>,
    pub diagnostics: Vec<trace_ir::Diagnostic>,
    pub anon_type_counter: u32,
}

pub fn merge_unit_index(program: &mut Program, unit: UnitIndex) {
    program.diagnostics.extend(unit.diagnostics);
    program.anon_type_counter = program.anon_type_counter.max(unit.anon_type_counter);

    let type_map = merge_types(&mut program.types, &unit.types);
    let file_id = program.symbols.add_file(unit.path);

    let mut fn_map: HashMap<FnId, FnId> = HashMap::new();
    for func in unit.functions {
        let old_id = func.id;
        let new_id = program.symbols.alloc_fn_id();
        let mut f = func;
        f.id = new_id;
        f.file = file_id;
        f.return_type = remap_type(f.return_type, &type_map);
        let merged = program.symbols.add_function(f);
        fn_map.insert(old_id, merged);
    }

    let mut var_map: HashMap<VarId, VarId> = HashMap::new();
    for var in unit.variables {
        let new_id = program.symbols.alloc_var_id();
        let mut v = var;
        let old = v.id;
        v.id = new_id;
        v.type_id = remap_type(v.type_id, &type_map);
        v.fn_id = v.fn_id.and_then(|id| fn_map.get(&id).copied());
        program.symbols.add_variable(v);
        var_map.insert(old, new_id);
    }

    let fn_index: HashMap<FnId, usize> = program
        .symbols
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.id, i))
        .collect();
    for &merged_id in fn_map.values() {
        let Some(&idx) = fn_index.get(&merged_id) else {
            continue;
        };
        let func = &mut program.symbols.functions[idx];
        func.params = func
            .params
            .iter()
            .filter_map(|v| var_map.get(v).copied())
            .collect();
        func.locals = func
            .locals
            .iter()
            .filter_map(|v| var_map.get(v).copied())
            .collect();
    }

    let mut call_map: HashMap<CallSiteId, CallSiteId> = HashMap::new();
    for cs in unit.call_sites {
        let new_id = program.symbols.alloc_call_id();
        let old = cs.id;
        let mut site = cs;
        site.id = new_id;
        site.caller = fn_map.get(&site.caller).copied().unwrap_or(site.caller);
        site.callee_var = site.callee_var.and_then(|v| var_map.get(&v).copied());
        site.var_args = site
            .var_args
            .into_iter()
            .filter_map(|(i, v)| var_map.get(&v).map(|nv| (i, *nv)))
            .collect();
        site.fn_args = site
            .fn_args
            .into_iter()
            .filter_map(|(i, f)| fn_map.get(&f).map(|nf| (i, *nf)))
            .collect();
        site.span.file = file_id;
        program.symbols.call_sites.push(site);
        call_map.insert(old, new_id);
    }

    for flow in unit.flow {
        program.flow.push(remap_flow(flow, &fn_map, &var_map));
    }

    for (old_fn, flows) in unit.fn_returns {
        let Some(&new_fn) = fn_map.get(&old_fn) else {
            continue;
        };
        let remapped: Vec<ReturnFlow> = flows
            .into_iter()
            .map(|f| remap_return_flow(f, &fn_map, &var_map))
            .collect();
        program
            .fn_returns
            .entry(new_fn)
            .or_default()
            .extend(remapped);
    }
}

fn remap_type(id: TypeId, map: &HashMap<TypeId, TypeId>) -> TypeId {
    map.get(&id).copied().unwrap_or(id)
}

fn merge_types(
    dst: &mut trace_ir::TypeTable,
    src: &trace_ir::TypeTable,
) -> HashMap<TypeId, TypeId> {
    let mut map = HashMap::new();
    for info in src.all() {
        let new_id = match &info.desc {
            TypeDesc::Struct { name, fields } if !fields.is_empty() => {
                dst.compute_struct_layout(name.clone(), fields.clone())
            }
            TypeDesc::Union { name, fields } if !fields.is_empty() => {
                dst.compute_union_layout(name.clone(), fields.clone())
            }
            other => dst.intern(other.clone()),
        };
        map.insert(info.id, new_id);
    }
    map
}

fn remap_flow(
    flow: FlowConstraint,
    fn_map: &HashMap<FnId, FnId>,
    var_map: &HashMap<VarId, VarId>,
) -> FlowConstraint {
    let rv = |v: VarId| var_map.get(&v).copied().unwrap_or(v);
    let rf = |f: FnId| fn_map.get(&f).copied().unwrap_or(f);
    match flow {
        FlowConstraint::Copy { dst, src } => FlowConstraint::Copy {
            dst: rv(dst),
            src: rv(src),
        },
        FlowConstraint::AddrOfVar { dst, src } => FlowConstraint::AddrOfVar {
            dst: rv(dst),
            src: rv(src),
        },
        FlowConstraint::AddrOfFn { dst, callee } => FlowConstraint::AddrOfFn {
            dst: rv(dst),
            callee: rf(callee),
        },
        FlowConstraint::Load { dst, src } => FlowConstraint::Load {
            dst: rv(dst),
            src: rv(src),
        },
        FlowConstraint::Store { dst, src } => FlowConstraint::Store {
            dst: rv(dst),
            src: rv(src),
        },
        FlowConstraint::GepField { dst, base, field } => FlowConstraint::GepField {
            dst: rv(dst),
            base: rv(base),
            field,
        },
        FlowConstraint::ArrayFnMember { array, callee } => FlowConstraint::ArrayFnMember {
            array: rv(array),
            callee: rf(callee),
        },
        FlowConstraint::CallReturn { dst, callee_name } => FlowConstraint::CallReturn {
            dst: rv(dst),
            callee_name,
        },
    }
}

fn remap_return_flow(
    flow: ReturnFlow,
    fn_map: &HashMap<FnId, FnId>,
    var_map: &HashMap<VarId, VarId>,
) -> ReturnFlow {
    let rv = |v: VarId| var_map.get(&v).copied().unwrap_or(v);
    let rf = |f: FnId| fn_map.get(&f).copied().unwrap_or(f);
    match flow {
        ReturnFlow::AddrOfVar { src } => ReturnFlow::AddrOfVar { src: rv(src) },
        ReturnFlow::AddrOfFn { callee } => ReturnFlow::AddrOfFn { callee: rf(callee) },
        ReturnFlow::Copy { src } => ReturnFlow::Copy { src: rv(src) },
        ReturnFlow::Call { callee_name } => ReturnFlow::Call { callee_name },
    }
}

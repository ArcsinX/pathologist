use trace_ir::Program;

pub fn apply_call_summary(_cs: &trace_ir::CallSite, _callee: trace_ir::FnId, _program: &Program) {
    let _name = &_program.symbols.function(_callee).name;
    match _name.as_str() {
        "malloc" | "calloc" | "realloc" | "free" | "memcpy" | "memmove" => {}
        _ => {}
    }
}

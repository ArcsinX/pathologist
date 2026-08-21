use crate::{FieldId, FnId, VarId};

/// Abstract value returned from a function body (may-analysis: union all return sites).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnFlow {
    AddrOfVar {
        src: VarId,
    },
    AddrOfFn {
        callee: FnId,
    },
    Copy {
        src: VarId,
    },
    /// Return value is whatever `callee` returns (expanded after all TUs are merged).
    Call {
        callee_name: String,
    },
}

/// Statement-level flow facts extracted during IR lowering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowConstraint {
    /// `dst = src` (pointer assignment)
    Copy { dst: VarId, src: VarId },
    /// `dst = &var`
    AddrOfVar { dst: VarId, src: VarId },
    /// `dst = &function`
    AddrOfFn { dst: VarId, callee: FnId },
    /// `dst = *src`
    Load { dst: VarId, src: VarId },
    /// `*dst = src` (store through pointer)
    Store { dst: VarId, src: VarId },
    /// `dst = src->field` or struct field address
    GepField {
        dst: VarId,
        base: VarId,
        field: FieldId,
    },
    /// Function-pointer array initializer: any subscript may target any listed callee.
    ArrayFnMember { array: VarId, callee: FnId },
    /// `dst = callee()` — callee resolved by name after all TUs are merged.
    CallReturn { dst: VarId, callee_name: String },
}

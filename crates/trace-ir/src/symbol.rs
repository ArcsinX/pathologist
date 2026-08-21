use crate::{CallSiteId, FileId, FnId, Span, TypeId, VarId};
use indexmap::IndexMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Linkage {
    External,
    Internal,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageClass {
    Global,
    FileStatic,
    FnStatic,
    Param,
    Local,
}

#[derive(Debug, Clone)]
pub struct Variable {
    pub id: VarId,
    pub name: String,
    pub type_id: TypeId,
    pub storage: StorageClass,
    pub fn_id: Option<FnId>,
    pub param_index: Option<u32>,
    pub span: Span,
    pub is_pointer: bool,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub id: FnId,
    pub name: String,
    pub linkage: Linkage,
    pub return_type: TypeId,
    pub params: Vec<VarId>,
    pub locals: Vec<VarId>,
    pub span: Span,
    pub file: FileId,
    pub is_defined: bool,
}

#[derive(Debug, Clone)]
pub struct CallSite {
    pub id: crate::CallSiteId,
    pub caller: FnId,
    pub callee_name: String,
    pub callee_var: Option<VarId>,
    pub var_args: Vec<(u32, VarId)>,
    pub fn_args: Vec<(u32, FnId)>,
    pub span: Span,
    pub is_direct: bool,
}

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub id: FileId,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    pub files: Vec<FileInfo>,
    pub functions: Vec<Function>,
    pub variables: Vec<Variable>,
    pub call_sites: Vec<CallSite>,
    pub fn_by_name: IndexMap<String, FnId>,
    pub global_by_name: IndexMap<String, VarId>,
    next_fn: u32,
    next_var: u32,
    next_call: u32,
}

impl SymbolTable {
    pub fn add_file(&mut self, path: PathBuf) -> FileId {
        let id = FileId(self.files.len() as u32);
        self.files.push(FileInfo { id, path });
        id
    }

    pub fn add_function(&mut self, func: Function) -> FnId {
        if func.linkage == Linkage::External {
            if let Some(existing_id) = self.fn_by_name.get(&func.name).copied() {
                if let Some(existing) = self.functions.iter_mut().find(|f| f.id == existing_id) {
                    if func.is_defined {
                        existing.is_defined = true;
                        existing.file = func.file;
                        existing.span = func.span;
                        if !func.params.is_empty() {
                            existing.params = func.params.clone();
                        }
                    } else if existing.params.is_empty() && !func.params.is_empty() {
                        existing.params = func.params.clone();
                    }
                    return existing_id;
                }
            }
            self.fn_by_name.insert(func.name.clone(), func.id);
        }
        let id = func.id;
        self.functions.push(func);
        id
    }

    pub fn add_variable(&mut self, var: Variable) -> VarId {
        let id = var.id;
        if var.storage == StorageClass::Global {
            self.global_by_name.insert(var.name.clone(), id);
        }
        self.variables.push(var);
        id
    }

    pub fn alloc_fn_id(&mut self) -> FnId {
        let id = FnId(self.next_fn);
        self.next_fn += 1;
        id
    }

    pub fn alloc_var_id(&mut self) -> VarId {
        let id = VarId(self.next_var);
        self.next_var += 1;
        id
    }

    pub fn alloc_call_id(&mut self) -> CallSiteId {
        let id = CallSiteId(self.next_call);
        self.next_call += 1;
        id
    }

    pub fn resolve_function(&self, name: &str) -> Option<FnId> {
        self.fn_by_name.get(name).copied()
    }

    /// Resolve by external name table first, then file-local/static definitions.
    pub fn resolve_function_in_scope(
        &self,
        name: &str,
        file: Option<crate::FileId>,
    ) -> Option<FnId> {
        if let Some(id) = self.fn_by_name.get(name) {
            return Some(*id);
        }
        let file = file?;
        self.functions
            .iter()
            .find(|f| f.name == name && f.file == file)
            .map(|f| f.id)
    }

    pub fn function_by_id(&self, id: FnId) -> Option<&Function> {
        self.functions.iter().find(|f| f.id == id)
    }

    pub fn function(&self, id: FnId) -> &Function {
        self.function_by_id(id)
            .unwrap_or_else(|| panic!("unknown function id {}", id.0))
    }

    pub fn variable_by_id(&self, id: VarId) -> Option<&Variable> {
        self.variables.get(id.0 as usize).filter(|v| v.id == id)
    }

    pub fn variable(&self, id: VarId) -> &Variable {
        self.variable_by_id(id)
            .unwrap_or_else(|| panic!("unknown variable id {}", id.0))
    }

    pub fn call_site_by_id(&self, id: CallSiteId) -> Option<&CallSite> {
        self.call_sites.iter().find(|c| c.id == id)
    }

    pub fn function_ids_unique(&self) -> bool {
        let mut seen = std::collections::HashSet::new();
        self.functions.iter().all(|f| seen.insert(f.id))
    }
}

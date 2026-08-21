use crate::{FieldId, TypeId};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TypeDesc {
    Void,
    Char,
    Int,
    Long,
    SizeT,
    Unknown,
    Ptr(Box<TypeDesc>),
    Array {
        elem: Box<TypeDesc>,
        size: Option<u64>,
    },
    Struct {
        name: String,
        fields: Vec<(String, TypeDesc)>,
    },
    Union {
        name: String,
        fields: Vec<(String, TypeDesc)>,
    },
    FnPtr {
        ret: Box<TypeDesc>,
        params: Vec<TypeDesc>,
    },
}

impl TypeDesc {
    pub fn is_pointer_like(&self) -> bool {
        matches!(self, TypeDesc::Ptr(_) | TypeDesc::FnPtr { .. })
    }

    pub fn pointee(&self) -> Option<&TypeDesc> {
        match self {
            TypeDesc::Ptr(inner) => Some(inner),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TypeKind {
    Void,
    Char,
    Int,
    Long,
    SizeT,
    Unknown,
    Ptr,
    Array,
    Struct,
    Union,
    FnPtr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeInfo {
    pub id: TypeId,
    pub desc: TypeDesc,
    pub size: u64,
    pub align: u64,
    pub layout: TypeLayout,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TypeLayout {
    pub fields: IndexMap<FieldId, FieldLayout>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldLayout {
    pub name: String,
    pub offset: u64,
    pub size: u64,
    pub type_id: TypeId,
}

#[derive(Debug, Clone)]
pub struct TypeTable {
    types: Vec<TypeInfo>,
    intern: IndexMap<TypeDesc, TypeId>,
}

impl Default for TypeTable {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeTable {
    pub fn new() -> Self {
        let mut table = Self {
            types: Vec::new(),
            intern: IndexMap::new(),
        };
        table.intern(TypeDesc::Void);
        table.intern(TypeDesc::Char);
        table.intern(TypeDesc::Int);
        table.intern(TypeDesc::Long);
        table.intern(TypeDesc::SizeT);
        table.intern(TypeDesc::Unknown);
        table
    }

    pub fn intern(&mut self, desc: TypeDesc) -> TypeId {
        if let Some(id) = self.lookup_tag_ref(&desc) {
            return id;
        }
        if let Some(id) = self.intern.get(&desc) {
            return *id;
        }
        let (size, align, layout) = compute_layout(&desc, self);
        let id = TypeId(self.types.len() as u32);
        self.types.push(TypeInfo {
            id,
            desc: desc.clone(),
            size,
            align,
            layout,
        });
        self.intern.insert(desc, id);
        id
    }

    pub fn get(&self, id: TypeId) -> &TypeInfo {
        &self.types[id.0 as usize]
    }

    pub fn void(&self) -> TypeId {
        TypeId(0)
    }

    pub fn int(&self) -> TypeId {
        TypeId(2)
    }

    pub fn ptr_to(&mut self, inner: TypeDesc) -> TypeId {
        self.intern(TypeDesc::Ptr(Box::new(inner)))
    }

    pub fn all(&self) -> &[TypeInfo] {
        &self.types
    }

    pub fn compute_struct_layout(
        &mut self,
        name: String,
        fields: Vec<(String, TypeDesc)>,
    ) -> TypeId {
        self.intern(TypeDesc::Struct { name, fields })
    }

    pub fn compute_union_layout(
        &mut self,
        name: String,
        fields: Vec<(String, TypeDesc)>,
    ) -> TypeId {
        self.intern(TypeDesc::Union { name, fields })
    }

    pub fn field_id_by_name(&self, type_id: TypeId, fname: &str) -> Option<FieldId> {
        let info = self.get(type_id);
        info.layout
            .fields
            .iter()
            .find(|(_, fl)| fl.name == fname)
            .map(|(id, _)| *id)
    }

    fn lookup_tag_ref(&self, desc: &TypeDesc) -> Option<TypeId> {
        match desc {
            TypeDesc::Struct { name, fields } if fields.is_empty() && !name.is_empty() => {
                self.type_id_by_tag(name, TypeKind::Struct)
            }
            TypeDesc::Union { name, fields } if fields.is_empty() && !name.is_empty() => {
                self.type_id_by_tag(name, TypeKind::Union)
            }
            _ => None,
        }
    }

    pub fn type_id_by_tag(&self, name: &str, kind: TypeKind) -> Option<TypeId> {
        self.types
            .iter()
            .filter(|t| tag_name_matches(&t.desc, name, kind))
            .max_by_key(|t| {
                let layout_n = t.layout.fields.len();
                let desc_n = match &t.desc {
                    TypeDesc::Struct { fields, .. } | TypeDesc::Union { fields, .. } => {
                        fields.len()
                    }
                    _ => 0,
                };
                layout_n.max(desc_n)
            })
            .map(|t| t.id)
    }

    pub fn resolve_type_id(&self, desc: &TypeDesc) -> TypeId {
        if let Some(id) = self.lookup_tag_ref(desc) {
            return id;
        }
        if let Some(id) = self.intern.get(desc) {
            return *id;
        }
        if let TypeDesc::Ptr(inner) = desc {
            let pointee = self.lookup_tag_ref(inner).unwrap_or_else(|| {
                self.intern
                    .get(inner.as_ref())
                    .copied()
                    .unwrap_or(TypeId(5))
            });
            if pointee != TypeId(5) {
                let pointee_desc = self.get(pointee).desc.clone();
                let ptr_desc = TypeDesc::Ptr(Box::new(pointee_desc));
                if let Some(id) = self.intern.get(&ptr_desc) {
                    return *id;
                }
            }
        }
        TypeId(5)
    }
}

fn tag_name_matches(desc: &TypeDesc, name: &str, kind: TypeKind) -> bool {
    match (desc, kind) {
        (TypeDesc::Struct { name: n, .. }, TypeKind::Struct) => n == name,
        (TypeDesc::Union { name: n, .. }, TypeKind::Union) => n == name,
        _ => false,
    }
}

fn compute_layout(desc: &TypeDesc, table: &mut TypeTable) -> (u64, u64, TypeLayout) {
    match desc {
        TypeDesc::Void => (0, 1, TypeLayout::default()),
        TypeDesc::Char => (1, 1, TypeLayout::default()),
        TypeDesc::Int => (4, 4, TypeLayout::default()),
        TypeDesc::Long => (8, 8, TypeLayout::default()),
        TypeDesc::SizeT => (8, 8, TypeLayout::default()),
        TypeDesc::Unknown => (8, 8, TypeLayout::default()),
        TypeDesc::Ptr(_) | TypeDesc::FnPtr { .. } => (8, 8, TypeLayout::default()),
        TypeDesc::Array { elem, size } => {
            let (elem_size, elem_align, _) = compute_layout(elem, table);
            let count = size.unwrap_or(0);
            (elem_size * count, elem_align, TypeLayout::default())
        }
        TypeDesc::Struct { fields, .. } | TypeDesc::Union { fields, .. } => {
            let mut layout = TypeLayout::default();
            let mut offset = 0u64;
            let mut max_align = 1u64;
            let mut total_size = 0u64;
            for (idx, (name, field_desc)) in fields.iter().enumerate() {
                let fid = FieldId(idx as u32);
                let field_type_id = table.intern(field_desc.clone());
                let (field_size, field_align, _) = compute_layout(field_desc, table);
                max_align = max_align.max(field_align);
                offset = align_up(offset, field_align);
                layout.fields.insert(
                    fid,
                    FieldLayout {
                        name: name.clone(),
                        offset,
                        size: field_size,
                        type_id: field_type_id,
                    },
                );
                offset += field_size;
                total_size = total_size.max(offset);
            }
            total_size = align_up(total_size, max_align);
            (total_size, max_align, layout)
        }
    }
}

fn align_up(value: u64, align: u64) -> u64 {
    if align == 0 {
        return value;
    }
    value.div_ceil(align) * align
}

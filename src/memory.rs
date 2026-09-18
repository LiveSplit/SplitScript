//! Type-directed process-memory layouts.
//!
//! This module is the single source of truth for the `MemoryReadable`
//! capability. Type checking, documentation/editor queries, and WebAssembly
//! lowering all consume the same deterministic layouts.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{EnumDecl, EnumId, EnumVariantId, StructDecl, StructFieldId, StructId},
    semantic::SemanticModel,
    stdlib::{RuntimeRepresentation, StandardLibrary, StdlibCapabilityId, StdlibFieldId},
    types::{BuiltinType, TypeId, TypeKind},
};

/// Fixed arrays are expanded into statically typed GC construction code.
/// These limits bound both the host read and compiler/module growth.
pub const MAX_FIXED_ARRAY_ELEMENTS: u32 = 4_096;
pub const MAX_FIXED_ARRAY_BYTES: u32 = 65_536;

/// Physical width of an `address` stored in a reader's memory domain.
///
/// SplitScript keeps the logical value 64-bit in both cases. This context only
/// controls the bytes, alignment, aggregate offsets, and array strides used at
/// the memory boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryAddressWidth {
    Bit32,
    Bit64,
}

impl MemoryAddressWidth {
    pub const fn bytes(self) -> u32 {
        match self {
            Self::Bit32 => 4,
            Self::Bit64 => 8,
        }
    }
}

const ADDRESS_WIDTHS: [MemoryAddressWidth; 2] =
    [MemoryAddressWidth::Bit32, MemoryAddressWidth::Bit64];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryFieldId {
    Source(StructFieldId),
    Standard(StdlibFieldId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryFieldLayout {
    pub field: MemoryFieldId,
    pub ty: TypeId,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructMemoryLayout {
    pub ty: TypeId,
    pub size: u32,
    pub alignment: u32,
    pub fields: Vec<MemoryFieldLayout>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedArrayMemoryLayout {
    pub ty: TypeId,
    pub element: TypeId,
    pub length: u32,
    pub stride: u32,
    pub size: u32,
    pub alignment: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumVariantMemoryLayout {
    pub variant: EnumVariantId,
    /// Mathematical value after range checking. Code generation converts it
    /// to the underlying integer's raw bits only at the load boundary.
    pub value: i128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumMemoryLayout {
    pub ty: TypeId,
    pub representation: BuiltinType,
    pub size: u32,
    pub alignment: u32,
    pub variants: Vec<EnumVariantMemoryLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTypeLayout<'a> {
    Scalar { size: u32, alignment: u32 },
    Enum(&'a EnumMemoryLayout),
    Struct(&'a StructMemoryLayout),
    FixedArray(&'a FixedArrayMemoryLayout),
}

impl MemoryTypeLayout<'_> {
    pub fn size(self) -> u32 {
        match self {
            Self::Scalar { size, .. } => size,
            Self::Enum(layout) => layout.size,
            Self::Struct(layout) => layout.size,
            Self::FixedArray(layout) => layout.size,
        }
    }

    pub fn alignment(self) -> u32 {
        match self {
            Self::Scalar { alignment, .. } => alignment,
            Self::Enum(layout) => layout.alignment,
            Self::Struct(layout) => layout.alignment,
            Self::FixedArray(layout) => layout.alignment,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemoryLayouts {
    standard_library: StandardLibrary,
    structs: HashMap<(TypeId, MemoryAddressWidth), Result<StructMemoryLayout, String>>,
    enums: HashMap<TypeId, Result<EnumMemoryLayout, String>>,
    arrays: HashMap<(TypeId, MemoryAddressWidth), Result<FixedArrayMemoryLayout, String>>,
    source_structs: HashMap<StructId, TypeId>,
    source_enums: HashMap<EnumId, TypeId>,
}

impl MemoryLayouts {
    pub fn build(structs: &[StructDecl], enums: &[EnumDecl], semantics: &SemanticModel) -> Self {
        Self::build_with_library(structs, enums, semantics, StandardLibrary::new())
    }

    pub fn build_with_library(
        structs: &[StructDecl],
        enums: &[EnumDecl],
        semantics: &SemanticModel,
        standard_library: StandardLibrary,
    ) -> Self {
        let mut layouts = Self {
            standard_library,
            structs: HashMap::new(),
            enums: HashMap::new(),
            arrays: HashMap::new(),
            source_structs: HashMap::new(),
            source_enums: HashMap::new(),
        };
        for enumeration in enums {
            let ty = semantics.types().id_for_enum(enumeration.id);
            layouts.source_enums.insert(enumeration.id, ty);
            if enumeration.representation.is_some() {
                let result = layouts.build_enum(ty, enumeration, semantics);
                layouts.enums.insert(ty, result);
            }
        }
        for structure in structs {
            let ty = semantics.types().id_for_struct(structure.id);
            layouts.source_structs.insert(structure.id, ty);
            for width in ADDRESS_WIDTHS {
                let mut visiting = HashSet::new();
                let _ = layouts.build_struct(ty, structs, enums, semantics, width, &mut visiting);
            }
        }
        let library = layouts.standard_library.clone();
        for standard in library.all_types().iter().filter(|standard| {
            library.type_has_capability(standard.id, StdlibCapabilityId::MemoryReadable)
                && matches!(
                    standard.representation,
                    RuntimeRepresentation::GcStruct { .. }
                )
        }) {
            for width in ADDRESS_WIDTHS {
                let mut visiting = HashSet::new();
                let _ = layouts.build_struct(
                    semantics.types().id_for_standard(standard.id),
                    structs,
                    enums,
                    semantics,
                    width,
                    &mut visiting,
                );
            }
        }
        let fixed_arrays = semantics
            .types()
            .iter()
            .filter_map(|(ty, kind)| {
                matches!(
                    kind,
                    TypeKind::Array {
                        length: Some(_),
                        ..
                    }
                )
                .then_some(ty)
            })
            .collect::<Vec<_>>();
        for ty in fixed_arrays {
            for width in ADDRESS_WIDTHS {
                let mut visiting = HashSet::new();
                let _ = layouts.build_array(ty, structs, enums, semantics, width, &mut visiting);
            }
        }
        layouts
    }

    pub fn layout<'a>(
        &'a self,
        ty: TypeId,
        semantics: &SemanticModel,
        address_width: MemoryAddressWidth,
    ) -> Result<MemoryTypeLayout<'a>, String> {
        match semantics.types().kind(ty) {
            TypeKind::Builtin(builtin) => {
                scalar_layout(&self.standard_library, *builtin, address_width)
                    .map(|(size, alignment)| MemoryTypeLayout::Scalar { size, alignment })
                    .ok_or_else(|| format!("type `{builtin}` is not MemoryReadable"))
            }
            TypeKind::Struct(structure) => self
                .structs
                .get(&(semantics.types().id_for_struct(*structure), address_width))
                .expect("every declared struct has a memory-layout result")
                .as_ref()
                .map(MemoryTypeLayout::Struct)
                .map_err(Clone::clone),
            TypeKind::Enum(_) => self
                .enums
                .get(&ty)
                .ok_or_else(|| "enum has no declared process-memory representation".to_owned())?
                .as_ref()
                .map(MemoryTypeLayout::Enum)
                .map_err(Clone::clone),
            TypeKind::Standard(standard) => {
                let library = &self.standard_library;
                let declaration = library.type_decl(*standard);
                if !library.type_has_capability(*standard, StdlibCapabilityId::MemoryReadable) {
                    return Err(format!("type `{}` is not MemoryReadable", declaration.name));
                }
                match declaration.representation {
                    RuntimeRepresentation::Scalar { storage } => library
                        .core_type(storage)
                        .memory_layout
                        .and_then(|_| scalar_layout(library, storage, address_width))
                        .map(|(size, alignment)| MemoryTypeLayout::Scalar { size, alignment })
                        .ok_or_else(|| {
                            format!("type `{}` is not MemoryReadable", declaration.name)
                        }),
                    RuntimeRepresentation::GcStruct { .. } => self
                        .structs
                        .get(&(ty, address_width))
                        .expect("every readable standard struct has a memory-layout result")
                        .as_ref()
                        .map(MemoryTypeLayout::Struct)
                        .map_err(Clone::clone),
                    RuntimeRepresentation::GcArray { .. } | RuntimeRepresentation::Enum { .. } => {
                        Err(format!("type `{}` is not MemoryReadable", declaration.name))
                    }
                }
            }
            TypeKind::Array {
                length: Some(_), ..
            } => self
                .arrays
                .get(&(ty, address_width))
                .expect("every fixed array has a memory-layout result")
                .as_ref()
                .map(MemoryTypeLayout::FixedArray)
                .map_err(Clone::clone),
            TypeKind::Array { length: None, .. } => Err(
                "an unsized `[T]` array has no fixed process-memory layout; use `[T; N]`"
                    .to_owned(),
            ),
            kind => Err(format!("type `{kind:?}` is not MemoryReadable")),
        }
    }

    /// Validates the portable `MemoryReadable` contract for both supported
    /// native pointer widths. A reader selects one of these already-checked
    /// layouts at the memory boundary.
    pub fn require_layout(&self, ty: TypeId, semantics: &SemanticModel) -> Result<(), String> {
        for width in ADDRESS_WIDTHS {
            self.layout(ty, semantics, width)?;
        }
        Ok(())
    }

    pub fn structure(
        &self,
        structure: StructId,
        address_width: MemoryAddressWidth,
    ) -> Result<&StructMemoryLayout, &str> {
        let ty = self
            .source_structs
            .get(&structure)
            .expect("every declared struct has a semantic type");
        self.structs
            .get(&(*ty, address_width))
            .expect("every declared struct has a memory-layout result")
            .as_ref()
            .map_err(String::as_str)
    }

    pub fn enumeration(&self, enumeration: EnumId) -> Result<&EnumMemoryLayout, &str> {
        let ty = self
            .source_enums
            .get(&enumeration)
            .ok_or("enum has no process-memory representation")?;
        self.enums
            .get(ty)
            .ok_or("enum has no process-memory representation")?
            .as_ref()
            .map_err(String::as_str)
    }

    /// Largest fixed-layout value represented by this analysis, including
    /// unused declarations. Runtime scratch is planned separately from the
    /// types of reachable reads rather than this whole-program upper bound.
    pub fn maximum_size(&self) -> u32 {
        self.structs
            .values()
            .filter_map(|layout| layout.as_ref().ok())
            .map(|layout| layout.size)
            .chain(
                self.arrays
                    .values()
                    .filter_map(|layout| layout.as_ref().ok())
                    .map(|layout| layout.size),
            )
            .max()
            .unwrap_or(0)
            .max(8)
    }

    /// Whether decoding this type needs the reader's pointer width even when
    /// the aggregate's total byte size happens to be equal in both layouts.
    pub fn depends_on_address_width(&self, ty: TypeId, semantics: &SemanticModel) -> bool {
        self.depends_on_address_width_inner(ty, semantics, &mut HashSet::new())
    }

    /// Whether a decoded value contains a represented enum whose raw
    /// discriminant must be validated before materialization.
    pub fn requires_validation(&self, ty: TypeId, semantics: &SemanticModel) -> bool {
        self.requires_validation_inner(ty, semantics, &mut HashSet::new())
    }

    fn requires_validation_inner(
        &self,
        ty: TypeId,
        semantics: &SemanticModel,
        visiting: &mut HashSet<TypeId>,
    ) -> bool {
        if !visiting.insert(ty) {
            return false;
        }
        let result = match semantics.types().kind(ty) {
            TypeKind::Enum(_) => true,
            TypeKind::Struct(_) | TypeKind::Standard(_) => {
                self.structs
                    .get(&(ty, MemoryAddressWidth::Bit64))
                    .and_then(|layout| layout.as_ref().ok())
                    .is_some_and(|layout| {
                        layout.fields.iter().any(|field| {
                            self.requires_validation_inner(field.ty, semantics, visiting)
                        })
                    })
            }
            TypeKind::Array {
                element,
                length: Some(_),
                ..
            } => self.requires_validation_inner(*element, semantics, visiting),
            _ => false,
        };
        visiting.remove(&ty);
        result
    }

    fn depends_on_address_width_inner(
        &self,
        ty: TypeId,
        semantics: &SemanticModel,
        visiting: &mut HashSet<TypeId>,
    ) -> bool {
        if !visiting.insert(ty) {
            return false;
        }
        let result = match semantics.types().kind(ty) {
            TypeKind::Builtin(BuiltinType::Address) => true,
            TypeKind::Builtin(_) | TypeKind::Enum(_) => false,
            TypeKind::Struct(_) | TypeKind::Standard(_) => self
                .structs
                .get(&(ty, MemoryAddressWidth::Bit64))
                .and_then(|layout| layout.as_ref().ok())
                .is_some_and(|layout| {
                    layout.fields.iter().any(|field| {
                        self.depends_on_address_width_inner(field.ty, semantics, visiting)
                    })
                }),
            TypeKind::Array {
                element,
                length: Some(_),
                ..
            } => self.depends_on_address_width_inner(*element, semantics, visiting),
            _ => false,
        };
        visiting.remove(&ty);
        result
    }

    fn build_enum(
        &self,
        ty: TypeId,
        enumeration: &EnumDecl,
        semantics: &SemanticModel,
    ) -> Result<EnumMemoryLayout, String> {
        let representation = semantics
            .enum_representation(enumeration.id)
            .ok_or_else(|| {
                format!(
                    "enum `{}` has no declared process-memory representation",
                    enumeration.name
                )
            })?;
        let TypeKind::Builtin(representation) = semantics.types().kind(representation) else {
            return Err(format!(
                "enum `{}` does not use a fixed-width integer representation",
                enumeration.name
            ));
        };
        let Some((minimum, maximum, size, alignment)) = integer_representation(*representation)
        else {
            return Err(format!(
                "enum `{}` does not use a fixed-width integer representation",
                enumeration.name
            ));
        };
        if enumeration.variants.is_empty() {
            return Err(format!(
                "process-readable enum `{}` must declare at least one variant",
                enumeration.name
            ));
        }

        let mut next = Some(0_i128);
        let mut seen = HashMap::<i128, &str>::new();
        let mut variants = Vec::with_capacity(enumeration.variants.len());
        for variant in &enumeration.variants {
            if variant.payload.is_some() {
                return Err(format!(
                    "process-readable enum variant `{}.{}` cannot carry a payload",
                    enumeration.name, variant.name
                ));
            }
            let value = if let Some(discriminant) = variant.discriminant {
                let magnitude = i128::from(discriminant.magnitude);
                if discriminant.negative {
                    -magnitude
                } else {
                    magnitude
                }
            } else {
                next.ok_or_else(|| {
                    format!(
                        "implicit discriminant for `{}.{}` overflows `{}`",
                        enumeration.name, variant.name, representation
                    )
                })?
            };
            if value < minimum || value > maximum {
                return Err(format!(
                    "discriminant `{value}` for `{}.{}` does not fit in `{}`",
                    enumeration.name, variant.name, representation
                ));
            }
            if let Some(previous) = seen.insert(value, &variant.name) {
                return Err(format!(
                    "enum `{}.{}` and `{}.{}` both use discriminant `{value}`",
                    enumeration.name, previous, enumeration.name, variant.name
                ));
            }
            variants.push(EnumVariantMemoryLayout {
                variant: variant.id,
                value,
            });
            next = value.checked_add(1).filter(|next| *next <= maximum);
        }
        Ok(EnumMemoryLayout {
            ty,
            representation: *representation,
            size,
            alignment,
            variants,
        })
    }

    fn build_array(
        &mut self,
        ty: TypeId,
        structs: &[StructDecl],
        enums: &[EnumDecl],
        semantics: &SemanticModel,
        address_width: MemoryAddressWidth,
        visiting: &mut HashSet<TypeId>,
    ) -> Result<FixedArrayMemoryLayout, String> {
        if let Some(layout) = self.arrays.get(&(ty, address_width)) {
            return layout.clone();
        }
        if !visiting.insert(ty) {
            return Err("recursive arrays do not have a finite process-memory layout".to_owned());
        }
        let result = (|| {
            let TypeKind::Array {
                element,
                length: Some(length),
                ..
            } = semantics.types().kind(ty)
            else {
                return Err("an unsized `[T]` array has no fixed process-memory layout".to_owned());
            };
            if *length == 0 {
                return Err(
                    "a zero-length array does not represent a process-memory read".to_owned(),
                );
            }
            if *length > MAX_FIXED_ARRAY_ELEMENTS {
                return Err(format!(
                    "fixed arrays are limited to {MAX_FIXED_ARRAY_ELEMENTS} elements"
                ));
            }
            let (element_size, alignment) =
                self.fixed_layout(*element, structs, enums, semantics, address_width, visiting)?;
            let stride = align_up(element_size, alignment);
            let size = stride
                .checked_mul(*length)
                .ok_or_else(|| "fixed array byte size overflows `u32`".to_owned())?;
            if size > MAX_FIXED_ARRAY_BYTES {
                return Err(format!(
                    "fixed process arrays are limited to {MAX_FIXED_ARRAY_BYTES} bytes"
                ));
            }
            Ok(FixedArrayMemoryLayout {
                ty,
                element: *element,
                length: *length,
                stride,
                size,
                alignment,
            })
        })();
        visiting.remove(&ty);
        self.arrays.insert((ty, address_width), result.clone());
        result
    }

    fn build_struct(
        &mut self,
        ty: TypeId,
        structs: &[StructDecl],
        enums: &[EnumDecl],
        semantics: &SemanticModel,
        address_width: MemoryAddressWidth,
        visiting: &mut HashSet<TypeId>,
    ) -> Result<StructMemoryLayout, String> {
        if let Some(layout) = self.structs.get(&(ty, address_width)) {
            return layout.clone();
        }
        if !visiting.insert(ty) {
            return Err("recursive structs do not have a finite process-memory layout".to_owned());
        }

        let result = (|| {
            let (name, declared_fields) = match semantics.types().kind(ty) {
                TypeKind::Struct(structure) => {
                    let declaration = structs
                        .iter()
                        .find(|declaration| declaration.id == *structure)
                        .expect("struct IDs refer to declarations");
                    (
                        declaration.name.clone(),
                        declaration
                            .fields
                            .iter()
                            .map(|field| {
                                (
                                    MemoryFieldId::Source(field.id),
                                    field.name.clone(),
                                    semantics
                                        .struct_field_type(field.id)
                                        .expect("checked struct fields have semantic types"),
                                )
                            })
                            .collect::<Vec<_>>(),
                    )
                }
                TypeKind::Standard(standard) => {
                    let library = &self.standard_library;
                    let declaration = library.type_decl(*standard);
                    if !library.type_has_capability(*standard, StdlibCapabilityId::MemoryReadable)
                        || !matches!(
                            declaration.representation,
                            RuntimeRepresentation::GcStruct { .. }
                        )
                    {
                        return Err(format!("type `{}` is not MemoryReadable", declaration.name));
                    }
                    (
                        declaration.name.to_owned(),
                        library
                            .fields_of(*standard)
                            .map(|field| {
                                (
                                    MemoryFieldId::Standard(field.id),
                                    field.name.to_owned(),
                                    semantics
                                        .standard_field_type(field.id)
                                        .expect("checked standard fields have semantic types"),
                                )
                            })
                            .collect::<Vec<_>>(),
                    )
                }
                kind => return Err(format!("type `{kind:?}` is not a struct")),
            };
            if declared_fields.is_empty() {
                return Err(format!("struct `{name}` has no readable fields"));
            }

            let mut offset = 0;
            let mut alignment = 1;
            let mut fields = Vec::with_capacity(declared_fields.len());
            for (field, field_name, field_ty) in declared_fields {
                let (field_size, field_alignment) = self
                    .fixed_layout(field_ty, structs, enums, semantics, address_width, visiting)
                    .map_err(|error| {
                        format!("struct `{name}.{field_name}` is not MemoryReadable: {error}")
                    })?;
                offset = align_up(offset, field_alignment);
                fields.push(MemoryFieldLayout {
                    field,
                    ty: field_ty,
                    offset,
                });
                offset = offset
                    .checked_add(field_size)
                    .ok_or_else(|| format!("struct `{name}` is too large"))?;
                alignment = alignment.max(field_alignment);
            }
            Ok(StructMemoryLayout {
                ty,
                size: align_up(offset, alignment),
                alignment,
                fields,
            })
        })();
        visiting.remove(&ty);
        self.structs.insert((ty, address_width), result.clone());
        result
    }

    fn fixed_layout(
        &mut self,
        ty: TypeId,
        structs: &[StructDecl],
        enums: &[EnumDecl],
        semantics: &SemanticModel,
        address_width: MemoryAddressWidth,
        visiting: &mut HashSet<TypeId>,
    ) -> Result<(u32, u32), String> {
        match semantics.types().kind(ty) {
            TypeKind::Builtin(builtin) => {
                scalar_layout(&self.standard_library, *builtin, address_width)
                    .ok_or_else(|| format!("`{builtin}` has no fixed process-memory layout"))
            }
            TypeKind::Struct(_) => self
                .build_struct(ty, structs, enums, semantics, address_width, visiting)
                .map(|layout| (layout.size, layout.alignment)),
            TypeKind::Enum(enumeration) => {
                if !self.enums.contains_key(&ty) {
                    let declaration = enums
                        .iter()
                        .find(|declaration| declaration.id == *enumeration)
                        .ok_or_else(|| "enum declaration is unavailable".to_owned())?;
                    let result = self.build_enum(ty, declaration, semantics);
                    self.enums.insert(ty, result);
                }
                self.enums
                    .get(&ty)
                    .expect("enum layout was just inserted")
                    .as_ref()
                    .map(|layout| (layout.size, layout.alignment))
                    .map_err(Clone::clone)
            }
            TypeKind::Standard(standard) => {
                let library = &self.standard_library;
                let declaration = library.type_decl(*standard);
                if !library.type_has_capability(*standard, StdlibCapabilityId::MemoryReadable) {
                    return Err(format!(
                        "`{}` has no fixed process-memory layout",
                        declaration.name
                    ));
                }
                match declaration.representation {
                    RuntimeRepresentation::Scalar { storage } => library
                        .core_type(storage)
                        .memory_layout
                        .and_then(|_| scalar_layout(library, storage, address_width))
                        .ok_or_else(|| {
                            format!("`{}` has no fixed process-memory layout", declaration.name)
                        }),
                    RuntimeRepresentation::GcStruct { .. } => self
                        .build_struct(ty, structs, enums, semantics, address_width, visiting)
                        .map(|layout| (layout.size, layout.alignment)),
                    RuntimeRepresentation::GcArray { .. } | RuntimeRepresentation::Enum { .. } => {
                        Err(format!(
                            "`{}` has no fixed process-memory layout",
                            declaration.name
                        ))
                    }
                }
            }
            TypeKind::Array {
                length: Some(_), ..
            } => self
                .build_array(ty, structs, enums, semantics, address_width, visiting)
                .map(|layout| (layout.size, layout.alignment)),
            TypeKind::Array { length: None, .. } => Err(
                "an unsized `[T]` array has no fixed process-memory layout; use `[T; N]`"
                    .to_owned(),
            ),
            kind => Err(format!("`{kind:?}` has no fixed process-memory layout")),
        }
    }
}

fn scalar_layout(
    library: &StandardLibrary,
    ty: BuiltinType,
    address_width: MemoryAddressWidth,
) -> Option<(u32, u32)> {
    if ty == BuiltinType::Address {
        let bytes = address_width.bytes();
        return Some((bytes, bytes));
    }
    library
        .core_type(ty)
        .memory_layout
        .map(|layout| (layout.size, layout.alignment))
}

fn integer_representation(ty: BuiltinType) -> Option<(i128, i128, u32, u32)> {
    let (minimum, maximum, size) = match ty {
        BuiltinType::I8 => (i128::from(i8::MIN), i128::from(i8::MAX), 1),
        BuiltinType::U8 => (0, i128::from(u8::MAX), 1),
        BuiltinType::I16 => (i128::from(i16::MIN), i128::from(i16::MAX), 2),
        BuiltinType::U16 => (0, i128::from(u16::MAX), 2),
        BuiltinType::I32 => (i128::from(i32::MIN), i128::from(i32::MAX), 4),
        BuiltinType::U32 => (0, i128::from(u32::MAX), 4),
        BuiltinType::I64 => (i128::from(i64::MIN), i128::from(i64::MAX), 8),
        BuiltinType::U64 => (0, i128::from(u64::MAX), 8),
        _ => return None,
    };
    Some((minimum, maximum, size, size))
}

fn align_up(value: u32, alignment: u32) -> u32 {
    debug_assert!(alignment.is_power_of_two());
    value.saturating_add(alignment - 1) & !(alignment - 1)
}

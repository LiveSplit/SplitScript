//! Shared semantic shape of user-defined structural values.
//!
//! Capability analysis and lazy backend derivations consume this one graph so
//! equality, display, and future structural capabilities cannot disagree about
//! which struct fields or enum payloads belong to a type.

use std::collections::HashMap;

use crate::{
    ast::{
        EnumDecl, EnumId, EnumVariantId, ManagedClassDecl, ManagedClassId, ManagedFieldId,
        StructDecl, StructFieldId, StructId,
    },
    semantic::SemanticModel,
    types::{TypeId, TypeKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum StructuralTypeId {
    Struct(StructId),
    Enum(EnumId),
    /// An immutable local snapshot produced from a live managed reference.
    /// Managed references themselves are not structural values.
    ManagedClass(ManagedClassId),
}

#[derive(Debug, Clone)]
pub(crate) struct StructuralMember {
    pub name: String,
    pub source: StructuralMemberId,
    /// Structs always have a member type. Payload-less enum variants do not.
    pub ty: Option<TypeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum StructuralMemberId {
    StructField(StructFieldId),
    EnumVariant(EnumVariantId),
    ManagedField(ManagedFieldId),
}

#[derive(Debug, Clone)]
pub(crate) struct StructuralType {
    pub id: StructuralTypeId,
    pub name: String,
    pub members: Vec<StructuralMember>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct StructuralTypes {
    by_type: HashMap<TypeId, StructuralType>,
    semantic_types: HashMap<StructuralTypeId, TypeId>,
    structs: Vec<TypeId>,
    enums: Vec<TypeId>,
    managed_classes: Vec<TypeId>,
}

impl StructuralTypes {
    pub fn build(
        structs: &[StructDecl],
        enums: &[EnumDecl],
        managed_classes: &[&ManagedClassDecl],
        semantics: &SemanticModel,
    ) -> Self {
        let semantic_types = semantics
            .types()
            .iter()
            .filter_map(|(ty, kind)| match kind {
                TypeKind::Struct(structure) => Some((StructuralTypeId::Struct(*structure), ty)),
                TypeKind::Enum(enumeration) => Some((StructuralTypeId::Enum(*enumeration), ty)),
                TypeKind::ManagedClass(class) => Some((StructuralTypeId::ManagedClass(*class), ty)),
                _ => None,
            })
            .collect::<HashMap<_, _>>();
        let mut by_type = HashMap::new();
        let mut struct_types = Vec::with_capacity(structs.len());
        for structure in structs {
            let id = StructuralTypeId::Struct(structure.id);
            let ty = semantic_types[&id];
            by_type.insert(
                ty,
                StructuralType {
                    id,
                    name: structure.name.clone(),
                    members: structure
                        .fields
                        .iter()
                        .map(|field| StructuralMember {
                            name: field.name.clone(),
                            source: StructuralMemberId::StructField(field.id),
                            ty: Some(
                                semantics
                                    .struct_field_type(field.id)
                                    .expect("checked struct fields have semantic types"),
                            ),
                        })
                        .collect(),
                },
            );
            struct_types.push(ty);
        }
        let mut enum_types = Vec::with_capacity(enums.len());
        for enumeration in enums {
            let id = StructuralTypeId::Enum(enumeration.id);
            let ty = semantic_types[&id];
            by_type.insert(
                ty,
                StructuralType {
                    id,
                    name: enumeration.name.clone(),
                    members: enumeration
                        .variants
                        .iter()
                        .map(|variant| StructuralMember {
                            name: variant.name.clone(),
                            source: StructuralMemberId::EnumVariant(variant.id),
                            ty: semantics.enum_variant_payload(variant.id),
                        })
                        .collect(),
                },
            );
            enum_types.push(ty);
        }
        let mut class_types = Vec::with_capacity(managed_classes.len());
        for class in managed_classes {
            let id = StructuralTypeId::ManagedClass(class.id);
            let ty = semantic_types[&id];
            by_type.insert(
                ty,
                StructuralType {
                    id,
                    name: class.name.clone(),
                    members: class
                        .all_fields()
                        .filter(|field| !field.is_static)
                        .map(|field| StructuralMember {
                            name: field.name.clone(),
                            source: StructuralMemberId::ManagedField(field.id),
                            ty: Some(
                                semantics
                                    .managed_field_snapshot_type(field.id)
                                    .expect("checked managed fields have semantic value types"),
                            ),
                        })
                        .collect(),
                },
            );
            class_types.push(ty);
        }
        Self {
            by_type,
            semantic_types,
            structs: struct_types,
            enums: enum_types,
            managed_classes: class_types,
        }
    }

    pub fn get(&self, ty: TypeId) -> Option<&StructuralType> {
        self.by_type.get(&ty)
    }

    pub fn semantic_type(&self, id: StructuralTypeId) -> TypeId {
        self.semantic_types[&id]
    }

    pub fn iter(&self) -> impl Iterator<Item = (StructuralTypeId, TypeId)> + '_ {
        self.structs
            .iter()
            .chain(&self.enums)
            .chain(&self.managed_classes)
            .map(|ty| (self.by_type[ty].id, *ty))
    }

    pub fn structs(&self) -> impl Iterator<Item = (TypeId, &StructuralType)> {
        self.structs.iter().map(|ty| (*ty, &self.by_type[ty]))
    }

    pub fn managed_classes(&self) -> impl Iterator<Item = (TypeId, &StructuralType)> {
        self.managed_classes
            .iter()
            .map(|ty| (*ty, &self.by_type[ty]))
    }

    pub fn enums(&self) -> impl Iterator<Item = (TypeId, &StructuralType)> {
        self.enums.iter().map(|ty| (*ty, &self.by_type[ty]))
    }
}

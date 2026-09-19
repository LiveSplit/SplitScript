//! Structural equality capabilities for nominal GC values.
//!
//! Primitive equality is intrinsic. Structs and enums gain equality
//! automatically when every contained field or payload is itself equatable.
//! This is shared by diagnostics, future editor queries, and Wasm helper
//! generation rather than being inferred independently in the backend.

use std::collections::HashMap;

use crate::{
    ast::{EnumDecl, EnumId, ManagedClassId, StructDecl, StructId},
    semantic::SemanticModel,
    stdlib::{StandardLibrary, StdlibCapabilityId},
    structural::{StructuralTypeId, StructuralTypes},
    types::{TypeId, TypeKind},
};

#[derive(Debug, Clone, Default)]
pub struct EqualityCapabilities {
    standard_library: StandardLibrary,
    structural: StructuralTypes,
    structs: HashMap<StructId, Result<(), String>>,
    enums: HashMap<EnumId, Result<(), String>>,
    managed_classes: HashMap<ManagedClassId, Result<(), String>>,
}

impl EqualityCapabilities {
    pub fn build(structs: &[StructDecl], enums: &[EnumDecl], semantics: &SemanticModel) -> Self {
        Self::build_with_library(structs, enums, semantics, StandardLibrary::new())
    }

    pub fn build_with_library(
        structs: &[StructDecl],
        enums: &[EnumDecl],
        semantics: &SemanticModel,
        standard_library: StandardLibrary,
    ) -> Self {
        let structural = StructuralTypes::build(structs, enums, &[], semantics);
        Self::build_with_structural(structural, semantics, standard_library)
    }

    pub(crate) fn build_with_structural(
        structural: StructuralTypes,
        semantics: &SemanticModel,
        standard_library: StandardLibrary,
    ) -> Self {
        let mut capabilities = Self {
            standard_library,
            structural,
            structs: HashMap::new(),
            enums: HashMap::new(),
            managed_classes: HashMap::new(),
        };
        let aggregates = capabilities.structural.iter().collect::<Vec<_>>();
        for (id, ty) in aggregates {
            let result = capabilities.check_type(ty, semantics, &mut Vec::new());
            match id {
                StructuralTypeId::Struct(structure) => {
                    capabilities.structs.entry(structure).or_insert(result);
                }
                StructuralTypeId::Enum(enumeration) => {
                    capabilities.enums.entry(enumeration).or_insert(result);
                }
                StructuralTypeId::ManagedClass(class) => {
                    capabilities.managed_classes.insert(class, result);
                }
            }
        }
        capabilities
    }

    pub fn require(&self, ty: TypeId, semantics: &SemanticModel) -> Result<(), String> {
        match semantics.types().kind(ty) {
            TypeKind::Builtin(builtin)
                if self
                    .standard_library
                    .core_type_has_capability(*builtin, StdlibCapabilityId::Equatable) =>
            {
                Ok(())
            }
            TypeKind::Standard(standard)
                if self
                    .standard_library
                    .type_has_capability(*standard, StdlibCapabilityId::Equatable) =>
            {
                Ok(())
            }
            TypeKind::ManagedClass(class) => self.managed_class(*class).map_err(str::to_owned),
            TypeKind::Struct(structure) => self.structure(*structure).map_err(str::to_owned),
            TypeKind::Enum(enumeration) => self.enumeration(*enumeration).map_err(str::to_owned),
            TypeKind::Option { value, .. } => self
                .require(*value, semantics)
                .map_err(|error| format!("optional value does not support equality: {error}")),
            TypeKind::Result { value, .. } => self
                .require(*value, semantics)
                .map_err(|error| format!("result value does not support equality: {error}")),
            TypeKind::Array { element, .. } => self
                .require(*element, semantics)
                .map_err(|error| format!("array element does not support equality: {error}")),
            _ => Err("this type does not support equality".to_owned()),
        }
    }

    pub fn structure(&self, structure: StructId) -> Result<(), &str> {
        self.structs
            .get(&structure)
            .expect("every struct has an equality result")
            .as_ref()
            .copied()
            .map_err(String::as_str)
    }

    pub fn enumeration(&self, enumeration: EnumId) -> Result<(), &str> {
        self.enums
            .get(&enumeration)
            .expect("every enum has an equality result")
            .as_ref()
            .copied()
            .map_err(String::as_str)
    }

    pub fn managed_class(&self, class: ManagedClassId) -> Result<(), &str> {
        self.managed_classes
            .get(&class)
            .expect("every managed class has an equality result")
            .as_ref()
            .copied()
            .map_err(String::as_str)
    }

    fn check_type(
        &mut self,
        ty: TypeId,
        semantics: &SemanticModel,
        visiting: &mut Vec<TypeId>,
    ) -> Result<(), String> {
        if let Some(start) = visiting.iter().position(|candidate| *candidate == ty) {
            // Snapshot decoding rejects object cycles and freezes its entire
            // owned graph. Recursive schemas through a class therefore produce
            // finite values. Keep rejecting cycles of ordinary local values.
            return if visiting[start..]
                .iter()
                .any(|ty| matches!(semantics.types().kind(*ty), TypeKind::ManagedClass(_)))
            {
                Ok(())
            } else {
                Err("recursive values do not currently support structural equality".to_owned())
            };
        }
        visiting.push(ty);
        let result = match semantics.types().kind(ty) {
            TypeKind::Builtin(builtin)
                if self
                    .standard_library
                    .core_type_has_capability(*builtin, StdlibCapabilityId::Equatable) =>
            {
                Ok(())
            }
            TypeKind::Standard(standard)
                if self
                    .standard_library
                    .type_has_capability(*standard, StdlibCapabilityId::Equatable) =>
            {
                Ok(())
            }
            TypeKind::ManagedClass(_) => self.check_aggregate(ty, semantics, visiting),
            TypeKind::Struct(structure) => self.check_aggregate(
                self.structural
                    .semantic_type(StructuralTypeId::Struct(*structure)),
                semantics,
                visiting,
            ),
            TypeKind::Enum(enumeration) => self.check_aggregate(
                self.structural
                    .semantic_type(StructuralTypeId::Enum(*enumeration)),
                semantics,
                visiting,
            ),
            TypeKind::Option { value, .. } => self.check_type(*value, semantics, visiting),
            TypeKind::Result { value, .. } => self.check_type(*value, semantics, visiting),
            TypeKind::Array { element, .. } => self.check_type(*element, semantics, visiting),
            _ => Err("the contained type does not support equality".to_owned()),
        };
        visiting.pop();
        result
    }

    fn check_aggregate(
        &mut self,
        ty: TypeId,
        semantics: &SemanticModel,
        visiting: &mut Vec<TypeId>,
    ) -> Result<(), String> {
        let aggregate = self
            .structural
            .get(ty)
            .expect("source aggregate types have shared structural metadata")
            .clone();
        for member in aggregate.members {
            let Some(member_ty) = member.ty else {
                continue;
            };
            self.check_type(member_ty, semantics, visiting)
                .map_err(|error| match aggregate.id {
                    StructuralTypeId::Struct(_) => format!(
                        "struct `{}.{}` does not support equality: {error}",
                        aggregate.name, member.name
                    ),
                    StructuralTypeId::Enum(_) => format!(
                        "enum `{}.{}` does not support equality: {error}",
                        aggregate.name, member.name
                    ),
                    StructuralTypeId::ManagedClass(_) => format!(
                        "managed class `{}.{}` does not support equality: {error}",
                        aggregate.name, member.name
                    ),
                })?;
        }
        Ok(())
    }
}

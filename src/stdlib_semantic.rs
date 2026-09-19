//! Compiler-semantic adapters for the backend-neutral standard-library graph.
//!
//! The catalog schema and graph deliberately do not depend on inference or
//! semantic `TypeKind` values. This module is the one-way adapter from those
//! compiler types into catalog candidate and applicability queries.

use crate::{
    stdlib::{
        CapabilityBehavior, Implementation, ItemKind, StandardBinaryOperator, StandardLibrary,
        StandardUnaryOperator, StdlibCapabilityId, StdlibItem, StdlibTypeConstructorId, TypeRef,
    },
    types::TypeKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallCandidate {
    pub item: &'static StdlibItem,
}

impl CallCandidate {
    pub const fn receiver(&self) -> Option<TypeRef> {
        match self.item.kind {
            ItemKind::Method { receiver } => Some(receiver),
            ItemKind::Function | ItemKind::Constant => None,
        }
    }
}

/// Compiler-specific queries layered over the backend-neutral catalog graph.
pub trait StandardLibrarySemanticExt {
    fn function_candidates(&self, path: &[String]) -> Vec<CallCandidate>;
    fn function_candidates_including_private(&self, path: &[String]) -> Vec<CallCandidate>;
    fn method_candidates(&self, name: &str) -> Vec<CallCandidate>;
    fn method_candidates_including_private(&self, name: &str) -> Vec<CallCandidate>;
    fn binary_operator_candidates(&self, operator: StandardBinaryOperator) -> Vec<CallCandidate>;
    fn unary_operator_candidates(&self, operator: StandardUnaryOperator) -> Vec<CallCandidate>;
    fn methods_for_type(&self, receiver: &TypeKind) -> Vec<&'static StdlibItem>;
    fn resolve_path(&self, path: &[String]) -> Option<CallCandidate>;
}

impl StandardLibrarySemanticExt for StandardLibrary {
    fn function_candidates(&self, path: &[String]) -> Vec<CallCandidate> {
        let qualified_name = path.join(".");
        if let Some(item) = self.item_by_name(&qualified_name) {
            return match item.kind {
                ItemKind::Function => vec![CallCandidate { item }],
                ItemKind::Method { .. } | ItemKind::Constant => Vec::new(),
            };
        }

        Vec::new()
    }

    fn function_candidates_including_private(&self, path: &[String]) -> Vec<CallCandidate> {
        let qualified_name = path.join(".");
        if let Some(item) = self.item_by_name_including_private(&qualified_name) {
            return match item.kind {
                ItemKind::Function => vec![CallCandidate { item }],
                ItemKind::Method { .. } | ItemKind::Constant => Vec::new(),
            };
        }
        Vec::new()
    }

    fn method_candidates(&self, name: &str) -> Vec<CallCandidate> {
        self.method_items_named(name)
            .map(|item| CallCandidate { item })
            .collect()
    }

    fn method_candidates_including_private(&self, name: &str) -> Vec<CallCandidate> {
        self.method_items_named_including_private(name)
            .map(|item| CallCandidate { item })
            .collect()
    }

    fn binary_operator_candidates(&self, operator: StandardBinaryOperator) -> Vec<CallCandidate> {
        self.binary_operator_items(operator)
            .map(|item| CallCandidate { item })
            .collect()
    }

    fn unary_operator_candidates(&self, operator: StandardUnaryOperator) -> Vec<CallCandidate> {
        self.unary_operator_items(operator)
            .map(|item| CallCandidate { item })
            .collect()
    }

    fn methods_for_type(&self, receiver: &TypeKind) -> Vec<&'static StdlibItem> {
        self.methods()
            .filter(|item| {
                item.implementation != Implementation::CapabilityRequirement
                    || matches!(receiver, TypeKind::Iterator { .. })
            })
            .filter(|item| catalog_method_accepts(self, item, receiver))
            .collect()
    }

    fn resolve_path(&self, path: &[String]) -> Option<CallCandidate> {
        self.function_candidates(path).into_iter().next()
    }
}

fn catalog_method_accepts(
    library: &StandardLibrary,
    item: &StdlibItem,
    receiver: &TypeKind,
) -> bool {
    let declared = match item.kind {
        ItemKind::Method { receiver } => receiver,
        ItemKind::Function | ItemKind::Constant => return false,
    };
    match declared {
        TypeRef::Core(expected) => {
            matches!(receiver, TypeKind::Builtin(actual) if *actual == expected)
        }
        TypeRef::Application { constructor, .. } => {
            (constructor == StdlibTypeConstructorId::Array
                && matches!(receiver, TypeKind::Array { .. }))
                || (constructor == StdlibTypeConstructorId::Option
                    && matches!(receiver, TypeKind::Option { .. }))
                || (constructor == StdlibTypeConstructorId::Result
                    && matches!(receiver, TypeKind::Result { .. }))
                || (constructor == StdlibTypeConstructorId::Set
                    && matches!(receiver, TypeKind::Set { .. }))
                || matches!(receiver, TypeKind::Application { constructor: actual, .. }
                    if *actual == constructor)
                || (constructor == StdlibTypeConstructorId::ExclusiveRange
                    && matches!(
                        receiver,
                        TypeKind::Range {
                            kind: crate::ast::RangeKind::Exclusive,
                            ..
                        }
                    ))
                || (constructor == StdlibTypeConstructorId::InclusiveRange
                    && matches!(
                        receiver,
                        TypeKind::Range {
                            kind: crate::ast::RangeKind::Inclusive,
                            ..
                        }
                    ))
        }
        TypeRef::FixedArray { length, .. } => {
            matches!(receiver, TypeKind::Array { length: Some(actual), .. } if *actual == length)
        }
        TypeRef::Standard(expected) => {
            matches!(receiver, TypeKind::Standard(actual) if *actual == expected)
                // SettingsView is program-shaped, but its shared methods are
                // still declared by the source-defined standard-library type.
                || (expected == crate::stdlib::StdlibTypeId::SettingsView
                    && matches!(receiver, TypeKind::SettingsView))
        }
        TypeRef::Parameter(name) => item
            .signature
            .type_parameters
            .iter()
            .find(|parameter| parameter.name == name)
            .is_none_or(|parameter| {
                parameter.constraints.iter().all(|constraint| {
                    semantic_type_may_have_capability(library, receiver, *constraint)
                })
            }),
        TypeRef::Associated(_) => false,
        TypeRef::Async(_) => matches!(receiver, TypeKind::Async { .. }),
        TypeRef::Iterator(_) => matches!(receiver, TypeKind::Iterator { .. }),
        TypeRef::Callable { .. } => matches!(receiver, TypeKind::Callable { .. }),
    }
}

// Candidate discovery has only a TypeKind, not the declarations required to
// prove recursive capabilities. CapabilityAnalysis performs final validation.
fn semantic_type_may_have_capability(
    library: &StandardLibrary,
    ty: &TypeKind,
    capability: StdlibCapabilityId,
) -> bool {
    if library.has_universal_debug_fallback(capability) {
        return !matches!(
            ty,
            TypeKind::Error
                | TypeKind::Builtin(crate::stdlib::CoreTypeId::Never)
                | TypeKind::GenericParameter { .. }
        );
    }
    let behavior = library.capability(capability).behavior;
    if behavior == CapabilityBehavior::StructuralManagedRead {
        return semantic_type_may_have_capability(library, ty, StdlibCapabilityId::MemoryReadable)
            || matches!(
                ty,
                TypeKind::Standard(crate::stdlib::StdlibTypeId::String) | TypeKind::Option { .. }
            );
    }
    match ty {
        TypeKind::Error => false,
        TypeKind::Builtin(builtin) => library.core_type_has_capability(*builtin, capability),
        TypeKind::Standard(standard) => library.type_has_capability(*standard, capability),
        TypeKind::StateSnapshot | TypeKind::SettingsView => false,
        TypeKind::Struct(_) => matches!(
            behavior,
            CapabilityBehavior::StructuralEquality
                | CapabilityBehavior::StructuralMemoryLayout
                | CapabilityBehavior::StructuralMethods
        ),
        TypeKind::Enum(_) => matches!(
            behavior,
            CapabilityBehavior::StructuralEquality
                | CapabilityBehavior::StructuralMemoryLayout
                | CapabilityBehavior::StructuralMethods
        ),
        TypeKind::ManagedClass(_) => false,
        TypeKind::ManagedReference(_) => false,
        TypeKind::Option { .. } | TypeKind::Result { .. } => {
            behavior == CapabilityBehavior::StructuralEquality
        }
        TypeKind::Array { length, .. } => {
            behavior == CapabilityBehavior::StructuralMemoryLayout && length.is_some()
        }
        TypeKind::GenericParameter { .. } => false,
        TypeKind::Iterator { .. } => matches!(
            capability,
            StdlibCapabilityId::Iterator
                | StdlibCapabilityId::Iterable
                | StdlibCapabilityId::Debug
                | StdlibCapabilityId::Display
        ),
        TypeKind::Async { .. } | TypeKind::Callable { .. } => false,
        TypeKind::Range { .. } => false,
        TypeKind::Set { .. } => false,
        TypeKind::Application { constructor, .. } => {
            library.type_constructor_has_capability(*constructor, capability)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn backend_neutral_catalog_does_not_depend_on_semantic_types() {
        for (path, source) in [
            ("stdlib.rs", include_str!("stdlib.rs")),
            ("stdlib/catalog.rs", include_str!("stdlib/catalog.rs")),
            (
                "stdlib/declarations.rs",
                include_str!("stdlib/declarations.rs"),
            ),
            ("stdlib/graph.rs", include_str!("stdlib/graph.rs")),
            ("stdlib/ids.rs", include_str!("stdlib/ids.rs")),
            ("stdlib/schema.rs", include_str!("stdlib/schema.rs")),
            (
                "stdlib/standard.split",
                include_str!("../stdlib/standard.split"),
            ),
            ("stdlib/validation.rs", include_str!("stdlib/validation.rs")),
        ] {
            assert!(
                !source.contains("crate::types")
                    && !source.contains("types::TypeKind")
                    && !source.contains("BuiltinType"),
                "backend-neutral catalog module `{path}` depends on compiler semantic types"
            );
        }
    }
}

//! Type-directed managed decoder nodes. TypeId is already interned by semantic
//! analysis, so child edges share one node rather than expanding type trees.
//! This graph is compiler data: constructing it does not retain Wasm readers.

use std::collections::HashMap;

pub(crate) const MAX_SNAPSHOT_DEPTH: u32 = 64;
pub(crate) const MAX_SNAPSHOT_OBJECTS: i64 = 1024;
pub(crate) const MAX_SNAPSHOT_WORK: i64 = 16_384;
/// Depth, object visits, field visits, active object path, then byte work.
pub(crate) const SNAPSHOT_BYTE_SLOT: u32 = 3 + MAX_SNAPSHOT_DEPTH;
pub(crate) const SNAPSHOT_CONTEXT_SLOTS: u32 = SNAPSHOT_BYTE_SLOT + 1;
/// Combined remote payload and owned storage charged to one materialization.
pub(crate) const MAX_MANAGED_READ_BYTES: i64 = 1024 * 1024;

use crate::{
    ast::{ManagedClassDecl, ManagedClassId},
    memory::MemoryLayouts,
    semantic::SemanticModel,
    stdlib::StdlibTypeId,
    types::{TypeId, TypeKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedDecoder {
    /// Fixed-layout base case, proven directly without re-entering capability
    /// implication (`MemoryReadable` itself implies `ManagedReadable`).
    Memory,
    String,
    Class {
        class: ManagedClassId,
    },
    /// A nullable object slot; the payload is decoded by its interned node.
    Optional {
        value: TypeId,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct ManagedReadTypes {
    nodes: HashMap<TypeId, ManagedDecoder>,
}

impl ManagedReadTypes {
    pub(crate) fn build(
        memory: &MemoryLayouts,
        semantics: &SemanticModel,
        classes: &[&ManagedClassDecl],
    ) -> Self {
        let mut nodes = HashMap::new();
        for (ty, kind) in semantics.types().iter() {
            let node = match kind {
                TypeKind::Standard(StdlibTypeId::String) => ManagedDecoder::String,
                TypeKind::Option { value, .. }
                    if matches!(
                        semantics.types().kind(*value),
                        TypeKind::Standard(StdlibTypeId::String) | TypeKind::ManagedClass(_)
                    ) =>
                {
                    ManagedDecoder::Optional { value: *value }
                }
                _ if memory.require_layout(ty, semantics).is_ok() => ManagedDecoder::Memory,
                _ => continue,
            };
            nodes.insert(ty, node);
        }
        // Intern all class nodes before checking children. Recursive schemas
        // are finite graphs; object cycles are rejected by the runtime path.
        for class in classes {
            nodes.insert(
                semantics.types().id_for_managed_class(class.id),
                ManagedDecoder::Class { class: class.id },
            );
        }
        loop {
            let mut invalid = classes
                .iter()
                .filter_map(|class| {
                    let ty = semantics.types().id_for_managed_class(class.id);
                    (nodes.contains_key(&ty)
                        && class
                            .all_fields()
                            .filter(|field| !field.is_static)
                            .any(|field| {
                                !nodes.contains_key(
                                    &semantics.managed_field_snapshot_type(field.id).unwrap(),
                                )
                            }))
                    .then_some(ty)
                })
                .collect::<Vec<_>>();
            invalid.extend(nodes.iter().filter_map(|(ty, node)| {
                matches!(node, ManagedDecoder::Optional { value } if !nodes.contains_key(value))
                    .then_some(*ty)
            }));
            if invalid.is_empty() {
                break;
            }
            for ty in invalid {
                nodes.remove(&ty);
            }
        }
        Self { nodes }
    }

    pub(crate) fn decoder(&self, ty: TypeId) -> Option<ManagedDecoder> {
        self.nodes.get(&ty).copied()
    }

    pub(crate) fn require(&self, ty: TypeId, semantics: &SemanticModel) -> Result<(), String> {
        self.decoder(ty).map(|_| ()).ok_or_else(|| format!(
            "type `{:?}` has no implemented managed decoder and does not satisfy `ManagedReadable`",
            semantics.types().kind(ty),
        ))
    }
}

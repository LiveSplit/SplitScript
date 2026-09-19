//! Type-directed managed decoder nodes. TypeId is already interned by semantic
//! analysis, so child edges share one node rather than expanding type trees.
//! This graph is compiler data: constructing it does not retain Wasm readers.

use std::collections::HashMap;

use crate::{
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
    pub(crate) fn build(memory: &MemoryLayouts, semantics: &SemanticModel) -> Self {
        let mut nodes = HashMap::new();
        for (ty, kind) in semantics.types().iter() {
            let node = match kind {
                TypeKind::Standard(StdlibTypeId::String) => ManagedDecoder::String,
                TypeKind::Option { value, .. }
                    if matches!(
                        semantics.types().kind(*value),
                        TypeKind::Standard(StdlibTypeId::String)
                    ) =>
                {
                    ManagedDecoder::Optional { value: *value }
                }
                _ if memory.require_layout(ty, semantics).is_ok() => ManagedDecoder::Memory,
                _ => continue,
            };
            nodes.insert(ty, node);
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

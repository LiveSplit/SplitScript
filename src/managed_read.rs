//! Type-directed managed decoder nodes, keyed by their remote storage type.
//! Each node also names its owned output type. Child edges identify storage
//! plans even when multiple remote representations produce the same value.
//! This graph is compiler data: constructing it does not retain Wasm readers.

use std::collections::HashMap;

pub(crate) const MAX_SNAPSHOT_DEPTH: u32 = 64;
pub(crate) const MAX_SNAPSHOT_OBJECTS: i64 = 1024;
pub(crate) const MAX_SNAPSHOT_WORK: i64 = 16_384;
/// Depth, object visits, field visits, active object path, then byte work.
/// Collection reads append a shared element counter to this base context.
pub(crate) const SNAPSHOT_BYTE_SLOT: u32 = 3 + MAX_SNAPSHOT_DEPTH;
pub(crate) const SNAPSHOT_CONTEXT_SLOTS: u32 = SNAPSHOT_BYTE_SLOT + 1;
pub(crate) const SNAPSHOT_ELEMENT_SLOT: u32 = SNAPSHOT_CONTEXT_SLOTS;
pub(crate) const MAX_MANAGED_ELEMENTS: i64 = 16_384;
pub(crate) const SNAPSHOT_SCAN_SLOT: u32 = SNAPSHOT_ELEMENT_SLOT + 1;
pub(crate) const MAX_MANAGED_SCANNED_SLOTS: i64 = 4096;
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
pub(crate) struct ManagedDecoder {
    pub output: TypeId,
    pub kind: ManagedDecoderKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedDecoderKind {
    /// Fixed-layout base case, proven directly without re-entering capability
    /// implication (`MemoryReadable` itself implies `ManagedReadable`).
    Memory,
    String,
    Array {
        element: TypeId,
    },
    List {
        element: TypeId,
    },
    Map {
        key: TypeId,
        value: TypeId,
    },
    Set {
        element: TypeId,
    },
    Class {
        class: ManagedClassId,
    },
    /// A nullable object slot; the payload is decoded by its interned node.
    Optional {
        value: TypeId,
    },
}

impl ManagedDecoderKind {
    pub(crate) fn children(self) -> impl Iterator<Item = TypeId> {
        match self {
            Self::Array { element }
            | Self::List { element }
            | Self::Set { element }
            | Self::Optional { value: element } => [Some(element), None],
            Self::Map { key, value } => [Some(key), Some(value)],
            _ => [None, None],
        }
        .into_iter()
        .flatten()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ManagedReadTypes {
    nodes: HashMap<TypeId, ManagedDecoder>,
}

impl ManagedReadTypes {
    pub(crate) fn build(
        memory: &MemoryLayouts,
        equality: &crate::equality::EqualityCapabilities,
        semantics: &SemanticModel,
        classes: &[&ManagedClassDecl],
    ) -> Self {
        let mut nodes = HashMap::new();
        for (ty, kind) in semantics.types().iter() {
            let node = match kind {
                TypeKind::Standard(StdlibTypeId::String) => ManagedDecoderKind::String,
                TypeKind::Array {
                    element,
                    length: None,
                    ..
                } => ManagedDecoderKind::Array { element: *element },
                TypeKind::Set { element, .. } => ManagedDecoderKind::Set { element: *element },
                TypeKind::Application {
                    constructor,
                    arguments,
                    ..
                } if *constructor == crate::stdlib::StdlibTypeConstructorId::List => {
                    ManagedDecoderKind::List {
                        element: arguments[0],
                    }
                }
                TypeKind::Application {
                    constructor,
                    arguments,
                    ..
                } if *constructor == crate::stdlib::StdlibTypeConstructorId::Map => {
                    ManagedDecoderKind::Map {
                        key: arguments[0],
                        value: arguments[1],
                    }
                }
                TypeKind::Option { value, .. }
                    if matches!(
                        semantics.types().kind(*value),
                        TypeKind::Standard(StdlibTypeId::String)
                            | TypeKind::ManagedClass(_)
                            | TypeKind::Array { length: None, .. }
                            | TypeKind::Set { .. }
                    ) || matches!(semantics.types().kind(*value), TypeKind::Application { constructor, .. }
                        if matches!(*constructor, crate::stdlib::StdlibTypeConstructorId::List | crate::stdlib::StdlibTypeConstructorId::Map)) =>
                {
                    ManagedDecoderKind::Optional { value: *value }
                }
                _ if memory.require_layout(ty, semantics).is_ok() => ManagedDecoderKind::Memory,
                _ => continue,
            };
            let Some(output) = semantics.try_managed_owned_type(ty) else {
                continue;
            };
            nodes.insert(ty, ManagedDecoder { output, kind: node });
        }
        // Intern all class nodes before checking children. Recursive schemas
        // are finite graphs; object cycles are rejected by the runtime path.
        for class in classes {
            nodes.insert(
                semantics.types().id_for_managed_class(class.id),
                ManagedDecoder {
                    output: semantics.types().id_for_managed_class(class.id),
                    kind: ManagedDecoderKind::Class { class: class.id },
                },
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
                                !nodes
                                    .contains_key(&semantics.managed_field_type(field.id).unwrap())
                            }))
                    .then_some(ty)
                })
                .collect::<Vec<_>>();
            invalid.extend(nodes.iter().filter_map(|(ty, node)| {
                if let ManagedDecoderKind::Map { key, .. }
                | ManagedDecoderKind::Set { element: key } = node.kind
                    && let Some(key) = nodes.get(&key)
                    && equality.require(key.output, semantics).is_err()
                {
                    return Some(*ty);
                }
                node.kind
                    .children()
                    .any(|child| !nodes.contains_key(&child))
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

/// Conservative owned-storage and element work for an inline base case.
/// Zero-byte native fields can still construct substantial GC arrays.
pub(crate) fn inline_materialization_cost(
    ty: TypeId,
    memory: &MemoryLayouts,
    semantics: &SemanticModel,
) -> (i64, i64) {
    use crate::memory::{MemoryAddressWidth, MemoryTypeLayout};
    let (bytes, elements) = match memory
        .layout(ty, semantics, MemoryAddressWidth::Bit64)
        .unwrap()
    {
        MemoryTypeLayout::Scalar { .. } => (0, 0),
        MemoryTypeLayout::Enum(_) => (16, 0),
        MemoryTypeLayout::Struct(layout) => layout.fields.iter().fold(
            (16 + layout.fields.len() as i64 * 8, 0i64),
            |(bytes, elements), field| {
                let (child_bytes, child_elements) =
                    inline_materialization_cost(field.ty, memory, semantics);
                (
                    bytes.saturating_add(child_bytes),
                    elements.saturating_add(child_elements),
                )
            },
        ),
        MemoryTypeLayout::FixedArray(layout) => {
            let (bytes, elements) = inline_materialization_cost(layout.element, memory, semantics);
            (
                48i64.saturating_add((8 + bytes).saturating_mul(layout.length as i64)),
                (1 + elements).saturating_mul(layout.length as i64),
            )
        }
    };
    (
        bytes.min(MAX_MANAGED_READ_BYTES + 1),
        elements.min(MAX_MANAGED_ELEMENTS + 1),
    )
}

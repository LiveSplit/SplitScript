//! Deep freezing for inline memory values materialized by managed readers.
//! Only types containing owned arrays need a function; native reads stay mutable.

use super::{
    Type, array_value, context::EmissionContext, reachability::Reachability, semantic_type,
};
use crate::{
    capabilities::CapabilityAnalysis,
    memory::{MemoryAddressWidth, MemoryLayouts, MemoryTypeLayout},
    semantic::SemanticModel,
    types::{TypeId, TypeKind},
};
use std::collections::BTreeSet;
use wasm_encoder::{BlockType, Function, Instruction as I, ValType};

pub(super) fn contains_array(
    ty: TypeId,
    memory: &MemoryLayouts,
    semantics: &SemanticModel,
) -> bool {
    match memory.layout(ty, semantics, MemoryAddressWidth::Bit64) {
        Ok(MemoryTypeLayout::FixedArray(_)) => true,
        Ok(MemoryTypeLayout::Struct(layout)) => layout
            .fields
            .iter()
            .any(|field| contains_array(field.ty, memory, semantics)),
        _ => false,
    }
}

pub(super) fn required(
    reachable: &Reachability,
    capabilities: &CapabilityAnalysis,
    semantics: &SemanticModel,
) -> BTreeSet<TypeId> {
    let mut required = BTreeSet::new();
    let mut pending = reachable.managed_decoders().collect::<Vec<_>>();
    while let Some(ty) = pending.pop() {
        if !contains_array(ty, capabilities.memory(), semantics) || !required.insert(ty) {
            continue;
        }
        match capabilities
            .memory()
            .layout(ty, semantics, MemoryAddressWidth::Bit64)
            .unwrap()
        {
            MemoryTypeLayout::FixedArray(layout) => pending.push(layout.element),
            MemoryTypeLayout::Struct(layout) => {
                pending.extend(layout.fields.iter().map(|field| field.ty))
            }
            _ => unreachable!(),
        }
    }
    required
}

pub(super) fn compile(ty: TypeId, l: &EmissionContext<'_>) -> Function {
    let mut f = Function::new([(1, ValType::I32)]);
    match l
        .memory
        .layout(ty, l.semantics, MemoryAddressWidth::Bit64)
        .unwrap()
    {
        MemoryTypeLayout::FixedArray(layout) => {
            let TypeKind::Array { layout: array, .. } = l.semantics.types().kind(ty) else {
                unreachable!()
            };
            f.instruction(&I::LocalGet(0))
                .instruction(&I::RefAsNonNull)
                .instruction(&I::I32Const(array_value::FROZEN_VERSION))
                .instruction(&I::StructSet {
                    struct_type_index: l.gc.index(Type::Array(*array)),
                    field_index: array_value::VERSION_FIELD,
                });
            if let Some(child) = l.managed_freezers.get(&layout.element) {
                let storage = array_value::storage_id(*array, l.arrays, l.semantics);
                f.instruction(&I::Block(BlockType::Empty))
                    .instruction(&I::Loop(BlockType::Empty))
                    .instruction(&I::LocalGet(1))
                    .instruction(&I::I32Const(layout.length as i32))
                    .instruction(&I::I32GeU)
                    .instruction(&I::BrIf(1))
                    .instruction(&I::LocalGet(0));
                array_value::emit_backing(&mut f, l.gc, *array);
                f.instruction(&I::LocalGet(1))
                    .instruction(&I::ArrayGet(l.gc.index(Type::ArrayStorage(storage))))
                    .instruction(&I::Call(*child))
                    .instruction(&I::LocalGet(1))
                    .instruction(&I::I32Const(1))
                    .instruction(&I::I32Add)
                    .instruction(&I::LocalSet(1))
                    .instruction(&I::Br(0))
                    .instruction(&I::End)
                    .instruction(&I::End);
            }
        }
        MemoryTypeLayout::Struct(layout) => {
            for (index, field) in layout.fields.iter().enumerate() {
                if let Some(child) = l.managed_freezers.get(&field.ty) {
                    f.instruction(&I::LocalGet(0))
                        .instruction(&I::RefAsNonNull)
                        .instruction(&I::StructGet {
                            struct_type_index: l.gc.index(semantic_type(ty, l.semantics)),
                            field_index: index as u32,
                        })
                        .instruction(&I::Call(*child));
                }
            }
        }
        _ => unreachable!(),
    }
    f.instruction(&I::End);
    f
}

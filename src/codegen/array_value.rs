//! Physical operations for source-level array values.
//!
//! A source array is a stable wrapper around replaceable raw Wasm GC array
//! storage. This keeps aliases valid when a growable `[T]` needs a larger
//! capacity while allowing `[T; N]` to retain its exact semantic length.

use wasm_encoder::{BlockType, Function, Instruction};

use crate::ast::ArrayTypeId;
use crate::semantic::SemanticModel;
use crate::types::ResolvedArrayType;

use super::{GcLayout, Type, try_array_element_type};

pub(super) const BACKING_FIELD: u32 = 0;
pub(super) const LENGTH_FIELD: u32 = 1;
pub(super) const VERSION_FIELD: u32 = 2;
pub(super) const FROZEN_VERSION: i32 = -1;

/// Consumes an array and traps before mutation if it belongs to a snapshot.
pub(super) fn emit_require_mutable(function: &mut Function, gc: &GcLayout, array: ArrayTypeId) {
    emit_version(function, gc, array);
    function
        .instruction(&Instruction::I32Const(FROZEN_VERSION))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Unreachable)
        .instruction(&Instruction::End);
}

/// Returns the general raw storage used by a source wrapper when one exists.
pub(super) fn storage_id(
    array: ArrayTypeId,
    arrays: &[ResolvedArrayType],
    semantics: &SemanticModel,
) -> ArrayTypeId {
    let declaration = arrays
        .iter()
        .find(|candidate| candidate.id == array)
        .expect("array values have resolved layouts");
    let element = try_array_element_type(array, semantics)
        .expect("array values have lowerable element types");
    arrays
        .iter()
        .find(|candidate| {
            candidate.length.is_none()
                && try_array_element_type(candidate.id, semantics) == Some(element)
        })
        .unwrap_or(declaration)
        .id
}

/// Wraps the raw array storage currently on the operand stack.
pub(super) fn emit_wrap(function: &mut Function, gc: &GcLayout, array: ArrayTypeId, length: u32) {
    function.instruction(&Instruction::I32Const(length as i32));
    emit_wrap_loaded(function, gc.index(Type::Array(array)));
}

/// Wraps backing storage and a logical length already on the operand stack.
///
/// Keeping wrapper construction here prevents runtime helpers from needing to
/// know about compiler-owned metadata fields such as the structural version.
pub(super) fn emit_wrap_loaded(function: &mut Function, array_type: u32) {
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::StructNew(array_type));
}

/// Creates exact raw storage from the values on the operand stack and wraps it.
pub(super) fn emit_new_fixed(
    function: &mut Function,
    gc: &GcLayout,
    array: ArrayTypeId,
    length: u32,
) {
    function.instruction(&Instruction::ArrayNewFixed {
        array_type_index: gc.index(Type::ArrayStorage(array)),
        array_size: length,
    });
    emit_wrap(function, gc, array, length);
}

/// Replaces a source array on the operand stack with its non-null raw backing.
pub(super) fn emit_backing(function: &mut Function, gc: &GcLayout, array: ArrayTypeId) {
    function
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.index(Type::Array(array)),
            field_index: BACKING_FIELD,
        })
        .instruction(&Instruction::RefAsNonNull);
}

/// Replaces a source array on the operand stack with its logical length.
pub(super) fn emit_length(function: &mut Function, gc: &GcLayout, array: ArrayTypeId) {
    function.instruction(&Instruction::StructGet {
        struct_type_index: gc.index(Type::Array(array)),
        field_index: LENGTH_FIELD,
    });
}

/// Replaces a source array on the operand stack with its structural version.
pub(super) fn emit_version(function: &mut Function, gc: &GcLayout, array: ArrayTypeId) {
    function.instruction(&Instruction::StructGet {
        struct_type_index: gc.index(Type::Array(array)),
        field_index: VERSION_FIELD,
    });
}

/// Increments the structural version of the source array in local zero.
pub(super) fn emit_increment_version(
    function: &mut Function,
    gc: &GcLayout,
    array: ArrayTypeId,
    freezing: bool,
) {
    let array_type = gc.index(Type::Array(array));
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::StructGet {
            struct_type_index: array_type,
            field_index: VERSION_FIELD,
        })
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add);
    if freezing {
        // Reserve negative versions for immutable owned values.
        function
            .instruction(&Instruction::I32Const(i32::MAX))
            .instruction(&Instruction::I32And);
    }
    function.instruction(&Instruction::StructSet {
        struct_type_index: array_type,
        field_index: VERSION_FIELD,
    });
}

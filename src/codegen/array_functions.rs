//! Compiler-generated operations for concrete growable source arrays.

use crate::codegen::GC_NULL_HEAP_TYPE;
use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, Instruction, StorageType, ValType};

use crate::{ast::ArrayTypeId, types::ResolvedArrayType};

use super::{GcLayout, Type, array_value};

#[derive(Debug, Default)]
pub(super) struct ArrayFunctions {
    pub freezing: bool,
    sets: HashMap<ArrayTypeId, u32>,
    pushes: HashMap<ArrayTypeId, u32>,
    removals: HashMap<ArrayTypeId, u32>,
    clears: HashMap<ArrayTypeId, u32>,
}

impl ArrayFunctions {
    pub(super) fn insert_set(&mut self, array: ArrayTypeId, function: u32) {
        self.sets.insert(array, function);
    }
    pub(super) fn set(&self, array: ArrayTypeId) -> Option<u32> {
        self.sets.get(&array).copied()
    }

    pub(super) fn insert_push(&mut self, array: ArrayTypeId, function: u32) {
        self.pushes.insert(array, function);
    }

    pub(super) fn push(&self, array: ArrayTypeId) -> u32 {
        self.pushes[&array]
    }

    pub(super) fn insert_remove_at(&mut self, array: ArrayTypeId, function: u32) {
        self.removals.insert(array, function);
    }

    pub(super) fn remove_at(&self, array: ArrayTypeId) -> u32 {
        self.removals[&array]
    }

    pub(super) fn insert_clear(&mut self, array: ArrayTypeId, function: u32) {
        self.clears.insert(array, function);
    }

    pub(super) fn clear(&self, array: ArrayTypeId) -> u32 {
        self.clears[&array]
    }
}

pub(super) fn compile(
    arrays: &[ResolvedArrayType],
    plans: &ArrayFunctions,
    semantics: &crate::semantic::SemanticModel,
    gc: &GcLayout,
) -> Vec<Function> {
    let mut functions = Vec::new();
    for array in arrays {
        if plans.sets.contains_key(&array.id) {
            let mut f = Function::new([]);
            f.instruction(&Instruction::LocalGet(0));
            array_value::emit_require_mutable(&mut f, gc, array.id);
            f.instruction(&Instruction::LocalGet(0));
            array_value::emit_backing(&mut f, gc, array.id);
            f.instruction(&Instruction::LocalGet(1))
                .instruction(&Instruction::LocalGet(2))
                .instruction(&Instruction::ArraySet(gc.index(Type::ArrayStorage(
                    array_value::storage_id(array.id, arrays, semantics),
                ))))
                .instruction(&Instruction::End);
            functions.push(f);
        }
        if plans.pushes.contains_key(&array.id) {
            functions.push(compile_push(array, arrays, semantics, gc, plans.freezing));
        }
        if plans.removals.contains_key(&array.id) {
            functions.push(compile_remove_at(
                array,
                arrays,
                semantics,
                gc,
                plans.freezing,
            ));
        }
        if plans.clears.contains_key(&array.id) {
            functions.push(compile_clear(array, arrays, semantics, gc, plans.freezing));
        }
    }
    functions
}

fn compile_push(
    array: &ResolvedArrayType,
    arrays: &[ResolvedArrayType],
    semantics: &crate::semantic::SemanticModel,
    gc: &GcLayout,
    freezing: bool,
) -> Function {
    debug_assert!(array.length.is_none());
    let storage = array_value::storage_id(array.id, arrays, semantics);
    let storage_type = gc.val_type(Type::ArrayStorage(storage));

    // Parameters: array, value. Locals: backing, replacement, length, capacity.
    let mut function = Function::new([(2, storage_type), (2, ValType::I32)]);
    if freezing {
        function.instruction(&Instruction::LocalGet(0));
        array_value::emit_require_mutable(&mut function, gc, array.id);
    }
    let backing = 2;
    let replacement = 3;
    let length = 4;
    let capacity = 5;
    let array_type = gc.index(Type::Array(array.id));
    let storage_type = gc.index(Type::ArrayStorage(storage));

    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::StructGet {
            struct_type_index: array_type,
            field_index: array_value::BACKING_FIELD,
        })
        .instruction(&Instruction::LocalSet(backing))
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::StructGet {
            struct_type_index: array_type,
            field_index: array_value::LENGTH_FIELD,
        })
        .instruction(&Instruction::LocalSet(length))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalTee(capacity))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(capacity))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(capacity))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32Mul)
        .instruction(&Instruction::End)
        .instruction(&Instruction::ArrayNewDefault(storage_type))
        .instruction(&Instruction::LocalTee(replacement))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::ArrayCopy {
            array_type_index_dst: storage_type,
            array_type_index_src: storage_type,
        })
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(replacement))
        .instruction(&Instruction::StructSet {
            struct_type_index: array_type,
            field_index: array_value::BACKING_FIELD,
        })
        .instruction(&Instruction::LocalGet(replacement))
        .instruction(&Instruction::LocalSet(backing))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::ArraySet(storage_type))
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::StructSet {
            struct_type_index: array_type,
            field_index: array_value::LENGTH_FIELD,
        });
    array_value::emit_increment_version(&mut function, gc, array.id, freezing);
    function.instruction(&Instruction::End);
    function
}

fn compile_remove_at(
    array: &ResolvedArrayType,
    arrays: &[ResolvedArrayType],
    semantics: &crate::semantic::SemanticModel,
    gc: &GcLayout,
    freezing: bool,
) -> Function {
    debug_assert!(array.length.is_none());
    let storage = array_value::storage_id(array.id, arrays, semantics);
    let storage_ref = gc.val_type(Type::ArrayStorage(storage));
    let storage_type = gc.index(Type::ArrayStorage(storage));
    let array_type = gc.index(Type::Array(array.id));
    let element = super::try_array_element_type(array.id, semantics)
        .expect("reachable arrays have lowerable element types");

    // Parameters: array, index. Locals: backing, previous length.
    let mut function = Function::new([(1, storage_ref), (1, ValType::I32)]);
    if freezing {
        function.instruction(&Instruction::LocalGet(0));
        array_value::emit_require_mutable(&mut function, gc, array.id);
    }
    let backing = 2;
    let length = 3;
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::StructGet {
            struct_type_index: array_type,
            field_index: array_value::BACKING_FIELD,
        })
        .instruction(&Instruction::LocalSet(backing))
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::StructGet {
            struct_type_index: array_type,
            field_index: array_value::LENGTH_FIELD,
        })
        .instruction(&Instruction::LocalSet(length))
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Unreachable)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::ArrayCopy {
            array_type_index_dst: storage_type,
            array_type_index_src: storage_type,
        });

    // Array copying leaves the final logical slot duplicated. Release a
    // reference there so the removed value is not retained by spare capacity.
    if let StorageType::Val(ValType::Ref(_)) = gc.storage_type(element) {
        function
            .instruction(&Instruction::LocalGet(backing))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::LocalGet(length))
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::I32Sub)
            .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
            .instruction(&Instruction::ArraySet(storage_type));
    }

    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::StructSet {
            struct_type_index: array_type,
            field_index: array_value::LENGTH_FIELD,
        });
    array_value::emit_increment_version(&mut function, gc, array.id, freezing);
    function.instruction(&Instruction::End);
    function
}

fn compile_clear(
    array: &ResolvedArrayType,
    arrays: &[ResolvedArrayType],
    semantics: &crate::semantic::SemanticModel,
    gc: &GcLayout,
    freezing: bool,
) -> Function {
    debug_assert!(array.length.is_none());
    let storage = array_value::storage_id(array.id, arrays, semantics);
    let storage_ref = gc.val_type(Type::ArrayStorage(storage));
    let storage_type = gc.index(Type::ArrayStorage(storage));
    let array_type = gc.index(Type::Array(array.id));
    let element = super::try_array_element_type(array.id, semantics)
        .expect("reachable arrays have lowerable element types");

    // Parameter: array. Locals: backing, previous length.
    let mut function = Function::new([(1, storage_ref), (1, ValType::I32)]);
    if freezing {
        function.instruction(&Instruction::LocalGet(0));
        array_value::emit_require_mutable(&mut function, gc, array.id);
    }
    let backing = 1;
    let length = 2;
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::StructGet {
            struct_type_index: array_type,
            field_index: array_value::BACKING_FIELD,
        })
        .instruction(&Instruction::LocalSet(backing))
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::StructGet {
            struct_type_index: array_type,
            field_index: array_value::LENGTH_FIELD,
        })
        .instruction(&Instruction::LocalSet(length));

    // Primitive slots do not keep GC objects alive. Null every live reference
    // slot while retaining the backing allocation and its capacity.
    if let StorageType::Val(ValType::Ref(_)) = gc.storage_type(element) {
        function
            .instruction(&Instruction::LocalGet(backing))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
            .instruction(&Instruction::LocalGet(length))
            .instruction(&Instruction::ArrayFill(storage_type));
    }

    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::StructSet {
            struct_type_index: array_type,
            field_index: array_value::LENGTH_FIELD,
        });
    array_value::emit_increment_version(&mut function, gc, array.id, freezing);
    function.instruction(&Instruction::End);
    function
}

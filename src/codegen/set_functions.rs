//! Compiler-generated operations for concrete `Set<T>` instantiations.

use std::collections::{HashMap, HashSet};

use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::{ast::TypeApplicationId, stdlib::IntrinsicId, types::ResolvedSetType};

use super::{
    EqualityFunctions, GcLayout, Type, emit_array_get, emit_default,
    runtime_helpers::emit_value_equality, set_element_type,
};

pub(super) const BACKING_FIELD: u32 = 0;
pub(super) const LENGTH_FIELD: u32 = 1;
pub(super) const VERSION_FIELD: u32 = 2;

#[derive(Debug, Clone, Copy)]
pub(super) struct SetFunctionPlan {
    pub new: Option<u32>,
    pub length: Option<u32>,
    pub contains: Option<u32>,
    pub insert: Option<u32>,
    pub remove: Option<u32>,
    pub clear: Option<u32>,
}

#[derive(Debug, Default)]
pub(super) struct SetFunctions {
    plans: HashMap<TypeApplicationId, SetFunctionPlan>,
}

impl SetFunctions {
    pub(super) fn insert(&mut self, set: TypeApplicationId, plan: SetFunctionPlan) {
        self.plans.insert(set, plan);
    }

    pub(super) fn function(&self, set: TypeApplicationId, intrinsic: IntrinsicId) -> u32 {
        let plan = self
            .plans
            .get(&set)
            .expect("reachable set operations have generated functions");
        match intrinsic {
            IntrinsicId::SetNew => plan.new,
            IntrinsicId::SetLength => plan.length,
            IntrinsicId::SetContains => plan.contains,
            IntrinsicId::SetInsert => plan.insert,
            IntrinsicId::SetRemove => plan.remove,
            IntrinsicId::SetClear => plan.clear,
            _ => unreachable!("only set intrinsics use the set-function plan"),
        }
        .expect("reachable set operations have generated functions")
    }
}

pub(super) fn compile(
    sets: &[ResolvedSetType],
    plans: &SetFunctions,
    semantics: &crate::semantic::SemanticModel,
    equality: &EqualityFunctions,
    string_equality: u32,
    gc: &GcLayout,
    frozen_sets: &HashSet<TypeApplicationId>,
) -> Vec<Function> {
    let mut bodies = Vec::new();
    for set in sets {
        let Some(plan) = plans.plans.get(&set.id).copied() else {
            continue;
        };
        let element = set_element_type(set.id, semantics);
        let frozen = frozen_sets.contains(&set.id);
        if plan.new.is_some() {
            bodies.push(compile_new(set, gc));
        }
        if plan.length.is_some() {
            bodies.push(compile_length(set, gc));
        }
        if plan.contains.is_some() {
            bodies.push(compile_contains(
                set,
                element,
                equality,
                string_equality,
                gc,
            ));
        }
        if plan.insert.is_some() {
            bodies.push(compile_insert(
                set,
                plan.contains.expect("set insertion requires contains"),
                gc,
                frozen,
            ));
        }
        if plan.remove.is_some() {
            bodies.push(compile_remove(
                set,
                element,
                equality,
                string_equality,
                gc,
                frozen,
            ));
        }
        if plan.clear.is_some() {
            bodies.push(compile_clear(set, gc, frozen));
        }
    }
    bodies
}

fn compile_new(set: &ResolvedSetType, gc: &GcLayout) -> Function {
    let mut function = Function::new([]);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::ArrayNewDefault(
            gc.index(Type::ArrayStorage(set.backing)),
        ))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::StructNew(gc.index(Type::Set(set.id))))
        .instruction(&Instruction::End);
    function
}

fn compile_length(set: &ResolvedSetType, gc: &GcLayout) -> Function {
    let mut function = Function::new([]);
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.index(Type::Set(set.id)),
            field_index: LENGTH_FIELD,
        })
        .instruction(&Instruction::End);
    function
}

fn compile_contains(
    set: &ResolvedSetType,
    element: Type,
    equality: &EqualityFunctions,
    string_equality: u32,
    gc: &GcLayout,
) -> Function {
    // Parameters: set, value. Locals: backing, length, index.
    let mut function = Function::new([
        (1, gc.val_type(Type::ArrayStorage(set.backing))),
        (2, ValType::I32),
    ]);
    let backing = 2;
    let length = 3;
    let index = 4;
    load_set_state(&mut function, set, backing, length, gc);
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index));
    emit_array_get(
        &mut function,
        gc.index(Type::ArrayStorage(set.backing)),
        element,
        gc,
    );
    function.instruction(&Instruction::LocalGet(1));
    emit_value_equality(&mut function, element, equality, string_equality);
    function
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::End);
    function
}

fn compile_insert(set: &ResolvedSetType, contains: u32, gc: &GcLayout, frozen: bool) -> Function {
    // Parameters: set, value. Locals: backing, replacement, length, capacity.
    let backing_type = gc.val_type(Type::ArrayStorage(set.backing));
    let mut function = Function::new([(2, backing_type), (2, ValType::I32)]);
    let backing = 2;
    let replacement = 3;
    let length = 4;
    let capacity = 5;
    require_mutable(&mut function, set, gc, frozen);
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::Call(contains))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
    load_set_state(&mut function, set, backing, length, gc);
    function
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
        .instruction(&Instruction::ArrayNewDefault(
            gc.index(Type::ArrayStorage(set.backing)),
        ))
        .instruction(&Instruction::LocalTee(replacement))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::ArrayCopy {
            array_type_index_dst: gc.index(Type::ArrayStorage(set.backing)),
            array_type_index_src: gc.index(Type::ArrayStorage(set.backing)),
        })
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(replacement))
        .instruction(&Instruction::StructSet {
            struct_type_index: gc.index(Type::Set(set.id)),
            field_index: BACKING_FIELD,
        })
        .instruction(&Instruction::LocalGet(replacement))
        .instruction(&Instruction::LocalSet(backing))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::ArraySet(
            gc.index(Type::ArrayStorage(set.backing)),
        ))
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::StructSet {
            struct_type_index: gc.index(Type::Set(set.id)),
            field_index: LENGTH_FIELD,
        })
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.index(Type::Set(set.id)),
            field_index: VERSION_FIELD,
        })
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::StructSet {
            struct_type_index: gc.index(Type::Set(set.id)),
            field_index: VERSION_FIELD,
        })
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End);
    function
}

fn compile_remove(
    set: &ResolvedSetType,
    element: Type,
    equality: &EqualityFunctions,
    string_equality: u32,
    gc: &GcLayout,
    frozen: bool,
) -> Function {
    // Parameters: set, value. Locals: backing, length, index.
    let mut function = Function::new([
        (1, gc.val_type(Type::ArrayStorage(set.backing))),
        (2, ValType::I32),
    ]);
    let backing = 2;
    let length = 3;
    let index = 4;
    require_mutable(&mut function, set, gc, frozen);
    load_set_state(&mut function, set, backing, length, gc);
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index));
    emit_array_get(
        &mut function,
        gc.index(Type::ArrayStorage(set.backing)),
        element,
        gc,
    );
    function.instruction(&Instruction::LocalGet(1));
    emit_value_equality(&mut function, element, equality, string_equality);
    function
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::ArrayCopy {
            array_type_index_dst: gc.index(Type::ArrayStorage(set.backing)),
            array_type_index_src: gc.index(Type::ArrayStorage(set.backing)),
        })
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub);
    emit_default(&mut function, element, gc);
    function
        .instruction(&Instruction::ArraySet(
            gc.index(Type::ArrayStorage(set.backing)),
        ))
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::StructSet {
            struct_type_index: gc.index(Type::Set(set.id)),
            field_index: LENGTH_FIELD,
        })
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.index(Type::Set(set.id)),
            field_index: VERSION_FIELD,
        })
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::StructSet {
            struct_type_index: gc.index(Type::Set(set.id)),
            field_index: VERSION_FIELD,
        })
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::End);
    function
}

fn compile_clear(set: &ResolvedSetType, gc: &GcLayout, frozen: bool) -> Function {
    let mut function = Function::new([]);
    require_mutable(&mut function, set, gc, frozen);
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::ArrayNewDefault(
            gc.index(Type::ArrayStorage(set.backing)),
        ))
        .instruction(&Instruction::StructSet {
            struct_type_index: gc.index(Type::Set(set.id)),
            field_index: BACKING_FIELD,
        })
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::StructSet {
            struct_type_index: gc.index(Type::Set(set.id)),
            field_index: LENGTH_FIELD,
        })
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.index(Type::Set(set.id)),
            field_index: VERSION_FIELD,
        })
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::StructSet {
            struct_type_index: gc.index(Type::Set(set.id)),
            field_index: VERSION_FIELD,
        })
        .instruction(&Instruction::End);
    function
}

fn load_set_state(
    function: &mut Function,
    set: &ResolvedSetType,
    backing: u32,
    length: u32,
    gc: &GcLayout,
) {
    let set_type = gc.index(Type::Set(set.id));
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::StructGet {
            struct_type_index: set_type,
            field_index: BACKING_FIELD,
        })
        .instruction(&Instruction::LocalSet(backing))
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::StructGet {
            struct_type_index: set_type,
            field_index: LENGTH_FIELD,
        })
        .instruction(&Instruction::LocalSet(length));
}

fn require_mutable(function: &mut Function, set: &ResolvedSetType, gc: &GcLayout, frozen: bool) {
    if !frozen {
        return;
    }
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.index(Type::Set(set.id)),
            field_index: VERSION_FIELD,
        })
        .instruction(&Instruction::I32Const(super::array_value::FROZEN_VERSION))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Unreachable)
        .instruction(&Instruction::End);
}

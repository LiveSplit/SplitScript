//! Demand-driven transactional readers for source-declared managed classes.
//!
//! A reader accepts one live `T.Ref` address and returns `T!`. Every active
//! instance field is read before the immutable GC snapshot is constructed, so
//! callers can never observe a partially populated object. Conditional fields
//! retain stable GC slots but are read only when their attachment-wide layout
//! predicate is active.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::{ast::ManagedClassId, types::TypeKind};

use super::{
    Type, emit_default, emit_result_success, emit_typed_struct_get,
    expression::{ExprContext, MatchLayout, emit_managed_field_read},
};

pub(super) fn compile(
    class: ManagedClassId,
    lowering: &super::context::EmissionContext<'_>,
) -> Function {
    let binding = lowering
        .managed
        .classes
        .iter()
        .find(|candidate| candidate.id == class)
        .expect("reachable managed snapshot classes have binding plans");
    let fields = binding
        .all_fields()
        .filter(|field| field.kind == crate::managed::ManagedFieldKind::Instance)
        .collect::<Vec<_>>();
    let snapshot_id = lowering.semantics.types().id_for_managed_class(class);
    let result = result_for(snapshot_id, lowering);

    // Parameter 0 is the live remote address. Each following local owns one
    // field Result so its success value remains available until all sibling
    // reads have succeeded.
    let mut locals = fields
        .iter()
        .map(|field| {
            let field_result = result_for(field.value_type, lowering);
            let mut ty = lowering.gc.val_type(Type::Result(field_result));
            let ValType::Ref(reference) = &mut ty else {
                unreachable!("Result values use GC references")
            };
            reference.nullable = true;
            (1, ty)
        })
        .collect::<Vec<_>>();
    let mut error_type = lowering
        .gc
        .val_type(Type::Standard(crate::stdlib::StdlibTypeId::String));
    let ValType::Ref(error_reference) = &mut error_type else {
        unreachable!("String values use GC references")
    };
    error_reference.nullable = true;
    locals.push((1, error_type));
    let mut function = Function::new(locals);
    let error_local = fields.len() as u32 + 1;
    let values = HashMap::new();
    let temporaries = HashMap::new();
    let matches = MatchLayout::default();
    let context = ExprContext::compiler_generated(lowering, &values, &temporaries, &matches);

    // All failures leave this block with their message stored in one shared
    // local. The outer Result error is then constructed once instead of
    // duplicating that relatively large sequence for every class field.
    function.instruction(&Instruction::Block(BlockType::Empty));
    for (index, field) in fields.iter().enumerate() {
        let field_result = result_for(field.value_type, lowering);
        let field_type = super::semantic_type(field.value_type, lowering.semantics);
        if let Some(predicate) = lowering.semantics.managed_field_shape_predicate(field.id) {
            super::update::emit_shape_predicate(
                &mut function,
                lowering.program,
                predicate,
                lowering.semantics,
                lowering.gc,
                lowering.globals,
                super::update::PredicateState::Unavailable,
            );
            function.instruction(&Instruction::If(BlockType::Result(
                lowering.gc.val_type(Type::Result(field_result)),
            )));
            emit_field_read(&mut function, field.id, &context);
            function.instruction(&Instruction::Else);
            emit_default(&mut function, field_type, lowering.gc);
            emit_result_success(&mut function, field_result, lowering.gc);
            function.instruction(&Instruction::End);
        } else {
            emit_field_read(&mut function, field.id, &context);
        }

        let local = index as u32 + 1;
        function
            .instruction(&Instruction::LocalTee(local))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(
            &mut function,
            lowering.gc.index(Type::Result(field_result)),
            1,
            Type::I32,
        );
        function.instruction(&Instruction::If(BlockType::Empty));
        function
            .instruction(&Instruction::LocalGet(local))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(
            &mut function,
            lowering.gc.index(Type::Result(field_result)),
            2,
            Type::Standard(crate::stdlib::StdlibTypeId::String),
        );
        function
            .instruction(&Instruction::LocalSet(error_local))
            .instruction(&Instruction::Br(1))
            .instruction(&Instruction::End);
    }

    for (index, field) in fields.iter().enumerate() {
        let field_result = result_for(field.value_type, lowering);
        function
            .instruction(&Instruction::LocalGet(index as u32 + 1))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(
            &mut function,
            lowering.gc.index(Type::Result(field_result)),
            0,
            super::semantic_type(field.value_type, lowering.semantics),
        );
    }
    function.instruction(&Instruction::StructNew(
        lowering.gc.index(Type::ManagedClass(class)),
    ));
    emit_result_success(&mut function, result, lowering.gc);
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    emit_default(&mut function, Type::ManagedClass(class), lowering.gc);
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalGet(error_local))
        // Unobserved diagnostics are erased in release builds. Propagating
        // failure must preserve that nullable payload without dereferencing it.
        .instruction(&Instruction::StructNew(
            lowering.gc.index(Type::Result(result)),
        ))
        .instruction(&Instruction::End);
    function
}

fn emit_field_read(
    function: &mut Function,
    field: crate::ast::ManagedFieldId,
    context: &ExprContext<'_>,
) {
    function
        .instruction(&Instruction::GlobalGet(context.runtime_globals.process))
        .instruction(&Instruction::LocalGet(0));
    emit_managed_field_read(function, field, context);
}

fn result_for(
    value: crate::types::TypeId,
    lowering: &super::context::EmissionContext<'_>,
) -> crate::ast::ResultTypeId {
    lowering
        .semantics
        .types()
        .iter()
        .find_map(|(_, kind)| match kind {
            TypeKind::Result {
                layout,
                value: candidate,
            } if *candidate == value => Some(*layout),
            _ => None,
        })
        .expect("managed reads have concrete Result layouts")
}

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
    Type, emit_default, emit_result_error, emit_result_success, emit_typed_struct_get,
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
    let enter = lowering
        .runtime_helpers
        .optional_function(crate::intrinsic_registry::RuntimeHelperId::EnterManagedObject);
    let parameter_count = 1 + u32::from(enter.is_some());

    // Parameters are the live remote address and the shared root context.
    // Each following local owns one
    // field Result so its success value remains available until all sibling
    // reads have succeeded.
    let mut locals = fields
        .iter()
        .map(|field| {
            let field_result = result_for(field.snapshot_type, lowering);
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
    let mut live_locals = HashMap::new();
    for field in &fields {
        if field.snapshot_type != field.value_type {
            let local = locals.len() as u32 + parameter_count;
            locals.push((
                1,
                lowering
                    .gc
                    .val_type(Type::Result(result_for(field.value_type, lowering))),
            ));
            let child_local = if let TypeKind::Option { value, .. } =
                lowering.semantics.types().kind(field.snapshot_type)
            {
                let child_local = locals.len() as u32 + parameter_count;
                locals.push((
                    1,
                    lowering
                        .gc
                        .val_type(Type::Result(result_for(*value, lowering))),
                ));
                Some(child_local)
            } else {
                None
            };
            live_locals.insert(field.id, (local, child_local));
        }
    }
    let mut function = Function::new(locals);
    let error_local = fields.len() as u32 + parameter_count;
    let values = HashMap::new();
    let temporaries = HashMap::new();
    let matches = MatchLayout::default();
    let mut context = ExprContext::compiler_generated(lowering, &values, &temporaries, &matches);
    context.managed_read_context = enter.map(|_| 1);

    if let Some(enter) = enter {
        function
            .instruction(&Instruction::LocalGet(1))
            .instruction(&Instruction::LocalGet(0))
            .instruction(&Instruction::Call(enter))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_result_error(
            &mut function,
            result,
            Type::ManagedClass(class),
            "managed snapshot encountered a null object, cycle, or object/depth limit",
            lowering.gc,
            lowering.failure_payloads,
        );
        function
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
    }

    // All failures leave this block with their message stored in one shared
    // local. The outer Result error is then constructed once instead of
    // duplicating that relatively large sequence for every class field.
    function.instruction(&Instruction::Block(BlockType::Empty));
    for (index, field) in fields.iter().enumerate() {
        let field_result = result_for(field.snapshot_type, lowering);
        let field_type = super::semantic_type(field.snapshot_type, lowering.semantics);
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
            emit_field_read(
                &mut function,
                field,
                live_locals.get(&field.id).copied(),
                lowering,
                &context,
            );
            function.instruction(&Instruction::Else);
            emit_default(&mut function, field_type, lowering.gc);
            emit_result_success(&mut function, field_result, lowering.gc);
            function.instruction(&Instruction::End);
        } else {
            emit_field_read(
                &mut function,
                field,
                live_locals.get(&field.id).copied(),
                lowering,
                &context,
            );
        }

        let local = index as u32 + parameter_count;
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

    if enter.is_some() {
        leave_object(&mut function, lowering.gc);
    }
    for (index, field) in fields.iter().enumerate() {
        let field_result = result_for(field.snapshot_type, lowering);
        function
            .instruction(&Instruction::LocalGet(index as u32 + parameter_count))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(
            &mut function,
            lowering.gc.index(Type::Result(field_result)),
            0,
            super::semantic_type(field.snapshot_type, lowering.semantics),
        );
    }
    function.instruction(&Instruction::StructNew(
        lowering.gc.index(Type::ManagedClass(class)),
    ));
    emit_result_success(&mut function, result, lowering.gc);
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    if enter.is_some() {
        leave_object(&mut function, lowering.gc);
    }
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
    field: &crate::managed::ManagedFieldBinding,
    live_local: Option<(u32, Option<u32>)>,
    lowering: &super::context::EmissionContext<'_>,
    context: &ExprContext<'_>,
) {
    let result = result_for(field.snapshot_type, lowering);
    let charge = context
        .runtime_helpers
        .optional_function(crate::intrinsic_registry::RuntimeHelperId::ChargeManagedWork);
    if let Some(charge) = charge {
        function
            .instruction(&Instruction::LocalGet(1))
            .instruction(&Instruction::Call(charge))
            .instruction(&Instruction::If(BlockType::Result(
                context.gc.val_type(Type::Result(result)),
            )));
    }
    function
        .instruction(&Instruction::GlobalGet(context.runtime_globals.process))
        .instruction(&Instruction::LocalGet(0));
    emit_managed_field_read(function, field.id, context);
    if let Some((local, child_local)) = live_local {
        let live_result = result_for(field.value_type, lowering);
        let live_gc = context.gc.index(Type::Result(live_result));
        function
            .instruction(&Instruction::LocalTee(local))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::StructGet {
                struct_type_index: live_gc,
                field_index: 1,
            })
            .instruction(&Instruction::If(BlockType::Result(
                context.gc.val_type(Type::Result(result)),
            )));
        emit_default(
            function,
            super::semantic_type(field.snapshot_type, context.semantics),
            context.gc,
        );
        function
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::LocalGet(local))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::StructGet {
                struct_type_index: live_gc,
                field_index: 2,
            })
            .instruction(&Instruction::StructNew(
                context.gc.index(Type::Result(result)),
            ))
            .instruction(&Instruction::Else)
            .instruction(&Instruction::LocalGet(local))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::StructGet {
                struct_type_index: live_gc,
                field_index: 0,
            });
        let (child_type, optional) = match context.semantics.types().kind(field.snapshot_type) {
            TypeKind::Option { layout, value } => (*value, Some(*layout)),
            _ => (field.snapshot_type, None),
        };
        let TypeKind::ManagedClass(child) = context.semantics.types().kind(child_type) else {
            unreachable!("only child class fields differ from their live projection")
        };
        if let Some(option) = optional {
            function
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::If(BlockType::Result(
                    context.gc.val_type(Type::Result(result)),
                )))
                .instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
                    context.gc.index(Type::Option(option)),
                )));
            emit_result_success(function, result, context.gc);
            let TypeKind::Option {
                layout: live_option,
                ..
            } = context.semantics.types().kind(field.value_type)
            else {
                unreachable!()
            };
            function
                .instruction(&Instruction::Else)
                .instruction(&Instruction::LocalGet(local))
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: live_gc,
                    field_index: 0,
                })
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.index(Type::Option(*live_option)),
                    field_index: 0,
                });
        }
        function
            .instruction(&Instruction::LocalGet(1))
            .instruction(&Instruction::Call(
                context.managed_snapshot_functions[child],
            ));
        if let Some(option) = optional {
            let child_result = result_for(child_type, lowering);
            let child_gc = context.gc.index(Type::Result(child_result));
            let child_local = child_local.expect("optional child decoding has a result local");
            function
                .instruction(&Instruction::LocalTee(child_local))
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: child_gc,
                    field_index: 1,
                })
                .instruction(&Instruction::If(BlockType::Result(
                    context.gc.val_type(Type::Result(result)),
                )));
            emit_default(function, Type::Option(option), context.gc);
            function
                .instruction(&Instruction::I32Const(1))
                .instruction(&Instruction::LocalGet(child_local))
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: child_gc,
                    field_index: 2,
                })
                .instruction(&Instruction::StructNew(
                    context.gc.index(Type::Result(result)),
                ))
                .instruction(&Instruction::Else)
                .instruction(&Instruction::LocalGet(child_local))
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: child_gc,
                    field_index: 0,
                })
                .instruction(&Instruction::StructNew(
                    context.gc.index(Type::Option(option)),
                ));
            emit_result_success(function, result, context.gc);
            function
                .instruction(&Instruction::End)
                .instruction(&Instruction::End);
        }
        function.instruction(&Instruction::End);
    }
    if charge.is_some() {
        function.instruction(&Instruction::Else);
        emit_result_error(
            function,
            result,
            super::semantic_type(field.snapshot_type, context.semantics),
            "managed read work limit exceeded",
            context.gc,
            context.failure_payloads,
        );
        function.instruction(&Instruction::End);
    }
}

fn leave_object(function: &mut Function, gc: &super::GcLayout) {
    let array = gc.standard_index(crate::stdlib::StdlibTypeId::ManagedReadContext);
    function
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::ArrayGet(array))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::ArraySet(array));
}

pub(super) fn result_for(
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

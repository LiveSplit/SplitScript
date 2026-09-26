//! Demand-driven transactional readers for source-declared managed classes.
//!
//! A reader accepts one live `T.Ref` address and returns `T!`. Every active
//! instance field is read before the immutable GC snapshot is constructed, so
//! callers can never observe a partially populated object. Conditional fields
//! retain stable GC slots but are read only when their attachment-wide layout
//! predicate is active.

use crate::codegen::GC_NULL_HEAP_TYPE;
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
    let (_, _, class_callable) =
        super::managed_decoders::binding(lowering, crate::stdlib::MANAGED_OBJECT_TYPE_FIELD);
    let class_result = result_for(
        lowering
            .semantics
            .types()
            .id_for_core(crate::stdlib::CoreTypeId::Address),
        lowering,
    );
    let class_callback_local = locals.len() as u32 + parameter_count;
    let nullable = |mut ty: ValType| {
        if let ValType::Ref(reference) = &mut ty {
            reference.nullable = true;
        }
        ty
    };
    locals.push((
        1,
        nullable(lowering.gc.val_type(Type::Callable(class_callable))),
    ));
    let class_result_local = locals.len() as u32 + parameter_count;
    locals.push((
        1,
        nullable(lowering.gc.val_type(Type::Result(class_result))),
    ));
    let actual_class_local = locals.len() as u32 + parameter_count;
    locals.push((1, ValType::I64));
    let class_context_local = if enter.is_some() {
        1
    } else {
        let local = locals.len() as u32 + parameter_count;
        locals.push((
            1,
            nullable(lowering.gc.val_type(Type::Standard(
                crate::stdlib::StdlibTypeId::ManagedReadContext,
            ))),
        ));
        local
    };
    let mut function = Function::new(locals);
    let error_local = fields.len() as u32 + parameter_count;
    let values = HashMap::new();
    let temporaries = HashMap::new();
    let matches = MatchLayout::default();
    let mut context = ExprContext::compiler_generated(lowering, &values, &temporaries, &matches);
    context.managed_read_context = Some(class_context_local);

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
    if enter.is_none() {
        function
            .instruction(&Instruction::I32Const(
                crate::managed_read::SNAPSHOT_CONTEXT_SLOTS as i32,
            ))
            .instruction(&Instruction::ArrayNewDefault(
                lowering
                    .gc
                    .standard_index(crate::stdlib::StdlibTypeId::ManagedReadContext),
            ))
            .instruction(&Instruction::LocalSet(class_context_local));
    }
    emit_class_check(
        &mut function,
        class,
        lowering,
        class_callback_local,
        class_result_local,
        actual_class_local,
        class_context_local,
        error_local,
        true,
    );
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
        if lowering.failure_payloads.is_demanded(result) {
            let name = &lowering
                .program
                .managed_class(class)
                .unwrap()
                .all_fields()
                .find(|declaration| declaration.id == field.id)
                .unwrap()
                .name;
            super::emit_string_literal(&mut function, name, lowering.gc);
            function.instruction(&Instruction::Call(
                lowering
                    .runtime_helpers
                    .function(crate::intrinsic_registry::RuntimeHelperId::ManagedErrorField),
            ));
        }
        function
            .instruction(&Instruction::LocalSet(error_local))
            .instruction(&Instruction::Br(1))
            .instruction(&Instruction::End);
    }

    emit_class_check(
        &mut function,
        class,
        lowering,
        class_callback_local,
        class_result_local,
        actual_class_local,
        class_context_local,
        error_local,
        false,
    );
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

#[allow(clippy::too_many_arguments)]
fn emit_class_check(
    f: &mut Function,
    class: ManagedClassId,
    l: &super::context::EmissionContext<'_>,
    callback_local: u32,
    result_local: u32,
    actual_local: u32,
    context_local: u32,
    error_local: u32,
    remember: bool,
) {
    use Instruction as I;
    let (structure, field, callable) =
        super::managed_decoders::binding(l, crate::stdlib::MANAGED_OBJECT_TYPE_FIELD);
    let callable_type = l.gc.index(Type::Callable(callable));
    let result = result_for(
        l.semantics
            .types()
            .id_for_core(crate::stdlib::CoreTypeId::Address),
        l,
    );
    let class_field_name = crate::stdlib::managed_class_address_name(class.index());
    let class_field = l
        .structs
        .iter()
        .find(|s| s.id == structure)
        .unwrap()
        .fields
        .iter()
        .position(|f| f.name == class_field_name)
        .unwrap() as u32;
    f.instruction(&I::GlobalGet(
        l.runtime_globals.provider_preparation_value.unwrap(),
    ))
    .instruction(&I::RefAsNonNull)
    .instruction(&I::StructGet {
        struct_type_index: l.gc.index(Type::Struct(structure)),
        field_index: field,
    })
    .instruction(&I::LocalTee(callback_local))
    .instruction(&I::StructGet {
        struct_type_index: callable_type,
        field_index: 1,
    })
    .instruction(&I::LocalGet(0))
    .instruction(&I::GlobalGet(
        l.runtime_globals.provider_preparation_value.unwrap(),
    ))
    .instruction(&I::RefAsNonNull)
    .instruction(&I::StructGet {
        struct_type_index: l.gc.index(Type::Struct(structure)),
        field_index: class_field,
    });
    if remember {
        f.instruction(&I::I64Const(0));
    } else {
        f.instruction(&I::LocalGet(actual_local));
    }
    f.instruction(&I::LocalGet(context_local))
        .instruction(&I::RefAsNonNull)
        .instruction(&I::LocalGet(callback_local))
        .instruction(&I::StructGet {
            struct_type_index: callable_type,
            field_index: 0,
        })
        .instruction(&I::CallRef(l.gc.callable_function_index(callable)))
        .instruction(&I::LocalTee(result_local))
        .instruction(&I::RefAsNonNull);
    emit_typed_struct_get(f, l.gc.index(Type::Result(result)), 1, Type::I32);
    f.instruction(&I::If(BlockType::Empty))
        .instruction(&I::LocalGet(result_local))
        .instruction(&I::RefAsNonNull);
    emit_typed_struct_get(
        f,
        l.gc.index(Type::Result(result)),
        2,
        Type::Standard(crate::stdlib::StdlibTypeId::String),
    );
    f.instruction(&I::LocalSet(error_local))
        .instruction(&I::Br(1))
        .instruction(&I::End);
    if remember {
        f.instruction(&I::LocalGet(result_local))
            .instruction(&I::RefAsNonNull);
        emit_typed_struct_get(f, l.gc.index(Type::Result(result)), 0, Type::Address);
        f.instruction(&I::LocalSet(actual_local));
    }
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
            .instruction(&Instruction::LocalGet(
                context.managed_read_context.expect("snapshot context"),
            ))
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
        if optional.is_some() {
            function
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::If(BlockType::Result(
                    context.gc.val_type(Type::Result(result)),
                )))
                .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE));
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
            .instruction(&Instruction::LocalGet(
                context.managed_read_context.expect("snapshot context"),
            ))
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

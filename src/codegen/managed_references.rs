//! Class-checked live references. Snapshot child reads use their snapshot check.
use wasm_encoder::{BlockType, Function, HeapType, Instruction as I, ValType};

use super::{
    Type, context::EmissionContext, emit_default, emit_result_error, emit_result_success,
    managed_decoders::binding, managed_snapshots::result_for,
};
use crate::{
    ast::ManagedClassId,
    intrinsic_registry::RuntimeHelperId,
    semantic::SemanticModel,
    stdlib::{CoreTypeId, MANAGED_OBJECT_TYPE_FIELD, managed_class_address_name},
    types::{TypeId, TypeKind},
};

pub(super) fn class(mut value: TypeId, semantics: &SemanticModel) -> Option<ManagedClassId> {
    if let TypeKind::Option { value: inner, .. } = semantics.types().kind(value) {
        value = *inner;
    }
    match semantics.types().kind(value) {
        TypeKind::ManagedReference(class) => Some(*class),
        _ => None,
    }
}

pub(super) fn compile(value: TypeId, l: &EmissionContext<'_>) -> Function {
    let result = result_for(value, l);
    let value_type = super::semantic_type(value, l.semantics);
    let class = class(value, l.semantics).unwrap();
    let option = match l.semantics.types().kind(value) {
        TypeKind::Option { layout, .. } => Some(*layout),
        _ => None,
    };
    let address_result = result_for(l.semantics.types().id_for_core(CoreTypeId::Address), l);
    let (structure, field, callable) = binding(l, MANAGED_OBJECT_TYPE_FIELD);
    let callable_type = l.gc.index(Type::Callable(callable));
    let nullable = |mut ty: ValType| {
        if let ValType::Ref(r) = &mut ty {
            r.nullable = true;
        }
        ty
    };
    // Arguments: process, slot, width, root context. Locals preserve the pointer
    // across metadata reads, which share the ordinary ABI read scratch storage.
    let mut f = Function::new([
        (1, ValType::I64),                                          // 4: object
        (1, nullable(l.gc.val_type(Type::Callable(callable)))),     // 5: checker
        (1, nullable(l.gc.val_type(Type::Result(address_result)))), // 6: proof
    ]);
    f.instruction(&I::LocalGet(0))
        .instruction(&I::LocalGet(1))
        .instruction(&I::I32Const(l.abi_read.destination(8)))
        .instruction(&I::LocalGet(2))
        .instruction(&I::LocalGet(2))
        .instruction(&I::Call(
            l.runtime_helpers
                .function(RuntimeHelperId::ReadManagedMemory),
        ))
        .instruction(&I::I32Eqz)
        .instruction(&I::If(BlockType::Empty));
    emit_result_error(
        &mut f,
        result,
        value_type,
        "managed field could not be read",
        l.gc,
        l.failure_payloads,
    );
    f.instruction(&I::Return).instruction(&I::End);
    f.instruction(&I::LocalGet(2))
        .instruction(&I::I32Const(8))
        .instruction(&I::I32Eq)
        .instruction(&I::If(BlockType::Result(ValType::I64)))
        .instruction(&I::I32Const(l.abi_read.destination(8)))
        .instruction(&I::I64Load(super::memarg()))
        .instruction(&I::Else)
        .instruction(&I::I32Const(l.abi_read.destination(8)))
        .instruction(&I::I32Load(super::memarg()))
        .instruction(&I::I64ExtendI32U)
        .instruction(&I::End)
        .instruction(&I::LocalTee(4))
        .instruction(&I::I64Eqz)
        .instruction(&I::If(BlockType::Empty));
    if let Some(option) = option {
        f.instruction(&I::RefNull(HeapType::Concrete(
            l.gc.index(Type::Option(option)),
        )));
        emit_result_success(&mut f, result, l.gc);
    } else {
        emit_result_error(
            &mut f,
            result,
            value_type,
            "managed field contained a null reference",
            l.gc,
            l.failure_payloads,
        );
    }
    f.instruction(&I::Return).instruction(&I::End);
    f.instruction(&I::GlobalGet(
        l.runtime_globals.provider_preparation_value.unwrap(),
    ))
    .instruction(&I::RefAsNonNull)
    .instruction(&I::StructGet {
        struct_type_index: l.gc.index(Type::Struct(structure)),
        field_index: field,
    })
    .instruction(&I::LocalTee(5))
    .instruction(&I::StructGet {
        struct_type_index: callable_type,
        field_index: 1,
    })
    .instruction(&I::LocalGet(4));
    let expected_name = managed_class_address_name(class.index());
    let expected = l
        .structs
        .iter()
        .find(|s| s.id == structure)
        .unwrap()
        .fields
        .iter()
        .position(|f| f.name == expected_name)
        .unwrap() as u32;
    f.instruction(&I::GlobalGet(
        l.runtime_globals.provider_preparation_value.unwrap(),
    ))
    .instruction(&I::RefAsNonNull)
    .instruction(&I::StructGet {
        struct_type_index: l.gc.index(Type::Struct(structure)),
        field_index: expected,
    })
    .instruction(&I::I64Const(0))
    .instruction(&I::LocalGet(3))
    .instruction(&I::LocalGet(5))
    .instruction(&I::RefAsNonNull)
    .instruction(&I::StructGet {
        struct_type_index: callable_type,
        field_index: 0,
    })
    .instruction(&I::CallRef(l.gc.callable_function_index(callable)))
    .instruction(&I::LocalTee(6))
    .instruction(&I::StructGet {
        struct_type_index: l.gc.index(Type::Result(address_result)),
        field_index: 1,
    })
    .instruction(&I::If(BlockType::Empty));
    emit_default(&mut f, value_type, l.gc);
    f.instruction(&I::I32Const(1))
        .instruction(&I::LocalGet(6))
        .instruction(&I::RefAsNonNull)
        .instruction(&I::StructGet {
            struct_type_index: l.gc.index(Type::Result(address_result)),
            field_index: 2,
        })
        .instruction(&I::StructNew(l.gc.index(Type::Result(result))))
        .instruction(&I::Return)
        .instruction(&I::End)
        .instruction(&I::LocalGet(4));
    if let Some(option) = option {
        f.instruction(&I::StructNew(l.gc.index(Type::Option(option))));
    }
    emit_result_success(&mut f, result, l.gc);
    f.instruction(&I::End);
    f
}

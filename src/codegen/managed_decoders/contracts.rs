//! Metadata proofs for reachable nested collection schemas. Each function is a
//! source-callable value, so the ordinary backend adapters invoke child proofs.
use wasm_encoder::{AbstractHeapType, BlockType, Function, HeapType, Instruction as I};

use super::super::{Type, context::EmissionContext, managed_snapshots::result_for};
use super::{
    Reader, binding,
    keyed::{callback_end, callback_start},
    nullable, storage_kinds, storage_width,
};
use crate::{
    ast::CallableTypeId,
    managed_read::ManagedDecoderKind as D,
    semantic::SemanticModel,
    stdlib::{CoreTypeId, StdlibTypeId},
    types::{TypeId, TypeKind},
};

pub(crate) fn checker_layout(semantics: &SemanticModel) -> CallableTypeId {
    let types = semantics.types();
    let parameters = [
        types.id_for_core(CoreTypeId::Address),
        types.id_for_core(CoreTypeId::U32),
        types.id_for_standard(StdlibTypeId::ManagedReadContext),
    ];
    types.iter().find_map(|(_, kind)| match kind {
        TypeKind::Callable { layout, parameters: actual, result }
            if actual.as_slice() == parameters && matches!(types.kind(*result), TypeKind::Result { value, .. } if *value == types.id_for_core(CoreTypeId::Bool)) => Some(*layout),
        _ => None,
    }).expect("recursive schema adapters declare the checker signature")
}

fn normalize(mut source: TypeId, l: &EmissionContext<'_>) -> TypeId {
    while let D::Optional { value } = l.capabilities.managed_decoder(source).unwrap().kind {
        source = value
    }
    source
}

pub(super) fn emit_checker(f: &mut Function, source: TypeId, l: &EmissionContext<'_>) {
    let source = normalize(source, l);
    f.instruction(&I::RefFunc(l.managed_contract_functions[&source]))
        .instruction(&I::RefNull(HeapType::Abstract {
            shared: false,
            ty: AbstractHeapType::Any,
        }))
        .instruction(&I::StructNew(
            l.gc.index(Type::Callable(checker_layout(l.semantics))),
        ));
}

pub(crate) fn compile(source: TypeId, l: &EmissionContext<'_>) -> Function {
    let kind = l.capabilities.managed_decoder(source).unwrap().kind;
    let name = crate::managed_read::schema_binding(kind);
    let (_, _, callable) = binding(l, name);
    let cached = matches!(
        kind,
        D::Array { .. } | D::List { .. } | D::Map { .. } | D::Set { .. }
    );
    let value = l.semantics.types().id_for_core(CoreTypeId::Bool);
    let result = result_for(value, l);
    let r = Reader {
        value,
        result,
        lowering: l,
        capabilities: l.capabilities,
        entered: None,
        list: false,
    };
    // Parameters match the source callable: environment, type, width, context.
    let mut locals = vec![(1, nullable(l.gc.val_type(Type::Callable(callable))))]; // 4
    if cached {
        let (_, _, proof) = binding(l, "__schema_proof");
        locals.push((1, nullable(l.gc.val_type(Type::Callable(proof))))); // 5
        locals.push((1, nullable(l.gc.val_type(Type::Result(result))))); // 6
    }
    let mut f = Function::new(locals);
    if cached {
        proof(&mut f, source, l, false);
        f.instruction(&I::LocalSet(6));
        r.forward_result_failure(&mut f, result, 6);
        r.result_field(&mut f, result, 6, 0, Type::Bool);
        f.instruction(&I::If(BlockType::Empty))
            .instruction(&I::LocalGet(6))
            .instruction(&I::RefAsNonNull)
            .instruction(&I::Return)
            .instruction(&I::End);
    }
    callback_start(&mut f, l, name, 4);
    f.instruction(&I::LocalGet(1));
    match kind {
        D::Memory | D::String => {
            storage_width(&mut f, &r, source);
            f.instruction(&I::I32Const(storage_kinds(&r, source) as i32))
                .instruction(&I::LocalGet(3));
        }
        D::Class { .. } => {
            r.expected_class(&mut f, source);
            f.instruction(&I::LocalGet(3));
        }
        _ => {
            f.instruction(&I::LocalGet(2)).instruction(&I::LocalGet(3));
            for child in kind.children() {
                emit_checker(&mut f, child, l)
            }
        }
    }
    callback_end(&mut f, l, callable, 4);
    if cached {
        f.instruction(&I::LocalSet(6));
        r.forward_result_failure(&mut f, result, 6);
        proof(&mut f, source, l, true);
    }
    f.instruction(&I::End);
    f
}

fn proof(f: &mut Function, source: TypeId, l: &EmissionContext<'_>, remember: bool) {
    let (_, _, callable) = binding(l, "__schema_proof");
    callback_start(f, l, "__schema_proof", 5);
    f.instruction(&I::LocalGet(1))
        .instruction(&I::I32Const(source.index() as i32))
        .instruction(&I::I32Const(i32::from(remember)))
        .instruction(&I::LocalGet(3));
    callback_end(f, l, callable, 5);
}

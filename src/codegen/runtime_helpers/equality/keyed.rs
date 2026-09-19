//! Order-independent, one-to-one matching of local map entries and set elements.

use wasm_encoder::{BlockType, Function, Instruction as I, ValType};

use super::{
    EqualityFunctions, GcLayout, SemanticModel, Type, array_value, emit_array_get,
    emit_typed_struct_get, emit_value_equality, semantic_type,
};
use crate::{
    stdlib::{StdlibTypeConstructorId as C, StdlibTypeId},
    types::{ResolvedArrayType, TypeId, TypeKind},
};

pub(super) fn compile(
    ty: TypeId,
    arrays: &[ResolvedArrayType],
    semantics: &SemanticModel,
    equality: &EqualityFunctions,
    string_eq: u32,
    gc: &GcLayout,
) -> Function {
    let collection = semantic_type(ty, semantics);
    let (storage, entries, entry, key, value) = match semantics.types().kind(ty) {
        TypeKind::Set {
            backing, element, ..
        } => (
            *backing,
            None,
            None,
            semantic_type(*element, semantics),
            None,
        ),
        TypeKind::Application {
            constructor: C::Map,
            arguments,
            ..
        } => {
            let entry = semantics.types().iter().find_map(|(ty, kind)| {
                matches!(kind, TypeKind::Application { constructor: C::MapEntry, arguments: entry_args, .. }
                    if entry_args == arguments).then_some(ty)
            }).expect("map storage has a concrete entry type");
            let entries = semantics
                .types()
                .iter()
                .find_map(|(_, kind)| match kind {
                    TypeKind::Array {
                        element,
                        layout,
                        length: None,
                    } if *element == entry => Some(*layout),
                    _ => None,
                })
                .expect("map storage has an entry array");
            (
                array_value::storage_id(entries, arrays, semantics),
                Some(entries),
                Some(semantic_type(entry, semantics)),
                semantic_type(arguments[0], semantics),
                Some(semantic_type(arguments[1], semantics)),
            )
        }
        _ => unreachable!(),
    };
    let storage_index = gc.index(Type::ArrayStorage(storage));
    let bytes_index = gc.standard_index(StdlibTypeId::String);
    // Parameters: left, right. Locals: their backing arrays, private matched
    // bitmap, length, and the two cursors. The bitmap uses the existing GC byte
    // array representation; it is never exposed as a language String value.
    let mut f = Function::new([
        (2, gc.val_type(Type::ArrayStorage(storage))),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
        (3, ValType::I32),
    ]);
    let left = 2;
    let right = 3;
    let matched = 4;
    let length = 5;
    let i = 6;
    let j = 7;
    let emit_length = |f: &mut Function, object| {
        f.instruction(&I::LocalGet(object))
            .instruction(&I::RefAsNonNull);
        if let Some(entries) = entries {
            emit_typed_struct_get(f, gc.index(collection), 0, Type::Array(entries));
            array_value::emit_length(f, gc, entries);
        } else {
            emit_typed_struct_get(f, gc.index(collection), 1, Type::U32);
        }
    };
    emit_length(&mut f, 0);
    f.instruction(&I::LocalTee(length));
    emit_length(&mut f, 1);
    f.instruction(&I::I32Ne)
        .instruction(&I::If(BlockType::Empty))
        .instruction(&I::I32Const(0))
        .instruction(&I::Return)
        .instruction(&I::End);
    for (object, backing) in [(0, left), (1, right)] {
        f.instruction(&I::LocalGet(object))
            .instruction(&I::RefAsNonNull);
        if let Some(entries) = entries {
            emit_typed_struct_get(&mut f, gc.index(collection), 0, Type::Array(entries));
            array_value::emit_backing(&mut f, gc, entries);
        } else {
            emit_typed_struct_get(&mut f, gc.index(collection), 0, Type::ArrayStorage(storage));
        }
        f.instruction(&I::LocalSet(backing));
    }
    f.instruction(&I::LocalGet(length))
        .instruction(&I::ArrayNewDefault(bytes_index))
        .instruction(&I::LocalSet(matched))
        .instruction(&I::Block(BlockType::Empty))
        .instruction(&I::Loop(BlockType::Empty))
        .instruction(&I::LocalGet(i))
        .instruction(&I::LocalGet(length))
        .instruction(&I::I32GeU)
        .instruction(&I::BrIf(1))
        .instruction(&I::I32Const(0))
        .instruction(&I::LocalSet(j))
        .instruction(&I::Block(BlockType::Empty))
        .instruction(&I::Loop(BlockType::Empty))
        .instruction(&I::LocalGet(j))
        .instruction(&I::LocalGet(length))
        .instruction(&I::I32GeU)
        .instruction(&I::If(BlockType::Empty))
        .instruction(&I::I32Const(0))
        .instruction(&I::Return)
        .instruction(&I::End)
        .instruction(&I::LocalGet(matched))
        .instruction(&I::RefAsNonNull)
        .instruction(&I::LocalGet(j))
        .instruction(&I::ArrayGetU(bytes_index))
        .instruction(&I::I32Eqz)
        .instruction(&I::If(BlockType::Result(ValType::I32)));
    let compare = |f: &mut Function, field_index, ty| {
        for (backing, index) in [(left, i), (right, j)] {
            f.instruction(&I::LocalGet(backing))
                .instruction(&I::RefAsNonNull)
                .instruction(&I::LocalGet(index));
            if let Some(entry) = entry {
                f.instruction(&I::ArrayGet(storage_index))
                    .instruction(&I::RefAsNonNull);
                emit_typed_struct_get(f, gc.index(entry), field_index, ty);
            } else {
                emit_array_get(f, storage_index, ty, gc);
            }
        }
        emit_value_equality(f, ty, equality, string_eq);
    };
    compare(&mut f, 0, key);
    if let Some(value) = value {
        f.instruction(&I::If(BlockType::Result(ValType::I32)));
        compare(&mut f, 1, value);
        f.instruction(&I::Else)
            .instruction(&I::I32Const(0))
            .instruction(&I::End);
    }
    f.instruction(&I::Else)
        .instruction(&I::I32Const(0))
        .instruction(&I::End)
        .instruction(&I::If(BlockType::Empty))
        .instruction(&I::LocalGet(matched))
        .instruction(&I::RefAsNonNull)
        .instruction(&I::LocalGet(j))
        .instruction(&I::I32Const(1))
        .instruction(&I::ArraySet(bytes_index))
        .instruction(&I::Br(2))
        .instruction(&I::End)
        .instruction(&I::LocalGet(j))
        .instruction(&I::I32Const(1))
        .instruction(&I::I32Add)
        .instruction(&I::LocalSet(j))
        .instruction(&I::Br(0))
        .instruction(&I::End)
        .instruction(&I::End)
        .instruction(&I::LocalGet(i))
        .instruction(&I::I32Const(1))
        .instruction(&I::I32Add)
        .instruction(&I::LocalSet(i))
        .instruction(&I::Br(0))
        .instruction(&I::End)
        .instruction(&I::End)
        .instruction(&I::I32Const(1))
        .instruction(&I::End);
    f
}

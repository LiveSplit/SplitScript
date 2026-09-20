//! Recursive dictionary and set materialization using validated, bounded slot scans.
use wasm_encoder::{BlockType, Function, Instruction as I, ValType};

use super::super::{
    Type, context::EmissionContext, emit_result_success, emit_typed_struct_get,
    managed_snapshots::result_for, semantic_type,
};
use super::{CONTEXT, Reader, nullable, read_word};
use crate::{
    capabilities::CapabilityAnalysis,
    intrinsic_registry::RuntimeHelperId as H,
    managed_read::ManagedDecoderKind,
    memory::MemoryAddressWidth,
    stdlib::{CoreTypeId, StdlibFieldId as F, StdlibTypeConstructorId as C, StdlibTypeId},
    types::{TypeId, TypeKind},
};

pub(super) fn compile(
    source: TypeId,
    key: Option<TypeId>,
    value: TypeId,
    l: &EmissionContext<'_>,
    capabilities: &CapabilityAnalysis,
) -> Function {
    let dictionary = key.is_some();
    let key = key.unwrap_or(value);
    let read_binding = if dictionary {
        crate::stdlib::MANAGED_MAP_READ_FIELD
    } else {
        crate::stdlib::MANAGED_SET_READ_FIELD
    };
    let noun = if dictionary { "dictionary" } else { "set" };
    let output = capabilities.managed_decoder(source).unwrap().output;
    let r = Reader {
        value: output,
        result: result_for(output, l),
        lowering: l,
        capabilities,
        entered: Some(15),
        list: true,
    };
    let key_type = semantic_type(r.output(key), l.semantics);
    let (entry_type, entries, storage) = if dictionary {
        let entry = l
            .semantics
            .types()
            .iter()
            .find_map(|(id, kind)| {
                matches!(kind, TypeKind::Application { constructor, arguments, .. }
        if *constructor == C::MapEntry && arguments.as_slice() == [r.output(key), r.output(value)])
                .then_some(id)
            })
            .unwrap();
        let entry_type = semantic_type(entry, l.semantics);
        let entries = l
            .semantics
            .types()
            .iter()
            .find_map(|(_, kind)| match kind {
                TypeKind::Array {
                    element,
                    length: None,
                    layout,
                } if *element == entry => Some(*layout),
                _ => None,
            })
            .unwrap();
        let storage = super::super::array_value::storage_id(entries, l.arrays, l.semantics);
        (Some(entry_type), Some(entries), storage)
    } else {
        let TypeKind::Set { backing, .. } = l.semantics.types().kind(output) else {
            unreachable!()
        };
        (None, None, *backing)
    };
    let storage_index = l.gc.index(Type::ArrayStorage(storage));
    let (_, _, read_callable) = binding(l, read_binding);
    let (_, _, verify_callable) = binding(l, crate::stdlib::MANAGED_KEYED_VERIFY_FIELD);
    let read_result = result_for(
        l.semantics
            .types()
            .id_for_standard(StdlibTypeId::UnityKeyedRead),
        l,
    );
    let verify_result = result_for(l.semantics.types().id_for_core(CoreTypeId::Bool), l);
    let slots = field_array(l, F::UnityKeyedReadSlots);
    let backing = field_array(l, F::UnityKeyedReadBacking);
    let mut f = Function::new([
        (1, nullable(l.gc.val_type(Type::Callable(read_callable)))), // 5
        (1, nullable(l.gc.val_type(Type::Result(read_result)))),     // 6
        (
            1,
            nullable(l.gc.val_type(Type::Standard(StdlibTypeId::UnityKeyedRead))),
        ), // 7
        (1, nullable(l.gc.val_type(Type::Array(slots)))),            // 8
        (1, nullable(l.gc.val_type(Type::Array(backing)))),          // 9
        (1, nullable(l.gc.val_type(Type::ArrayStorage(storage)))),   // 10
        (
            1,
            nullable(l.gc.val_type(Type::Result(result_for(r.output(key), l)))),
        ), // 11
        (
            1,
            nullable(l.gc.val_type(Type::Result(result_for(r.output(value), l)))),
        ), // 12
        (1, nullable(l.gc.val_type(Type::Callable(verify_callable)))), // 13
        (1, nullable(l.gc.val_type(Type::Result(verify_result)))),   // 14
        (4, ValType::I32), // 15 entered, 16 index, 17 length, 18 comparison index
        (1, nullable(l.gc.val_type(key_type))), // 19
        (
            1,
            nullable(l.gc.val_type(Type::Standard(StdlibTypeId::UnityKeyedSlot))),
        ), // 20
        (1, ValType::I64), // 21 backing address
    ]);
    // The common ABI receives either an object or its reference slot.
    f.instruction(&I::LocalGet(4))
        .instruction(&I::I32Eqz)
        .instruction(&I::If(BlockType::Empty))
        .instruction(&I::LocalGet(0))
        .instruction(&I::LocalGet(1))
        .instruction(&I::I32Const(l.abi_read.destination(8)))
        .instruction(&I::LocalGet(2))
        .instruction(&I::LocalGet(2))
        .instruction(&I::Call(l.runtime_helpers.function(H::ReadManagedMemory)))
        .instruction(&I::I32Eqz);
    r.fail_if(
        &mut f,
        &format!("managed {noun} reference could not be read"),
    );
    read_word(&mut f, l, false);
    f.instruction(&I::LocalSet(1)).instruction(&I::End);
    f.instruction(&I::LocalGet(CONTEXT))
        .instruction(&I::LocalGet(1))
        .instruction(&I::Call(l.runtime_helpers.function(H::EnterManagedObject)))
        .instruction(&I::LocalTee(15))
        .instruction(&I::I32Eqz);
    r.fail_if(
        &mut f,
        &format!("managed {noun} encountered a null object, cycle, or object/depth limit"),
    );
    callback_start(&mut f, l, read_binding, 5);
    f.instruction(&I::LocalGet(1));
    if dictionary {
        width(&mut f, &r, key);
    } else {
        f.instruction(&I::I32Const(0));
    }
    width(&mut f, &r, value);
    f.instruction(&I::I32Const(storage_kinds(&r, key) as i32))
        .instruction(&I::I32Const(storage_kinds(&r, value) as i32));
    remaining(
        &mut f,
        l,
        crate::managed_read::SNAPSHOT_SCAN_SLOT,
        crate::managed_read::MAX_MANAGED_SCANNED_SLOTS,
        true,
    );
    remaining(
        &mut f,
        l,
        crate::managed_read::SNAPSHOT_ELEMENT_SLOT,
        crate::managed_read::MAX_MANAGED_ELEMENTS,
        true,
    );
    remaining(
        &mut f,
        l,
        crate::managed_read::SNAPSHOT_BYTE_SLOT,
        crate::managed_read::MAX_MANAGED_READ_BYTES,
        false,
    );
    f.instruction(&I::LocalGet(CONTEXT));
    callback_end(&mut f, l, read_callable, 5);
    f.instruction(&I::LocalSet(6));
    r.forward_result_failure(&mut f, read_result, 6);
    r.result_field(
        &mut f,
        read_result,
        6,
        0,
        Type::Standard(StdlibTypeId::UnityKeyedRead),
    );
    f.instruction(&I::LocalSet(7));
    get(
        &mut f,
        l,
        7,
        StdlibTypeId::UnityKeyedRead,
        F::UnityKeyedReadSlots,
    );
    f.instruction(&I::LocalSet(8));
    f.instruction(&I::LocalGet(8)).instruction(&I::RefAsNonNull);
    super::super::array_value::emit_length(&mut f, l.gc, slots);
    f.instruction(&I::LocalSet(17));
    f.instruction(&I::LocalGet(CONTEXT));
    get(
        &mut f,
        l,
        7,
        StdlibTypeId::UnityKeyedRead,
        F::UnityKeyedReadScanned,
    );
    f.instruction(&I::I64ExtendI32U)
        .instruction(&I::Call(l.runtime_helpers.function(H::ChargeManagedScan)))
        .instruction(&I::I32Eqz);
    r.fail_if(
        &mut f,
        &format!("managed {noun} exceeds the shared scan budget"),
    );
    f.instruction(&I::LocalGet(CONTEXT))
        .instruction(&I::LocalGet(17))
        .instruction(&I::I64ExtendI32U)
        .instruction(&I::Call(
            l.runtime_helpers.function(H::ChargeManagedElements),
        ))
        .instruction(&I::I32Eqz);
    r.fail_if(
        &mut f,
        &format!("managed {noun} exceeds the shared element budget"),
    );
    f.instruction(&I::LocalGet(CONTEXT));
    get(
        &mut f,
        l,
        7,
        StdlibTypeId::UnityKeyedRead,
        F::UnityKeyedReadBytes,
    );
    // Final entry structs and backing storage, in addition to scanner temporaries.
    f.instruction(&I::LocalGet(17))
        .instruction(&I::I64ExtendI32U)
        .instruction(&I::I64Const(64))
        .instruction(&I::I64Mul)
        .instruction(&I::I64Add)
        .instruction(&I::I64Const(64))
        .instruction(&I::I64Add)
        .instruction(&I::Call(l.runtime_helpers.function(H::ChargeManagedBytes)))
        .instruction(&I::I32Eqz);
    r.fail_if(
        &mut f,
        &format!("managed {noun} exceeds the shared byte budget"),
    );
    // Enter each allocated backing object before recursive decoding. Failure
    // unwinds exactly the entries successfully pushed onto the active path.
    get(
        &mut f,
        l,
        7,
        StdlibTypeId::UnityKeyedRead,
        F::UnityKeyedReadBacking,
    );
    f.instruction(&I::LocalSet(9));
    f.instruction(&I::Block(BlockType::Empty))
        .instruction(&I::Loop(BlockType::Empty))
        .instruction(&I::LocalGet(16))
        .instruction(&I::LocalGet(9))
        .instruction(&I::RefAsNonNull);
    super::super::array_value::emit_length(&mut f, l.gc, backing);
    f.instruction(&I::I32GeU).instruction(&I::BrIf(1));
    array_element(&mut f, l, backing, 9, 16);
    f.instruction(&I::LocalTee(21))
        .instruction(&I::I64Eqz)
        .instruction(&I::I32Eqz)
        .instruction(&I::If(BlockType::Empty))
        .instruction(&I::Block(BlockType::Empty))
        .instruction(&I::I32Const(0))
        .instruction(&I::LocalSet(18))
        .instruction(&I::Block(BlockType::Empty))
        .instruction(&I::Loop(BlockType::Empty))
        .instruction(&I::LocalGet(18))
        .instruction(&I::LocalGet(16))
        .instruction(&I::I32GeU)
        .instruction(&I::BrIf(1));
    // Parallel layouts can reuse an empty backing array. Sibling aliases
    // enter the active path only once; references from children still cycle.
    array_element(&mut f, l, backing, 9, 18);
    f.instruction(&I::LocalGet(21))
        .instruction(&I::I64Eq)
        .instruction(&I::BrIf(2));
    increment(&mut f, 18);
    f.instruction(&I::Br(0))
        .instruction(&I::End)
        .instruction(&I::End)
        .instruction(&I::LocalGet(CONTEXT))
        .instruction(&I::LocalGet(21))
        .instruction(&I::Call(l.runtime_helpers.function(H::EnterManagedObject)))
        .instruction(&I::I32Eqz);
    r.fail_if(
        &mut f,
        &format!("managed {noun} backing storage encountered a cycle or object/depth limit"),
    );
    increment(&mut f, 15);
    f.instruction(&I::End).instruction(&I::End);
    increment(&mut f, 16);
    f.instruction(&I::Br(0))
        .instruction(&I::End)
        .instruction(&I::End)
        .instruction(&I::I32Const(0))
        .instruction(&I::LocalSet(16));
    f.instruction(&I::LocalGet(17))
        .instruction(&I::ArrayNewDefault(storage_index))
        .instruction(&I::LocalSet(10));
    f.instruction(&I::Block(BlockType::Empty))
        .instruction(&I::Loop(BlockType::Empty))
        .instruction(&I::LocalGet(16))
        .instruction(&I::LocalGet(17))
        .instruction(&I::I32GeU)
        .instruction(&I::BrIf(1));
    array_element(&mut f, l, slots, 8, 16);
    f.instruction(&I::LocalSet(20));
    for (child, field, local) in [
        (key, F::UnityKeyedSlotKey, 11),
        (value, F::UnityKeyedSlotValue, 12),
    ] {
        if !dictionary && local == 11 {
            continue;
        }
        f.instruction(&I::LocalGet(0));
        get(&mut f, l, 20, StdlibTypeId::UnityKeyedSlot, field);
        f.instruction(&I::LocalGet(2))
            .instruction(&I::LocalGet(CONTEXT))
            .instruction(&I::I32Const(0))
            .instruction(&I::Call(l.managed_decoder_functions[&child]))
            .instruction(&I::LocalSet(local));
        r.forward_failure(&mut f, child, local);
    }
    r.child_field(&mut f, key, if dictionary { 11 } else { 12 }, 0, key_type);
    f.instruction(&I::LocalSet(19));
    // Local equality can merge keys that the game's comparer distinguishes.
    // Refuse that loss and bound pair comparisons with the same root work limit.
    f.instruction(&I::I32Const(0))
        .instruction(&I::LocalSet(18))
        .instruction(&I::Block(BlockType::Empty))
        .instruction(&I::Loop(BlockType::Empty))
        .instruction(&I::LocalGet(18))
        .instruction(&I::LocalGet(16))
        .instruction(&I::I32GeU)
        .instruction(&I::BrIf(1))
        .instruction(&I::LocalGet(CONTEXT))
        .instruction(&I::Call(l.runtime_helpers.function(H::ChargeManagedWork)))
        .instruction(&I::I32Eqz);
    r.fail_if(
        &mut f,
        &format!("managed {noun} exceeds the shared comparison work budget"),
    );
    f.instruction(&I::LocalGet(10))
        .instruction(&I::RefAsNonNull)
        .instruction(&I::LocalGet(18));
    if let Some(entry_type) = entry_type {
        f.instruction(&I::ArrayGet(storage_index))
            .instruction(&I::RefAsNonNull);
        emit_typed_struct_get(&mut f, l.gc.index(entry_type), 0, key_type);
    } else {
        super::super::emit_array_get(&mut f, storage_index, key_type, l.gc);
    }
    f.instruction(&I::LocalGet(19));
    super::super::runtime_helpers::emit_equality_call(
        &mut f,
        key_type,
        l.managed_equality_functions,
        l.managed_equality_functions.string.unwrap_or(0),
        CONTEXT,
    );
    // Preserve the boolean on the stack while checking the sticky exhaustion
    // marker. A comparison that ran out of work must not admit another key.
    f.instruction(&I::LocalGet(CONTEXT))
        .instruction(&I::I32Const(2))
        .instruction(&I::ArrayGet(
            l.gc.standard_index(StdlibTypeId::ManagedReadContext),
        ))
        .instruction(&I::I64Const(crate::managed_read::MAX_SNAPSHOT_WORK))
        .instruction(&I::I64GtU);
    r.fail_if(
        &mut f,
        &format!("managed {noun} exceeds the shared comparison work budget"),
    );
    r.fail_if(
        &mut f,
        if dictionary {
            "managed dictionary contains duplicate decoded keys under local equality"
        } else {
            "managed set contains duplicate decoded values under local equality"
        },
    );
    increment(&mut f, 18);
    f.instruction(&I::Br(0))
        .instruction(&I::End)
        .instruction(&I::End);
    f.instruction(&I::LocalGet(10))
        .instruction(&I::RefAsNonNull)
        .instruction(&I::LocalGet(16))
        .instruction(&I::LocalGet(19));
    if let Some(entry_type) = entry_type {
        r.child_field(
            &mut f,
            value,
            12,
            0,
            semantic_type(r.output(value), l.semantics),
        );
        f.instruction(&I::StructNew(l.gc.index(entry_type)));
    }
    f.instruction(&I::ArraySet(storage_index));
    increment(&mut f, 16);
    f.instruction(&I::Br(0))
        .instruction(&I::End)
        .instruction(&I::End);
    callback_start(&mut f, l, crate::stdlib::MANAGED_KEYED_VERIFY_FIELD, 13);
    f.instruction(&I::LocalGet(7));
    callback_end(&mut f, l, verify_callable, 13);
    f.instruction(&I::LocalSet(14));
    r.forward_result_failure(&mut f, verify_result, 14);
    r.leave(&mut f);
    f.instruction(&I::LocalGet(10))
        .instruction(&I::LocalGet(17))
        .instruction(&I::I32Const(super::super::array_value::FROZEN_VERSION));
    if let Some(entries) = entries {
        f.instruction(&I::StructNew(l.gc.index(Type::Array(entries))));
    }
    f.instruction(&I::StructNew(
        l.gc.index(semantic_type(output, l.semantics)),
    ));
    emit_result_success(&mut f, r.result, l.gc);
    f.instruction(&I::End);
    f
}

fn field_array(l: &EmissionContext<'_>, field: F) -> crate::ast::ArrayTypeId {
    let TypeKind::Array { layout, .. } = l
        .semantics
        .types()
        .kind(l.semantics.standard_field_type(field).unwrap())
    else {
        unreachable!()
    };
    *layout
}
fn get(f: &mut Function, l: &EmissionContext<'_>, local: u32, owner: StdlibTypeId, field: F) {
    f.instruction(&I::LocalGet(local))
        .instruction(&I::RefAsNonNull);
    emit_typed_struct_get(
        f,
        l.gc.standard_index(owner),
        l.gc.standard_field_index(field),
        semantic_type(l.semantics.standard_field_type(field).unwrap(), l.semantics),
    );
}
fn array_element(
    f: &mut Function,
    l: &EmissionContext<'_>,
    array: crate::ast::ArrayTypeId,
    local: u32,
    index: u32,
) {
    f.instruction(&I::LocalGet(local))
        .instruction(&I::RefAsNonNull);
    super::super::array_value::emit_backing(f, l.gc, array);
    f.instruction(&I::LocalGet(index));
    let storage = super::super::array_value::storage_id(array, l.arrays, l.semantics);
    super::super::emit_array_get(
        f,
        l.gc.index(Type::ArrayStorage(storage)),
        super::super::array_element_type(array, l.semantics),
        l.gc,
    );
}
fn increment(f: &mut Function, local: u32) {
    f.instruction(&I::LocalGet(local))
        .instruction(&I::I32Const(1))
        .instruction(&I::I32Add)
        .instruction(&I::LocalSet(local));
}
fn remaining(f: &mut Function, l: &EmissionContext<'_>, slot: u32, limit: i64, narrow: bool) {
    f.instruction(&I::I64Const(limit))
        .instruction(&I::LocalGet(CONTEXT))
        .instruction(&I::I32Const(slot as i32))
        .instruction(&I::ArrayGet(
            l.gc.standard_index(StdlibTypeId::ManagedReadContext),
        ))
        .instruction(&I::I64Sub);
    if narrow {
        f.instruction(&I::I32WrapI64);
    }
}
fn width(f: &mut Function, r: &Reader<'_, '_>, source: TypeId) {
    if matches!(
        r.capabilities.managed_decoder(source).unwrap().kind,
        ManagedDecoderKind::Memory
    ) {
        let sizes = [MemoryAddressWidth::Bit32, MemoryAddressWidth::Bit64].map(|w| {
            r.lowering
                .memory
                .layout(source, r.lowering.semantics, w)
                .unwrap()
                .size() as i32
        });
        f.instruction(&I::I32Const(sizes[0]))
            .instruction(&I::I32Const(sizes[1]))
            .instruction(&I::LocalGet(2))
            .instruction(&I::I32Const(4))
            .instruction(&I::I32Eq)
            .instruction(&I::Select);
    } else {
        f.instruction(&I::LocalGet(2));
    }
}

// CLR type-tag masks describe remote storage, not the projected owned type.
// Nullable references have the same storage as their non-null child. Enum
// schemas also admit their explicit scalar representation; field/class facts
// are still needed to validate remote value types and generic instances fully.
fn storage_kinds(r: &Reader<'_, '_>, source: TypeId) -> u32 {
    match r.capabilities.managed_decoder(source).unwrap().kind {
        ManagedDecoderKind::Optional { value } => storage_kinds(r, value),
        ManagedDecoderKind::String => 1 << 0x0e,
        ManagedDecoderKind::Array { .. } => 1 << 0x1d,
        ManagedDecoderKind::Class { .. }
        | ManagedDecoderKind::List { .. }
        | ManagedDecoderKind::Map { .. }
        | ManagedDecoderKind::Set { .. } => (1 << 0x12) | (1 << 0x15),
        ManagedDecoderKind::Memory => match r.lowering.semantics.types().kind(source) {
            TypeKind::Builtin(core) => match core {
                CoreTypeId::Bool => 1 << 0x02,
                CoreTypeId::Char => 1 << 0x03,
                CoreTypeId::I8 => 1 << 0x04,
                CoreTypeId::U8 => 1 << 0x05,
                CoreTypeId::I16 => 1 << 0x06,
                CoreTypeId::U16 => (1 << 0x03) | (1 << 0x07),
                CoreTypeId::I32 => 1 << 0x08,
                CoreTypeId::U32 => 1 << 0x09,
                CoreTypeId::I64 => 1 << 0x0a,
                CoreTypeId::U64 => 1 << 0x0b,
                CoreTypeId::F32 => 1 << 0x0c,
                CoreTypeId::F64 => 1 << 0x0d,
                CoreTypeId::Address => {
                    (1 << 0x18)
                        | (1 << 0x19)
                        | (1 << 0x0e)
                        | (1 << 0x12)
                        | (1 << 0x1c)
                        | (1 << 0x1d)
                }
                _ => unreachable!("non-memory scalar in a managed memory decoder"),
            },
            TypeKind::Enum(enumeration) => {
                let representation = r
                    .lowering
                    .semantics
                    .enum_representation(*enumeration)
                    .unwrap();
                (1 << 0x11) | storage_kinds(r, representation)
            }
            _ => (1 << 0x11) | (1 << 0x15),
        },
    }
}
fn binding(
    l: &EmissionContext<'_>,
    name: &str,
) -> (crate::ast::StructId, u32, crate::ast::CallableTypeId) {
    let structure = l
        .structs
        .iter()
        .find(|s| s.name == crate::stdlib::PROVIDER_BINDINGS_TYPE)
        .unwrap();
    let (index, field) = structure
        .fields
        .iter()
        .enumerate()
        .find(|(_, f)| f.name == name)
        .unwrap();
    let TypeKind::Callable { layout, .. } = l
        .semantics
        .types()
        .kind(l.semantics.struct_field_type(field.id).unwrap())
    else {
        unreachable!()
    };
    (structure.id, index as u32, *layout)
}
fn callback_start(f: &mut Function, l: &EmissionContext<'_>, name: &str, local: u32) {
    let (structure, field, callable) = binding(l, name);
    f.instruction(&I::GlobalGet(
        l.runtime_globals.provider_preparation_value.unwrap(),
    ))
    .instruction(&I::RefAsNonNull)
    .instruction(&I::StructGet {
        struct_type_index: l.gc.index(Type::Struct(structure)),
        field_index: field,
    })
    .instruction(&I::LocalTee(local))
    .instruction(&I::StructGet {
        struct_type_index: l.gc.index(Type::Callable(callable)),
        field_index: 1,
    });
}
fn callback_end(
    f: &mut Function,
    l: &EmissionContext<'_>,
    callable: crate::ast::CallableTypeId,
    local: u32,
) {
    f.instruction(&I::LocalGet(local))
        .instruction(&I::StructGet {
            struct_type_index: l.gc.index(Type::Callable(callable)),
            field_index: 0,
        })
        .instruction(&I::CallRef(l.gc.callable_function_index(callable)));
}

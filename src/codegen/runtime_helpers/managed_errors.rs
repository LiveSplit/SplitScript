//! Bounded context strings for observed managed-read failures.
//!
//! Paths are prepended while failures unwind. If another segment would exceed
//! the diagnostic bound, preserve the innermost path and original cause intact.
use wasm_encoder::{BlockType, Function, Instruction as I, ValType};

use super::super::Type;
use super::RuntimeHelperInputs;
use crate::{intrinsic_registry::RuntimeHelperId, stdlib::StdlibTypeId};

pub(super) fn field(inputs: &RuntimeHelperInputs<'_>) -> Function {
    let array = inputs.gc.standard_index(StdlibTypeId::String);
    // Parameters: cause, source field name. Locals: lengths, total, output.
    let mut f = Function::new([
        (3, ValType::I32),
        (1, inputs.gc.val_type(Type::Standard(StdlibTypeId::String))),
    ]);
    preserve_null(&mut f);
    length(&mut f, 0, 2);
    length(&mut f, 1, 3);
    bound(&mut f, &[2, 3], 2);
    f.instruction(&I::LocalGet(2))
        .instruction(&I::LocalGet(3))
        .instruction(&I::I32Add)
        .instruction(&I::I32Const(2))
        .instruction(&I::I32Add)
        .instruction(&I::LocalTee(4))
        .instruction(&I::ArrayNewDefault(array))
        .instruction(&I::LocalSet(5));
    copy(&mut f, array, 5, 1, 3, None);
    byte(&mut f, array, 5, 3, 0, b':');
    byte(&mut f, array, 5, 3, 1, b' ');
    copy(&mut f, array, 5, 0, 2, Some((3, 2)));
    f.instruction(&I::LocalGet(5)).instruction(&I::End);
    f
}

pub(super) fn index(inputs: &RuntimeHelperInputs<'_>) -> Function {
    let array = inputs.gc.standard_index(StdlibTypeId::String);
    // Parameters: cause, traversal index, label. Locals: digits, output,
    // cause/label/digit/prefix lengths. Labels are supplied only by used readers.
    let mut f = Function::new([
        (2, inputs.gc.val_type(Type::Standard(StdlibTypeId::String))),
        (4, ValType::I32),
    ]);
    preserve_null(&mut f);
    f.instruction(&I::LocalGet(1))
        .instruction(&I::I64ExtendI32U)
        .instruction(&I::I32Const(10))
        .instruction(&I::I32Const(0))
        .instruction(&I::Call(inputs.plan.function(RuntimeHelperId::FormatI64)))
        .instruction(&I::LocalSet(3));
    length(&mut f, 0, 5);
    length(&mut f, 2, 6);
    length(&mut f, 3, 7);
    bound(&mut f, &[5, 6, 7], 4);
    f.instruction(&I::LocalGet(6))
        .instruction(&I::LocalGet(7))
        .instruction(&I::I32Add)
        .instruction(&I::I32Const(2))
        .instruction(&I::I32Add)
        .instruction(&I::LocalTee(8))
        .instruction(&I::LocalGet(5))
        .instruction(&I::I32Add)
        .instruction(&I::I32Const(2))
        .instruction(&I::I32Add)
        .instruction(&I::ArrayNewDefault(array))
        .instruction(&I::LocalSet(4));
    copy(&mut f, array, 4, 2, 6, None);
    byte(&mut f, array, 4, 6, 0, b'[');
    copy(&mut f, array, 4, 3, 7, Some((6, 1)));
    byte(&mut f, array, 4, 8, -1, b']');
    byte(&mut f, array, 4, 8, 0, b':');
    byte(&mut f, array, 4, 8, 1, b' ');
    copy(&mut f, array, 4, 0, 5, Some((8, 2)));
    f.instruction(&I::LocalGet(4)).instruction(&I::End);
    f
}

fn preserve_null(f: &mut Function) {
    f.instruction(&I::LocalGet(0))
        .instruction(&I::RefIsNull)
        .instruction(&I::If(BlockType::Empty))
        .instruction(&I::LocalGet(0))
        .instruction(&I::Return)
        .instruction(&I::End);
}

fn length(f: &mut Function, source: u32, target: u32) {
    f.instruction(&I::LocalGet(source))
        .instruction(&I::ArrayLen)
        .instruction(&I::LocalSet(target));
}

fn bound(f: &mut Function, lengths: &[u32], extra: i64) {
    f.instruction(&I::I64Const(extra));
    for local in lengths {
        f.instruction(&I::LocalGet(*local))
            .instruction(&I::I64ExtendI32U)
            .instruction(&I::I64Add);
    }
    f.instruction(&I::I64Const(crate::managed_read::MAX_MANAGED_ERROR_BYTES))
        .instruction(&I::I64GtU)
        .instruction(&I::If(BlockType::Empty))
        .instruction(&I::LocalGet(0))
        .instruction(&I::Return)
        .instruction(&I::End);
}

fn offset(f: &mut Function, at: Option<(u32, i32)>) {
    if let Some((local, displacement)) = at {
        f.instruction(&I::LocalGet(local));
        if displacement != 0 {
            f.instruction(&I::I32Const(displacement))
                .instruction(&I::I32Add);
        }
    } else {
        f.instruction(&I::I32Const(0));
    }
}

fn copy(
    f: &mut Function,
    array: u32,
    output: u32,
    source: u32,
    length: u32,
    at: Option<(u32, i32)>,
) {
    f.instruction(&I::LocalGet(output));
    offset(f, at);
    f.instruction(&I::LocalGet(source))
        .instruction(&I::I32Const(0))
        .instruction(&I::LocalGet(length))
        .instruction(&I::ArrayCopy {
            array_type_index_dst: array,
            array_type_index_src: array,
        });
}

fn byte(f: &mut Function, array: u32, output: u32, at: u32, displacement: i32, value: u8) {
    f.instruction(&I::LocalGet(output));
    offset(f, Some((at, displacement)));
    f.instruction(&I::I32Const(i32::from(value)))
        .instruction(&I::ArraySet(array));
}

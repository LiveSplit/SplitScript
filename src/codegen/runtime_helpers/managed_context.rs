//! Shared counters and active-path checks for a managed materialization root.

use super::super::GcLayout;
use crate::stdlib::StdlibTypeId;
use wasm_encoder::{BlockType, Function, Instruction as I, ValType};

pub(super) fn enter(gc: &GcLayout) -> Function {
    let array = gc.standard_index(StdlibTypeId::ManagedReadContext);
    // Parameters: context, object address. Locals: depth, scan index.
    let mut f = Function::new([(2, ValType::I32)]);
    f.instruction(&I::LocalGet(1)).instruction(&I::I64Eqz);
    fail_if(&mut f);
    get(&mut f, array, 0);
    f.instruction(&I::I32WrapI64)
        .instruction(&I::LocalTee(2))
        .instruction(&I::I32Const(crate::managed_read::MAX_SNAPSHOT_DEPTH as i32))
        .instruction(&I::I32GeU);
    fail_if(&mut f);
    get(&mut f, array, 1);
    f.instruction(&I::I64Const(crate::managed_read::MAX_SNAPSHOT_OBJECTS))
        .instruction(&I::I64GeU);
    fail_if(&mut f);
    f.instruction(&I::Block(BlockType::Empty))
        .instruction(&I::Loop(BlockType::Empty))
        .instruction(&I::LocalGet(3))
        .instruction(&I::LocalGet(2))
        .instruction(&I::I32GeU)
        .instruction(&I::BrIf(1))
        .instruction(&I::LocalGet(0))
        .instruction(&I::LocalGet(3))
        .instruction(&I::I32Const(3))
        .instruction(&I::I32Add)
        .instruction(&I::ArrayGet(array))
        .instruction(&I::LocalGet(1))
        .instruction(&I::I64Eq);
    fail_if(&mut f);
    f.instruction(&I::LocalGet(3))
        .instruction(&I::I32Const(1))
        .instruction(&I::I32Add)
        .instruction(&I::LocalSet(3))
        .instruction(&I::Br(0))
        .instruction(&I::End)
        .instruction(&I::End)
        .instruction(&I::LocalGet(0))
        .instruction(&I::LocalGet(2))
        .instruction(&I::I32Const(3))
        .instruction(&I::I32Add)
        .instruction(&I::LocalGet(1))
        .instruction(&I::ArraySet(array));
    increment(&mut f, array, 0);
    increment(&mut f, array, 1);
    f.instruction(&I::I32Const(1)).instruction(&I::End);
    f
}

pub(super) fn charge_work(gc: &GcLayout) -> Function {
    let array = gc.standard_index(StdlibTypeId::ManagedReadContext);
    let mut f = Function::new([]);
    get(&mut f, array, 2);
    f.instruction(&I::I64Const(crate::managed_read::MAX_SNAPSHOT_WORK))
        .instruction(&I::I64GeU);
    fail_if(&mut f);
    increment(&mut f, array, 2);
    f.instruction(&I::I32Const(1)).instruction(&I::End);
    f
}

pub(super) fn charge_bytes(gc: &GcLayout) -> Function {
    charge(
        gc,
        crate::managed_read::SNAPSHOT_BYTE_SLOT as i32,
        crate::managed_read::MAX_MANAGED_READ_BYTES,
    )
}

pub(super) fn charge_elements(gc: &GcLayout) -> Function {
    charge(
        gc,
        crate::managed_read::SNAPSHOT_ELEMENT_SLOT as i32,
        crate::managed_read::MAX_MANAGED_ELEMENTS,
    )
}

fn charge(gc: &GcLayout, slot: i32, limit: i64) -> Function {
    let array = gc.standard_index(StdlibTypeId::ManagedReadContext);
    let mut f = Function::new([]);
    // Compare against remaining capacity before adding, including unsigned
    // rejection of negative/overflowed requests.
    f.instruction(&I::LocalGet(1))
        .instruction(&I::I64Const(limit));
    get(&mut f, array, slot);
    f.instruction(&I::I64Sub).instruction(&I::I64GtU);
    fail_if(&mut f);
    f.instruction(&I::LocalGet(0))
        .instruction(&I::I32Const(slot));
    get(&mut f, array, slot);
    f.instruction(&I::LocalGet(1))
        .instruction(&I::I64Add)
        .instruction(&I::ArraySet(array))
        .instruction(&I::I32Const(1))
        .instruction(&I::End);
    f
}

fn fail_if(f: &mut Function) {
    f.instruction(&I::If(BlockType::Empty))
        .instruction(&I::I32Const(0))
        .instruction(&I::Return)
        .instruction(&I::End);
}

fn get(f: &mut Function, array: u32, slot: i32) {
    f.instruction(&I::LocalGet(0))
        .instruction(&I::I32Const(slot))
        .instruction(&I::ArrayGet(array));
}

fn increment(f: &mut Function, array: u32, slot: i32) {
    f.instruction(&I::LocalGet(0))
        .instruction(&I::I32Const(slot));
    get(f, array, slot);
    f.instruction(&I::I64Const(1))
        .instruction(&I::I64Add)
        .instruction(&I::ArraySet(array));
}

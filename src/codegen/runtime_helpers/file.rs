//! Read-only whole-file helpers built directly on WASI Preview 1.
//!
//! No descriptor crosses a helper boundary other than the private open/read
//! composition below, and every successful `path_open` is paired with
//! `fd_close` before control returns to user code.

use crate::codegen::GC_NULL_HEAP_TYPE;
use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::{abi::AbiImportId, ast::ArrayTypeId, stdlib::StdlibTypeId};

use super::super::memory_plan::RuntimeScratch;
use super::super::{GcLayout, Type, array_value, imports::Abi, memarg};

const MAX_PATH_BYTES: i32 = 65_536;
const FILE_READ_CHUNK: i32 = 65_536;
const FIRST_PREOPEN_DESCRIPTOR: i32 = 3;
const MAX_PREOPEN_DESCRIPTOR: i32 = 1_024;
const WASI_LOOKUP_SYMLINK_FOLLOW: i32 = 1;
const WASI_RIGHT_FD_READ: i64 = 1 << 1;

pub(super) fn compile_open_read_only(
    abi: &Abi,
    gc: &GcLayout,
    scratch: RuntimeScratch,
) -> Function {
    let path_start = scratch.host_strings_start;
    let preopen_path_start = path_start + MAX_PATH_BYTES;
    let staging_end = preopen_path_start + MAX_PATH_BYTES;
    let prestat_pointer = scratch.abi_read.at(4);
    let opened_descriptor_pointer = scratch.abi_read.start();
    let string_type = gc.standard_index(StdlibTypeId::String);

    // Parameters: path and the exact rights required by the caller. Keeping
    // path resolution shared lets whole-file reads request only `fd_read`,
    // while bounded fingerprints can additionally request seek and filestat.
    let mut function = Function::new([(12, ValType::I32)]);
    let path = 0;
    let rights = 1;
    let path_length = 2;
    let index = 3;
    let resolved_length = 4;
    let descriptor = 5;
    let preopen_length = 6;
    let best_descriptor = 7;
    let best_prefix_length = 8;
    let compare_index = 9;
    let matches = 10;
    let required_pages = 11;
    let relative_pointer = 12;
    let relative_length = 13;

    emit_ensure_linear_capacity_for_open(&mut function, staging_end, required_pages);
    function
        .instruction(&Instruction::LocalGet(path))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalTee(path_length))
        .instruction(&Instruction::I32Const(MAX_PATH_BYTES))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_open_failure(&mut function);
    function.instruction(&Instruction::End);

    // The host exposes native files only through its absolute portable WASI
    // namespace. There is no stable working directory: normal runtime clients
    // do not provide the autosplitter's path to the guest.
    function
        .instruction(&Instruction::LocalGet(path_length))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_open_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(path))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::I32Const(b'/' as i32))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_open_failure(&mut function);
    function.instruction(&Instruction::End);
    emit_copy_string_to_memory(
        &mut function,
        path,
        path_start,
        path_length,
        index,
        string_type,
    );
    function
        .instruction(&Instruction::LocalGet(path_length))
        .instruction(&Instruction::LocalSet(resolved_length));

    // Select the longest matching preopen prefix, so mapped network paths win
    // over their containing drive preopen.
    function
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::LocalSet(best_descriptor))
        .instruction(&Instruction::I32Const(FIRST_PREOPEN_DESCRIPTOR))
        .instruction(&Instruction::LocalSet(descriptor))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(descriptor))
        .instruction(&Instruction::I32Const(MAX_PREOPEN_DESCRIPTOR))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(descriptor))
        .instruction(&Instruction::I32Const(prestat_pointer))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::WasiFdPrestatGet),
        ))
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(prestat_pointer))
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(prestat_pointer))
        .instruction(&Instruction::I32Load(wasm_encoder::MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        }))
        .instruction(&Instruction::LocalTee(preopen_length))
        .instruction(&Instruction::I32Const(MAX_PATH_BYTES))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(descriptor))
        .instruction(&Instruction::I32Const(preopen_path_start))
        .instruction(&Instruction::LocalGet(preopen_length))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::WasiFdPrestatDirName),
        ))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(preopen_length))
        .instruction(&Instruction::LocalGet(resolved_length))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(matches))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(compare_index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(compare_index))
        .instruction(&Instruction::LocalGet(preopen_length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(preopen_path_start))
        .instruction(&Instruction::LocalGet(compare_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::I32Const(path_start))
        .instruction(&Instruction::LocalGet(compare_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(matches))
        .instruction(&Instruction::Br(2))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(compare_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(compare_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(matches))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(preopen_length))
        .instruction(&Instruction::LocalGet(resolved_length))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(preopen_length))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::I32Const(preopen_path_start))
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::I32Const(b'/' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::I32Const(path_start))
        .instruction(&Instruction::LocalGet(preopen_length))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::I32Const(b'/' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(preopen_length))
        .instruction(&Instruction::LocalGet(best_prefix_length))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(descriptor))
        .instruction(&Instruction::LocalSet(best_descriptor))
        .instruction(&Instruction::LocalGet(preopen_length))
        .instruction(&Instruction::LocalSet(best_prefix_length))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(descriptor))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(descriptor))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(best_descriptor))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32LtS)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_open_failure(&mut function);
    function.instruction(&Instruction::End);

    function
        .instruction(&Instruction::I32Const(path_start))
        .instruction(&Instruction::LocalGet(best_prefix_length))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(relative_pointer))
        .instruction(&Instruction::LocalGet(resolved_length))
        .instruction(&Instruction::LocalGet(best_prefix_length))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(relative_length))
        .instruction(&Instruction::LocalGet(relative_length))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(relative_pointer))
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::I32Const(b'/' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::End)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(relative_pointer))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(relative_pointer))
        .instruction(&Instruction::LocalGet(relative_length))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(relative_length))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(best_descriptor))
        .instruction(&Instruction::I32Const(WASI_LOOKUP_SYMLINK_FOLLOW))
        .instruction(&Instruction::LocalGet(relative_pointer))
        .instruction(&Instruction::LocalGet(relative_length))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(rights))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(opened_descriptor_pointer))
        .instruction(&Instruction::Call(abi.function(AbiImportId::WasiPathOpen)))
        .instruction(&Instruction::If(BlockType::Empty));
    emit_open_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(opened_descriptor_pointer))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::End);
    function
}

/// Reads the file into a private byte-storage value plus its logical length.
///
/// The storage deliberately uses the same raw GC-array representation as a
/// string without asserting that its contents are UTF-8. It never crosses the
/// public intrinsic boundary: the byte API copies it into `[u8]`, while the
/// text API validates it before exposing it as `String`. Keeping this private
/// representation lets `File.readAllText` remain independent of whether user
/// code happens to materialize a `[u8]` layout.
pub(super) fn compile_read_all_storage(
    abi: &Abi,
    open_read_only: u32,
    gc: &GcLayout,
    scratch: RuntimeScratch,
) -> Function {
    let chunk_start = scratch.host_strings_start;
    let iovec_pointer = scratch.abi_read.start();
    let bytes_read_pointer = scratch.abi_read.at(8);
    let storage_type = gc.standard_index(StdlibTypeId::String);
    let storage_value = gc.val_type(Type::Standard(StdlibTypeId::String));
    let mut function = Function::new([(8, ValType::I32), (2, storage_value)]);
    let path = 0;
    let descriptor = 1;
    let length = 2;
    let capacity = 3;
    let bytes_read = 4;
    let index = 5;
    let required = 6;
    let new_capacity = 7;
    let required_pages = 8;
    let backing = 9;
    let replacement = 10;

    function
        .instruction(&Instruction::I32Const(chunk_start + FILE_READ_CHUNK))
        .instruction(&Instruction::I32Const(65_535))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(16))
        .instruction(&Instruction::I32ShrU)
        .instruction(&Instruction::LocalTee(required_pages))
        .instruction(&Instruction::MemorySize(0))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(required_pages))
        .instruction(&Instruction::MemorySize(0))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::MemoryGrow(0))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_storage_read_failure(&mut function, None, abi);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
    function
        .instruction(&Instruction::LocalGet(path))
        .instruction(&Instruction::I64Const(WASI_RIGHT_FD_READ))
        .instruction(&Instruction::Call(open_read_only))
        .instruction(&Instruction::LocalTee(descriptor))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32LtS)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_storage_read_failure(&mut function, None, abi);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::ArrayNewDefault(storage_type))
        .instruction(&Instruction::LocalSet(backing))
        .instruction(&Instruction::I32Const(iovec_pointer))
        .instruction(&Instruction::I32Const(chunk_start))
        .instruction(&Instruction::I32Store(memarg()))
        .instruction(&Instruction::I32Const(iovec_pointer + 4))
        .instruction(&Instruction::I32Const(FILE_READ_CHUNK))
        .instruction(&Instruction::I32Store(memarg()))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::I32Const(bytes_read_pointer))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Store(memarg()))
        .instruction(&Instruction::LocalGet(descriptor))
        .instruction(&Instruction::I32Const(iovec_pointer))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Const(bytes_read_pointer))
        .instruction(&Instruction::Call(abi.function(AbiImportId::WasiFdRead)))
        .instruction(&Instruction::If(BlockType::Empty));
    emit_storage_read_failure(&mut function, Some(descriptor), abi);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(bytes_read_pointer))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::LocalTee(bytes_read))
        .instruction(&Instruction::I32Const(FILE_READ_CHUNK))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_storage_read_failure(&mut function, Some(descriptor), abi);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(bytes_read))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::LocalGet(bytes_read))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalTee(required))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_storage_read_failure(&mut function, Some(descriptor), abi);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(required))
        .instruction(&Instruction::LocalGet(capacity))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(capacity))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(required))
        .instruction(&Instruction::I32Const(4_096))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(required))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(4_096))
        .instruction(&Instruction::End)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(capacity))
        .instruction(&Instruction::I32Const(i32::MAX))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(capacity))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32Mul)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(required))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalTee(new_capacity))
        .instruction(&Instruction::LocalGet(required))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(required))
        .instruction(&Instruction::LocalSet(new_capacity))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(new_capacity))
        .instruction(&Instruction::ArrayNewDefault(storage_type))
        .instruction(&Instruction::LocalTee(replacement))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::ArrayCopy {
            array_type_index_dst: storage_type,
            array_type_index_src: storage_type,
        })
        .instruction(&Instruction::LocalGet(replacement))
        .instruction(&Instruction::LocalSet(backing))
        .instruction(&Instruction::LocalGet(new_capacity))
        .instruction(&Instruction::LocalSet(capacity))
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(bytes_read))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(chunk_start))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::ArraySet(storage_type))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(required))
        .instruction(&Instruction::LocalSet(length))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(descriptor))
        .instruction(&Instruction::Call(abi.function(AbiImportId::WasiFdClose)))
        .instruction(&Instruction::If(BlockType::Empty));
    emit_storage_read_failure(&mut function, None, abi);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::End);
    function
}

pub(super) fn compile_read_all_bytes(
    read_all_storage: u32,
    array: ArrayTypeId,
    storage: ArrayTypeId,
    gc: &GcLayout,
) -> Function {
    let array_type = gc.index(Type::Array(array));
    let storage_type = gc.index(Type::ArrayStorage(storage));
    let string_type = gc.standard_index(StdlibTypeId::String);
    let string_value = gc.val_type(Type::Standard(StdlibTypeId::String));
    let storage_value = gc.val_type(Type::ArrayStorage(storage));
    let mut function = Function::new([(2, ValType::I32), (1, string_value), (1, storage_value)]);
    let path = 0;
    let length = 1;
    let index = 2;
    let raw = 3;
    let backing = 4;

    function
        .instruction(&Instruction::LocalGet(path))
        .instruction(&Instruction::Call(read_all_storage))
        .instruction(&Instruction::LocalSet(length))
        .instruction(&Instruction::LocalSet(raw))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::ArrayNewDefault(storage_type))
        .instruction(&Instruction::LocalSet(backing))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(raw))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::ArraySet(storage_type))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::LocalGet(length));
    array_value::emit_wrap_loaded(&mut function, array_type);
    function.instruction(&Instruction::End);
    function
}

pub(super) fn compile_utf8_string_from_storage(gc: &GcLayout) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let string_value = gc.val_type(Type::Standard(StdlibTypeId::String));
    let mut function = Function::new([(7, ValType::I32), (1, string_value)]);
    let backing = 0;
    let length = 1;
    let index = 2;
    let first = 3;
    let second = 4;
    let third = 5;
    let fourth = 6;
    let width = 7;
    let copy_index = 8;
    let output = 9;

    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1));
    emit_array_byte(&mut function, backing, index, first, string_type);
    function
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(width))
        .instruction(&Instruction::Else);
    emit_between(&mut function, first, 0xC2, 0xDF);
    function
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::LocalSet(width))
        .instruction(&Instruction::Else);
    emit_between(&mut function, first, 0xE0, 0xEF);
    function
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(3))
        .instruction(&Instruction::LocalSet(width))
        .instruction(&Instruction::Else);
    emit_between(&mut function, first, 0xF0, 0xF4);
    function
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::LocalSet(width))
        .instruction(&Instruction::Else);
    emit_utf8_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalGet(width))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_utf8_failure(&mut function);
    function.instruction(&Instruction::End);

    function
        .instruction(&Instruction::LocalGet(width))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_array_byte_at_offset(&mut function, backing, index, 1, second, string_type);
    emit_continuation_test(&mut function, second);
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_utf8_failure(&mut function);
    function.instruction(&Instruction::End);
    function
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(0xE0))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(second))
        .instruction(&Instruction::I32Const(0xA0))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(0xED))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(second))
        .instruction(&Instruction::I32Const(0xA0))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(0xF0))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(second))
        .instruction(&Instruction::I32Const(0x90))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(0xF4))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(second))
        .instruction(&Instruction::I32Const(0x90))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_utf8_failure(&mut function);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);

    function
        .instruction(&Instruction::LocalGet(width))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_array_byte_at_offset(&mut function, backing, index, 2, third, string_type);
    emit_continuation_test(&mut function, third);
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_utf8_failure(&mut function);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);

    function
        .instruction(&Instruction::LocalGet(width))
        .instruction(&Instruction::I32Const(3))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_array_byte_at_offset(&mut function, backing, index, 3, fourth, string_type);
    emit_continuation_test(&mut function, fourth);
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_utf8_failure(&mut function);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);

    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(width))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::ArrayNewDefault(string_type))
        .instruction(&Instruction::LocalSet(output))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::ArraySet(string_type))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(copy_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

pub(super) fn compile_read_all_text(
    read_all_storage: u32,
    utf8_string_from_storage: u32,
    gc: &GcLayout,
) -> Function {
    let string_value = gc.val_type(Type::Standard(StdlibTypeId::String));
    let mut function = Function::new([(1, ValType::I32), (1, string_value)]);
    let path = 0;
    let length = 1;
    let storage = 2;
    function
        .instruction(&Instruction::LocalGet(path))
        .instruction(&Instruction::Call(read_all_storage))
        .instruction(&Instruction::LocalSet(length))
        .instruction(&Instruction::LocalSet(storage))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(storage))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::Call(utf8_string_from_storage))
        .instruction(&Instruction::End);
    function
}

fn emit_copy_string_to_memory(
    function: &mut Function,
    string: u32,
    destination: i32,
    length: u32,
    index: u32,
    string_type: u32,
) {
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(destination))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(string))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::I32Store8(memarg()))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
}

fn emit_ensure_linear_capacity_for_open(function: &mut Function, end: i32, required_pages: u32) {
    function
        .instruction(&Instruction::I32Const(end))
        .instruction(&Instruction::I32Const(65_535))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(16))
        .instruction(&Instruction::I32ShrU)
        .instruction(&Instruction::LocalTee(required_pages))
        .instruction(&Instruction::MemorySize(0))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(required_pages))
        .instruction(&Instruction::MemorySize(0))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::MemoryGrow(0))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_open_failure(function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
}

fn emit_open_failure(function: &mut Function) {
    function
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::Return);
}

fn emit_storage_read_failure(function: &mut Function, descriptor: Option<u32>, abi: &Abi) {
    if let Some(descriptor) = descriptor {
        function
            .instruction(&Instruction::LocalGet(descriptor))
            .instruction(&Instruction::Call(abi.function(AbiImportId::WasiFdClose)))
            .instruction(&Instruction::Drop);
    }
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return);
}

fn emit_array_byte(
    function: &mut Function,
    backing: u32,
    index: u32,
    destination: u32,
    storage_type: u32,
) {
    function
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(storage_type))
        .instruction(&Instruction::LocalSet(destination));
}

fn emit_array_byte_at_offset(
    function: &mut Function,
    backing: u32,
    index: u32,
    offset: i32,
    destination: u32,
    storage_type: u32,
) {
    function
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(offset))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayGetU(storage_type))
        .instruction(&Instruction::LocalSet(destination));
}

fn emit_between(function: &mut Function, local: u32, minimum: i32, maximum: i32) {
    function
        .instruction(&Instruction::LocalGet(local))
        .instruction(&Instruction::I32Const(minimum))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(local))
        .instruction(&Instruction::I32Const(maximum))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And);
}

fn emit_continuation_test(function: &mut Function, local: u32) {
    function
        .instruction(&Instruction::LocalGet(local))
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::LocalGet(local))
        .instruction(&Instruction::I32Const(0xC0))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::I32Or);
}

fn emit_utf8_failure(function: &mut Function) {
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
        .instruction(&Instruction::Return);
}

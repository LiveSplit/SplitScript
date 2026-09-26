//! Cooperative MD5 helpers used by [`Module.md5`](crate::stdlib).
//!
//! The public operation is a stateful future. Each poll opens the module file,
//! validates the same size and modification time, hashes at most one bounded
//! window, and closes the descriptor before returning. Attachment-lifetime
//! cancellation therefore cannot strand a WASI descriptor. If the file
//! changes between polls, hashing restarts from the initial MD5 state.

use crate::codegen::GC_NULL_HEAP_TYPE;
use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::{abi::AbiImportId, stdlib::StdlibTypeId};

use super::super::memory_plan::RuntimeScratch;
use super::super::{GcLayout, Type, imports::Abi, memarg};

/// One host update hashes at most 512 KiB. At the default attached rate of
/// 120 Hz this permits roughly 60 MiB/s while returning control between
/// windows for larger executables.
const BYTES_PER_POLL: i32 = 512 * 1024;
const STAGING_PREFIX: i32 = 128;
const WASI_RIGHT_FD_READ: i64 = 1 << 1;
const WASI_RIGHT_FD_SEEK: i64 = 1 << 2;
const WASI_RIGHT_FD_FILESTAT_GET: i64 = 1 << 21;
const WASI_WHENCE_SET: i32 = 0;

const INITIAL_A: i32 = 0x6745_2301;
const INITIAL_B: i32 = 0xefcd_ab89_u32 as i32;
const INITIAL_C: i32 = 0x98ba_dcfe_u32 as i32;
const INITIAL_D: i32 = 0x1032_5476;

#[derive(Clone, Copy)]
struct PollStateLocals {
    descriptor: u32,
    initialized: u32,
    offset: u32,
    size: u32,
    mtime: u32,
    packed_ab: u32,
    packed_cd: u32,
}

const SHIFTS: [i32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const CONSTANTS: [u32; 64] = [
    0xd76a_a478,
    0xe8c7_b756,
    0x2420_70db,
    0xc1bd_ceee,
    0xf57c_0faf,
    0x4787_c62a,
    0xa830_4613,
    0xfd46_9501,
    0x6980_98d8,
    0x8b44_f7af,
    0xffff_5bb1,
    0x895c_d7be,
    0x6b90_1122,
    0xfd98_7193,
    0xa679_438e,
    0x49b4_0821,
    0xf61e_2562,
    0xc040_b340,
    0x265e_5a51,
    0xe9b6_c7aa,
    0xd62f_105d,
    0x0244_1453,
    0xd8a1_e681,
    0xe7d3_fbc8,
    0x21e1_cde6,
    0xc337_07d6,
    0xf4d5_0d87,
    0x455a_14ed,
    0xa9e3_e905,
    0xfcef_a3f8,
    0x676f_02d9,
    0x8d2a_4c8a,
    0xfffa_3942,
    0x8771_f681,
    0x6d9d_6122,
    0xfde5_380c,
    0xa4be_ea44,
    0x4bde_cfa9,
    0xf6bb_4b60,
    0xbebf_bc70,
    0x289b_7ec6,
    0xeaa1_27fa,
    0xd4ef_3085,
    0x0488_1d05,
    0xd9d4_d039,
    0xe6db_99e5,
    0x1fa2_7cf8,
    0xc4ac_5665,
    0xf429_2244,
    0x432a_ff97,
    0xab94_23a7,
    0xfc93_a039,
    0x655b_59c3,
    0x8f0c_cc92,
    0xffef_f47d,
    0x8584_5dd1,
    0x6fa8_7e4f,
    0xfe2c_e6e0,
    0xa301_4314,
    0x4e08_11a1,
    0xf753_7e82,
    0xbd3a_f235,
    0x2ad7_d2bb,
    0xeb86_d391,
];

/// Updates four MD5 words from a multiple-of-64-byte linear-memory range.
pub(super) fn compile_update_blocks() -> Function {
    // Parameters: pointer, length, a, b, c, d. Locals: saved a-d and temp.
    let mut function = Function::new([(6, ValType::I32)]);
    let pointer = 0;
    let length = 1;
    let a = 2;
    let b = 3;
    let c = 4;
    let d = 5;
    let saved_a = 6;
    let saved_b = 7;
    let saved_c = 8;
    let saved_d = 9;
    let temporary = 10;
    let temporary_d = 11;

    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1));

    for (source, target) in [(a, saved_a), (b, saved_b), (c, saved_c), (d, saved_d)] {
        function
            .instruction(&Instruction::LocalGet(source))
            .instruction(&Instruction::LocalSet(target));
    }

    for round in 0..64 {
        // Compute F on the stack.
        match round {
            0..=15 => {
                function
                    .instruction(&Instruction::LocalGet(b))
                    .instruction(&Instruction::LocalGet(c))
                    .instruction(&Instruction::I32And)
                    .instruction(&Instruction::LocalGet(b))
                    .instruction(&Instruction::I32Const(-1))
                    .instruction(&Instruction::I32Xor)
                    .instruction(&Instruction::LocalGet(d))
                    .instruction(&Instruction::I32And)
                    .instruction(&Instruction::I32Or);
            }
            16..=31 => {
                function
                    .instruction(&Instruction::LocalGet(d))
                    .instruction(&Instruction::LocalGet(b))
                    .instruction(&Instruction::I32And)
                    .instruction(&Instruction::LocalGet(d))
                    .instruction(&Instruction::I32Const(-1))
                    .instruction(&Instruction::I32Xor)
                    .instruction(&Instruction::LocalGet(c))
                    .instruction(&Instruction::I32And)
                    .instruction(&Instruction::I32Or);
            }
            32..=47 => {
                function
                    .instruction(&Instruction::LocalGet(b))
                    .instruction(&Instruction::LocalGet(c))
                    .instruction(&Instruction::I32Xor)
                    .instruction(&Instruction::LocalGet(d))
                    .instruction(&Instruction::I32Xor);
            }
            _ => {
                function
                    .instruction(&Instruction::LocalGet(d))
                    .instruction(&Instruction::I32Const(-1))
                    .instruction(&Instruction::I32Xor)
                    .instruction(&Instruction::LocalGet(b))
                    .instruction(&Instruction::I32Or)
                    .instruction(&Instruction::LocalGet(c))
                    .instruction(&Instruction::I32Xor);
            }
        }
        function.instruction(&Instruction::LocalSet(temporary));

        let word = match round {
            0..=15 => round,
            16..=31 => (5 * round + 1) % 16,
            32..=47 => (3 * round + 5) % 16,
            _ => (7 * round) % 16,
        };

        // temp = d; d = c; c = b;
        function
            .instruction(&Instruction::LocalGet(d))
            .instruction(&Instruction::LocalSet(temporary_d))
            .instruction(&Instruction::LocalGet(c))
            .instruction(&Instruction::LocalSet(d))
            .instruction(&Instruction::LocalGet(b))
            .instruction(&Instruction::LocalSet(c))
            .instruction(&Instruction::LocalGet(b))
            .instruction(&Instruction::LocalGet(a))
            .instruction(&Instruction::LocalGet(temporary))
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::I32Const(CONSTANTS[round] as i32))
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::LocalGet(pointer))
            .instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                offset: (word * 4) as u64,
                align: 2,
                memory_index: 0,
            }))
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::I32Const(SHIFTS[round]))
            .instruction(&Instruction::I32Rotl)
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::LocalSet(b))
            .instruction(&Instruction::LocalGet(temporary_d))
            .instruction(&Instruction::LocalSet(a));
    }

    for (value, saved) in [(a, saved_a), (b, saved_b), (c, saved_c), (d, saved_d)] {
        function
            .instruction(&Instruction::LocalGet(value))
            .instruction(&Instruction::LocalGet(saved))
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::LocalSet(value));
    }
    function
        .instruction(&Instruction::LocalGet(pointer))
        .instruction(&Instruction::I32Const(64))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(pointer))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(64))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(length))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(a))
        .instruction(&Instruction::LocalGet(b))
        .instruction(&Instruction::LocalGet(c))
        .instruction(&Instruction::LocalGet(d))
        .instruction(&Instruction::End);
    function
}

/// Formats MD5's little-endian digest words as canonical uppercase hex.
pub(super) fn compile_format(gc: &GcLayout) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let string_value = gc.val_type(Type::Standard(StdlibTypeId::String));
    // Parameters: a, b, c, d. Locals: byte and output string.
    let mut function = Function::new([(2, ValType::I32), (1, string_value)]);
    let byte = 4;
    let digit = 5;
    let output = 6;
    function
        .instruction(&Instruction::I32Const(32))
        .instruction(&Instruction::ArrayNewDefault(string_type))
        .instruction(&Instruction::LocalSet(output));

    for index in 0..16_u32 {
        let word = index / 4;
        let shift = (index % 4) * 8;
        function
            .instruction(&Instruction::LocalGet(word))
            .instruction(&Instruction::I32Const(shift as i32))
            .instruction(&Instruction::I32ShrU)
            .instruction(&Instruction::I32Const(0xff))
            .instruction(&Instruction::I32And)
            .instruction(&Instruction::LocalSet(byte));
        emit_hex_digit(
            &mut function,
            output,
            index * 2,
            byte,
            digit,
            4,
            string_type,
        );
        emit_hex_digit(
            &mut function,
            output,
            index * 2 + 1,
            byte,
            digit,
            0,
            string_type,
        );
    }
    function
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

fn emit_hex_digit(
    function: &mut Function,
    output: u32,
    index: u32,
    byte: u32,
    digit: u32,
    shift: i32,
    string_type: u32,
) {
    function
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(index as i32))
        .instruction(&Instruction::LocalGet(byte));
    if shift != 0 {
        function
            .instruction(&Instruction::I32Const(shift))
            .instruction(&Instruction::I32ShrU);
    }
    function
        .instruction(&Instruction::I32Const(0xf))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::LocalTee(digit))
        .instruction(&Instruction::I32Const(10))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(digit))
        .instruction(&Instruction::I32Const(b'0' as i32))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(digit))
        .instruction(&Instruction::I32Const((b'A' - 10) as i32))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::End)
        .instruction(&Instruction::ArraySet(string_type));
}

pub(super) fn compile_module_poll(
    abi: &Abi,
    open_read_only: u32,
    update_blocks: u32,
    format: u32,
    gc: &GcLayout,
    scratch: RuntimeScratch,
) -> Function {
    let stat_pointer = scratch.host_strings_start;
    let iovec_pointer = stat_pointer + 64;
    let bytes_read_pointer = stat_pointer + 72;
    let new_offset_pointer = stat_pointer + 80;
    let bytes_start = stat_pointer + STAGING_PREFIX;
    let staging_end = bytes_start + BYTES_PER_POLL + 128;
    let string_value = gc.val_type(Type::Standard(StdlibTypeId::String));

    // Parameters: path, initialized, offset, size, mtime, packed AB, packed CD.
    let mut function = Function::new([(13, ValType::I32), (3, ValType::I64), (1, string_value)]);
    let path = 0;
    let initialized = 1;
    let offset = 2;
    let expected_size = 3;
    let expected_mtime = 4;
    let packed_ab = 5;
    let packed_cd = 6;
    let descriptor = 7;
    let required_pages = 8;
    let bytes_read = 9;
    let total_read = 10;
    let eof = 11;
    let processed = 12;
    let a = 13;
    let b = 14;
    let c = 15;
    let d = 16;
    let padding_length = 17;
    let index = 18;
    let errno = 19;
    let actual_size = 20;
    let actual_mtime = 21;
    let new_offset = 22;
    let hash = 23;
    let poll = PollStateLocals {
        descriptor,
        initialized,
        offset,
        size: expected_size,
        mtime: expected_mtime,
        packed_ab,
        packed_cd,
    };

    emit_ensure_capacity(&mut function, staging_end, required_pages);
    function
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_return(&mut function, poll, 1, None);
    function.instruction(&Instruction::End);
    function
        .instruction(&Instruction::LocalGet(path))
        .instruction(&Instruction::I64Const(
            WASI_RIGHT_FD_READ | WASI_RIGHT_FD_SEEK | WASI_RIGHT_FD_FILESTAT_GET,
        ))
        .instruction(&Instruction::Call(open_read_only))
        .instruction(&Instruction::LocalTee(descriptor))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32LtS)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_return(&mut function, poll, 1, None);
    function.instruction(&Instruction::End);

    emit_filestat(
        &mut function,
        abi,
        descriptor,
        stat_pointer,
        actual_size,
        actual_mtime,
        errno,
    );
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_close_and_return(&mut function, abi, poll, 1, None);
    function.instruction(&Instruction::End);

    // Initialize on the first poll, or restart if the file changed between
    // polls. Size plus mtime are checked again after every read window.
    function
        .instruction(&Instruction::LocalGet(initialized))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::LocalGet(actual_size))
        .instruction(&Instruction::LocalGet(expected_size))
        .instruction(&Instruction::I64Ne)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalGet(actual_mtime))
        .instruction(&Instruction::LocalGet(expected_mtime))
        .instruction(&Instruction::I64Ne)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::LocalSet(initialized))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(offset))
        .instruction(&Instruction::LocalGet(actual_size))
        .instruction(&Instruction::LocalSet(expected_size))
        .instruction(&Instruction::LocalGet(actual_mtime))
        .instruction(&Instruction::LocalSet(expected_mtime))
        .instruction(&Instruction::I64Const(pack_words(INITIAL_A, INITIAL_B)))
        .instruction(&Instruction::LocalSet(packed_ab))
        .instruction(&Instruction::I64Const(pack_words(INITIAL_C, INITIAL_D)))
        .instruction(&Instruction::LocalSet(packed_cd))
        .instruction(&Instruction::End);

    function
        .instruction(&Instruction::LocalGet(descriptor))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I32Const(WASI_WHENCE_SET))
        .instruction(&Instruction::I32Const(new_offset_pointer))
        .instruction(&Instruction::Call(abi.function(AbiImportId::WasiFdSeek)))
        .instruction(&Instruction::If(BlockType::Empty));
    emit_close_and_return(&mut function, abi, poll, 1, None);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(new_offset_pointer))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(new_offset))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Ne)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_close_and_return(&mut function, abi, poll, 1, None);
    function.instruction(&Instruction::End);

    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(total_read))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(eof))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(total_read))
        .instruction(&Instruction::I32Const(BYTES_PER_POLL))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(iovec_pointer))
        .instruction(&Instruction::I32Const(bytes_start))
        .instruction(&Instruction::LocalGet(total_read))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Store(memarg()))
        .instruction(&Instruction::I32Const(iovec_pointer + 4))
        .instruction(&Instruction::I32Const(BYTES_PER_POLL))
        .instruction(&Instruction::LocalGet(total_read))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Store(memarg()))
        .instruction(&Instruction::I32Const(bytes_read_pointer))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Store(memarg()))
        .instruction(&Instruction::LocalGet(descriptor))
        .instruction(&Instruction::I32Const(iovec_pointer))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Const(bytes_read_pointer))
        .instruction(&Instruction::Call(abi.function(AbiImportId::WasiFdRead)))
        .instruction(&Instruction::If(BlockType::Empty));
    emit_close_and_return(&mut function, abi, poll, 1, None);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(bytes_read_pointer))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::LocalTee(bytes_read))
        .instruction(&Instruction::I32Const(BYTES_PER_POLL))
        .instruction(&Instruction::LocalGet(total_read))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_close_and_return(&mut function, abi, poll, 1, None);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(bytes_read))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(eof))
        .instruction(&Instruction::Br(2))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(total_read))
        .instruction(&Instruction::LocalGet(bytes_read))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(total_read))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);

    emit_filestat(
        &mut function,
        abi,
        descriptor,
        stat_pointer,
        actual_size,
        actual_mtime,
        errno,
    );
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_close_and_return(&mut function, abi, poll, 1, None);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(descriptor))
        .instruction(&Instruction::Call(abi.function(AbiImportId::WasiFdClose)))
        .instruction(&Instruction::If(BlockType::Empty));
    emit_return(&mut function, poll, 1, None);
    function.instruction(&Instruction::End);

    function
        .instruction(&Instruction::LocalGet(actual_size))
        .instruction(&Instruction::LocalGet(expected_size))
        .instruction(&Instruction::I64Ne)
        .instruction(&Instruction::LocalGet(actual_mtime))
        .instruction(&Instruction::LocalGet(expected_mtime))
        .instruction(&Instruction::I64Ne)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(initialized));
    emit_return(&mut function, poll, 0, None);
    function.instruction(&Instruction::End);

    emit_unpack_digest(&mut function, packed_ab, packed_cd, a, b, c, d);
    function
        .instruction(&Instruction::LocalGet(total_read))
        .instruction(&Instruction::I32Const(!63))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::LocalTee(processed))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(bytes_start))
        .instruction(&Instruction::LocalGet(processed))
        .instruction(&Instruction::LocalGet(a))
        .instruction(&Instruction::LocalGet(b))
        .instruction(&Instruction::LocalGet(c))
        .instruction(&Instruction::LocalGet(d))
        .instruction(&Instruction::Call(update_blocks))
        .instruction(&Instruction::LocalSet(d))
        .instruction(&Instruction::LocalSet(c))
        .instruction(&Instruction::LocalSet(b))
        .instruction(&Instruction::LocalSet(a))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::LocalGet(processed))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(offset));
    emit_pack_digest(&mut function, packed_ab, packed_cd, a, b, c, d);

    function
        .instruction(&Instruction::LocalGet(eof))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_return(&mut function, poll, 0, None);
    function.instruction(&Instruction::End);

    // A stable filestat says exactly how many bytes should precede EOF.
    function
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::LocalGet(total_read))
        .instruction(&Instruction::LocalGet(processed))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(expected_size))
        .instruction(&Instruction::I64Ne)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_return(&mut function, poll, 1, None);
    function.instruction(&Instruction::End);

    // Append MD5 padding after the unprocessed tail already in staging.
    function
        .instruction(&Instruction::LocalGet(total_read))
        .instruction(&Instruction::LocalGet(processed))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(56))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(64))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(128))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(padding_length))
        .instruction(&Instruction::I32Const(bytes_start))
        .instruction(&Instruction::LocalGet(processed))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32Store8(memarg()))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(padding_length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(bytes_start))
        .instruction(&Instruction::LocalGet(processed))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Store8(memarg()))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(bytes_start))
        .instruction(&Instruction::LocalGet(processed))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(padding_length))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalGet(expected_size))
        .instruction(&Instruction::I64Const(3))
        .instruction(&Instruction::I64Shl)
        .instruction(&Instruction::I64Store(memarg()))
        .instruction(&Instruction::I32Const(bytes_start))
        .instruction(&Instruction::LocalGet(processed))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(padding_length))
        .instruction(&Instruction::LocalGet(a))
        .instruction(&Instruction::LocalGet(b))
        .instruction(&Instruction::LocalGet(c))
        .instruction(&Instruction::LocalGet(d))
        .instruction(&Instruction::Call(update_blocks))
        .instruction(&Instruction::LocalSet(d))
        .instruction(&Instruction::LocalSet(c))
        .instruction(&Instruction::LocalSet(b))
        .instruction(&Instruction::LocalSet(a))
        .instruction(&Instruction::LocalGet(a))
        .instruction(&Instruction::LocalGet(b))
        .instruction(&Instruction::LocalGet(c))
        .instruction(&Instruction::LocalGet(d))
        .instruction(&Instruction::Call(format))
        .instruction(&Instruction::LocalSet(hash));
    emit_pack_digest(&mut function, packed_ab, packed_cd, a, b, c, d);
    emit_return(&mut function, poll, 2, Some(hash));
    function.instruction(&Instruction::End);
    function
}

fn emit_ensure_capacity(function: &mut Function, end: i32, required_pages: u32) {
    function
        .instruction(&Instruction::I32Const(end))
        .instruction(&Instruction::I32Const(65_535))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(16))
        .instruction(&Instruction::I32ShrU)
        .instruction(&Instruction::LocalTee(required_pages))
        .instruction(&Instruction::MemorySize(0))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(required_pages))
        .instruction(&Instruction::MemorySize(0))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::MemoryGrow(0))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End);
}

fn emit_filestat(
    function: &mut Function,
    abi: &Abi,
    descriptor: u32,
    pointer: i32,
    size: u32,
    mtime: u32,
    errno: u32,
) {
    function
        .instruction(&Instruction::LocalGet(descriptor))
        .instruction(&Instruction::I32Const(pointer))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::WasiFdFilestatGet),
        ))
        .instruction(&Instruction::LocalTee(errno))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(pointer))
        .instruction(&Instruction::I64Load(wasm_encoder::MemArg {
            offset: 32,
            align: 3,
            memory_index: 0,
        }))
        .instruction(&Instruction::LocalSet(size))
        .instruction(&Instruction::I32Const(pointer))
        .instruction(&Instruction::I64Load(wasm_encoder::MemArg {
            offset: 48,
            align: 3,
            memory_index: 0,
        }))
        .instruction(&Instruction::LocalSet(mtime))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(errno));
}

fn emit_close_and_return(
    function: &mut Function,
    abi: &Abi,
    poll: PollStateLocals,
    status: i32,
    hash: Option<u32>,
) {
    function
        .instruction(&Instruction::LocalGet(poll.descriptor))
        .instruction(&Instruction::Call(abi.function(AbiImportId::WasiFdClose)))
        .instruction(&Instruction::Drop);
    emit_return(function, poll, status, hash);
}

fn emit_return(function: &mut Function, poll: PollStateLocals, status: i32, hash: Option<u32>) {
    function
        .instruction(&Instruction::I32Const(status))
        .instruction(&Instruction::LocalGet(poll.initialized))
        .instruction(&Instruction::LocalGet(poll.offset))
        .instruction(&Instruction::LocalGet(poll.size))
        .instruction(&Instruction::LocalGet(poll.mtime))
        .instruction(&Instruction::LocalGet(poll.packed_ab))
        .instruction(&Instruction::LocalGet(poll.packed_cd));
    if let Some(hash) = hash {
        function.instruction(&Instruction::LocalGet(hash));
    } else {
        function.instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE));
    }
    function.instruction(&Instruction::Return);
}

fn emit_unpack_digest(
    function: &mut Function,
    packed_ab: u32,
    packed_cd: u32,
    a: u32,
    b: u32,
    c: u32,
    d: u32,
) {
    for (packed, low, high) in [(packed_ab, a, b), (packed_cd, c, d)] {
        function
            .instruction(&Instruction::LocalGet(packed))
            .instruction(&Instruction::I32WrapI64)
            .instruction(&Instruction::LocalSet(low))
            .instruction(&Instruction::LocalGet(packed))
            .instruction(&Instruction::I64Const(32))
            .instruction(&Instruction::I64ShrU)
            .instruction(&Instruction::I32WrapI64)
            .instruction(&Instruction::LocalSet(high));
    }
}

fn emit_pack_digest(
    function: &mut Function,
    packed_ab: u32,
    packed_cd: u32,
    a: u32,
    b: u32,
    c: u32,
    d: u32,
) {
    for (target, low, high) in [(packed_ab, a, b), (packed_cd, c, d)] {
        function
            .instruction(&Instruction::LocalGet(low))
            .instruction(&Instruction::I64ExtendI32U)
            .instruction(&Instruction::LocalGet(high))
            .instruction(&Instruction::I64ExtendI32U)
            .instruction(&Instruction::I64Const(32))
            .instruction(&Instruction::I64Shl)
            .instruction(&Instruction::I64Or)
            .instruction(&Instruction::LocalSet(target));
    }
}

const fn pack_words(low: i32, high: i32) -> i64 {
    (low as u32 as u64 | ((high as u32 as u64) << 32)) as i64
}

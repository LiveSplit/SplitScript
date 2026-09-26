//! String host-boundary, formatting, and concatenation runtime helpers.

use crate::codegen::GC_NULL_HEAP_TYPE;
use wasm_encoder::{BlockType, Function, HeapType, Instruction, RefType, ValType};

use crate::{abi::AbiImportId, stdlib::StdlibTypeId};

use super::super::imports::Abi;
use super::super::memory_plan::RuntimeScratch;
use super::super::{GcLayout, Type, emit_array_get, memarg};
pub(in crate::codegen::runtime_helpers) fn compile_print_string(
    abi: &Abi,
    gc: &GcLayout,
    scratch: RuntimeScratch,
) -> Function {
    let host_strings = scratch.host_strings_start;
    let mut function = Function::new([(3, ValType::I32)]);
    let string = 0;
    let len = 1;
    let index = 2;
    let required_pages = 3;

    function
        .instruction(&Instruction::LocalGet(string))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(len))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::I32Add)
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
        .instruction(&Instruction::Drop)
        .instruction(&Instruction::End)
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(string))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::I32Store8(memarg()))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::RuntimePrintMessage),
        ))
        .instruction(&Instruction::End);
    function
}

pub(in crate::codegen::runtime_helpers) fn compile_timer_set_variable(
    abi: &Abi,
    gc: &GcLayout,
    scratch: RuntimeScratch,
) -> Function {
    let host_strings = scratch.host_strings_start;
    let mut function = Function::new([(4, ValType::I32)]);
    let key = 0;
    let value = 1;
    let key_len = 2;
    let value_len = 3;
    let index = 4;
    let required_pages = 5;
    function
        .instruction(&Instruction::LocalGet(key))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(key_len))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(value_len))
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(key_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32Add)
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
        .instruction(&Instruction::Drop)
        .instruction(&Instruction::End);
    emit_gc_string_copy_to_memory(&mut function, key, key_len, index, host_strings, None, gc);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index));
    emit_gc_string_copy_to_memory(
        &mut function,
        value,
        value_len,
        index,
        host_strings,
        Some(key_len),
        gc,
    );
    function
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(key_len))
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(key_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::TimerSetVariable),
        ))
        .instruction(&Instruction::End);
    function
}

fn emit_gc_string_copy_to_memory(
    function: &mut Function,
    string: u32,
    len: u32,
    index: u32,
    base: i32,
    additional_offset: Option<u32>,
    gc: &GcLayout,
) {
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(base));
    if let Some(offset) = additional_offset {
        function
            .instruction(&Instruction::LocalGet(offset))
            .instruction(&Instruction::I32Add);
    }
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(string))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::I32Store8(memarg()))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
}

pub(in crate::codegen::runtime_helpers) fn compile_format_i64(gc: &GcLayout) -> Function {
    let mut function = Function::new([
        (2, ValType::I64),
        (4, ValType::I32),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
    ]);
    let input = 0;
    let radix = 1;
    let signed = 2;
    let magnitude = 3;
    let remaining = 4;
    let digits = 5;
    let index = 6;
    let negative = 7;
    let digit = 8;
    let output = 9;
    function
        .instruction(&Instruction::LocalGet(radix))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::LocalGet(radix))
        .instruction(&Instruction::I32Const(36))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Result(
            gc.val_type(Type::Standard(StdlibTypeId::String)),
        )))
        .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(signed))
        .instruction(&Instruction::LocalGet(input))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::I64LtS)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::LocalSet(negative))
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalGet(input))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(input))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalTee(magnitude))
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(digits))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(radix))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(radix))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64DivU)
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::LocalGet(digits))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(digits))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(digits))
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayNewDefault(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::LocalSet(output))
        .instruction(&Instruction::LocalGet(digits))
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::LocalGet(magnitude))
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(radix))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64RemU)
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::LocalTee(digit))
        .instruction(&Instruction::I32Const(10))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(digit))
        .instruction(&Instruction::I32Const(b'0' as i32))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(digit))
        .instruction(&Instruction::I32Const((b'a' - 10) as i32))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::End)
        .instruction(&Instruction::ArraySet(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(radix))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64DivU)
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(b'-' as i32))
        .instruction(&Instruction::ArraySet(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
    function
}

/// Encodes one validated Unicode scalar into an immutable UTF-8 String.
pub(in crate::codegen::runtime_helpers) fn compile_format_char(gc: &GcLayout) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let mut function = Function::new([
        (4, ValType::I32),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
    ]);
    let value = 0;
    let byte_len = 1;
    let index = 2;
    let remaining = 3;
    let prefix = 4;
    let output = 5;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::I32Const(0x800))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::I32Const(0x10000))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(3))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalTee(byte_len))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0xC0))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::I32Const(3))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0xE0))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(0xF0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(prefix))
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::ArrayNewDefault(string_type))
        .instruction(&Instruction::LocalSet(output))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::LocalSet(remaining))
        // Emit continuation bytes from the end towards the leading byte.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalTee(index))
        .instruction(&Instruction::LocalSet(byte_len))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I32Const(0x3F))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::ArraySet(string_type))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I32Const(6))
        .instruction(&Instruction::I32ShrU)
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(prefix))
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::ArraySet(string_type))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

/// Quotes and escapes a UTF-8 value for a structural `Debug` representation.
/// `quote` is either `"` for strings or `'` for characters.
pub(in crate::codegen::runtime_helpers) fn compile_quote_debug_string(gc: &GcLayout) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let string_value = gc.val_type(Type::Standard(StdlibTypeId::String));
    let mut function = Function::new([(7, ValType::I32), (1, string_value)]);
    let input = 0;
    let quote = 1;
    let length = 2;
    let input_index = 3;
    let output_length = 4;
    let output_index = 5;
    let byte = 6;
    let escape = 7;
    let nibble = 8;
    let output = 9;

    function
        .instruction(&Instruction::LocalGet(input))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(length))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::LocalSet(output_length))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty));
    emit_loop_end_test(&mut function, input_index, length);
    emit_input_byte(&mut function, input, input_index, byte, string_type);
    emit_short_escape_condition(&mut function, byte, quote);
    function
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::Else);
    emit_control_condition(&mut function, byte);
    function
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output_length))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_length));
    emit_increment(&mut function, input_index);
    function
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output_length))
        .instruction(&Instruction::ArrayNewDefault(string_type))
        .instruction(&Instruction::LocalSet(output));
    emit_output_local(&mut function, output, 0, quote, string_type, true);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(input_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty));
    emit_loop_end_test(&mut function, input_index, length);
    emit_input_byte(&mut function, input, input_index, byte, string_type);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(escape))
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::LocalGet(quote))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(quote))
        .instruction(&Instruction::LocalSet(escape))
        .instruction(&Instruction::End);
    for (source, escaped) in [
        (b'\\', b'\\'),
        (b'\n', b'n'),
        (b'\r', b'r'),
        (b'\t', b't'),
        (0, b'0'),
    ] {
        function
            .instruction(&Instruction::LocalGet(byte))
            .instruction(&Instruction::I32Const(source as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(escaped as i32))
            .instruction(&Instruction::LocalSet(escape))
            .instruction(&Instruction::End);
    }
    function
        .instruction(&Instruction::LocalGet(escape))
        .instruction(&Instruction::I32Eqz);
    emit_control_condition(&mut function, byte);
    function
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0xff))
        .instruction(&Instruction::LocalSet(escape))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(escape))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_output_local(
        &mut function,
        output,
        output_index,
        byte,
        string_type,
        false,
    );
    emit_increment(&mut function, output_index);
    function.instruction(&Instruction::Else);
    emit_output_const(&mut function, output, output_index, b'\\', string_type);
    emit_increment(&mut function, output_index);
    function
        .instruction(&Instruction::LocalGet(escape))
        .instruction(&Instruction::I32Const(0xff))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_output_const(&mut function, output, output_index, b'x', string_type);
    emit_increment(&mut function, output_index);
    for shift in [4, 0] {
        function
            .instruction(&Instruction::LocalGet(byte))
            .instruction(&Instruction::I32Const(shift))
            .instruction(&Instruction::I32ShrU)
            .instruction(&Instruction::I32Const(0xf))
            .instruction(&Instruction::I32And)
            .instruction(&Instruction::LocalSet(nibble))
            .instruction(&Instruction::LocalGet(output))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::LocalGet(output_index))
            .instruction(&Instruction::LocalGet(nibble))
            .instruction(&Instruction::I32Const(10))
            .instruction(&Instruction::I32LtU)
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
            .instruction(&Instruction::LocalGet(nibble))
            .instruction(&Instruction::I32Const(b'0' as i32))
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::Else)
            .instruction(&Instruction::LocalGet(nibble))
            .instruction(&Instruction::I32Const((b'a' - 10) as i32))
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::End)
            .instruction(&Instruction::ArraySet(string_type));
        emit_increment(&mut function, output_index);
    }
    function.instruction(&Instruction::Else);
    emit_output_local(
        &mut function,
        output,
        output_index,
        escape,
        string_type,
        false,
    );
    emit_increment(&mut function, output_index);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
    emit_increment(&mut function, input_index);
    function
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
    emit_output_local(
        &mut function,
        output,
        output_index,
        quote,
        string_type,
        false,
    );
    function
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

fn emit_short_escape_condition(function: &mut Function, byte: u32, quote: u32) {
    function
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::LocalGet(quote))
        .instruction(&Instruction::I32Eq);
    for escaped in [b'\\', b'\n', b'\r', b'\t', 0] {
        function
            .instruction(&Instruction::LocalGet(byte))
            .instruction(&Instruction::I32Const(escaped as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::I32Or);
    }
}

fn emit_control_condition(function: &mut Function, byte: u32) {
    function
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(0x20))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(0x7f))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::I32Or);
}

fn emit_loop_end_test(function: &mut Function, index: u32, length: u32) {
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1));
}

fn emit_input_byte(function: &mut Function, input: u32, index: u32, byte: u32, string_type: u32) {
    function
        .instruction(&Instruction::LocalGet(input))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalSet(byte));
}

fn emit_output_local(
    function: &mut Function,
    output: u32,
    index: u32,
    byte: u32,
    string_type: u32,
    constant_index: bool,
) {
    function
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull);
    if constant_index {
        function.instruction(&Instruction::I32Const(index as i32));
    } else {
        function.instruction(&Instruction::LocalGet(index));
    }
    function
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::ArraySet(string_type));
}

fn emit_output_const(function: &mut Function, output: u32, index: u32, byte: u8, string_type: u32) {
    function
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(byte as i32))
        .instruction(&Instruction::ArraySet(string_type));
}

fn emit_increment(function: &mut Function, local: u32) {
    function
        .instruction(&Instruction::LocalGet(local))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(local));
}

/// Compares a UTF-8 byte sequence using contains (0), starts with (1), ends with
/// (2), or equal while ignoring ASCII case (3) semantics. Exact byte matching
/// is equivalent to Unicode scalar matching for the first three modes because
/// both strings are valid UTF-8. The final mode folds only ASCII letters and is
/// deliberately not Unicode case folding.
pub(in crate::codegen::runtime_helpers) fn compile_string_match(gc: &GcLayout) -> Function {
    let mut function = Function::new([(7, ValType::I32)]);
    let value = 0;
    let needle = 1;
    let mode = 2;
    let value_len = 3;
    let needle_len = 4;
    let start = 5;
    let last_start = 6;
    let index = 7;
    let value_byte = 8;
    let needle_byte = 9;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(value_len))
        .instruction(&Instruction::LocalGet(needle))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(needle_len))
        .instruction(&Instruction::LocalGet(mode))
        .instruction(&Instruction::I32Const(3))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(last_start))
        .instruction(&Instruction::LocalGet(mode))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(last_start))
        .instruction(&Instruction::LocalSet(start))
        .instruction(&Instruction::End)
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::LocalSet(value_byte))
        .instruction(&Instruction::LocalGet(needle))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::LocalSet(needle_byte))
        .instruction(&Instruction::LocalGet(mode))
        .instruction(&Instruction::I32Const(3))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value_byte))
        .instruction(&Instruction::I32Const(b'A' as i32))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(value_byte))
        .instruction(&Instruction::I32Const(b'Z' as i32))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value_byte))
        .instruction(&Instruction::I32Const(32))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(value_byte))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(needle_byte))
        .instruction(&Instruction::I32Const(b'A' as i32))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(needle_byte))
        .instruction(&Instruction::I32Const(b'Z' as i32))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(needle_byte))
        .instruction(&Instruction::I32Const(32))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(needle_byte))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value_byte))
        .instruction(&Instruction::LocalGet(needle_byte))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(mode))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(last_start))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(start))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::End);
    function
}

/// Converts ASCII letters to the requested case while preserving all other
/// UTF-8 bytes. `mode` is zero for lowercase and one for uppercase. Immutable
/// strings that are already normalized reuse their existing GC object; only
/// an actual transformation allocates.
pub(in crate::codegen::runtime_helpers) fn compile_string_ascii_case(gc: &GcLayout) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let mut function = Function::new([
        (4, ValType::I32),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
    ]);
    let value = 0;
    let mode = 1;
    let len = 2;
    let index = 3;
    let byte = 4;
    let first = 5;
    let output = 6;

    function
        .instruction(&Instruction::I32Const(b'A' as i32))
        .instruction(&Instruction::LocalGet(mode))
        .instruction(&Instruction::I32Const(32))
        .instruction(&Instruction::I32Mul)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(first))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(len))
        // First find whether any transformation is needed. Returning the
        // immutable receiver avoids one allocation for already-normalized text.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalSet(byte))
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(25))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::ArrayNewDefault(string_type))
        .instruction(&Instruction::LocalSet(output))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalSet(byte))
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(25))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(32))
        .instruction(&Instruction::I32Xor)
        .instruction(&Instruction::LocalSet(byte))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::ArraySet(string_type))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

/// Finds the first exact UTF-8 byte match at or after `start`, returning `-1`
/// when no match exists. Both operands are valid UTF-8, so any non-empty match
/// starts and ends at code-point boundaries.
pub(in crate::codegen::runtime_helpers) fn compile_string_find(gc: &GcLayout) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let mut function = Function::new([(5, ValType::I32)]);
    let value = 0;
    let needle = 1;
    let start = 2;
    let value_len = 3;
    let needle_len = 4;
    let candidate = 5;
    let last_start = 6;
    let index = 7;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(value_len))
        .instruction(&Instruction::LocalGet(needle))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(needle_len))
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::End)
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(last_start))
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(last_start))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalSet(candidate))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(candidate))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(candidate))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalGet(needle))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(candidate))
        .instruction(&Instruction::LocalGet(last_start))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(candidate))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(candidate))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::End);
    function
}

/// Finds the last exact UTF-8 byte match, returning `-1` when no match exists.
/// Both operands are valid UTF-8, so any non-empty match starts and ends at
/// code-point boundaries. The empty string matches the final byte boundary.
pub(in crate::codegen::runtime_helpers) fn compile_string_rfind(gc: &GcLayout) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let mut function = Function::new([(4, ValType::I32)]);
    let value = 0;
    let needle = 1;
    let value_len = 2;
    let needle_len = 3;
    let candidate = 4;
    let index = 5;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(value_len))
        .instruction(&Instruction::LocalGet(needle))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(needle_len))
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(candidate))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(candidate))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(candidate))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalGet(needle))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(candidate))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(candidate))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(candidate))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::End);
    function
}

/// Replaces exact, non-overlapping matches after first computing the precise
/// output size. A null reference is the failure sentinel for an empty search or
/// a result whose byte length cannot be represented by a WebAssembly array.
pub(in crate::codegen::runtime_helpers) fn compile_string_replace_all(
    string_find: u32,
    gc: &GcLayout,
) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let mut function = Function::new([
        (10, ValType::I32),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
    ]);
    let value = 0;
    let search = 1;
    let replacement = 2;
    let value_len = 3;
    let search_len = 4;
    let replacement_len = 5;
    let scan_index = 6;
    let match_index = 7;
    let match_count = 8;
    let length_delta = 9;
    let output_len = 10;
    let input_index = 11;
    let output_index = 12;
    let output = 13;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(value_len))
        .instruction(&Instruction::LocalGet(search))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalTee(search_len))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_null_string(&mut function);
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(replacement))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(replacement_len))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::LocalGet(search))
        .instruction(&Instruction::LocalGet(scan_index))
        .instruction(&Instruction::Call(string_find))
        .instruction(&Instruction::LocalTee(match_index))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(match_count))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(match_count))
        .instruction(&Instruction::LocalGet(match_index))
        .instruction(&Instruction::LocalGet(search_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(scan_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(match_count))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(replacement_len))
        .instruction(&Instruction::LocalGet(search_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(replacement_len))
        .instruction(&Instruction::LocalGet(search_len))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalTee(length_delta))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalGet(match_count))
        .instruction(&Instruction::I32DivU)
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_null_string(&mut function);
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::LocalGet(match_count))
        .instruction(&Instruction::LocalGet(length_delta))
        .instruction(&Instruction::I32Mul)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_len))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(search_len))
        .instruction(&Instruction::LocalGet(replacement_len))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(length_delta))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::LocalGet(match_count))
        .instruction(&Instruction::LocalGet(length_delta))
        .instruction(&Instruction::I32Mul)
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(output_len))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output_len))
        .instruction(&Instruction::ArrayNewDefault(string_type))
        .instruction(&Instruction::LocalSet(output))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::LocalGet(search))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::Call(string_find))
        .instruction(&Instruction::LocalTee(match_index))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::ArrayCopy {
            array_type_index_dst: string_type,
            array_type_index_src: string_type,
        })
        .instruction(&Instruction::Br(2))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::LocalGet(match_index))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::ArrayCopy {
            array_type_index_dst: string_type,
            array_type_index_src: string_type,
        })
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(match_index))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(replacement))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(replacement_len))
        .instruction(&Instruction::ArrayCopy {
            array_type_index_dst: string_type,
            array_type_index_src: string_type,
        })
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(replacement_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::LocalGet(match_index))
        .instruction(&Instruction::LocalGet(search_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(input_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

/// Splits at exact, non-overlapping UTF-8 delimiters while preserving empty
/// segments. A null array is the helper ABI's failure sentinel for an empty
/// delimiter or an unrepresentable segment count.
pub(in crate::codegen::runtime_helpers) fn compile_string_split(
    string_find: u32,
    strings_array: u32,
    strings_storage: u32,
    gc: &GcLayout,
) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let mut function = Function::new([
        (8, ValType::I32),
        (
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(strings_storage),
            }),
        ),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
    ]);
    let value = 0;
    let delimiter = 1;
    let value_len = 2;
    let delimiter_len = 3;
    let scan_index = 4;
    let match_index = 5;
    let segment_count = 6;
    let segment_start = 7;
    let output_index = 8;
    let segment_len = 9;
    let output = 10;
    let segment = 11;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(value_len))
        .instruction(&Instruction::LocalGet(delimiter))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalTee(delimiter_len))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(segment_count))
        // Count matches before allocating the exact-length result array.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::LocalGet(delimiter))
        .instruction(&Instruction::LocalGet(scan_index))
        .instruction(&Instruction::Call(string_find))
        .instruction(&Instruction::LocalTee(match_index))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(segment_count))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalTee(segment_count))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(match_index))
        .instruction(&Instruction::LocalGet(delimiter_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(scan_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(segment_count))
        .instruction(&Instruction::ArrayNewDefault(strings_storage))
        .instruction(&Instruction::LocalSet(output))
        // Materialize each segment, including zero-length edge segments.
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::LocalGet(delimiter))
        .instruction(&Instruction::LocalGet(segment_start))
        .instruction(&Instruction::Call(string_find))
        .instruction(&Instruction::LocalTee(match_index))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::LocalGet(segment_start))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(segment_len));
    emit_split_segment(
        &mut function,
        value,
        segment_start,
        segment_len,
        output,
        output_index,
        segment,
        string_type,
        strings_storage,
    );
    function
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::LocalGet(segment_count));
    super::super::array_value::emit_wrap_loaded(&mut function, strings_array);
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(match_index))
        .instruction(&Instruction::LocalGet(segment_start))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(segment_len));
    emit_split_segment(
        &mut function,
        value,
        segment_start,
        segment_len,
        output,
        output_index,
        segment,
        string_type,
        strings_storage,
    );
    function
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::LocalGet(match_index))
        .instruction(&Instruction::LocalGet(delimiter_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(segment_start))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
        .instruction(&Instruction::End);
    function
}

#[allow(clippy::too_many_arguments)]
fn emit_split_segment(
    function: &mut Function,
    value: u32,
    segment_start: u32,
    segment_len: u32,
    output: u32,
    output_index: u32,
    segment: u32,
    string_type: u32,
    strings_array: u32,
) {
    function
        .instruction(&Instruction::LocalGet(segment_len))
        .instruction(&Instruction::ArrayNewDefault(string_type))
        .instruction(&Instruction::LocalSet(segment))
        .instruction(&Instruction::LocalGet(segment))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(segment_start))
        .instruction(&Instruction::LocalGet(segment_len))
        .instruction(&Instruction::ArrayCopy {
            array_type_index_dst: string_type,
            array_type_index_src: string_type,
        })
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(segment))
        .instruction(&Instruction::ArraySet(strings_array));
}

/// Parses strict ASCII decimal integers into an unsigned magnitude before
/// applying the sign. The caller supplies the exact positive and negative
/// magnitude limits for its inferred integer representation. The first result
/// is a success flag; the second is the parsed two's-complement bit pattern.
pub(in crate::codegen::runtime_helpers) fn compile_string_parse_integer(gc: &GcLayout) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let mut function = Function::new([(4, ValType::I32), (3, ValType::I64)]);
    let value = 0;
    let allow_negative = 1;
    let positive_limit = 2;
    let negative_limit = 3;
    let len = 4;
    let index = 5;
    let byte = 6;
    let negative = 7;
    let limit = 8;
    let magnitude = 9;
    let digit = 10;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalTee(len))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_integer_parse_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalSet(byte))
        // Consume one optional leading sign.
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'+' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'-' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(allow_negative))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_integer_parse_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(negative))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_integer_parse_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::LocalGet(negative_limit))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(positive_limit))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(limit))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalSet(byte))
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'0' as i32))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'9' as i32))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_integer_parse_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'0' as i32))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalSet(digit))
        // magnitude > (limit - digit) / 10 would overflow the target.
        .instruction(&Instruction::LocalGet(magnitude))
        .instruction(&Instruction::LocalGet(limit))
        .instruction(&Instruction::LocalGet(digit))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64DivU)
        .instruction(&Instruction::I64GtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_integer_parse_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(magnitude))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::LocalGet(digit))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(magnitude))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalGet(magnitude))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(magnitude))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
    function
}

fn emit_integer_parse_failure(function: &mut Function) {
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return);
}

/// Reads either one raw UTF-8 byte (mode 0) or the Unicode scalar beginning at
/// a UTF-8 byte boundary (mode 1). The `(success, value)` result keeps both
/// operations allocation-free; expression lowering supplies the language-level
/// error value. Although every SplitScript String is valid UTF-8, decoding is
/// deliberately defensive so an invalid internal value cannot cause an
/// out-of-bounds array access or manufacture a Unicode scalar.
pub(in crate::codegen::runtime_helpers) fn compile_string_inspect(gc: &GcLayout) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let mut function = Function::new([(6, ValType::I32)]);
    let value = 0;
    let index = 1;
    let mode = 2;
    let value_len = 3;
    let first = 4;
    let second = 5;
    let third = 6;
    let fourth = 7;
    let code_point = 8;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalTee(value_len))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_string_inspect_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalSet(first))
        // Raw byte inspection accepts continuation bytes because the operation
        // intentionally exposes the UTF-8 representation.
        .instruction(&Instruction::LocalGet(mode))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_string_inspect_success(&mut function, first);
    function
        .instruction(&Instruction::End)
        // ASCII is already the scalar value.
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_string_inspect_success(&mut function, first);
    function
        .instruction(&Instruction::End)
        // A valid two-byte sequence starts at C2. C0/C1 are overlong and
        // 80..BF are continuation bytes rather than scalar boundaries.
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(0xC2))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_string_inspect_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(0xE0))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_string_inspect_require_bytes(&mut function, value_len, index, 2);
    emit_string_inspect_load_byte(&mut function, string_type, value, index, 1, second);
    emit_string_inspect_require_continuation(&mut function, second);
    function
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(0x1F))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Const(6))
        .instruction(&Instruction::I32Shl)
        .instruction(&Instruction::LocalGet(second))
        .instruction(&Instruction::I32Const(0x3F))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalSet(code_point));
    emit_string_inspect_success(&mut function, code_point);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(0xF0))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_string_inspect_require_bytes(&mut function, value_len, index, 3);
    emit_string_inspect_load_byte(&mut function, string_type, value, index, 1, second);
    emit_string_inspect_load_byte(&mut function, string_type, value, index, 2, third);
    emit_string_inspect_require_continuation(&mut function, second);
    emit_string_inspect_require_continuation(&mut function, third);
    // E0 A0..BF excludes overlong encodings; ED 80..9F excludes UTF-16
    // surrogate code points, which are not Unicode scalar values.
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
        .instruction(&Instruction::If(BlockType::Empty));
    emit_string_inspect_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(0x0F))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Const(12))
        .instruction(&Instruction::I32Shl)
        .instruction(&Instruction::LocalGet(second))
        .instruction(&Instruction::I32Const(0x3F))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Const(6))
        .instruction(&Instruction::I32Shl)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalGet(third))
        .instruction(&Instruction::I32Const(0x3F))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalSet(code_point));
    emit_string_inspect_success(&mut function, code_point);
    function
        .instruction(&Instruction::End)
        // Unicode ends at U+10FFFF, so F5..FF can never begin valid UTF-8.
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(0xF4))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_string_inspect_failure(&mut function);
    function.instruction(&Instruction::End);
    emit_string_inspect_require_bytes(&mut function, value_len, index, 4);
    emit_string_inspect_load_byte(&mut function, string_type, value, index, 1, second);
    emit_string_inspect_load_byte(&mut function, string_type, value, index, 2, third);
    emit_string_inspect_load_byte(&mut function, string_type, value, index, 3, fourth);
    emit_string_inspect_require_continuation(&mut function, second);
    emit_string_inspect_require_continuation(&mut function, third);
    emit_string_inspect_require_continuation(&mut function, fourth);
    // F0 90..BF excludes overlong encodings; F4 80..8F caps the result at
    // U+10FFFF.
    function
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(0xF0))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(second))
        .instruction(&Instruction::I32Const(0x90))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(0xF4))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(second))
        .instruction(&Instruction::I32Const(0x90))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_string_inspect_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::I32Const(0x07))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Const(18))
        .instruction(&Instruction::I32Shl)
        .instruction(&Instruction::LocalGet(second))
        .instruction(&Instruction::I32Const(0x3F))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Const(12))
        .instruction(&Instruction::I32Shl)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalGet(third))
        .instruction(&Instruction::I32Const(0x3F))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Const(6))
        .instruction(&Instruction::I32Shl)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalGet(fourth))
        .instruction(&Instruction::I32Const(0x3F))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalSet(code_point));
    emit_string_inspect_success(&mut function, code_point);
    function.instruction(&Instruction::End);
    function
}

fn emit_string_inspect_failure(function: &mut Function) {
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return);
}

fn emit_string_inspect_success(function: &mut Function, value: u32) {
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::Return);
}

fn emit_string_inspect_require_bytes(
    function: &mut Function,
    value_len: u32,
    index: u32,
    required: i32,
) {
    function
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Const(required))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_string_inspect_failure(function);
    function.instruction(&Instruction::End);
}

fn emit_string_inspect_load_byte(
    function: &mut Function,
    string_type: u32,
    value: u32,
    index: u32,
    offset: i32,
    destination: u32,
) {
    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(offset))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalSet(destination));
}

fn emit_string_inspect_require_continuation(function: &mut Function, byte: u32) {
    function
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(0xC0))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_string_inspect_failure(function);
    function.instruction(&Instruction::End);
}

/// Extracts a UTF-8 byte range. Returning a null reference is the helper ABI's
/// failure sentinel for reversed or out-of-range bounds and offsets that split
/// a UTF-8 code point; expression lowering converts it to `String!`.
pub(in crate::codegen::runtime_helpers) fn compile_string_slice(gc: &GcLayout) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let mut function = Function::new([
        (3, ValType::I32),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
    ]);
    let value = 0;
    let start = 1;
    let end = 2;
    let value_len = 3;
    let output_len = 4;
    let copy_index = 5;
    let output = 6;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(value_len))
        // Reject reversed and out-of-range bounds.
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_null_string(&mut function);
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        // An offset at the end is a boundary. Every other boundary starts with
        // a byte whose two high bits are not `10`.
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::I32Const(0xC0))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_null_string(&mut function);
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::I32Const(0xC0))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_null_string(&mut function);
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalTee(output_len))
        .instruction(&Instruction::ArrayNewDefault(string_type))
        .instruction(&Instruction::LocalSet(output))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::LocalGet(output_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::ArraySet(string_type))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(copy_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

/// Trims the six ASCII whitespace bytes from both ends of a UTF-8 string.
/// Every removed byte is a complete one-byte code point, so the resulting
/// bounds are valid inputs to the shared UTF-8 slice helper. An unchanged
/// string reuses its existing immutable GC object.
pub(in crate::codegen::runtime_helpers) fn compile_string_trim_ascii_whitespace(
    string_slice: u32,
    gc: &GcLayout,
) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let mut function = Function::new([(4, ValType::I32)]);
    let value = 0;
    let len = 1;
    let start = 2;
    let end = 3;
    let byte = 4;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalTee(len))
        .instruction(&Instruction::LocalSet(end))
        // Scan the leading ASCII whitespace.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalSet(byte));
    emit_is_ascii_whitespace(&mut function, byte);
    function
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(start))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        // Scan the trailing ASCII whitespace without crossing `start`.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalSet(byte));
    emit_is_ascii_whitespace(&mut function, byte);
    function
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(end))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        // Avoid allocating when both boundaries are unchanged.
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::Call(string_slice))
        .instruction(&Instruction::End);
    function
}

/// Reports whether an immutable UTF-8 string is empty or consists entirely of
/// Unicode whitespace. Scalar decoding is delegated to `StringInspect`, which
/// also keeps this helper defensive if an invalid internal String is ever
/// manufactured.
///
/// The ranges in `emit_is_unicode_white_space` are the complete `White_Space`
/// property from Unicode 17.0.0, `PropList-17.0.0.txt`, dated 2025-06-30:
/// https://www.unicode.org/Public/17.0.0/ucd/PropList.txt
///
/// When updating Unicode, update the version and source above together with
/// the ranges below and the boundary cases in `tests/string_predicates.split`.
pub(in crate::codegen::runtime_helpers) fn compile_string_is_blank(
    string_inspect: u32,
) -> Function {
    let mut function = Function::new([(4, ValType::I32)]);
    let value = 0;
    let value_len = 1;
    let index = 2;
    let success = 3;
    let code_point = 4;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(value_len))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Call(string_inspect))
        // Multiple results are consumed from right to left.
        .instruction(&Instruction::LocalSet(code_point))
        .instruction(&Instruction::LocalSet(success))
        .instruction(&Instruction::LocalGet(success))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
    emit_is_unicode_white_space(&mut function, code_point);
    function
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(code_point))
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(code_point))
        .instruction(&Instruction::I32Const(0x800))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::Else)
        // Unicode White_Space currently contains no four-byte scalar.
        .instruction(&Instruction::I32Const(3))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End);
    function
}

/// Pushes whether `code_point` has Unicode 17.0.0's `White_Space` property.
fn emit_is_unicode_white_space(function: &mut Function, code_point: u32) {
    function
        // U+0009..U+000D
        .instruction(&Instruction::LocalGet(code_point))
        .instruction(&Instruction::I32Const(0x0009))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(code_point))
        .instruction(&Instruction::I32Const(0x000D))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And);
    for scalar in [0x0020, 0x0085, 0x00A0, 0x1680] {
        function
            .instruction(&Instruction::LocalGet(code_point))
            .instruction(&Instruction::I32Const(scalar))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::I32Or);
    }
    function
        // U+2000..U+200A
        .instruction(&Instruction::LocalGet(code_point))
        .instruction(&Instruction::I32Const(0x2000))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(code_point))
        .instruction(&Instruction::I32Const(0x200A))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or);
    for scalar in [0x2028, 0x2029, 0x202F, 0x205F, 0x3000] {
        function
            .instruction(&Instruction::LocalGet(code_point))
            .instruction(&Instruction::I32Const(scalar))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::I32Or);
    }
}

/// Pads an immutable UTF-8 string to a minimum Unicode-scalar length. The
/// original GC object is reused when no padding is required; otherwise the
/// result is allocated once at its exact byte length. `pad_end` selects whether
/// the padding precedes (0) or follows (1) the original bytes.
pub(in crate::codegen::runtime_helpers) fn compile_string_pad(gc: &GcLayout) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let mut function = Function::new([
        (12, ValType::I32),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
    ]);
    let value = 0;
    let width = 1;
    let fill = 2;
    let pad_end = 3;
    let value_len = 4;
    let char_count = 5;
    let input_index = 6;
    let byte = 7;
    let fill_len = 8;
    let padding_count = 9;
    let padding_bytes = 10;
    let output_len = 11;
    let output_index = 12;
    let padding_index = 13;
    let remaining = 14;
    let char_byte_index = 15;
    let output = 16;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(value_len))
        // Count UTF-8 leading bytes, which are exactly the Unicode scalars.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalSet(byte))
        .instruction(&Instruction::LocalGet(char_count))
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(0xc0))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(char_count))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(input_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(char_count))
        .instruction(&Instruction::LocalGet(width))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(fill))
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(fill))
        .instruction(&Instruction::I32Const(0x800))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(fill))
        .instruction(&Instruction::I32Const(0x10000))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(3))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(fill_len))
        .instruction(&Instruction::LocalGet(width))
        .instruction(&Instruction::LocalGet(char_count))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalTee(padding_count))
        .instruction(&Instruction::LocalGet(fill_len))
        .instruction(&Instruction::I32Mul)
        .instruction(&Instruction::LocalTee(padding_bytes))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalTee(output_len))
        .instruction(&Instruction::ArrayNewDefault(string_type))
        .instruction(&Instruction::LocalSet(output))
        // Copy the original bytes into their final position.
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(pad_end))
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(padding_bytes))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::ArrayCopy {
            array_type_index_dst: string_type,
            array_type_index_src: string_type,
        })
        .instruction(&Instruction::LocalGet(pad_end))
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(output_index))
        // Encode the fill character directly for each required scalar.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(padding_index))
        .instruction(&Instruction::LocalGet(padding_count))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(fill))
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::LocalGet(fill_len))
        .instruction(&Instruction::LocalSet(char_byte_index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(char_byte_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(char_byte_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(char_byte_index))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(char_byte_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I32Const(0x3f))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::ArraySet(string_type))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I32Const(6))
        .instruction(&Instruction::I32ShrU)
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(fill_len))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(fill_len))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0xc0))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(fill_len))
        .instruction(&Instruction::I32Const(3))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0xe0))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(0xf0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::ArraySet(string_type))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(fill_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::LocalGet(padding_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(padding_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

fn emit_is_ascii_whitespace(function: &mut Function, byte: u32) {
    function
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b' ' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'\t' as i32))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'\r' as i32))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or);
}

fn emit_null_string(function: &mut Function) {
    function.instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE));
}

pub(in crate::codegen::runtime_helpers) fn compile_join_strings(
    strings_array: u32,
    strings_storage: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([
        (6, ValType::I32),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
        (
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(strings_storage),
            }),
        ),
    ]);
    let strings = 0;
    let separator = 1;
    let string_index = 2;
    let total_len = 3;
    let byte_index = 4;
    let output_index = 5;
    let separator_len = 6;
    let string_count = 7;
    let current = 8;
    let output = 9;
    let strings_backing = 10;
    function
        .instruction(&Instruction::LocalGet(strings))
        .instruction(&Instruction::StructGet {
            struct_type_index: strings_array,
            field_index: super::super::array_value::LENGTH_FIELD,
        })
        .instruction(&Instruction::LocalSet(string_count))
        .instruction(&Instruction::LocalGet(strings))
        .instruction(&Instruction::StructGet {
            struct_type_index: strings_array,
            field_index: super::super::array_value::BACKING_FIELD,
        })
        .instruction(&Instruction::LocalSet(strings_backing))
        .instruction(&Instruction::LocalGet(separator))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(separator))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(separator_len))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(string_index))
        .instruction(&Instruction::LocalGet(string_count))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(strings_backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(string_index));
    emit_array_get(
        &mut function,
        strings_storage,
        Type::Standard(StdlibTypeId::String),
        gc,
    );
    function
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalGet(total_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(total_len))
        .instruction(&Instruction::LocalGet(string_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(string_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(string_count))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(total_len))
        .instruction(&Instruction::LocalGet(string_count))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalGet(separator_len))
        .instruction(&Instruction::I32Mul)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(total_len))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(total_len))
        .instruction(&Instruction::ArrayNewDefault(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::LocalSet(output))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(string_index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(string_index))
        .instruction(&Instruction::LocalGet(string_count))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(string_index))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::LocalGet(separator_len))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(separator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(separator_len))
        .instruction(&Instruction::ArrayCopy {
            array_type_index_dst: gc.standard_index(StdlibTypeId::String),
            array_type_index_src: gc.standard_index(StdlibTypeId::String),
        })
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(separator_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(strings_backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(string_index));
    emit_array_get(
        &mut function,
        strings_storage,
        Type::Standard(StdlibTypeId::String),
        gc,
    );
    function
        .instruction(&Instruction::LocalSet(current))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(byte_index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(byte_index))
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(byte_index))
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::ArraySet(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::LocalGet(byte_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(byte_index))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(string_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(string_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

/// Turns one nested structural value into an indented, comma-terminated entry.
pub(in crate::codegen::runtime_helpers) fn compile_wrap_debug_entry(
    indent: u32,
    join: u32,
    string_array: u32,
    string_storage: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([]);
    super::super::emit_string_literal(&mut function, "    ", gc);
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::Call(indent));
    super::super::emit_string_literal(&mut function, ",\n", gc);
    function
        .instruction(&Instruction::ArrayNewFixed {
            array_type_index: string_storage,
            array_size: 3,
        })
        .instruction(&Instruction::I32Const(3));
    super::super::array_value::emit_wrap_loaded(&mut function, string_array);
    function
        .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
        .instruction(&Instruction::Call(join))
        .instruction(&Instruction::End);
    function
}

pub(in crate::codegen::runtime_helpers) fn compile_wrap_debug_variant(
    wrap_entry: u32,
    join: u32,
    string_array: u32,
    string_storage: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([]);
    function.instruction(&Instruction::LocalGet(1));
    super::super::emit_string_literal(&mut function, "(\n", gc);
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::Call(wrap_entry));
    super::super::emit_string_literal(&mut function, ")", gc);
    function
        .instruction(&Instruction::ArrayNewFixed {
            array_type_index: string_storage,
            array_size: 4,
        })
        .instruction(&Instruction::I32Const(4));
    super::super::array_value::emit_wrap_loaded(&mut function, string_array);
    function
        .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
        .instruction(&Instruction::Call(join))
        .instruction(&Instruction::End);
    function
}

/// Indents every line after the first by four ASCII spaces. Derived structural
/// formatters prefix the first line themselves, then use this helper so nested
/// multiline values remain visually nested without flattening them.
pub(in crate::codegen::runtime_helpers) fn compile_indent_display(gc: &GcLayout) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let string_value = gc.val_type(Type::Standard(StdlibTypeId::String));
    let mut function = Function::new([(6, ValType::I32), (1, string_value)]);
    let input = 0;
    let length = 1;
    let input_index = 2;
    let newline_count = 3;
    let output_index = 4;
    let byte = 5;
    let spaces = 6;
    let output = 7;

    function
        .instruction(&Instruction::LocalGet(input))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(length))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(input))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::I32Const(b'\n' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(newline_count))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(newline_count))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(input_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::LocalGet(newline_count))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::I32Mul)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayNewDefault(string_type))
        .instruction(&Instruction::LocalSet(output))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(input_index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(input))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalTee(byte))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::ArraySet(string_type))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'\n' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(spaces))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(spaces))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(b' ' as i32))
        .instruction(&Instruction::ArraySet(string_type))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::LocalGet(spaces))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(spaces))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(input_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

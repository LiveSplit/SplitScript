//! Typed recursive managed slot readers. Only reachable decoder nodes are emitted.

use wasm_encoder::{BlockType, Function, Instruction as I, ValType};

use crate::{
    ast::ResultTypeId,
    capabilities::CapabilityAnalysis,
    intrinsic_registry::RuntimeHelperId as H,
    managed_read::ManagedDecoder,
    memory::MemoryAddressWidth,
    stdlib::StdlibTypeId,
    types::{TypeId, TypeKind},
};

use super::{
    MemoryByteOrder, Type, context::EmissionContext, emit_default, emit_result_error,
    emit_result_success, emit_typed_struct_get, managed_snapshots::result_for, memarg,
    semantic_type,
};

// Parameters: process, slot/object address, target pointer width, shared
// context, and whether the address has already been dereferenced.
const CONTEXT: u32 = 3;

pub(super) fn compile(
    value: TypeId,
    capabilities: &CapabilityAnalysis,
    lowering: &EmissionContext<'_>,
) -> Function {
    let node = capabilities.managed_decoder(value).unwrap();
    let result = result_for(value, lowering);
    let mut reader = Reader {
        value,
        result,
        lowering,
        entered: None,
    };
    let mut locals = Vec::new();
    match node {
        ManagedDecoder::String => locals.push((
            1,
            nullable(lowering.gc.val_type(Type::Standard(StdlibTypeId::String))),
        )),
        ManagedDecoder::Optional { value } => locals.push((
            1,
            nullable(
                lowering
                    .gc
                    .val_type(Type::Result(result_for(value, lowering))),
            ),
        )),
        ManagedDecoder::Array { element } => {
            let TypeKind::Array { layout, .. } = lowering.semantics.types().kind(value) else {
                unreachable!()
            };
            let storage =
                super::array_value::storage_id(*layout, lowering.arrays, lowering.semantics);
            locals.extend([(1, ValType::I64), (3, ValType::I32)]);
            locals.push((
                1,
                nullable(lowering.gc.val_type(Type::ArrayStorage(storage))),
            ));
            locals.push((
                1,
                nullable(
                    lowering
                        .gc
                        .val_type(Type::Result(result_for(element, lowering))),
                ),
            ));
            reader.entered = Some(8);
        }
        _ => {}
    }
    let mut f = Function::new(locals);
    if !matches!(node, ManagedDecoder::Memory) {
        f.instruction(&I::LocalGet(4))
            .instruction(&I::I32Eqz)
            .instruction(&I::If(BlockType::Empty))
            .instruction(&I::LocalGet(0))
            .instruction(&I::LocalGet(1))
            .instruction(&I::I32Const(lowering.abi_read.destination(8)))
            .instruction(&I::LocalGet(2))
            .instruction(&I::LocalGet(2))
            .instruction(&I::Call(
                lowering.runtime_helpers.function(H::ReadManagedMemory),
            ))
            .instruction(&I::I32Eqz);
        reader.fail_if(&mut f, "managed element reference could not be read");
        read_word(&mut f, lowering, false);
        f.instruction(&I::LocalSet(1)).instruction(&I::End);
    }
    match node {
        ManagedDecoder::Memory => {
            let (bytes, elements) = crate::managed_read::inline_materialization_cost(
                value,
                lowering.memory,
                lowering.semantics,
            );
            for (cost, helper) in [
                (bytes, H::ChargeManagedBytes),
                (elements, H::ChargeManagedElements),
            ] {
                if cost != 0 {
                    f.instruction(&I::LocalGet(CONTEXT))
                        .instruction(&I::I64Const(cost))
                        .instruction(&I::Call(lowering.runtime_helpers.function(helper)))
                        .instruction(&I::I32Eqz);
                    reader.fail_if(
                        &mut f,
                        "managed inline element exceeds the shared materialization budget",
                    );
                }
            }
            f.instruction(&I::LocalGet(0)).instruction(&I::LocalGet(1));
            super::emit_native_memory_read_destination_and_size(
                &mut f,
                value,
                lowering.abi_read,
                lowering.memory,
                lowering.semantics,
                lowering.runtime_globals.process_pointer_size,
            );
            f.instruction(&I::LocalGet(2))
                .instruction(&I::Call(
                    lowering.runtime_helpers.function(H::ReadManagedMemory),
                ))
                .instruction(&I::I32Eqz);
            reader.fail_if(&mut f, "managed inline element could not be read");
            super::emit_native_memory_value_result(
                &mut f,
                value,
                result,
                "managed inline element is invalid",
                lowering.abi_read,
                0,
                lowering.memory,
                lowering.semantics,
                lowering.gc,
                lowering.failure_payloads,
                lowering.runtime_globals.process_pointer_size,
                MemoryByteOrder::Little,
            );
        }
        ManagedDecoder::String => {
            arguments(&mut f);
            f.instruction(&I::Call(
                lowering.runtime_helpers.function(H::ReadManagedString),
            ))
            .instruction(&I::LocalSet(5))
            .instruction(&I::I32Eqz);
            reader.fail_if(
                &mut f,
                "managed string payload is invalid, unreadable, or exceeds the read budget",
            );
            f.instruction(&I::LocalGet(5));
            emit_result_success(&mut f, result, lowering.gc);
        }
        ManagedDecoder::Class { class } => {
            f.instruction(&I::LocalGet(1))
                .instruction(&I::LocalGet(CONTEXT))
                .instruction(&I::Call(lowering.managed_snapshot_functions[&class]));
        }
        ManagedDecoder::Optional { value: child } => {
            let TypeKind::Option { layout, .. } = lowering.semantics.types().kind(value) else {
                unreachable!()
            };
            f.instruction(&I::LocalGet(1))
                .instruction(&I::I64Eqz)
                .instruction(&I::If(BlockType::Empty));
            emit_default(&mut f, Type::Option(*layout), lowering.gc);
            emit_result_success(&mut f, result, lowering.gc);
            f.instruction(&I::Return).instruction(&I::End);
            arguments(&mut f);
            f.instruction(&I::I32Const(1))
                .instruction(&I::Call(lowering.managed_decoder_functions[&child]))
                .instruction(&I::LocalSet(5));
            reader.forward_failure(&mut f, child, 5);
            reader.child_field(
                &mut f,
                child,
                5,
                0,
                semantic_type(child, lowering.semantics),
            );
            f.instruction(&I::StructNew(lowering.gc.index(Type::Option(*layout))));
            emit_result_success(&mut f, result, lowering.gc);
        }
        ManagedDecoder::Array { element } => reader.array(&mut f, element, capabilities),
    }
    f.instruction(&I::End);
    f
}

struct Reader<'a, 'b> {
    value: TypeId,
    result: ResultTypeId,
    lowering: &'a EmissionContext<'b>,
    entered: Option<u32>,
}

impl Reader<'_, '_> {
    fn leave(&self, f: &mut Function) {
        if let Some(entered) = self.entered {
            let context = self
                .lowering
                .gc
                .standard_index(StdlibTypeId::ManagedReadContext);
            f.instruction(&I::LocalGet(entered))
                .instruction(&I::If(BlockType::Empty))
                .instruction(&I::LocalGet(CONTEXT))
                .instruction(&I::I32Const(0))
                .instruction(&I::LocalGet(CONTEXT))
                .instruction(&I::I32Const(0))
                .instruction(&I::ArrayGet(context))
                .instruction(&I::I64Const(1))
                .instruction(&I::I64Sub)
                .instruction(&I::ArraySet(context))
                .instruction(&I::End);
        }
    }

    fn fail_if(&self, f: &mut Function, message: &str) {
        f.instruction(&I::If(BlockType::Empty));
        self.leave(f);
        emit_result_error(
            f,
            self.result,
            semantic_type(self.value, self.lowering.semantics),
            message,
            self.lowering.gc,
            self.lowering.failure_payloads,
        );
        f.instruction(&I::Return).instruction(&I::End);
    }

    fn child_field(&self, f: &mut Function, child: TypeId, local: u32, field: u32, ty: Type) {
        f.instruction(&I::LocalGet(local))
            .instruction(&I::RefAsNonNull);
        emit_typed_struct_get(
            f,
            self.lowering
                .gc
                .index(Type::Result(result_for(child, self.lowering))),
            field,
            ty,
        );
    }

    fn forward_failure(&self, f: &mut Function, child: TypeId, local: u32) {
        self.child_field(f, child, local, 1, Type::I32);
        f.instruction(&I::If(BlockType::Empty));
        self.leave(f);
        emit_default(
            f,
            semantic_type(self.value, self.lowering.semantics),
            self.lowering.gc,
        );
        f.instruction(&I::I32Const(1));
        self.child_field(f, child, local, 2, Type::Standard(StdlibTypeId::String));
        f.instruction(&I::StructNew(
            self.lowering.gc.index(Type::Result(self.result)),
        ))
        .instruction(&I::Return)
        .instruction(&I::End);
    }

    fn array(&self, f: &mut Function, element: TypeId, capabilities: &CapabilityAnalysis) {
        let l = self.lowering;
        let TypeKind::Array { layout, .. } = l.semantics.types().kind(self.value) else {
            unreachable!()
        };
        let storage = super::array_value::storage_id(*layout, l.arrays, l.semantics);
        let storage_type = l.gc.index(Type::ArrayStorage(storage));
        let strides = if matches!(
            capabilities.managed_decoder(element),
            Some(ManagedDecoder::Memory)
        ) {
            [MemoryAddressWidth::Bit32, MemoryAddressWidth::Bit64]
                .map(|width| l.memory.layout(element, l.semantics, width).unwrap().size() as i64)
        } else {
            [4, 8]
        };
        let stride = |f: &mut Function| {
            if strides[0] == strides[1] {
                f.instruction(&I::I64Const(strides[0]));
            } else {
                f.instruction(&I::LocalGet(2))
                    .instruction(&I::I32Const(4))
                    .instruction(&I::I32Eq)
                    .instruction(&I::If(BlockType::Result(ValType::I64)))
                    .instruction(&I::I64Const(strides[0]))
                    .instruction(&I::Else)
                    .instruction(&I::I64Const(strides[1]))
                    .instruction(&I::End);
            }
        };
        f.instruction(&I::LocalGet(CONTEXT))
            .instruction(&I::LocalGet(1))
            .instruction(&I::Call(l.runtime_helpers.function(H::EnterManagedObject)))
            .instruction(&I::LocalTee(8))
            .instruction(&I::I32Eqz);
        self.fail_if(
            f,
            "managed array encountered a null object, cycle, or object/depth limit",
        );
        super::runtime_helpers::process::emit_invalid_managed_span(f, 1, 2, |f| {
            f.instruction(&I::LocalGet(2))
                .instruction(&I::I64ExtendI32U)
                .instruction(&I::I64Const(4))
                .instruction(&I::I64Mul);
        });
        self.fail_if(f, "managed array header exceeds the target address space");
        f.instruction(&I::LocalGet(0))
            .instruction(&I::LocalGet(1))
            .instruction(&I::LocalGet(2))
            .instruction(&I::I64ExtendI32U)
            .instruction(&I::I64Const(2))
            .instruction(&I::I64Mul)
            .instruction(&I::I64Add)
            .instruction(&I::I32Const(l.abi_read.destination(16)))
            .instruction(&I::LocalGet(2))
            .instruction(&I::I32Const(2))
            .instruction(&I::I32Mul)
            .instruction(&I::LocalGet(2))
            .instruction(&I::Call(l.runtime_helpers.function(H::ReadManagedMemory)))
            .instruction(&I::I32Eqz);
        self.fail_if(f, "managed array header could not be read");
        read_word(f, l, false);
        f.instruction(&I::I64Const(0)).instruction(&I::I64Ne);
        self.fail_if(
            f,
            "managed array requires a zero-based vector representation",
        );
        read_word(f, l, true);
        f.instruction(&I::LocalSet(5))
            .instruction(&I::LocalGet(CONTEXT))
            .instruction(&I::LocalGet(5))
            .instruction(&I::Call(
                l.runtime_helpers.function(H::ChargeManagedElements),
            ))
            .instruction(&I::I32Eqz);
        self.fail_if(f, "managed array exceeds the shared element budget");
        super::runtime_helpers::process::emit_invalid_managed_span(f, 1, 2, |f| {
            f.instruction(&I::LocalGet(2))
                .instruction(&I::I64ExtendI32U)
                .instruction(&I::I64Const(4))
                .instruction(&I::I64Mul)
                .instruction(&I::LocalGet(5));
            stride(f);
            f.instruction(&I::I64Mul).instruction(&I::I64Add);
        });
        self.fail_if(f, "managed array payload exceeds the target address space");
        f.instruction(&I::LocalGet(CONTEXT))
            .instruction(&I::LocalGet(5));
        stride(f);
        f.instruction(&I::I64Const(8))
            .instruction(&I::I64Add)
            .instruction(&I::I64Mul)
            .instruction(&I::Call(l.runtime_helpers.function(H::ChargeManagedBytes)))
            .instruction(&I::I32Eqz);
        self.fail_if(f, "managed array exceeds the shared byte budget");
        f.instruction(&I::LocalGet(5))
            .instruction(&I::I32WrapI64)
            .instruction(&I::LocalTee(6))
            .instruction(&I::ArrayNewDefault(storage_type))
            .instruction(&I::LocalSet(9))
            .instruction(&I::Block(BlockType::Empty))
            .instruction(&I::Loop(BlockType::Empty))
            .instruction(&I::LocalGet(7))
            .instruction(&I::LocalGet(6))
            .instruction(&I::I32GeU)
            .instruction(&I::BrIf(1))
            .instruction(&I::LocalGet(0))
            .instruction(&I::LocalGet(1))
            .instruction(&I::LocalGet(2))
            .instruction(&I::I64ExtendI32U)
            .instruction(&I::I64Const(4))
            .instruction(&I::I64Mul)
            .instruction(&I::I64Add)
            .instruction(&I::LocalGet(7))
            .instruction(&I::I64ExtendI32U);
        stride(f);
        f.instruction(&I::I64Mul)
            .instruction(&I::I64Add)
            .instruction(&I::LocalGet(2))
            .instruction(&I::LocalGet(CONTEXT))
            .instruction(&I::I32Const(0))
            .instruction(&I::Call(l.managed_decoder_functions[&element]))
            .instruction(&I::LocalSet(10));
        self.forward_failure(f, element, 10);
        f.instruction(&I::LocalGet(9))
            .instruction(&I::RefAsNonNull)
            .instruction(&I::LocalGet(7));
        self.child_field(f, element, 10, 0, semantic_type(element, l.semantics));
        f.instruction(&I::ArraySet(storage_type))
            .instruction(&I::LocalGet(7))
            .instruction(&I::I32Const(1))
            .instruction(&I::I32Add)
            .instruction(&I::LocalSet(7))
            .instruction(&I::Br(0))
            .instruction(&I::End)
            .instruction(&I::End);
        self.leave(f);
        f.instruction(&I::LocalGet(9)).instruction(&I::LocalGet(6));
        super::array_value::emit_wrap_loaded(f, l.gc.index(Type::Array(*layout)));
        emit_result_success(f, self.result, l.gc);
    }
}

fn nullable(mut ty: ValType) -> ValType {
    if let ValType::Ref(reference) = &mut ty {
        reference.nullable = true;
    }
    ty
}

fn arguments(f: &mut Function) {
    for local in 0..4 {
        f.instruction(&I::LocalGet(local));
    }
}

fn read_word(f: &mut Function, l: &EmissionContext<'_>, next: bool) {
    f.instruction(&I::LocalGet(2))
        .instruction(&I::I32Const(4))
        .instruction(&I::I32Eq)
        .instruction(&I::If(BlockType::Result(ValType::I64)));
    for wide in [false, true] {
        f.instruction(&I::I32Const(l.abi_read.start()));
        if next {
            f.instruction(&I::LocalGet(2)).instruction(&I::I32Add);
        }
        if wide {
            f.instruction(&I::I64Load(memarg()));
        } else {
            f.instruction(&I::I32Load(memarg()))
                .instruction(&I::I64ExtendI32U)
                .instruction(&I::Else);
        }
    }
    f.instruction(&I::End);
}

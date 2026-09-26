//! Typed recursive managed slot readers. Only reachable decoder nodes are emitted.

use wasm_encoder::{BlockType, Function, Instruction as I, ValType};

use crate::{
    ast::ResultTypeId,
    capabilities::CapabilityAnalysis,
    intrinsic_registry::RuntimeHelperId as H,
    managed_read::ManagedDecoderKind,
    memory::MemoryAddressWidth,
    stdlib::{CoreTypeId, StdlibTypeId},
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

pub(super) mod contracts;
mod keyed;

pub(super) fn compile(
    source: TypeId,
    capabilities: &CapabilityAnalysis,
    lowering: &EmissionContext<'_>,
) -> Function {
    let plan = capabilities.managed_decoder(source).unwrap();
    if let ManagedDecoderKind::Map { key, value } = plan.kind {
        return keyed::compile(source, Some(key), value, lowering, capabilities);
    }
    if let ManagedDecoderKind::Set { element } = plan.kind {
        return keyed::compile(source, None, element, lowering, capabilities);
    }
    let value = plan.output;
    let node = plan.kind;
    let result = result_for(value, lowering);
    let mut reader = Reader {
        value,
        result,
        lowering,
        capabilities,
        entered: None,
        list: matches!(node, ManagedDecoderKind::List { .. }),
    };
    let mut locals = Vec::new();
    match node {
        ManagedDecoderKind::Memory if lowering.managed_freezers.contains_key(&value) => {
            locals.push((1, nullable(lowering.gc.val_type(Type::Result(result)))))
        }
        ManagedDecoderKind::String => locals.push((
            1,
            nullable(lowering.gc.val_type(Type::Standard(StdlibTypeId::String))),
        )),
        ManagedDecoderKind::Optional { value } => locals.push((
            1,
            nullable(
                lowering
                    .gc
                    .val_type(Type::Result(result_for(reader.output(value), lowering))),
            ),
        )),
        ManagedDecoderKind::Array { element } | ManagedDecoderKind::List { element } => {
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
                        .val_type(Type::Result(result_for(reader.output(element), lowering))),
                ),
            ));
            reader.entered = Some(8);
            if reader.list {
                let (_, _, callable) = binding(
                    lowering,
                    &reader.collection_binding(crate::stdlib::MANAGED_LIST_LAYOUT_FIELD, element),
                );
                locals.push((1, nullable(lowering.gc.val_type(Type::Callable(callable)))));
                locals.push((
                    1,
                    nullable(
                        lowering
                            .gc
                            .val_type(Type::Result(list_layout_result(lowering))),
                    ),
                ));
                locals.push((3, ValType::I64));
            }
            {
                let (_, _, callable) = binding(
                    lowering,
                    &reader.collection_binding(crate::stdlib::MANAGED_ARRAY_TYPE_FIELD, element),
                );
                locals.push((1, nullable(lowering.gc.val_type(Type::Callable(callable)))));
                locals.push((
                    1,
                    nullable(
                        lowering
                            .gc
                            .val_type(Type::Result(array_type_result(lowering))),
                    ),
                ));
                locals.push((1, ValType::I64));
            }
        }
        _ => {}
    }
    let mut f = Function::new(locals);
    if !matches!(node, ManagedDecoderKind::Memory) {
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
        ManagedDecoderKind::Memory => {
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
            if let Some(freeze) = lowering.managed_freezers.get(&value) {
                f.instruction(&I::LocalSet(5));
                reader.child_field(&mut f, value, 5, 1, Type::I32);
                f.instruction(&I::I32Eqz)
                    .instruction(&I::If(BlockType::Empty));
                reader.child_field(
                    &mut f,
                    value,
                    5,
                    0,
                    semantic_type(value, lowering.semantics),
                );
                f.instruction(&I::Call(*freeze))
                    .instruction(&I::End)
                    .instruction(&I::LocalGet(5));
            }
        }
        ManagedDecoderKind::String => {
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
        ManagedDecoderKind::Class { class } => {
            f.instruction(&I::LocalGet(1))
                .instruction(&I::LocalGet(CONTEXT))
                .instruction(&I::Call(lowering.managed_snapshot_functions[&class]));
        }
        ManagedDecoderKind::Optional { value: child } => {
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
                semantic_type(reader.output(child), lowering.semantics),
            );
            f.instruction(&I::StructNew(lowering.gc.index(Type::Option(*layout))));
            emit_result_success(&mut f, result, lowering.gc);
        }
        ManagedDecoderKind::Map { .. } | ManagedDecoderKind::Set { .. } => unreachable!(),
        ManagedDecoderKind::Array { element } => reader.array(&mut f, element),
        ManagedDecoderKind::List { element } => {
            reader.list_header(&mut f, element);
            reader.array(&mut f, element);
        }
    }
    f.instruction(&I::End);
    f
}

struct Reader<'a, 'b> {
    value: TypeId,
    result: ResultTypeId,
    lowering: &'a EmissionContext<'b>,
    capabilities: &'a CapabilityAnalysis,
    entered: Option<u32>,
    list: bool,
}

impl Reader<'_, '_> {
    fn output(&self, source: TypeId) -> TypeId {
        self.capabilities.managed_decoder(source).unwrap().output
    }

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
                .instruction(&I::ArrayGet(context));
            if self.list {
                f.instruction(&I::LocalGet(entered))
                    .instruction(&I::I64ExtendI32U);
            } else {
                f.instruction(&I::I64Const(1));
            }
            f.instruction(&I::I64Sub)
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
        self.result_field(
            f,
            result_for(self.output(child), self.lowering),
            local,
            field,
            ty,
        );
    }

    fn result_field(
        &self,
        f: &mut Function,
        result: ResultTypeId,
        local: u32,
        field: u32,
        ty: Type,
    ) {
        f.instruction(&I::LocalGet(local));
        emit_typed_struct_get(f, self.lowering.gc.index(Type::Result(result)), field, ty);
    }

    fn forward_failure(&self, f: &mut Function, child: TypeId, local: u32) {
        self.forward_result_failure(f, result_for(self.output(child), self.lowering), local);
    }

    fn forward_failure_at(
        &self,
        f: &mut Function,
        child: TypeId,
        local: u32,
        index: u32,
        label: &str,
    ) {
        self.forward_result_failure_with_path(
            f,
            result_for(self.output(child), self.lowering),
            local,
            Some((index, label)),
        );
    }

    fn forward_result_failure(&self, f: &mut Function, result: ResultTypeId, local: u32) {
        self.forward_result_failure_with_path(f, result, local, None);
    }

    fn forward_result_failure_with_path(
        &self,
        f: &mut Function,
        result: ResultTypeId,
        local: u32,
        path: Option<(u32, &str)>,
    ) {
        self.result_field(f, result, local, 1, Type::I32);
        f.instruction(&I::If(BlockType::Empty));
        self.leave(f);
        emit_default(
            f,
            semantic_type(self.value, self.lowering.semantics),
            self.lowering.gc,
        );
        f.instruction(&I::I32Const(1));
        self.result_field(f, result, local, 2, Type::Standard(StdlibTypeId::String));
        if self.lowering.failure_payloads.is_demanded(self.result)
            && let Some((index, label)) = path
        {
            f.instruction(&I::LocalGet(index));
            super::emit_string_literal(f, label, self.lowering.gc);
            f.instruction(&I::Call(
                self.lowering.runtime_helpers.function(H::ManagedErrorIndex),
            ));
        }
        f.instruction(&I::StructNew(
            self.lowering.gc.index(Type::Result(self.result)),
        ))
        .instruction(&I::Return)
        .instruction(&I::End);
    }

    fn list_class(&self, f: &mut Function) {
        let l = self.lowering;
        self.result_field(
            f,
            list_layout_result(l),
            12,
            0,
            Type::Standard(StdlibTypeId::UnityListLayout),
        );
        f.instruction(&I::StructGet {
            struct_type_index: l.gc.standard_index(StdlibTypeId::UnityListLayout),
            field_index: l
                .gc
                .standard_field_index(crate::stdlib::StdlibFieldId::UnityListLayoutRuntimeClass),
        });
    }

    fn list_type(&self, f: &mut Function, element: TypeId, depth: u32, check_class: bool) {
        let l = self.lowering;
        let (structure, field, callable) = binding(
            l,
            &self.collection_binding(crate::stdlib::MANAGED_LIST_LAYOUT_FIELD, element),
        );
        let callable_type = l.gc.index(Type::Callable(callable));
        let result = list_layout_result(l);
        f.instruction(&I::GlobalGet(
            l.runtime_globals.provider_preparation_value.unwrap(),
        ))
        .instruction(&I::StructGet {
            struct_type_index: l.gc.index(Type::Struct(structure)),
            field_index: field,
        })
        .instruction(&I::LocalTee(11))
        .instruction(&I::StructGet {
            struct_type_index: callable_type,
            field_index: 1,
        })
        .instruction(&I::LocalGet(15))
        .instruction(&I::I32Const(depth as i32));
        storage_width(f, self, element);
        f.instruction(&I::I32Const(storage_kinds(self, element) as i32));
        if check_class {
            self.list_class(f);
        } else {
            f.instruction(&I::I64Const(0));
        }
        if crate::managed_read::needs_schema(element, self.capabilities) {
            contracts::emit_checker(f, element, l);
        } else if crate::managed_read::nominal_class(element, self.capabilities).is_some() {
            self.expected_class(f, element);
        }
        f.instruction(&I::LocalGet(CONTEXT))
            .instruction(&I::LocalGet(11))
            .instruction(&I::StructGet {
                struct_type_index: callable_type,
                field_index: 0,
            })
            .instruction(&I::CallRef(l.gc.callable_function_index(callable)))
            .instruction(&I::LocalSet(12));
        self.forward_result_failure(f, result, 12);
    }

    fn list_header(&self, f: &mut Function, element: TypeId) {
        let l = self.lowering;
        f.instruction(&I::LocalGet(1)).instruction(&I::LocalSet(15));
        f.instruction(&I::LocalGet(CONTEXT))
            .instruction(&I::LocalGet(1))
            .instruction(&I::Call(l.runtime_helpers.function(H::EnterManagedObject)))
            .instruction(&I::LocalTee(8))
            .instruction(&I::I32Eqz);
        self.fail_if(
            f,
            "managed list encountered a null object, cycle, or object/depth limit",
        );
        let (leaf, depth) = self.array_leaf(element);
        self.list_type(f, leaf, depth, false);
        for (field, count) in [
            (crate::stdlib::StdlibFieldId::UnityListLayoutSize, true),
            (crate::stdlib::StdlibFieldId::UnityListLayoutItems, false),
        ] {
            self.list_field(f, field, count);
            if count {
                f.instruction(&I::LocalTee(13))
                    .instruction(&I::I64Const(0))
                    .instruction(&I::I64LtS);
                self.fail_if(f, "managed list count is negative");
            } else {
                f.instruction(&I::LocalSet(1));
            }
        }
    }

    fn list_field(&self, f: &mut Function, field: crate::stdlib::StdlibFieldId, count: bool) {
        let l = self.lowering;
        self.result_field(
            f,
            list_layout_result(l),
            12,
            0,
            Type::Standard(StdlibTypeId::UnityListLayout),
        );
        f.instruction(&I::StructGet {
            struct_type_index: l.gc.standard_index(StdlibTypeId::UnityListLayout),
            field_index: l.gc.standard_field_index(field),
        })
        .instruction(&I::I64ExtendI32U)
        .instruction(&I::LocalSet(14));
        super::runtime_helpers::process::emit_invalid_managed_span(f, 15, 2, |f| {
            f.instruction(&I::LocalGet(14));
            if count {
                f.instruction(&I::I64Const(4));
            } else {
                f.instruction(&I::LocalGet(2))
                    .instruction(&I::I64ExtendI32U);
            }
            f.instruction(&I::I64Add);
        });
        self.fail_if(f, "managed list field exceeds the target address space");
        f.instruction(&I::LocalGet(0))
            .instruction(&I::LocalGet(15))
            .instruction(&I::LocalGet(14))
            .instruction(&I::I64Add)
            .instruction(&I::I32Const(l.abi_read.destination(8)));
        if count {
            f.instruction(&I::I32Const(4));
        } else {
            f.instruction(&I::LocalGet(2));
        }
        f.instruction(&I::LocalGet(2))
            .instruction(&I::Call(l.runtime_helpers.function(H::ReadManagedMemory)))
            .instruction(&I::I32Eqz);
        self.fail_if(f, "managed list field could not be read");
        if count {
            f.instruction(&I::I32Const(l.abi_read.start()))
                .instruction(&I::I32Load(memarg()))
                .instruction(&I::I64ExtendI32S);
        } else {
            read_word(f, l, false);
        }
    }

    fn verify_list_header(&self, f: &mut Function, element: TypeId) {
        for (field, count, original) in [
            (crate::stdlib::StdlibFieldId::UnityListLayoutSize, true, 13),
            (crate::stdlib::StdlibFieldId::UnityListLayoutItems, false, 1),
        ] {
            self.list_field(f, field, count);
            f.instruction(&I::LocalGet(original)).instruction(&I::I64Ne);
            self.fail_if(
                f,
                "managed list changed size or backing array during the read",
            );
        }
        let (leaf, depth) = self.array_leaf(element);
        self.list_type(f, leaf, depth, true);
    }

    fn array_type(&self, f: &mut Function, element: TypeId, depth: u32, remember: bool) {
        let l = self.lowering;
        let (callback_local, result_local, class_local) = if self.list {
            (16, 17, 18)
        } else {
            (11, 12, 13)
        };
        let (structure, field, callable) = binding(
            l,
            &self.collection_binding(crate::stdlib::MANAGED_ARRAY_TYPE_FIELD, element),
        );
        let callable_type = l.gc.index(Type::Callable(callable));
        f.instruction(&I::GlobalGet(
            l.runtime_globals.provider_preparation_value.unwrap(),
        ))
        .instruction(&I::StructGet {
            struct_type_index: l.gc.index(Type::Struct(structure)),
            field_index: field,
        })
        .instruction(&I::LocalTee(callback_local))
        .instruction(&I::StructGet {
            struct_type_index: callable_type,
            field_index: 1,
        })
        .instruction(&I::LocalGet(1))
        .instruction(&I::I32Const(depth as i32));
        storage_width(f, self, element);
        f.instruction(&I::I32Const(storage_kinds(self, element) as i32));
        if crate::managed_read::needs_schema(element, self.capabilities) {
            contracts::emit_checker(f, element, l);
        } else if crate::managed_read::nominal_class(element, self.capabilities).is_some() {
            self.expected_class(f, element);
        }
        f.instruction(&I::LocalGet(CONTEXT))
            .instruction(&I::LocalGet(callback_local))
            .instruction(&I::StructGet {
                struct_type_index: callable_type,
                field_index: 0,
            })
            .instruction(&I::CallRef(l.gc.callable_function_index(callable)))
            .instruction(&I::LocalSet(result_local));
        let result = array_type_result(l);
        self.forward_result_failure(f, result, result_local);
        self.result_field(f, result, result_local, 0, Type::Address);
        if remember {
            f.instruction(&I::LocalSet(class_local));
        } else {
            f.instruction(&I::LocalGet(class_local))
                .instruction(&I::I64Ne);
            self.fail_if(f, "managed array class changed during the read");
        }
    }

    fn collection_binding(&self, base: &str, element: TypeId) -> String {
        crate::managed_read::typed_collection_binding(base, &[element], self.capabilities)
    }

    fn expected_class(&self, f: &mut Function, element: TypeId) {
        let l = self.lowering;
        let Some(class) = crate::managed_read::nominal_class(element, self.capabilities) else {
            f.instruction(&I::I64Const(0));
            return;
        };
        let structure = l
            .structs
            .iter()
            .find(|s| s.name == crate::stdlib::PROVIDER_BINDINGS_TYPE)
            .unwrap();
        let name = crate::stdlib::managed_class_address_name(class.index());
        let field = structure
            .fields
            .iter()
            .position(|f| f.name == name)
            .unwrap() as u32;
        f.instruction(&I::GlobalGet(
            l.runtime_globals.provider_preparation_value.unwrap(),
        ))
        .instruction(&I::StructGet {
            struct_type_index: l.gc.index(Type::Struct(structure.id)),
            field_index: field,
        });
    }

    // Every entry before the leaf is a vector by construction. Checking the
    // full depth and leaf in one callback avoids caching a partially validated
    // nested contract and makes malformed metadata repairable on retry.
    fn array_leaf(&self, mut element: TypeId) -> (TypeId, u32) {
        let mut depth = 0;
        loop {
            while let ManagedDecoderKind::Optional { value } =
                self.capabilities.managed_decoder(element).unwrap().kind
            {
                element = value;
            }
            let ManagedDecoderKind::Array { element: child } =
                self.capabilities.managed_decoder(element).unwrap().kind
            else {
                return (element, depth);
            };
            element = child;
            depth += 1;
        }
    }

    fn array_types(&self, f: &mut Function, element: TypeId, remember: bool) {
        let (leaf, depth) = self.array_leaf(element);
        self.array_type(f, leaf, depth, remember);
    }

    fn array(&self, f: &mut Function, element: TypeId) {
        let l = self.lowering;
        let TypeKind::Array { layout, .. } = l.semantics.types().kind(self.value) else {
            unreachable!()
        };
        let storage = super::array_value::storage_id(*layout, l.arrays, l.semantics);
        let storage_type = l.gc.index(Type::ArrayStorage(storage));
        let strides = if matches!(
            self.capabilities
                .managed_decoder(element)
                .map(|plan| plan.kind),
            Some(ManagedDecoderKind::Memory)
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
        if self.list {
            // An empty list may have no allocated backing vector.
            f.instruction(&I::LocalGet(1))
                .instruction(&I::I64Eqz)
                .instruction(&I::If(BlockType::Empty))
                .instruction(&I::LocalGet(13))
                .instruction(&I::I64Eqz)
                .instruction(&I::I32Eqz);
            self.fail_if(f, "managed list has live elements without a backing array");
            self.verify_list_header(f, element);
            self.leave(f);
            f.instruction(&I::I32Const(0))
                .instruction(&I::ArrayNewDefault(storage_type))
                .instruction(&I::I32Const(0))
                .instruction(&I::I32Const(super::array_value::FROZEN_VERSION))
                .instruction(&I::StructNew(l.gc.index(Type::Array(*layout))));
            emit_result_success(f, self.result, l.gc);
            f.instruction(&I::Return).instruction(&I::End);
        }
        f.instruction(&I::LocalGet(CONTEXT))
            .instruction(&I::LocalGet(1))
            .instruction(&I::Call(l.runtime_helpers.function(H::EnterManagedObject)));
        if !self.list {
            f.instruction(&I::LocalTee(8));
        }
        f.instruction(&I::I32Eqz);
        self.fail_if(
            f,
            "managed array encountered a null object, cycle, or object/depth limit",
        );
        if self.list {
            f.instruction(&I::I32Const(2)).instruction(&I::LocalSet(8));
        }
        self.array_types(f, element, true);
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
        f.instruction(&I::LocalSet(5));
        if self.list {
            f.instruction(&I::LocalGet(13))
                .instruction(&I::LocalGet(5))
                .instruction(&I::I64GtU);
            self.fail_if(f, "managed list count exceeds its backing array capacity");
            if strides != [0, 0] {
                f.instruction(&I::LocalGet(5))
                    .instruction(&I::I64Const(u32::MAX as i64))
                    .instruction(&I::I64Const(-1))
                    .instruction(&I::LocalGet(2))
                    .instruction(&I::I32Const(4))
                    .instruction(&I::I32Eq)
                    .instruction(&I::Select)
                    .instruction(&I::LocalGet(2))
                    .instruction(&I::I64ExtendI32U)
                    .instruction(&I::I64Const(4))
                    .instruction(&I::I64Mul)
                    .instruction(&I::I64Sub);
                stride(f);
                f.instruction(&I::I64DivU).instruction(&I::I64GtU);
                self.fail_if(
                    f,
                    "managed list backing array capacity overflows its storage span",
                );
            }
        }
        f.instruction(&I::LocalGet(CONTEXT))
            .instruction(&I::LocalGet(if self.list { 13 } else { 5 }))
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
        if self.list {
            f.instruction(&I::LocalGet(13)).instruction(&I::LocalSet(5));
        }
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
        self.forward_failure_at(f, element, 10, 7, "");
        f.instruction(&I::LocalGet(9))
            .instruction(&I::RefAsNonNull)
            .instruction(&I::LocalGet(7));
        self.child_field(
            f,
            element,
            10,
            0,
            semantic_type(self.output(element), l.semantics),
        );
        f.instruction(&I::ArraySet(storage_type))
            .instruction(&I::LocalGet(7))
            .instruction(&I::I32Const(1))
            .instruction(&I::I32Add)
            .instruction(&I::LocalSet(7))
            .instruction(&I::Br(0))
            .instruction(&I::End)
            .instruction(&I::End);
        if self.list {
            self.verify_list_header(f, element);
        }
        self.array_types(f, element, false);
        self.leave(f);
        f.instruction(&I::LocalGet(9)).instruction(&I::LocalGet(6));
        f.instruction(&I::I32Const(super::array_value::FROZEN_VERSION))
            .instruction(&I::StructNew(l.gc.index(Type::Array(*layout))));
        emit_result_success(f, self.result, l.gc);
    }
}

fn nullable(mut ty: ValType) -> ValType {
    if let ValType::Ref(reference) = &mut ty {
        reference.nullable = true;
    }
    ty
}

fn list_layout_result(l: &EmissionContext<'_>) -> ResultTypeId {
    result_for(
        l.semantics
            .types()
            .id_for_standard(StdlibTypeId::UnityListLayout),
        l,
    )
}

fn array_type_result(l: &EmissionContext<'_>) -> ResultTypeId {
    result_for(l.semantics.types().id_for_core(CoreTypeId::Address), l)
}

pub(super) fn binding(
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
        .find(|(_, field)| field.name == name)
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

fn storage_width(f: &mut Function, r: &Reader<'_, '_>, source: TypeId) {
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

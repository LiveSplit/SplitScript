/// Emits one fallible state-field polling body. Pointer-backed fields
/// perform host reads and expression-backed fields execute their Wasm-IR plan.
pub(super) fn compile_read(
    field: &StateField,
    function_index: u32,
    shared_prefix: Option<super::pointer_prefixes::FieldPrefix>,
    abi: &Abi,
    strings: &StringPool,
    lowering: &EmissionContext<'_>,
) -> Function {
    let has_dependencies = !lowering.semantics.state_dependencies(field.id).is_empty();
    let candidate_parameter = has_dependencies.then_some(1);
    let StateSource::Pointer(path) = &field.source else {
        let StateSource::Expression(_) = &field.source else {
            unreachable!();
        };
        let mut matches = MatchLayout::default();
        let mut local_types = Vec::new();
        let planned = lowering
            .wasm_ir
            .state_expression(field.id)
            .expect("expression-backed state fields have Wasm IR plans");
        let mut planned_locals = HashMap::new();
        plan_wasm_locals(
            &planned.locals,
            &mut planned_locals,
            &mut matches,
            &mut local_types,
            LocalPlanOptions {
                parameter_count: 1 + u32::from(has_dependencies),
                semantics: lowering.semantics,
                wasm_ir: lowering.wasm_ir,
                gc: lowering.gc,
                reachability: lowering.reachability,
                instance: None,
                include_values: true,
            },
        );
        let mut function = Function::new_with_locals_types(local_types);
        let locals = planned_locals;
        let context = ExprContext {
            standard_library: lowering.standard_library,
            reachability: lowering.reachability,
            failure_payloads: lowering.failure_payloads,
            abi: lowering.abi,
            locals: LocalStorage::Wasm {
                values: &locals,
                temporaries: &matches.temporaries,
            },
            globals: lowering.globals,
            global_types: lowering.global_types,
            settings: lowering.settings,
            runtime_globals: lowering.runtime_globals,
            provider_values: lowering.provider_values,
            process_names: lowering.process_names,
            state_candidate: candidate_parameter,
            managed_read_context: None,
            runtime_helpers: lowering.runtime_helpers,
            functions: lowering.functions,
            closures: lowering.closures,
            function_values: lowering.function_values,
            closure_resumes: lowering.closure_resumes,
            closure_environment: None,
            leaf_futures: lowering.leaf_futures,
            display_functions: lowering.display_functions,
            equality_functions: lowering.equality_functions,
            array_functions: lowering.array_functions,
            set_functions: lowering.set_functions,
            structs: lowering.structs,
            managed: lowering.managed,
            managed_state_reads: lowering.managed_state_reads,
            managed_state_read_functions: lowering.managed_state_read_functions,
            managed_snapshot_functions: lowering.managed_snapshot_functions,
            managed_decoder_functions: lowering.managed_decoder_functions,
            managed_reference_functions: lowering.managed_reference_functions,
            enums: lowering.enums,
            arrays: lowering.arrays,
            memory: lowering.memory,
            abi_read: lowering.abi_read,
            runtime_scratch: lowering.runtime_scratch,
            signatures: lowering.signatures,
            matches: &matches,
            semantics: lowering.semantics,
            wasm_ir: lowering.wasm_ir,
            gc: lowering.gc,
            async_frames: lowering.async_frames,
            exact_runtime: lowering.exact_runtime,
            intrinsic_capture: None,
            debug: lowering.debug_emission(function_index),
            function_instance: None,
            loop_control: None,
            bare_return: BareReturn::None,
            materialize_none: true,
        };
        compile_block(&mut function, &planned.entry, &context, None);
        function.instruction(&Instruction::End);
        return function;
    };
    let field_type_id = lowering
        .semantics
        .value_type(field.id)
        .expect("checked state fields have semantic types");
    let field_type = value_type(field.id, lowering.semantics);
    let (memory_type_id, optional) = match lowering.semantics.types().kind(field_type_id) {
        crate::types::TypeKind::Option { layout, value } => (*value, Some(*layout)),
        _ => (field_type_id, None),
    };
    let poll_result = semantic_type(
        lowering
            .semantics
            .state_poll_result(field.id)
            .expect("checked state fields have poll-result types"),
        lowering.semantics,
    );
    let Type::Result(result_type) = poll_result else {
        unreachable!("state poll-result types are Result layouts")
    };
    if let Some(provider_id) = lowering.semantics.state_field_provider(field.id) {
        let provider = provider_id;
        let provider = lowering.standard_library.state_provider(provider);
        let Implementation::Intrinsic(direct_read) = lowering
            .standard_library
            .item(provider.direct_read)
            .implementation
        else {
            unreachable!("validated state-provider reads are intrinsic")
        };
        if let Some(contract) = crate::intrinsic_registry::provider_read_contract(direct_read) {
            let field_size = lowering
                .memory
                .layout(memory_type_id, lowering.semantics, contract.address_width)
                .expect("provider pointer fields are MemoryReadable")
                .size();
            return compile_provider_direct_read(
                &path.base,
                candidate_parameter,
                memory_type_id,
                field_type,
                optional,
                field_size,
                result_type,
                contract,
                provider_id,
                &path.offsets,
                lowering,
            );
        }
        debug_assert_eq!(direct_read, IntrinsicId::ProcessRead);
    }
    let decoded_string = path.decoder.is_some();
    let parameter_count =
        1 + u32::from(has_dependencies) + if shared_prefix.is_some() { 2 } else { 0 };
    let mut locals = vec![(1, ValType::I64)];
    if decoded_string {
        locals.push((
            1,
            lowering.gc.val_type(Type::Standard(StdlibTypeId::String)),
        ));
    }
    let mut function = Function::new(locals);
    let address_local = parameter_count;
    let offsets = &path.offsets;
    if let Some(prefix) = shared_prefix {
        let prefix_address = 1 + u32::from(has_dependencies);
        let prefix_status = prefix_address + 1;
        function
            .instruction(&Instruction::LocalGet(prefix_status))
            .instruction(&Instruction::I32Const(
                super::pointer_prefixes::PREFIX_RESOLVED,
            ))
            .instruction(&Instruction::I32Ne)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::LocalGet(prefix_status))
            .instruction(&Instruction::I32Const(
                super::pointer_prefixes::PREFIX_MODULE_MISSING,
            ))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Result(
                lowering.gc.val_type(Type::Result(result_type)),
            )));
        emit_pointer_read_failure(
            &mut function,
            result_type,
            field_type,
            optional,
            "process module was not found",
            lowering.gc,
            lowering.failure_payloads,
        );
        function.instruction(&Instruction::Else);
        emit_pointer_read_failure(
            &mut function,
            result_type,
            field_type,
            optional,
            "process read failed",
            lowering.gc,
            lowering.failure_payloads,
        );
        function
            .instruction(&Instruction::End)
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::LocalGet(prefix_address))
            .instruction(&Instruction::I64Const(prefix.initial_offset))
            .instruction(&Instruction::I64Add)
            .instruction(&Instruction::LocalSet(address_local));
    } else if let crate::ast::PointerPathBase::Module { name, offset } = &path.base {
        let (ptr, len) = strings.get(name);
        function
            .instruction(&Instruction::LocalGet(0))
            .instruction(&Instruction::I32Const(ptr as i32))
            .instruction(&Instruction::I32Const(len as i32))
            .instruction(&Instruction::Call(
                abi.function(AbiImportId::ProcessGetModuleAddress),
            ))
            .instruction(&Instruction::LocalTee(address_local))
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_pointer_read_failure(
            &mut function,
            result_type,
            field_type,
            optional,
            "process module was not found",
            lowering.gc,
            lowering.failure_payloads,
        );
        function
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::LocalGet(address_local))
            .instruction(&Instruction::I64Const(*offset))
            .instruction(&Instruction::I64Add)
            .instruction(&Instruction::LocalSet(address_local));
    } else if let crate::ast::PointerPathBase::Absolute(address) = path.base {
        function
            .instruction(&Instruction::I64Const(address as i64))
            .instruction(&Instruction::LocalSet(address_local));
    } else if let crate::ast::PointerPathBase::Expression(expression) = &path.base {
        emit_dynamic_state_base(
            &mut function,
            expression,
            candidate_parameter.expect("dynamic state bases record a sibling dependency"),
            lowering,
        );
        function.instruction(&Instruction::LocalSet(address_local));
    }

    // Resolve the complete pointer chain before either decoding a string or
    // reading an ordinary MemoryReadable value. A decoder changes how the
    // final address is consumed, not how that address is discovered.
    let process_read = ProcessReadEmission {
        abi,
        address_local,
        fallback_ty: field_type,
        optional,
        result_type,
        gc: lowering.gc,
        failure_payloads: lowering.failure_payloads,
        abi_read: lowering.abi_read,
        memory: lowering.memory,
        semantics: lowering.semantics,
        process_pointer_size: lowering.runtime_globals.process_pointer_size,
        read_failure: "process read failed",
    };
    let offset_start = shared_prefix.map_or(0, |prefix| prefix.offset_start);
    for offset in &offsets[offset_start..] {
        let address_type = lowering
            .semantics
            .types()
            .id_for_core(crate::stdlib::CoreTypeId::Address);
        emit_process_read(&mut function, &process_read, address_type);
        emit_native_memory_value(
            &mut function,
            address_type,
            lowering.abi_read,
            0,
            lowering.memory,
            lowering.semantics,
            lowering.gc,
            lowering.runtime_globals.process_pointer_size,
            MemoryByteOrder::Little,
        );
        function
            .instruction(&Instruction::I64Const(*offset))
            .instruction(&Instruction::I64Add)
            .instruction(&Instruction::LocalSet(address_local));
    }

    if let Some(decoder) = path.decoder {
        let (maximum, helper, failure) = match decoder {
            crate::ast::StateMemoryDecoder::Utf8 { max_bytes, .. } => (
                max_bytes,
                RuntimeHelperId::ReadUtf8String,
                "UTF-8 string could not be read",
            ),
            crate::ast::StateMemoryDecoder::Utf16Le { max_units, .. } => (
                max_units,
                RuntimeHelperId::ReadUtf16LeString,
                "UTF-16LE string could not be read",
            ),
        };
        let string_local = address_local + 1;
        function
            .instruction(&Instruction::GlobalGet(lowering.runtime_globals.process))
            .instruction(&Instruction::LocalGet(address_local))
            .instruction(&Instruction::I32Const(maximum as i32))
            .instruction(&Instruction::Call(
                lowering.runtime_helpers.function(helper),
            ))
            .instruction(&Instruction::LocalTee(string_local))
            .instruction(&Instruction::RefIsNull)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_pointer_read_failure(
            &mut function,
            result_type,
            field_type,
            optional,
            failure,
            lowering.gc,
            lowering.failure_payloads,
        );
        function
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::LocalGet(string_local));
        emit_pointer_read_success(&mut function, result_type, optional, lowering.gc);
        function.instruction(&Instruction::End);
        return function;
    }

    emit_process_read(&mut function, &process_read, memory_type_id);
    emit_native_memory_validation_failure(
        &mut function,
        memory_type_id,
        result_type,
        field_type,
        optional,
        "process read failed",
        MemoryByteOrder::Little,
        lowering,
    );
    emit_native_memory_value(
        &mut function,
        memory_type_id,
        lowering.abi_read,
        0,
        lowering.memory,
        lowering.semantics,
        lowering.gc,
        lowering.runtime_globals.process_pointer_size,
        MemoryByteOrder::Little,
    );
    emit_pointer_read_success(&mut function, result_type, optional, lowering.gc);
    function.instruction(&Instruction::End);
    function
}

fn emit_dynamic_state_base(
    function: &mut Function,
    expression: &crate::ast::Expr,
    candidate_parameter: u32,
    lowering: &EmissionContext<'_>,
) {
    let values = HashMap::new();
    let temporaries = HashMap::new();
    let matches = MatchLayout::default();
    let mut context = ExprContext::compiler_generated(lowering, &values, &temporaries, &matches);
    context.state_candidate = Some(candidate_parameter);
    let root = lowering
        .semantics
        .value(expression.id)
        .expect("checked dynamic state bases have a resolved sibling root");
    debug_assert!(matches!(
        root,
        crate::semantic::ResolvedValue::StateCandidate(_)
    ));
    let members = lowering
        .semantics
        .path_members(expression.id)
        .unwrap_or(&[]);
    compile_resolved_path(function, root, members, &context);
}

/// Emits a fallible per-field candidate transform. Its parameter is the newly
/// read value and its result determines whether the field accepts that value.
pub(super) fn compile_state_transform(
    field: &StateField,
    function_index: u32,
    lowering: &EmissionContext<'_>,
) -> Function {
    let transform = field
        .transform
        .as_ref()
        .expect("only filtered fields have transform bodies");
    let planned = lowering
        .wasm_ir
        .state_transform(field.id)
        .expect("checked state transforms have Wasm IR plans");
    let field_type = value_type(field.id, lowering.semantics);
    let has_dependencies = !lowering.semantics.state_dependencies(field.id).is_empty();
    let mut matches = MatchLayout::default();
    let mut local_types = Vec::new();
    let mut planned_locals = HashMap::new();
    plan_wasm_locals(
        &planned.locals,
        &mut planned_locals,
        &mut matches,
        &mut local_types,
        LocalPlanOptions {
            parameter_count: 1 + u32::from(has_dependencies),
            semantics: lowering.semantics,
            wasm_ir: lowering.wasm_ir,
            gc: lowering.gc,
            reachability: lowering.reachability,
            instance: None,
            include_values: true,
        },
    );
    let mut function = Function::new_with_locals_types(local_types);
    let mut locals = planned_locals;
    locals.insert(transform.value, (0, field_type));
    let context = ExprContext {
        standard_library: lowering.standard_library,
        reachability: lowering.reachability,
        failure_payloads: lowering.failure_payloads,
        abi: lowering.abi,
        locals: LocalStorage::Wasm {
            values: &locals,
            temporaries: &matches.temporaries,
        },
        globals: lowering.globals,
        global_types: lowering.global_types,
        settings: lowering.settings,
        runtime_globals: lowering.runtime_globals,
        provider_values: lowering.provider_values,
        process_names: lowering.process_names,
        state_candidate: has_dependencies.then_some(1),
        managed_read_context: None,
        runtime_helpers: lowering.runtime_helpers,
        functions: lowering.functions,
        closures: lowering.closures,
        function_values: lowering.function_values,
        closure_resumes: lowering.closure_resumes,
        closure_environment: None,
        leaf_futures: lowering.leaf_futures,
        display_functions: lowering.display_functions,
        equality_functions: lowering.equality_functions,
        array_functions: lowering.array_functions,
        set_functions: lowering.set_functions,
        structs: lowering.structs,
        managed: lowering.managed,
        managed_state_reads: lowering.managed_state_reads,
        managed_state_read_functions: lowering.managed_state_read_functions,
        managed_snapshot_functions: lowering.managed_snapshot_functions,
        managed_decoder_functions: lowering.managed_decoder_functions,
        managed_reference_functions: lowering.managed_reference_functions,
        enums: lowering.enums,
        arrays: lowering.arrays,
        memory: lowering.memory,
        abi_read: lowering.abi_read,
        runtime_scratch: lowering.runtime_scratch,
        signatures: lowering.signatures,
        matches: &matches,
        semantics: lowering.semantics,
        wasm_ir: lowering.wasm_ir,
        gc: lowering.gc,
        async_frames: lowering.async_frames,
        exact_runtime: lowering.exact_runtime,
        intrinsic_capture: None,
        debug: lowering.debug_emission(function_index),
        function_instance: None,
        loop_control: None,
        bare_return: BareReturn::None,
        materialize_none: true,
    };
    compile_block(&mut function, &planned.entry, &context, None);
    function.instruction(&Instruction::End);
    function
}

#[allow(clippy::too_many_arguments)]
fn compile_provider_direct_read(
    base: &crate::ast::PointerPathBase,
    candidate_parameter: Option<u32>,
    memory_type_id: crate::types::TypeId,
    field_type: Type,
    optional: Option<crate::ast::OptionTypeId>,
    field_size: u32,
    result_type: ResultTypeId,
    contract: crate::intrinsic_registry::ProviderReadContract,
    provider: crate::stdlib::StdlibStateProviderId,
    offsets: &[i64],
    lowering: &EmissionContext<'_>,
) -> Function {
    let mut function = Function::new([(1, ValType::I64), (1, ValType::I32)]);
    let status_local = 1 + u32::from(candidate_parameter.is_some());
    let guest_address_local = status_local + 1;
    match base {
        crate::ast::PointerPathBase::Absolute(address) => {
            function.instruction(&Instruction::I32Const(*address as i32));
        }
        crate::ast::PointerPathBase::Module { .. } => {
            unreachable!("provider direct reads reject module-relative roots")
        }
        crate::ast::PointerPathBase::Expression(expression) => emit_dynamic_state_base(
            &mut function,
            expression,
            candidate_parameter.expect("dynamic state bases record a sibling dependency"),
            lowering,
        ),
    }
    function.instruction(&Instruction::LocalSet(guest_address_local));
    for offset in offsets {
        emit_provider_read(
            &mut function,
            guest_address_local,
            status_local,
            4,
            field_type,
            optional,
            result_type,
            contract,
            provider,
            lowering,
        );
        emit_memory_load(
            &mut function,
            Type::U32,
            lowering.abi_read.start(),
            contract.address_width,
            contract.byte_order.into(),
        );
        function
            .instruction(&Instruction::I32Const(*offset as i32))
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::LocalSet(guest_address_local));
    }
    emit_provider_read(
        &mut function,
        guest_address_local,
        status_local,
        field_size,
        field_type,
        optional,
        result_type,
        contract,
        provider,
        lowering,
    );
    emit_memory_validation_failure(
        &mut function,
        memory_type_id,
        result_type,
        field_type,
        optional,
        contract.read_failure,
        contract.address_width,
        contract.byte_order.into(),
        lowering,
    );
    emit_memory_value(
        &mut function,
        memory_type_id,
        lowering.abi_read,
        0,
        lowering.memory,
        lowering.semantics,
        lowering.gc,
        contract.address_width,
        contract.byte_order.into(),
    );
    emit_pointer_read_success(&mut function, result_type, optional, lowering.gc);
    function.instruction(&Instruction::End);
    function
}

#[allow(clippy::too_many_arguments)]
fn emit_provider_read(
    function: &mut Function,
    guest_address_local: u32,
    status_local: u32,
    size: u32,
    field_type: Type,
    optional: Option<crate::ast::OptionTypeId>,
    result_type: ResultTypeId,
    contract: crate::intrinsic_registry::ProviderReadContract,
    provider: crate::stdlib::StdlibStateProviderId,
    lowering: &EmissionContext<'_>,
) {
    function
        .instruction(&Instruction::GlobalGet(lowering.runtime_globals.process))
        .instruction(&Instruction::GlobalGet(
            lowering
                .provider_values
                .get(&provider)
                .copied()
                .or(lowering.runtime_globals.provider_value)
                .expect("provider direct reads require provider storage"),
        ))
        .instruction(&Instruction::LocalGet(guest_address_local))
        .instruction(&Instruction::I32Const(lowering.abi_read.destination(size)))
        .instruction(&Instruction::I32Const(size as i32))
        .instruction(&Instruction::Call(
            lowering.runtime_helpers.function(contract.reader),
        ))
        .instruction(&Instruction::LocalTee(status_local))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Ne)
        .instruction(&Instruction::If(BlockType::Empty));
    function
        .instruction(&Instruction::LocalGet(status_local))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Result(
            lowering.gc.val_type(Type::Result(result_type)),
        )));
    emit_pointer_read_failure(
        function,
        result_type,
        field_type,
        optional,
        contract.invalid_address,
        lowering.gc,
        lowering.failure_payloads,
    );
    function.instruction(&Instruction::Else);
    emit_pointer_read_failure(
        function,
        result_type,
        field_type,
        optional,
        contract.read_failure,
        lowering.gc,
        lowering.failure_payloads,
    );
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
}

struct ProcessReadEmission<'a> {
    abi: &'a Abi,
    address_local: u32,
    fallback_ty: Type,
    optional: Option<crate::ast::OptionTypeId>,
    result_type: ResultTypeId,
    gc: &'a GcLayout,
    failure_payloads: &'a super::failure_payload::FailurePayloadDemand,
    abi_read: memory_plan::AbiReadScratch,
    memory: &'a crate::memory::MemoryLayouts,
    semantics: &'a crate::semantic::SemanticModel,
    process_pointer_size: Option<u32>,
    read_failure: &'a str,
}

fn emit_process_read(
    function: &mut Function,
    emission: &ProcessReadEmission<'_>,
    ty: crate::types::TypeId,
) {
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::LocalGet(emission.address_local));
    emit_native_memory_read_destination_and_size(
        function,
        ty,
        emission.abi_read,
        emission.memory,
        emission.semantics,
        emission.process_pointer_size,
    );
    function
        .instruction(&Instruction::Call(
            emission.abi.function(AbiImportId::ProcessRead),
        ))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_pointer_read_failure(
        function,
        emission.result_type,
        emission.fallback_ty,
        emission.optional,
        emission.read_failure,
        emission.gc,
        emission.failure_payloads,
    );
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
}

fn emit_pointer_read_failure(
    function: &mut Function,
    result_type: ResultTypeId,
    field_type: Type,
    optional: Option<crate::ast::OptionTypeId>,
    message: &str,
    gc: &GcLayout,
    failure_payloads: &super::failure_payload::FailurePayloadDemand,
) {
    if optional.is_some() {
        function.instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE));
        emit_result_success(function, result_type, gc);
    } else {
        emit_result_error(
            function,
            result_type,
            field_type,
            message,
            gc,
            failure_payloads,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_memory_validation_failure(
    function: &mut Function,
    memory_type: crate::types::TypeId,
    result_type: ResultTypeId,
    field_type: Type,
    optional: Option<crate::ast::OptionTypeId>,
    message: &str,
    address_width: crate::memory::MemoryAddressWidth,
    byte_order: MemoryByteOrder,
    lowering: &EmissionContext<'_>,
) {
    if !emit_memory_value_is_valid(
        function,
        memory_type,
        lowering.abi_read,
        0,
        lowering.memory,
        lowering.semantics,
        address_width,
        byte_order,
    ) {
        return;
    }
    function
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_pointer_read_failure(
        function,
        result_type,
        field_type,
        optional,
        message,
        lowering.gc,
        lowering.failure_payloads,
    );
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
}

#[allow(clippy::too_many_arguments)]
fn emit_native_memory_validation_failure(
    function: &mut Function,
    memory_type: crate::types::TypeId,
    result_type: ResultTypeId,
    field_type: Type,
    optional: Option<crate::ast::OptionTypeId>,
    message: &str,
    byte_order: MemoryByteOrder,
    lowering: &EmissionContext<'_>,
) {
    if !emit_native_memory_value_is_valid(
        function,
        memory_type,
        lowering.abi_read,
        0,
        lowering.memory,
        lowering.semantics,
        lowering.runtime_globals.process_pointer_size,
        byte_order,
    ) {
        return;
    }
    function
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_pointer_read_failure(
        function,
        result_type,
        field_type,
        optional,
        message,
        lowering.gc,
        lowering.failure_payloads,
    );
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
}

fn emit_pointer_read_success(
    function: &mut Function,
    result_type: ResultTypeId,
    optional: Option<crate::ast::OptionTypeId>,
    gc: &GcLayout,
) {
    if let Some(option) = optional {
        function.instruction(&Instruction::StructNew(gc.index(Type::Option(option))));
    }
    emit_result_success(function, result_type, gc);
}

/// Materializes repeated immutable managed-snapshot roots once at entry to a
/// synchronous body. The Wasm-IR planner deliberately emits no such locals for
/// async bodies, where a suspension could observe a later state snapshot.
fn emit_snapshot_projection_prologue(
    function: &mut Function,
    planned: &[wasm_ir::Local],
    context: &ExprContext<'_>,
) {
    for local in planned {
        let LocalPurpose::SnapshotProjection(projection) = local.purpose else {
            continue;
        };
        let &(destination, projected_type) = context
            .matches
            .snapshot_projections
            .get(&projection)
            .expect("planned snapshot projections have physical locals");
        let root_projection = wasm_ir::SnapshotProjection {
            root: projection.root,
            field: projection.field,
            member: None,
        };
        let field_type = if projection.member.is_some()
            && let Some(&(root_local, root_type)) =
                context.matches.snapshot_projections.get(&root_projection)
        {
            function.instruction(&Instruction::LocalGet(root_local));
            root_type
        } else {
            function
                .instruction(&Instruction::GlobalGet(match projection.root {
                    wasm_ir::SnapshotRoot::Current => context.runtime_globals.current,
                    wasm_ir::SnapshotRoot::Old => context.runtime_globals.old,
                }))
                .instruction(&Instruction::RefAsNonNull);
            let (field_index, storage) = state_storage_index(projection.field, context.semantics);
            let field_type = value_type(storage, context.semantics);
            emit_struct_get(function, field_index, field_type);
            field_type
        };
        if let Some(member) = projection.member {
            let lowered_type = emit_path_fields(
                function,
                &[crate::semantic::ResolvedMember::ManagedField(member)],
                field_type,
                context,
            );
            debug_assert_eq!(projected_type, lowered_type);
        } else {
            debug_assert_eq!(projected_type, field_type);
        }
        function.instruction(&Instruction::LocalSet(destination));
    }
}

pub(super) fn compile_user_function(
    declaration: &FunctionDecl,
    instance: &crate::semantic::FunctionInstance,
    function_index: u32,
    lowering: &EmissionContext<'_>,
) -> Function {
    let wasm_body = lowering
        .wasm_ir
        .body(BodyOwner::Function(instance.clone()))
        .expect("checked functions have Wasm IR bodies");
    let mut locals = HashMap::new();
    let mut boxed_parameters = Vec::new();
    let mut physical_parameter_count = 0;
    for parameter in &declaration.params {
        let ty = semantic_type(
            lowering.semantics.specialize_type(
                instance,
                lowering
                    .semantics
                    .value_type(parameter.id)
                    .expect("checked parameters have types"),
            ),
            lowering.semantics,
        );
        let index = if !ty.has_runtime_value() {
            u32::MAX
        } else {
            let index = physical_parameter_count;
            physical_parameter_count += 1;
            index
        };
        locals.insert(parameter.id, (index, ty));
        if index != u32::MAX && lowering.wasm_ir.is_mutably_captured(parameter.id) {
            boxed_parameters.push((parameter.id, index, ty));
        }
    }
    let mut local_types = Vec::new();
    for (parameter, _, ty) in &boxed_parameters {
        let cell_local = physical_parameter_count + local_types.len() as u32;
        local_types.push(lowering.gc.capture_cell_val_type(*ty));
        locals.insert(*parameter, (cell_local, *ty));
    }
    let mut matches = MatchLayout::default();
    plan_wasm_locals(
        &wasm_body.locals,
        &mut locals,
        &mut matches,
        &mut local_types,
        LocalPlanOptions {
            parameter_count: physical_parameter_count,
            semantics: lowering.semantics,
            wasm_ir: lowering.wasm_ir,
            gc: lowering.gc,
            reachability: lowering.reachability,
            instance: Some(instance),
            include_values: true,
        },
    );
    if let Some(debug) = lowering.debug {
        for parameter in &declaration.params {
            let (local, ty) = locals[&parameter.id];
            debug.register_variable(function_index, parameter.id, local, ty, true);
        }
        for (&value, &(local, ty)) in &locals {
            if !declaration
                .params
                .iter()
                .any(|parameter| parameter.id == value)
            {
                debug.register_variable(function_index, value, local, ty, false);
            }
        }
    }
    let mut function = Function::new_with_locals_types(local_types);
    for (parameter, source_local, ty) in boxed_parameters {
        function
            .instruction(&Instruction::LocalGet(source_local))
            .instruction(&Instruction::StructNew(lowering.gc.capture_cell_index(ty)))
            .instruction(&Instruction::LocalSet(locals[&parameter].0));
    }
    let context = ExprContext {
        standard_library: lowering.standard_library,
        reachability: lowering.reachability,
        failure_payloads: lowering.failure_payloads,
        abi: lowering.abi,
        locals: LocalStorage::Wasm {
            values: &locals,
            temporaries: &matches.temporaries,
        },
        globals: lowering.globals,
        global_types: lowering.global_types,
        settings: lowering.settings,
        runtime_globals: lowering.runtime_globals,
        provider_values: lowering.provider_values,
        process_names: lowering.process_names,
        state_candidate: None,
        managed_read_context: None,
        runtime_helpers: lowering.runtime_helpers,
        functions: lowering.functions,
        closures: lowering.closures,
        function_values: lowering.function_values,
        closure_resumes: lowering.closure_resumes,
        closure_environment: None,
        leaf_futures: lowering.leaf_futures,
        display_functions: lowering.display_functions,
        equality_functions: lowering.equality_functions,
        array_functions: lowering.array_functions,
        set_functions: lowering.set_functions,
        structs: lowering.structs,
        managed: lowering.managed,
        managed_state_reads: lowering.managed_state_reads,
        managed_state_read_functions: lowering.managed_state_read_functions,
        managed_snapshot_functions: lowering.managed_snapshot_functions,
        managed_decoder_functions: lowering.managed_decoder_functions,
        managed_reference_functions: lowering.managed_reference_functions,
        enums: lowering.enums,
        arrays: lowering.arrays,
        memory: lowering.memory,
        abi_read: lowering.abi_read,
        runtime_scratch: lowering.runtime_scratch,
        signatures: lowering.signatures,
        matches: &matches,
        semantics: lowering.semantics,
        wasm_ir: lowering.wasm_ir,
        gc: lowering.gc,
        async_frames: lowering.async_frames,
        exact_runtime: lowering.exact_runtime,
        intrinsic_capture: None,
        debug: lowering.debug_emission(function_index),
        function_instance: Some(instance),
        loop_control: None,
        bare_return: BareReturn::None,
        materialize_none: true,
    };
    emit_snapshot_projection_prologue(&mut function, &wasm_body.locals, &context);
    compile_block(&mut function, &wasm_body.entry, &context, None);
    let result = semantic_type(
        lowering.semantics.specialize_type(
            instance,
            lowering
                .semantics
                .function_completion(declaration.id)
                .expect("checked functions have result types"),
        ),
        lowering.semantics,
    );
    if result.has_runtime_value() {
        function.instruction(&Instruction::Unreachable);
    }
    function.instruction(&Instruction::End);
    function
}

pub(super) fn compile_closure(
    instance: &crate::semantic::ClosureInstance,
    closure: &wasm_ir::ClosureBody,
    function_index: u32,
    lowering: &EmissionContext<'_>,
) -> Function {
    let expression = lowering
        .wasm_ir
        .expression(closure.expression)
        .expect("closure expressions belong to Wasm IR");
    let expression_ty = instance.owner.as_ref().map_or(expression.ty, |owner| {
        lowering.semantics.specialize_type(owner, expression.ty)
    });
    let crate::types::TypeKind::Callable {
        parameters, result, ..
    } = lowering.semantics.types().kind(expression_ty)
    else {
        unreachable!("checked closure expressions have callable types")
    };
    let mut locals = HashMap::new();
    let mut boxed_parameters = Vec::new();
    let mut physical_parameter_count = 1;
    for (parameter, ty) in closure.parameters.iter().zip(parameters) {
        let ty = semantic_type(*ty, lowering.semantics);
        let index = if ty.has_runtime_value() {
            let index = physical_parameter_count;
            physical_parameter_count += 1;
            index
        } else {
            u32::MAX
        };
        locals.insert(*parameter, (index, ty));
        if index != u32::MAX && lowering.wasm_ir.is_mutably_captured(*parameter) {
            boxed_parameters.push((*parameter, index, ty));
        }
    }
    let mut local_types = Vec::new();
    for (parameter, _, ty) in &boxed_parameters {
        let cell_local = physical_parameter_count + local_types.len() as u32;
        local_types.push(lowering.gc.capture_cell_val_type(*ty));
        locals.insert(*parameter, (cell_local, *ty));
    }
    let mut matches = MatchLayout::default();
    plan_wasm_locals(
        &closure.locals,
        &mut locals,
        &mut matches,
        &mut local_types,
        LocalPlanOptions {
            parameter_count: physical_parameter_count,
            semantics: lowering.semantics,
            wasm_ir: lowering.wasm_ir,
            gc: lowering.gc,
            reachability: lowering.reachability,
            instance: instance.owner.as_ref(),
            include_values: true,
        },
    );
    let captures = closure
        .captures
        .iter()
        .enumerate()
        .map(|(field, capture)| {
            (
                capture.value,
                (
                    field as u32,
                    instance.owner.as_ref().map_or_else(
                        || value_type(capture.value, lowering.semantics),
                        |owner| {
                            semantic_type(
                                lowering.semantics.specialize_type(
                                    owner,
                                    lowering
                                        .semantics
                                        .value_type(capture.value)
                                        .expect("checked captures have types"),
                                ),
                                lowering.semantics,
                            )
                        },
                    ),
                    capture.mutable,
                ),
            )
        })
        .collect::<HashMap<_, _>>();
    let environment = lowering
        .gc
        .closure_environment_index(instance)
        .map(|struct_type| ClosureEnvironment {
            local: 0,
            struct_type,
            captures: &captures,
        });
    if let Some(debug) = lowering.debug {
        for parameter in &closure.parameters {
            let (local, ty) = locals[parameter];
            debug.register_variable(function_index, *parameter, local, ty, true);
        }
        for (&value, &(local, ty)) in &locals {
            if !closure.parameters.contains(&value) {
                debug.register_variable(function_index, value, local, ty, false);
            }
        }
    }
    let mut function = Function::new_with_locals_types(local_types);
    for (parameter, source_local, ty) in boxed_parameters {
        function
            .instruction(&Instruction::LocalGet(source_local))
            .instruction(&Instruction::StructNew(lowering.gc.capture_cell_index(ty)))
            .instruction(&Instruction::LocalSet(locals[&parameter].0));
    }
    let context = ExprContext {
        standard_library: lowering.standard_library,
        reachability: lowering.reachability,
        failure_payloads: lowering.failure_payloads,
        abi: lowering.abi,
        locals: LocalStorage::Wasm {
            values: &locals,
            temporaries: &matches.temporaries,
        },
        globals: lowering.globals,
        global_types: lowering.global_types,
        settings: lowering.settings,
        runtime_globals: lowering.runtime_globals,
        provider_values: lowering.provider_values,
        process_names: lowering.process_names,
        state_candidate: None,
        managed_read_context: None,
        runtime_helpers: lowering.runtime_helpers,
        functions: lowering.functions,
        closures: lowering.closures,
        function_values: lowering.function_values,
        closure_resumes: lowering.closure_resumes,
        closure_environment: environment,
        leaf_futures: lowering.leaf_futures,
        display_functions: lowering.display_functions,
        equality_functions: lowering.equality_functions,
        array_functions: lowering.array_functions,
        set_functions: lowering.set_functions,
        structs: lowering.structs,
        managed: lowering.managed,
        managed_state_reads: lowering.managed_state_reads,
        managed_state_read_functions: lowering.managed_state_read_functions,
        managed_snapshot_functions: lowering.managed_snapshot_functions,
        managed_decoder_functions: lowering.managed_decoder_functions,
        managed_reference_functions: lowering.managed_reference_functions,
        enums: lowering.enums,
        arrays: lowering.arrays,
        memory: lowering.memory,
        abi_read: lowering.abi_read,
        runtime_scratch: lowering.runtime_scratch,
        signatures: lowering.signatures,
        matches: &matches,
        semantics: lowering.semantics,
        wasm_ir: lowering.wasm_ir,
        gc: lowering.gc,
        async_frames: lowering.async_frames,
        exact_runtime: lowering.exact_runtime,
        intrinsic_capture: None,
        debug: lowering.debug_emission(function_index),
        function_instance: instance.owner.as_ref(),
        loop_control: None,
        bare_return: BareReturn::None,
        materialize_none: true,
    };
    compile_block(&mut function, &closure.entry, &context, None);
    let result = semantic_type(*result, lowering.semantics);
    if result.has_runtime_value() {
        function.instruction(&Instruction::Unreachable);
    }
    function.instruction(&Instruction::End);
    function
}

/// Bridges the environment-first callable ABI to an ordinary source function.
/// Named function values are captureless, so local zero is deliberately
/// ignored and every physical callable argument is forwarded unchanged.
pub(super) fn compile_function_value_adapter(
    instance: &crate::semantic::FunctionValueInstance,
    lowering: &EmissionContext<'_>,
) -> Function {
    let crate::types::TypeKind::Callable { parameters, .. } =
        lowering.semantics.types().kind(instance.ty)
    else {
        unreachable!("function-value adapters have callable layouts")
    };
    let mut function = Function::new([]);
    let mut local = 1;
    for parameter in parameters {
        let parameter = semantic_type(*parameter, lowering.semantics);
        if parameter.has_runtime_value() {
            function.instruction(&Instruction::LocalGet(local));
            local += 1;
        }
    }
    function.instruction(&Instruction::Call(
        lowering.functions[&instance.function].call,
    ));
    function.instruction(&Instruction::End);
    function
}

pub(super) fn compile_async_closure_init(
    instance: &crate::semantic::ClosureInstance,
    closure: &wasm_ir::ClosureBody,
    layout: &AsyncFrameLayout,
    lowering: &EmissionContext<'_>,
) -> Function {
    let expression = lowering
        .wasm_ir
        .expression(closure.expression)
        .expect("closure expressions belong to Wasm IR");
    let expression_ty = instance.owner.as_ref().map_or(expression.ty, |owner| {
        lowering.semantics.specialize_type(owner, expression.ty)
    });
    let crate::types::TypeKind::Callable { parameters, .. } =
        lowering.semantics.types().kind(expression_ty)
    else {
        unreachable!("checked closure expressions have callable types")
    };
    let mut next_parameter = 1;
    let parameter_locals = closure
        .parameters
        .iter()
        .zip(parameters)
        .map(|(parameter, ty)| {
            let ty = semantic_type(*ty, lowering.semantics);
            let local = ty.has_runtime_value().then(|| {
                let local = next_parameter;
                next_parameter += 1;
                local
            });
            (*parameter, local)
        })
        .collect::<HashMap<_, _>>();
    let environment = lowering.gc.closure_environment_index(instance);
    let mut function = Function::new([]);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(
            lowering.gc.closure_frame_tag(instance) as i32,
        ))
        .instruction(&Instruction::I64Const(-1));
    for (position, ty) in layout.types.iter().copied().enumerate() {
        let field = layout.base_fields + position as u32;
        if let Some((capture_index, capture)) =
            closure.captures.iter().enumerate().find(|(_, capture)| {
                layout
                    .fields
                    .get(&capture.value)
                    .is_some_and(|(candidate, _)| *candidate == field)
            })
        {
            let environment = environment.expect("capturing closures have environment layouts");
            function.instruction(&Instruction::LocalGet(0)).instruction(
                &Instruction::RefCastNonNull(HeapType::Concrete(environment)),
            );
            if capture.mutable {
                function.instruction(&Instruction::StructGet {
                    struct_type_index: environment,
                    field_index: capture_index as u32,
                });
            } else {
                emit_typed_struct_get(&mut function, environment, capture_index as u32, ty);
            }
        } else if let Some(parameter) = closure.parameters.iter().find(|parameter| {
            layout
                .fields
                .get(parameter)
                .is_some_and(|(candidate, _)| *candidate == field)
        }) {
            if let Some(local) = parameter_locals[parameter] {
                function.instruction(&Instruction::LocalGet(local));
                if layout.capture_cell_fields.contains(&field) {
                    function
                        .instruction(&Instruction::StructNew(lowering.gc.capture_cell_index(ty)));
                }
            } else if layout.capture_cell_fields.contains(&field) {
                function.instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE));
            } else {
                emit_default(&mut function, ty, lowering.gc);
            }
        } else if layout.capture_cell_fields.contains(&field) {
            function.instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE));
        } else {
            emit_default(&mut function, ty, lowering.gc);
        }
    }
    function
        .instruction(&Instruction::StructNew(
            lowering.gc.closure_frame_index(instance),
        ))
        .instruction(&Instruction::End);
    function
}

pub(super) fn compile_action(
    action: &Action,
    function_index: u32,
    lowering: &EmissionContext<'_>,
) -> Function {
    let wasm_body = lowering
        .wasm_ir
        .body(BodyOwner::Action(action.kind))
        .expect("checked actions have Wasm IR bodies");
    let mut locals = HashMap::new();
    let mut local_types = Vec::new();
    let mut matches = MatchLayout::default();
    plan_wasm_locals(
        &wasm_body.locals,
        &mut locals,
        &mut matches,
        &mut local_types,
        LocalPlanOptions {
            parameter_count: if matches!(
                action.kind,
                ActionKind::Setup
                    | ActionKind::SelectProcess
                    | ActionKind::OnDetach
                    | ActionKind::OnStart
                    | ActionKind::OnReset
            ) {
                0
            } else {
                2
            },
            semantics: lowering.semantics,
            wasm_ir: lowering.wasm_ir,
            gc: lowering.gc,
            reachability: lowering.reachability,
            instance: None,
            include_values: true,
        },
    );
    if let Some(debug) = lowering.debug {
        for (&value, &(local, ty)) in &locals {
            debug.register_variable(function_index, value, local, ty, false);
        }
    }
    let mut function = Function::new_with_locals_types(local_types);
    let context = ExprContext {
        standard_library: lowering.standard_library,
        reachability: lowering.reachability,
        failure_payloads: lowering.failure_payloads,
        abi: lowering.abi,
        locals: LocalStorage::Wasm {
            values: &locals,
            temporaries: &matches.temporaries,
        },
        globals: lowering.globals,
        global_types: lowering.global_types,
        settings: lowering.settings,
        runtime_globals: lowering.runtime_globals,
        provider_values: lowering.provider_values,
        process_names: lowering.process_names,
        state_candidate: None,
        managed_read_context: None,
        runtime_helpers: lowering.runtime_helpers,
        functions: lowering.functions,
        closures: lowering.closures,
        function_values: lowering.function_values,
        closure_resumes: lowering.closure_resumes,
        closure_environment: None,
        leaf_futures: lowering.leaf_futures,
        display_functions: lowering.display_functions,
        equality_functions: lowering.equality_functions,
        array_functions: lowering.array_functions,
        set_functions: lowering.set_functions,
        structs: lowering.structs,
        managed: lowering.managed,
        managed_state_reads: lowering.managed_state_reads,
        managed_state_read_functions: lowering.managed_state_read_functions,
        managed_snapshot_functions: lowering.managed_snapshot_functions,
        managed_decoder_functions: lowering.managed_decoder_functions,
        managed_reference_functions: lowering.managed_reference_functions,
        enums: lowering.enums,
        arrays: lowering.arrays,
        memory: lowering.memory,
        abi_read: lowering.abi_read,
        runtime_scratch: lowering.runtime_scratch,
        signatures: lowering.signatures,
        matches: &matches,
        semantics: lowering.semantics,
        wasm_ir: lowering.wasm_ir,
        gc: lowering.gc,
        async_frames: lowering.async_frames,
        exact_runtime: lowering.exact_runtime,
        intrinsic_capture: None,
        debug: lowering.debug_emission(function_index),
        function_instance: None,
        loop_control: None,
        bare_return: BareReturn::Action(action.kind),
        materialize_none: true,
    };
    emit_snapshot_projection_prologue(&mut function, &wasm_body.locals, &context);
    compile_block(&mut function, &wasm_body.entry, &context, Some(action.kind));
    emit_action_default(&mut function, action.kind, lowering.semantics, lowering.gc);
    function.instruction(&Instruction::End);
    function
}

pub(super) fn emit_action_default(
    function: &mut Function,
    action: ActionKind,
    semantics: &SemanticModel,
    gc: &GcLayout,
) {
    match action {
        ActionKind::SelectProcess => {
            let ty = semantics
                .action_result(action)
                .expect("checked actions have result types");
            let crate::types::TypeKind::Result { layout, .. } = semantics.types().kind(ty) else {
                unreachable!("selectProcess has a fallible boolean ABI result")
            };
            function.instruction(&Instruction::I32Const(0));
            emit_result_success(function, *layout, gc);
        }
        ActionKind::Start | ActionKind::Split | ActionKind::Reset => {
            function.instruction(&Instruction::I32Const(0));
        }
        ActionKind::WhileAttached => {
            function.instruction(&Instruction::I32Const(1));
        }
        ActionKind::IsLoading => {
            function.instruction(&Instruction::I32Const(-1));
        }
        ActionKind::GameTime => {
            function.instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE));
        }
        ActionKind::Setup
        | ActionKind::OnDetach
        | ActionKind::OnAttach
        | ActionKind::OnStateReady
        | ActionKind::OnStart
        | ActionKind::OnReset => {}
    }
}

pub(super) struct LocalPlanOptions<'a> {
    pub(super) parameter_count: u32,
    pub(super) semantics: &'a SemanticModel,
    pub(super) wasm_ir: &'a wasm_ir::Program,
    pub(super) gc: &'a GcLayout,
    pub(super) reachability: &'a super::reachability::Reachability,
    pub(super) instance: Option<&'a crate::semantic::FunctionInstance>,
    pub(super) include_values: bool,
}

pub(super) fn plan_wasm_locals(
    planned: &[wasm_ir::Local],
    locals: &mut HashMap<ValueId, (u32, Type)>,
    matches: &mut MatchLayout,
    types: &mut Vec<ValType>,
    options: LocalPlanOptions<'_>,
) {
    let mut specialized_scratch = Vec::new();
    if let Some(instance) = options.instance {
        for (owner, expression) in options.reachability.expression_instances() {
            if owner.as_ref() != Some(instance) {
                continue;
            }
            let expression_ir = options
                .wasm_ir
                .expression(expression)
                .expect("reachable expressions belong to Wasm IR");
            let wasm_ir::ExpressionKind::Call { target, .. } = &expression_ir.kind else {
                continue;
            };
            if !matches!(target, wasm_ir::CallTarget::CapabilityRequirement { .. }) {
                continue;
            }
            let resolved =
                options
                    .reachability
                    .resolved_call_target(Some(instance), expression, target);
            if let wasm_ir::CallTarget::GeneratorNext { receiver_type, .. } = resolved {
                specialized_scratch.push((
                    options.semantics.specialize_type(instance, *receiver_type),
                    LocalPurpose::IntrinsicScratch {
                        expression,
                        slot: 0,
                    },
                ));
                continue;
            }
            let wasm_ir::CallTarget::Intrinsic {
                intrinsic,
                receiver_type,
                ..
            } = resolved
            else {
                continue;
            };
            let expression_ty = options
                .semantics
                .specialize_type(instance, expression_ir.ty);
            let receiver_ty =
                receiver_type.map(|receiver| options.semantics.specialize_type(instance, receiver));
            let mut slot_base = 0;
            for policy in crate::intrinsic_registry::contract(*intrinsic).synchronous_scratch {
                let scratch_ty = match policy.ty {
                    ScratchType::Core(core) => options.semantics.types().id_for_core(core),
                    ScratchType::Standard(standard) => {
                        options.semantics.types().id_for_standard(standard)
                    }
                    ScratchType::Expression => expression_ty,
                    ScratchType::ResultValue => {
                        let crate::types::TypeKind::Result { value, .. } =
                            options.semantics.types().kind(expression_ty)
                        else {
                            unreachable!("result-value scratch requires a Result expression")
                        };
                        *value
                    }
                    ScratchType::AsyncArgumentValue(_) => {
                        unreachable!("synchronous scratch cannot depend on an async argument value")
                    }
                    ScratchType::Receiver => {
                        receiver_ty.expect("receiver scratch requires a method-shaped intrinsic")
                    }
                };
                specialized_scratch.extend((0..policy.slots).map(|slot| {
                    (
                        scratch_ty,
                        LocalPurpose::IntrinsicScratch {
                            expression,
                            slot: slot_base + slot,
                        },
                    )
                }));
                slot_base += policy.slots;
            }
        }
    }

    for (local_ty, purpose) in planned
        .iter()
        .map(|local| (local.ty, local.purpose))
        .chain(specialized_scratch)
    {
        if matches!(purpose, LocalPurpose::Value(_)) && !options.include_values {
            continue;
        }
        let index = options.parameter_count + types.len() as u32;
        let ty = semantic_type(
            options.instance.map_or(local_ty, |instance| {
                options.semantics.specialize_type(instance, local_ty)
            }),
            options.semantics,
        );
        if ty == Type::Never {
            match purpose {
                LocalPurpose::Value(value) => {
                    locals.insert(value, (u32::MAX, ty));
                }
                LocalPurpose::Temporary(temporary) => {
                    matches.temporaries.insert(temporary, (u32::MAX, ty));
                }
                LocalPurpose::SnapshotProjection(projection) => {
                    matches
                        .snapshot_projections
                        .insert(projection, (u32::MAX, ty));
                }
                LocalPurpose::MatchValue(expression) => {
                    matches.values.insert(expression, u32::MAX);
                }
                LocalPurpose::FallbackValue(expression) => {
                    matches.fallback_values.insert(expression, u32::MAX);
                }
                LocalPurpose::IntrinsicScratch { expression, .. } => {
                    matches
                        .intrinsic_temps
                        .entry(expression)
                        .or_default()
                        .push(u32::MAX);
                }
                LocalPurpose::SuspensionScratch(expression) => {
                    matches.suspension_temps.insert(expression, u32::MAX);
                }
            }
            continue;
        }
        if ty == Type::None {
            match purpose {
                LocalPurpose::Value(value) => {
                    locals.insert(value, (u32::MAX, ty));
                    continue;
                }
                LocalPurpose::IntrinsicScratch { expression, .. } => {
                    matches
                        .intrinsic_temps
                        .entry(expression)
                        .or_default()
                        .push(u32::MAX);
                    continue;
                }
                _ => {}
            }
        }
        let val_type = match purpose {
            LocalPurpose::Value(value) if options.wasm_ir.is_mutably_captured(value) => {
                options.gc.capture_cell_val_type(ty)
            }
            _ => options.gc.val_type(ty),
        };
        types.push(val_type);
        match purpose {
            LocalPurpose::Value(value) => {
                locals.insert(value, (index, ty));
            }
            LocalPurpose::Temporary(temporary) => {
                matches.temporaries.insert(temporary, (index, ty));
            }
            LocalPurpose::SnapshotProjection(projection) => {
                matches.snapshot_projections.insert(projection, (index, ty));
            }
            LocalPurpose::MatchValue(expression) => {
                matches.values.insert(expression, index);
            }
            LocalPurpose::FallbackValue(expression) => {
                matches.fallback_values.insert(expression, index);
            }
            LocalPurpose::IntrinsicScratch { expression, .. } => {
                matches
                    .intrinsic_temps
                    .entry(expression)
                    .or_default()
                    .push(index);
            }
            LocalPurpose::SuspensionScratch(expression) => {
                matches.suspension_temps.insert(expression, index);
            }
        }
    }
}
use crate::codegen::GC_NULL_HEAP_TYPE;
use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, HeapType, Instruction, ValType};

use crate::{
    abi::AbiImportId,
    ast::{Action, ActionKind, FunctionDecl, ResultTypeId, StateField, StateSource, ValueId},
    intrinsic_registry::{RuntimeHelperId, ScratchType},
    semantic::SemanticModel,
    stdlib::{Implementation, IntrinsicId, StdlibTypeId},
    wasm_ir::{self, BodyOwner, LocalPurpose},
};

use super::{
    GcLayout, MemoryByteOrder, Type,
    async_frame::AsyncFrameLayout,
    context::EmissionContext,
    data_plan::StringPool,
    emit_default, emit_memory_load, emit_memory_value, emit_memory_value_is_valid,
    emit_native_memory_read_destination_and_size, emit_native_memory_value,
    emit_native_memory_value_is_valid, emit_result_error, emit_result_success, emit_struct_get,
    emit_typed_struct_get,
    expression::{
        BareReturn, ClosureEnvironment, ExprContext, LocalStorage, MatchLayout, compile_block,
        compile_resolved_path, emit_path_fields,
    },
    imports::Abi,
    memory_plan, semantic_type, state_storage_index, value_type,
};

pub(super) fn compile_async_function_init(
    declaration: &FunctionDecl,
    instance: &crate::semantic::FunctionInstance,
    layout: &AsyncFrameLayout,
    lowering: &EmissionContext<'_>,
) -> Function {
    let mut next_parameter = 0;
    let parameter_locals = declaration
        .params
        .iter()
        .map(|parameter| {
            let ty = semantic_type(
                lowering.semantics.specialize_type(
                    instance,
                    lowering
                        .semantics
                        .value_type(parameter.id)
                        .expect("checked parameters have types"),
                ),
                lowering.semantics,
            );
            let local = ty.has_runtime_value().then(|| {
                let local = next_parameter;
                next_parameter += 1;
                local
            });
            (parameter.id, local)
        })
        .collect::<HashMap<_, _>>();
    let mut function = Function::new([]);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(
            lowering.gc.function_frame_tag(instance) as i32,
        ))
        .instruction(&Instruction::I64Const(-1));
    for (position, ty) in layout.types.iter().copied().enumerate() {
        let field = position as u32 + layout.base_fields;
        let captured_cell = layout.capture_cell_fields.contains(&field);
        if let Some(parameter) = declaration.params.iter().find_map(|parameter| {
            layout
                .fields
                .get(&parameter.id)
                .is_some_and(|(candidate, _)| *candidate == field)
                .then_some(parameter.id)
        }) {
            if let Some(local) = parameter_locals[&parameter] {
                function.instruction(&Instruction::LocalGet(local));
                if captured_cell {
                    function
                        .instruction(&Instruction::StructNew(lowering.gc.capture_cell_index(ty)));
                }
            } else if captured_cell {
                function.instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE));
            } else {
                emit_default(&mut function, ty, lowering.gc);
            }
        } else if captured_cell {
            function.instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE));
        } else {
            emit_default(&mut function, ty, lowering.gc);
        }
    }
    function
        .instruction(&Instruction::StructNew(
            lowering.gc.function_frame_index(instance),
        ))
        .instruction(&Instruction::End);
    function
}

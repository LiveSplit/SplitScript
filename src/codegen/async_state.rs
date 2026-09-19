//! Wasm-IR async state-machine, suspension, retry, and cancellation emission.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, HeapType, Instruction, ValType};

use crate::semantic::FunctionInstance;
use crate::{
    abi::AbiImportId,
    ast::{Action, ExprId, SuspensionMode, ValueId},
    intrinsic_registry::RuntimeHelperId,
    stdlib::{
        IntrinsicId, MANAGED_POINTER_SIZE_FIELD, StdlibFieldId, StdlibTypeId,
        managed_instance_header_name,
    },
    types::TypeKind,
    wasm_ir::{self, BodyOwner},
};

use super::{
    LocalPlanOptions, MemoryByteOrder, Type, array_element_type,
    async_frame::{
        AsyncFrameLayout, AsyncFrameRef, AsyncFrameSource, CONTINUATION_POLL_EPOCH_FIELD,
        CONTINUATION_STATE_FIELD, CONTINUATION_TAG_FIELD, LeafFutureInstance, LeafFutureLayout,
    },
    call_target,
    context::AttachContext,
    data_plan::StringPool,
    emit_array_get, emit_default, emit_frame_typed_struct_get, emit_monotonic_nanoseconds,
    emit_native_memory_read_destination_and_size, emit_native_memory_value,
    emit_native_memory_value_is_valid, emit_result_error, emit_result_success, emit_string_literal,
    emit_typed_struct_get,
    expression::{
        BareReturn, ExprContext, IntrinsicCapture, LocalStorage, LoopControl, MatchLayout,
        compile_assignment, compile_expr, compile_fallback_condition, compile_for_bind_and_advance,
        compile_for_has_next, compile_for_init, compile_irrefutable_pattern, compile_receiver,
        compile_statement_pattern, compile_temporary_set, emit_failure_return,
        emit_managed_binding_field, error_may_have_effects, store_match_binding,
    },
    imports::Abi,
    plan_wasm_locals, resolved_intrinsic, semantic_type,
};

/// Candidate start positions inspected by one signature-future poll. At the
/// default attached rate of 120 Hz, 512 KiB provides roughly 60 MiB/s of scan
/// throughput. The range helper reads smaller pages within this window, and
/// control returns to the host between windows so large modules cannot
/// monopolize the autosplitter runtime.
const SIGNATURE_SCAN_CANDIDATES_PER_POLL: i64 = 512 * 1024;

/// Mapped-range metadata entries inspected by one `findMemoryRange` future
/// poll. Enumerating an entry performs only three scalar host queries and does
/// not read process memory, so its cooperative budget can be substantially
/// larger than one without risking the stalls that bound signature scanning.
/// At the default attached rate, 128 entries per poll traverses roughly one
/// thousand mappings in under one tenth of a second.
const MEMORY_RANGE_ENTRIES_PER_POLL: i64 = 128;

pub(super) fn compile_async_action(
    action: &Action,
    function_index: u32,
    layout: &AsyncFrameLayout,
    runtime: &AttachContext<'_>,
) -> Function {
    let wasm_body = runtime
        .lowering
        .wasm_ir
        .body(BodyOwner::Action(action.kind))
        .expect("checked actions have Wasm IR bodies");
    let frame = AsyncFrameRef {
        struct_type: runtime.lowering.gc.async_frame_index(),
        source: AsyncFrameSource::Global(runtime.lowering.runtime_globals.async_frame),
    };
    let result_global = match action.kind {
        crate::ast::ActionKind::OnAttach => runtime
            .lowering
            .state
            .provider_value
            .is_some_and(|_| runtime.lowering.explicit_shape_selection)
            .then_some(runtime.lowering.runtime_globals.selected_provider)
            .flatten(),
        crate::ast::ActionKind::WhileAttached => Some(
            runtime
                .lowering
                .runtime_globals
                .while_attached_result
                .expect("suspending whileAttached has result storage"),
        ),
        _ => unreachable!("only suspending lifecycle actions use the async action compiler"),
    };
    compile_continuation_body(
        &wasm_body.entry,
        &wasm_body.locals,
        wasm_body.async_state_count,
        wasm_body.cancellation_region,
        layout,
        runtime,
        frame,
        None,
        BareReturn::AsyncAction {
            action: action.kind,
            result_global,
        },
        result_global,
        function_index,
    )
}

pub(super) fn compile_async_function_poll(
    instance: &FunctionInstance,
    function_index: u32,
    layout: &AsyncFrameLayout,
    runtime: &AttachContext<'_>,
) -> Function {
    let wasm_body = runtime
        .lowering
        .wasm_ir
        .body(BodyOwner::Function(instance.clone()))
        .expect("checked functions have Wasm IR bodies");
    let frame = AsyncFrameRef {
        struct_type: runtime.lowering.gc.function_frame_index(instance),
        source: AsyncFrameSource::NonNullLocal(0),
    };
    compile_continuation_body(
        &wasm_body.entry,
        &wasm_body.locals,
        wasm_body.async_state_count,
        wasm_body.cancellation_region,
        layout,
        runtime,
        frame,
        Some(instance),
        BareReturn::AsyncFuture {
            frame,
            completion: layout.completion,
        },
        None,
        function_index,
    )
}

pub(super) fn compile_generator_function_next(
    instance: &FunctionInstance,
    function_index: u32,
    layout: &AsyncFrameLayout,
    runtime: &AttachContext<'_>,
) -> Function {
    let wasm_body = runtime
        .lowering
        .wasm_ir
        .body(BodyOwner::Function(instance.clone()))
        .expect("checked functions have Wasm IR bodies");
    let wasm_ir::BodyAbi::Generator(generator) = wasm_body.abi else {
        unreachable!("generator continuation entry points have generator bodies")
    };
    let item = runtime
        .lowering
        .semantics
        .specialize_type(instance, generator.item);
    let step = super::function_plan::iterator_step_type(item, runtime.lowering.semantics)
        .expect("consumed generator item types have materialized IteratorStep types");
    let frame = AsyncFrameRef {
        struct_type: runtime.lowering.gc.function_frame_index(instance),
        source: AsyncFrameSource::NonNullLocal(0),
    };
    compile_continuation_body(
        &wasm_body.entry,
        &wasm_body.locals,
        wasm_body.async_state_count,
        None,
        layout,
        runtime,
        frame,
        Some(instance),
        BareReturn::Generator { frame, step },
        None,
        function_index,
    )
}

pub(super) fn compile_async_closure_poll(
    instance: &crate::semantic::ClosureInstance,
    closure: &wasm_ir::ClosureBody,
    function_index: u32,
    layout: &AsyncFrameLayout,
    runtime: &AttachContext<'_>,
) -> Function {
    let frame = AsyncFrameRef {
        struct_type: runtime.lowering.gc.closure_frame_index(instance),
        source: AsyncFrameSource::NonNullLocal(0),
    };
    compile_continuation_body(
        &closure.entry,
        &closure.locals,
        closure.async_state_count,
        Some(wasm_ir::CancellationRegion::AttachmentLifetime),
        layout,
        runtime,
        frame,
        instance.owner.as_ref(),
        BareReturn::AsyncFuture {
            frame,
            completion: layout.completion,
        },
        None,
        function_index,
    )
}

pub(super) fn compile_generator_closure_next(
    instance: &crate::semantic::ClosureInstance,
    closure: &wasm_ir::ClosureBody,
    function_index: u32,
    layout: &AsyncFrameLayout,
    runtime: &AttachContext<'_>,
) -> Function {
    let wasm_ir::BodyAbi::Generator(generator) = closure.abi else {
        unreachable!("generator closure continuation entry points have generator bodies")
    };
    let item = instance.owner.as_ref().map_or(generator.item, |owner| {
        runtime
            .lowering
            .semantics
            .specialize_type(owner, generator.item)
    });
    let step = super::function_plan::iterator_step_type(item, runtime.lowering.semantics)
        .expect("consumed generator closure items have materialized IteratorStep types");
    let frame = AsyncFrameRef {
        struct_type: runtime.lowering.gc.closure_frame_index(instance),
        source: AsyncFrameSource::NonNullLocal(0),
    };
    compile_continuation_body(
        &closure.entry,
        &closure.locals,
        closure.async_state_count,
        None,
        layout,
        runtime,
        frame,
        instance.owner.as_ref(),
        BareReturn::Generator { frame, step },
        None,
        function_index,
    )
}

pub(super) fn compile_leaf_future_poll(
    instance: &LeafFutureInstance,
    function_index: u32,
    layout: &LeafFutureLayout,
    runtime: &AttachContext<'_>,
) -> Function {
    let frame = AsyncFrameRef {
        struct_type: runtime.lowering.gc.leaf_frame_index(instance),
        source: AsyncFrameSource::NonNullLocal(0),
    };
    let planned = wasm_ir::leaf_future_locals(
        instance.expression,
        runtime.lowering.wasm_ir,
        runtime.lowering.semantics,
        runtime.lowering.capabilities,
    );
    let mut matches = MatchLayout::default();
    let mut local_types = Vec::new();
    let mut values = HashMap::new();
    plan_wasm_locals(
        &planned,
        &mut values,
        &mut matches,
        &mut local_types,
        LocalPlanOptions {
            parameter_count: 1,
            semantics: runtime.lowering.semantics,
            wasm_ir: runtime.lowering.wasm_ir,
            gc: runtime.lowering.gc,
            reachability: runtime.lowering.reachability,
            instance: instance.owner.as_ref(),
            include_values: false,
        },
    );
    let managed_instances = matches!(
        call_target(runtime.lowering.wasm_ir, instance.expression),
        Some(wasm_ir::CallTarget::ManagedInstances { .. })
    );
    if managed_instances {
        debug_assert!(local_types.is_empty());
        local_types.extend([
            ValType::I64,
            ValType::I64,
            ValType::I64,
            ValType::I64,
            ValType::I64,
            ValType::I64,
            ValType::I32,
            ValType::I32,
        ]);
    }
    let mut function = Function::new_with_locals_types(local_types);
    let empty_values = HashMap::new();
    let empty_temporaries = HashMap::new();
    let context = ExprContext {
        standard_library: runtime.lowering.standard_library,
        reachability: runtime.lowering.reachability,
        failure_payloads: runtime.lowering.failure_payloads,
        abi: runtime.abi,
        locals: LocalStorage::Hybrid {
            frame,
            wasm_values: &empty_values,
            frame_values: &empty_values,
            wasm_temporaries: &empty_temporaries,
            frame_temporaries: &empty_temporaries,
        },
        globals: runtime.lowering.globals,
        global_types: runtime.lowering.global_types,
        settings: runtime.lowering.settings,
        runtime_globals: runtime.lowering.runtime_globals,
        provider_values: runtime.lowering.provider_values,
        process_names: runtime.lowering.process_names,
        state_candidate: None,
        managed_read_context: None,
        runtime_helpers: runtime.lowering.runtime_helpers,
        functions: runtime.lowering.functions,
        closures: runtime.lowering.closures,
        function_values: runtime.lowering.function_values,
        closure_resumes: runtime.lowering.closure_resumes,
        closure_environment: None,
        leaf_futures: runtime.lowering.leaf_futures,
        display_functions: runtime.lowering.display_functions,
        equality_functions: runtime.lowering.equality_functions,
        array_functions: runtime.lowering.array_functions,
        set_functions: runtime.lowering.set_functions,
        structs: runtime.lowering.structs,
        managed: runtime.lowering.managed,
        managed_state_reads: runtime.lowering.managed_state_reads,
        managed_state_read_functions: runtime.lowering.managed_state_read_functions,
        managed_snapshot_functions: runtime.lowering.managed_snapshot_functions,
        managed_decoder_functions: runtime.lowering.managed_decoder_functions,
        enums: runtime.lowering.enums,
        arrays: runtime.lowering.arrays,
        memory: runtime.lowering.memory,
        abi_read: runtime.lowering.abi_read,
        runtime_scratch: runtime.lowering.runtime_scratch,
        signatures: runtime.lowering.signatures,
        matches: &matches,
        semantics: runtime.lowering.semantics,
        wasm_ir: runtime.lowering.wasm_ir,
        gc: runtime.lowering.gc,
        async_frames: runtime.lowering.async_frames,
        exact_runtime: runtime.lowering.exact_runtime,
        intrinsic_capture: Some(IntrinsicCapture { frame, layout }),
        debug: runtime.lowering.debug_emission(function_index),
        function_instance: instance.owner.as_ref(),
        loop_control: None,
        bare_return: BareReturn::AsyncFuture {
            frame,
            completion: layout.completion,
        },
        materialize_none: true,
    };

    frame.emit(&mut function);
    function
        .instruction(&Instruction::StructGet {
            struct_type_index: frame.struct_type,
            field_index: 0,
        })
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    if call_target(runtime.lowering.wasm_ir, instance.expression).and_then(resolved_intrinsic)
        == Some(IntrinsicId::NextTick)
    {
        frame.emit(&mut function);
        function
            .instruction(&Instruction::StructGet {
                struct_type_index: frame.struct_type,
                field_index: 0,
            })
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty));
        frame.emit(&mut function);
        function
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::StructSet {
                struct_type_index: frame.struct_type,
                field_index: 0,
            })
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
    }

    let destination_layout = AsyncFrameLayout::for_leaf_completion(layout.completion);
    compile_suspension_poll(
        &mut function,
        SuspensionMode::Await,
        wasm_ir::SuspensionDestination::BodyResult,
        instance.expression,
        runtime.abi,
        runtime.strings,
        &destination_layout,
        &context,
    );
    mark_future_complete(&mut function, context.bare_return);
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End);
    function
}

#[allow(clippy::too_many_arguments)]
fn compile_continuation_body(
    entry: &wasm_ir::Block,
    locals: &[wasm_ir::Local],
    async_state_count: u32,
    cancellation_region: Option<wasm_ir::CancellationRegion>,
    layout: &AsyncFrameLayout,
    runtime: &AttachContext<'_>,
    frame: AsyncFrameRef,
    function_instance: Option<&FunctionInstance>,
    bare_return: BareReturn,
    result_global: Option<u32>,
    function_index: u32,
) -> Function {
    let mut matches = MatchLayout::default();
    let mut local_types = Vec::new();
    let mut planned_locals = HashMap::new();
    plan_wasm_locals(
        locals,
        &mut planned_locals,
        &mut matches,
        &mut local_types,
        LocalPlanOptions {
            parameter_count: match bare_return {
                BareReturn::AsyncAction {
                    action: crate::ast::ActionKind::WhileAttached,
                    ..
                } => 2,
                BareReturn::AsyncAction { .. }
                | BareReturn::AsyncFuture { .. }
                | BareReturn::Generator { .. } => 1,
                BareReturn::None | BareReturn::Action(_) => {
                    unreachable!("direct bodies do not use the async compiler")
                }
            },
            semantics: runtime.lowering.semantics,
            wasm_ir: runtime.lowering.wasm_ir,
            gc: runtime.lowering.gc,
            reachability: runtime.lowering.reachability,
            instance: function_instance,
            include_values: true,
        },
    );
    let mut function = Function::new_with_locals_types(local_types);
    let context = ExprContext {
        standard_library: runtime.lowering.standard_library,
        reachability: runtime.lowering.reachability,
        failure_payloads: runtime.lowering.failure_payloads,
        abi: runtime.abi,
        locals: LocalStorage::Hybrid {
            frame,
            wasm_values: &planned_locals,
            frame_values: &layout.fields,
            wasm_temporaries: &matches.temporaries,
            frame_temporaries: &layout.temporaries,
        },
        globals: runtime.lowering.globals,
        global_types: runtime.lowering.global_types,
        settings: runtime.lowering.settings,
        runtime_globals: runtime.lowering.runtime_globals,
        provider_values: runtime.lowering.provider_values,
        process_names: runtime.lowering.process_names,
        state_candidate: None,
        managed_read_context: None,
        runtime_helpers: runtime.lowering.runtime_helpers,
        functions: runtime.lowering.functions,
        closures: runtime.lowering.closures,
        function_values: runtime.lowering.function_values,
        closure_resumes: runtime.lowering.closure_resumes,
        closure_environment: None,
        leaf_futures: runtime.lowering.leaf_futures,
        display_functions: runtime.lowering.display_functions,
        equality_functions: runtime.lowering.equality_functions,
        array_functions: runtime.lowering.array_functions,
        set_functions: runtime.lowering.set_functions,
        structs: runtime.lowering.structs,
        managed: runtime.lowering.managed,
        managed_state_reads: runtime.lowering.managed_state_reads,
        managed_state_read_functions: runtime.lowering.managed_state_read_functions,
        managed_snapshot_functions: runtime.lowering.managed_snapshot_functions,
        managed_decoder_functions: runtime.lowering.managed_decoder_functions,
        enums: runtime.lowering.enums,
        arrays: runtime.lowering.arrays,
        memory: runtime.lowering.memory,
        abi_read: runtime.lowering.abi_read,
        runtime_scratch: runtime.lowering.runtime_scratch,
        signatures: runtime.lowering.signatures,
        matches: &matches,
        semantics: runtime.lowering.semantics,
        wasm_ir: runtime.lowering.wasm_ir,
        gc: runtime.lowering.gc,
        async_frames: runtime.lowering.async_frames,
        exact_runtime: runtime.lowering.exact_runtime,
        intrinsic_capture: None,
        debug: runtime.lowering.debug_emission(function_index),
        function_instance,
        loop_control: None,
        bare_return,
        materialize_none: true,
    };

    let mut states = (0..async_state_count).map(|_| None).collect::<Vec<_>>();
    states[wasm_ir::AsyncStateId::ENTRY.index() as usize] = Some(AsyncState::Block {
        block: entry,
        loop_targets: None,
        resume_source: None,
    });
    collect_async_states(entry, &mut states, None);
    debug_assert!(states.iter().all(Option::is_some));

    // A table avoids repeating the frame load and PC comparison for every
    // state. A single-state table costs more bytes; bound structured nesting for
    // unusually large machines. Both profiles use the same dispatcher.
    let table_dispatch = (2..=128).contains(&async_state_count);
    function.instruction(&Instruction::Loop(BlockType::Empty));
    if table_dispatch {
        // Innermost block is state 0. The outermost block handles an invalid
        // or completed PC with the same return as the linear dispatcher.
        for _ in 0..=async_state_count {
            function.instruction(&Instruction::Block(BlockType::Empty));
        }
        frame.emit(&mut function);
        function
            .instruction(&Instruction::StructGet {
                struct_type_index: frame.struct_type,
                field_index: 0,
            })
            .instruction(&Instruction::BrTable(
                (0..async_state_count).collect::<Vec<_>>().into(),
                async_state_count,
            ));
    }
    for (pc, state) in states.into_iter().enumerate() {
        let state = state.expect("every async state is assigned during lowering");
        let loop_depth = if table_dispatch {
            function.instruction(&Instruction::End);
            async_state_count - pc as u32
        } else {
            frame.emit(&mut function);
            function
                .instruction(&Instruction::StructGet {
                    struct_type_index: frame.struct_type,
                    field_index: 0,
                })
                .instruction(&Instruction::I32Const(pc as i32))
                .instruction(&Instruction::I32Eq)
                .instruction(&Instruction::If(BlockType::Empty));
            1
        };
        if let Some(debug) = context.debug {
            match state {
                AsyncState::Block { resume_source, .. } => {
                    debug.mark_resume(&function, resume_source);
                }
                AsyncState::Poll { source, .. } => {
                    debug.mark_suspend(&function, source);
                }
                AsyncState::ForHeader { .. } => {}
            }
        }

        let falls_through = match state {
            AsyncState::Block {
                block,
                loop_targets,
                ..
            } => compile_async_flow(
                &mut function,
                block,
                loop_depth,
                loop_targets.map(|targets| targets.control(loop_depth)),
                result_global,
                cancellation_region,
                layout,
                &context,
            ),
            AsyncState::ForHeader {
                binding,
                pattern,
                iterable_value,
                index_value,
                version_value,
                iterator_step,
                body,
                header_state,
                exit_state,
            } => {
                compile_for_has_next(
                    &mut function,
                    iterable_value,
                    index_value,
                    version_value,
                    iterator_step,
                    &context,
                );
                function.instruction(&Instruction::If(BlockType::Empty));
                compile_for_bind_and_advance(
                    &mut function,
                    binding,
                    iterable_value,
                    index_value,
                    version_value,
                    &context,
                );
                compile_irrefutable_pattern(&mut function, pattern, binding, &context);
                let body_falls_through = compile_async_flow(
                    &mut function,
                    body,
                    loop_depth + 1,
                    Some(
                        AsyncLoopTargets {
                            break_state: exit_state,
                            continue_state: header_state,
                            break_destination: None,
                        }
                        .control(loop_depth + 1),
                    ),
                    result_global,
                    cancellation_region,
                    layout,
                    &context,
                );
                function.instruction(&Instruction::Else);
                set_async_state(&mut function, exit_state, frame);
                function
                    .instruction(&Instruction::Br(loop_depth + 1))
                    .instruction(&Instruction::End);
                body_falls_through
            }
            AsyncState::Poll {
                mode,
                destination,
                value,
                resume_state,
                cancellation,
                ..
            } => {
                if call_target(context.wasm_ir, value)
                    .and_then(resolved_intrinsic)
                    .is_some()
                {
                    assert_eq!(
                        cancellation, cancellation_region,
                        "awaited standard-library operation must participate in its body's cancellation region"
                    );
                }
                compile_suspension_poll(
                    &mut function,
                    mode,
                    destination,
                    value,
                    runtime.abi,
                    runtime.strings,
                    layout,
                    &context,
                );
                set_async_state(&mut function, resume_state, frame);
                function.instruction(&Instruction::Br(loop_depth));
                false
            }
        };
        if falls_through {
            emit_continuation_fallthrough(&mut function, bare_return, context.gc);
        }
        if !table_dispatch {
            function.instruction(&Instruction::End);
        }
    }
    if table_dispatch {
        function.instruction(&Instruction::End);
    }
    emit_continuation_result(&mut function, bare_return, context.gc);
    function.instruction(&Instruction::Return);
    function.instruction(&Instruction::End);
    emit_continuation_result(&mut function, bare_return, context.gc);
    function.instruction(&Instruction::End);
    function
}

fn emit_async_action_default(function: &mut Function, target: BareReturn) {
    if let BareReturn::AsyncAction {
        action: crate::ast::ActionKind::WhileAttached,
        result_global: Some(result),
    } = target
    {
        function
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::GlobalSet(result));
    }
}

fn mark_future_complete(function: &mut Function, target: BareReturn) {
    if let BareReturn::AsyncFuture { frame, .. } | BareReturn::Generator { frame, .. } = target {
        frame.emit(function);
        function
            .instruction(&Instruction::I32Const(-1))
            .instruction(&Instruction::StructSet {
                struct_type_index: frame.struct_type,
                field_index: 0,
            });
    }
}

fn emit_continuation_result(function: &mut Function, target: BareReturn, gc: &super::GcLayout) {
    match target {
        BareReturn::Generator { step, .. } => {
            function.instruction(&Instruction::RefNull(HeapType::Concrete(gc.index(step))));
        }
        BareReturn::AsyncAction { .. } | BareReturn::AsyncFuture { .. } => {
            function.instruction(&Instruction::I32Const(1));
        }
        BareReturn::None | BareReturn::Action(_) => {
            unreachable!("direct bodies do not use the continuation compiler")
        }
    }
}

fn emit_continuation_fallthrough(
    function: &mut Function,
    target: BareReturn,
    gc: &super::GcLayout,
) {
    emit_async_action_default(function, target);
    emit_continuation_return(function, target, gc);
}

fn emit_continuation_return(function: &mut Function, target: BareReturn, gc: &super::GcLayout) {
    mark_future_complete(function, target);
    emit_continuation_result(function, target, gc);
    function.instruction(&Instruction::Return);
}

fn intrinsic_state(context: &ExprContext<'_>, slot: usize) -> (AsyncFrameRef, u32, Type) {
    let capture = context
        .intrinsic_capture
        .expect("stateful intrinsics are polled through their own future frame");
    let (field, ty) = capture.layout.state[slot];
    (capture.frame, field, ty)
}

fn emit_intrinsic_state_get(function: &mut Function, context: &ExprContext<'_>, slot: usize) {
    let (frame, field, ty) = intrinsic_state(context, slot);
    frame.emit(function);
    emit_frame_typed_struct_get(function, frame.struct_type, field, ty, context.gc);
}

fn emit_intrinsic_state_set_local(
    function: &mut Function,
    context: &ExprContext<'_>,
    slot: usize,
    local: u32,
) {
    let (frame, field, _) = intrinsic_state(context, slot);
    frame.emit(function);
    function
        .instruction(&Instruction::LocalGet(local))
        .instruction(&Instruction::StructSet {
            struct_type_index: frame.struct_type,
            field_index: field,
        });
}

fn emit_intrinsic_state_set_zero(function: &mut Function, context: &ExprContext<'_>, slot: usize) {
    let (frame, field, _) = intrinsic_state(context, slot);
    frame.emit(function);
    function
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::StructSet {
            struct_type_index: frame.struct_type,
            field_index: field,
        });
}

fn emit_intrinsic_state_set_one(function: &mut Function, context: &ExprContext<'_>, slot: usize) {
    let (frame, field, _) = intrinsic_state(context, slot);
    frame.emit(function);
    function
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::StructSet {
            struct_type_index: frame.struct_type,
            field_index: field,
        });
}

fn emit_range_scan_exhausted(function: &mut Function, context: &ExprContext<'_>, finite: bool) {
    emit_intrinsic_state_set_zero(function, context, 0);
    if finite {
        emit_intrinsic_state_set_one(function, context, 2);
    }
}

fn emit_signature_length_i64(function: &mut Function, signature: u32) {
    function
        .instruction(&Instruction::LocalGet(signature))
        .instruction(&Instruction::I64Const(32))
        .instruction(&Instruction::I64ShrU);
}

fn emit_signature_length_i32(function: &mut Function, signature: u32) {
    emit_signature_length_i64(function, signature);
    function.instruction(&Instruction::I32WrapI64);
}

fn emit_signature_window_limit(function: &mut Function, signature: u32) {
    emit_signature_length_i64(function, signature);
    function
        .instruction(&Instruction::I64Const(
            SIGNATURE_SCAN_CANDIDATES_PER_POLL - 1,
        ))
        .instruction(&Instruction::I64Add);
}

fn emit_signature_arguments(function: &mut Function, signature: u32) {
    // A first-class Signature packs its static needle pointer into the low 32
    // bits and its length into the high 32 bits. The mask immediately follows
    // the needle in the compiler-owned static-data pool.
    function
        .instruction(&Instruction::LocalGet(signature))
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::LocalGet(signature))
        .instruction(&Instruction::I32WrapI64);
    emit_signature_length_i32(function, signature);
    function.instruction(&Instruction::I32Add);
    emit_signature_length_i32(function, signature);
}

/// Polls one bounded window of an explicit memory range. The caller has
/// already placed the range address and size in scratch slots 1 and 2. A
/// successful address is left in slot 3; absence returns `pending` directly.
fn emit_cooperative_range_scan(
    function: &mut Function,
    target: &wasm_ir::CallTarget,
    scratch: &[u32],
    process_from_runtime: bool,
    relative_target: Option<(u32, u32)>,
    finite: bool,
    context: &ExprContext<'_>,
) {
    let cursor = scratch[0];
    let address = scratch[1];
    let remaining = scratch[2];
    let matched = scratch[3];
    let signature = scratch[4];

    // Deliver a match saved by the previous poll without doing more scanning
    // work. A scan that inspects memory always yields once, so several awaited
    // scans chained by the caller cannot all run during a single host update.
    emit_intrinsic_state_get(function, context, 1);
    function.instruction(&Instruction::LocalSet(matched));
    if finite {
        emit_intrinsic_state_get(function, context, 2);
    } else {
        function.instruction(&Instruction::LocalGet(matched));
    }
    function
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));

    emit_intrinsic_state_get(function, context, 0);
    function
        .instruction(&Instruction::LocalSet(cursor))
        // remaining = size - cursor, unless no candidate can start here.
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_range_scan_exhausted(function, context, finite);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalTee(remaining));
    emit_signature_length_i64(function, signature);
    function
        .instruction(&Instruction::I64LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_range_scan_exhausted(function, context, finite);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    if process_from_runtime {
        function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
    } else {
        compile_receiver(function, target, context);
    }
    function
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(remaining));
    emit_signature_window_limit(function, signature);
    function
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::Else);
    emit_signature_window_limit(function, signature);
    function.instruction(&Instruction::End);
    emit_signature_arguments(function, signature);
    if let Some((displacement_offset, relative_target)) = relative_target {
        function
            .instruction(&Instruction::LocalGet(displacement_offset))
            .instruction(&Instruction::LocalGet(relative_target));
    }
    function
        .instruction(&Instruction::Call(context.runtime_helpers.function(
            if relative_target.is_some() {
                RuntimeHelperId::ScanRelative32TargetRange
            } else {
                RuntimeHelperId::ScanProcessRange
            },
        )))
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(remaining));
    emit_signature_window_limit(function, signature);
    function
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_range_scan_exhausted(function, context, finite);
    function.instruction(&Instruction::Else);
    let (frame, field, _) = intrinsic_state(context, 0);
    frame.emit(function);
    function
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::I64Const(SIGNATURE_SCAN_CANDIDATES_PER_POLL))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::StructSet {
            struct_type_index: frame.struct_type,
            field_index: field,
        })
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    emit_intrinsic_state_set_local(function, context, 1, matched);
    if finite {
        emit_intrinsic_state_set_one(function, context, 2);
    }
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
    emit_intrinsic_state_set_zero(function, context, 0);
    emit_intrinsic_state_set_zero(function, context, 1);
    if finite {
        emit_intrinsic_state_set_zero(function, context, 2);
    }
}

fn emit_process_scan_advance_range(
    function: &mut Function,
    context: &ExprContext<'_>,
    next_range: u32,
) {
    emit_intrinsic_state_set_local(function, context, 0, next_range);
    emit_intrinsic_state_set_zero(function, context, 1);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return);
}

/// Polls at most one bounded window from one readable process memory range.
/// State slot 0 encodes the current reverse range as `index + 1`; zero starts
/// a fresh snapshot. State slot 1 is the byte cursor within that range.
fn emit_cooperative_process_scan(
    function: &mut Function,
    target: &wasm_ir::CallTarget,
    scratch: &[u32],
    abi: &Abi,
    context: &ExprContext<'_>,
) {
    let range_cursor = scratch[0];
    let offset = scratch[1];
    let index = scratch[2];
    let address = scratch[3];
    let remaining = scratch[4];
    let matched = scratch[5];
    let signature = scratch[6];

    // A non-zero result was found in the preceding poll. Deliver it now and
    // clear the future-local traversal state for a later execution of the same
    // await expression.
    emit_intrinsic_state_get(function, context, 2);
    function
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));

    emit_intrinsic_state_get(function, context, 0);
    function.instruction(&Instruction::LocalSet(range_cursor));
    emit_intrinsic_state_get(function, context, 1);
    function.instruction(&Instruction::LocalSet(offset));

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeCount),
        ))
        .instruction(&Instruction::LocalSet(index))
        // Start a new reverse traversal, or recover if the host's mapping list
        // shrank while this future was suspended.
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64GtU)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalSet(range_cursor))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(offset))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_zero(function, context, 0);
    emit_intrinsic_state_set_zero(function, context, 1);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalSet(index));

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeFlags),
        ))
        .instruction(&Instruction::I64Const(1 << 1))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_advance_range(function, context, index);
    function.instruction(&Instruction::End);

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeAddress),
        ))
        .instruction(&Instruction::LocalSet(address));
    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeSize),
        ))
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_advance_range(function, context, index);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalTee(remaining));
    emit_signature_length_i64(function, signature);
    function
        .instruction(&Instruction::I64LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_advance_range(function, context, index);
    function.instruction(&Instruction::End);

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(remaining));
    emit_signature_window_limit(function, signature);
    function
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::Else);
    emit_signature_window_limit(function, signature);
    function.instruction(&Instruction::End);
    emit_signature_arguments(function, signature);
    function
        .instruction(&Instruction::Call(
            context
                .runtime_helpers
                .function(RuntimeHelperId::ScanProcessRange),
        ))
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(remaining));
    emit_signature_window_limit(function, signature);
    function
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_advance_range(function, context, index);
    function.instruction(&Instruction::Else);
    emit_intrinsic_state_set_local(function, context, 0, range_cursor);
    let (frame, field, _) = intrinsic_state(context, 1);
    frame.emit(function);
    function
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Const(SIGNATURE_SCAN_CANDIDATES_PER_POLL))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::StructSet {
            struct_type_index: frame.struct_type,
            field_index: field,
        })
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);

    emit_intrinsic_state_set_local(function, context, 2, matched);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
    emit_intrinsic_state_set_zero(function, context, 0);
    emit_intrinsic_state_set_zero(function, context, 1);
    emit_intrinsic_state_set_zero(function, context, 2);
}

fn emit_process_scan_any_advance_range(
    function: &mut Function,
    context: &ExprContext<'_>,
    next_range: u32,
) {
    emit_intrinsic_state_set_local(function, context, 0, next_range);
    emit_intrinsic_state_set_zero(function, context, 1);
    emit_intrinsic_state_set_zero(function, context, 2);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return);
}

fn emit_signature_array_length(
    function: &mut Function,
    signatures_expression: ExprId,
    context: &ExprContext<'_>,
) {
    compile_expr(function, signatures_expression, context);
    let Type::Array(array) = context.expression_type(signatures_expression) else {
        unreachable!("signature candidates are arrays")
    };
    super::array_value::emit_length(function, context.gc, array);
    function.instruction(&Instruction::I64ExtendI32U);
}

fn emit_cooperative_module_scan_any(
    function: &mut Function,
    target: &wasm_ir::CallTarget,
    signatures_expression: ExprId,
    destination: wasm_ir::SuspensionDestination,
    layout: &AsyncFrameLayout,
    scratch: &[u32],
    context: &ExprContext<'_>,
) {
    let offset = scratch[0];
    let remaining = scratch[1];
    let matched = scratch[2];
    let signature = scratch[3];
    let signature_index = scratch[4];
    let Type::Array(signature_array) = context.expression_type(signatures_expression) else {
        unreachable!("module scan candidates are arrays")
    };

    emit_intrinsic_state_get(function, context, 2);
    function
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_get(function, context, 0);
    function.instruction(&Instruction::LocalSet(offset));
    emit_intrinsic_state_get(function, context, 1);
    function.instruction(&Instruction::LocalSet(signature_index));
    compile_expr(function, signatures_expression, context);
    super::array_value::emit_backing(function, context.gc, signature_array);
    let signature_storage =
        super::array_value::storage_id(signature_array, context.arrays, context.semantics);
    function
        .instruction(&Instruction::LocalGet(signature_index))
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::ArrayGet(
            context.gc.index(Type::ArrayStorage(signature_storage)),
        ))
        .instruction(&Instruction::LocalSet(signature));

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::StructGet {
            struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
            field_index: context.gc.standard_field_index(StdlibFieldId::ModuleSize),
        })
        .instruction(&Instruction::LocalTee(remaining))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_zero(function, context, 0);
    emit_intrinsic_state_set_zero(function, context, 1);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalTee(remaining));
    emit_signature_length_i64(function, signature);
    function
        .instruction(&Instruction::I64LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_zero(function, context, 0);
    emit_intrinsic_state_set_zero(function, context, 1);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::StructGet {
            struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
            field_index: context
                .gc
                .standard_field_index(StdlibFieldId::ModuleAddress),
        })
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(remaining));
    emit_signature_window_limit(function, signature);
    function
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::Else);
    emit_signature_window_limit(function, signature);
    function.instruction(&Instruction::End);
    emit_signature_arguments(function, signature);
    function
        .instruction(&Instruction::Call(
            context
                .runtime_helpers
                .function(RuntimeHelperId::ScanProcessRange),
        ))
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(signature_index))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(signature_index));
    emit_signature_array_length(function, signatures_expression, context);
    function
        .instruction(&Instruction::LocalGet(signature_index))
        .instruction(&Instruction::I64GtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_local(function, context, 1, signature_index);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Const(SIGNATURE_SCAN_CANDIDATES_PER_POLL))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(offset));
    emit_intrinsic_state_set_local(function, context, 0, offset);
    emit_intrinsic_state_set_zero(function, context, 1);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
    emit_intrinsic_state_set_local(function, context, 2, matched);
    emit_intrinsic_state_set_local(function, context, 3, signature_index);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    if let Some((field, _)) = layout.field(destination) {
        context.locals.frame().emit(function);
        function.instruction(&Instruction::LocalGet(matched));
        emit_intrinsic_state_get(function, context, 3);
        function
            .instruction(&Instruction::I32WrapI64)
            .instruction(&Instruction::StructNew(
                context.gc.standard_index(StdlibTypeId::SignatureMatch),
            ))
            .instruction(&Instruction::StructSet {
                struct_type_index: context.locals.frame().struct_type,
                field_index: field,
            });
    }
    for slot in 0..4 {
        emit_intrinsic_state_set_zero(function, context, slot);
    }
}

/// Traverses one bounded batch of mapped-range metadata per poll. A complete
/// miss refreshes the host's range list on the next poll, so the future remains
/// pending until a matching mapping appears or process-close cancellation
/// discards it. Status values are zero (refresh the snapshot), one (found), and
/// three (scanning).
#[allow(clippy::too_many_arguments)]
fn emit_process_find_memory_range(
    function: &mut Function,
    target: &wasm_ir::CallTarget,
    args: &[ExprId],
    destination: wasm_ir::SuspensionDestination,
    layout: &AsyncFrameLayout,
    scratch: &[u32],
    abi: &Abi,
    context: &ExprContext<'_>,
) {
    let status = scratch[0];
    let cursor = scratch[1];
    let address = scratch[2];
    let size = scratch[3];
    let flags = scratch[4];
    let budget = scratch[5];
    let wanted_size = scratch[6];
    let wanted_flags = scratch[7];

    emit_intrinsic_state_get(function, context, 0);
    function
        .instruction(&Instruction::LocalSet(status))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::LocalGet(status))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::If(BlockType::Empty));
    if let Some((field, _)) = layout.field(destination) {
        context.locals.frame().emit(function);
        emit_intrinsic_state_get(function, context, 2);
        emit_intrinsic_state_get(function, context, 3);
        for mask in [2_i64, 4, 8] {
            emit_intrinsic_state_get(function, context, 4);
            function
                .instruction(&Instruction::I64Const(mask))
                .instruction(&Instruction::I64And)
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::I32Eqz);
        }
        function
            .instruction(&Instruction::StructNew(
                context.gc.standard_index(StdlibTypeId::MemoryRange),
            ))
            .instruction(&Instruction::StructSet {
                struct_type_index: context.locals.frame().struct_type,
                field_index: field,
            });
    }
    for slot in 0..5 {
        emit_intrinsic_state_set_zero(function, context, slot);
    }
    function
        .instruction(&Instruction::Br(1))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(status))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeCount),
        ))
        .instruction(&Instruction::LocalSet(cursor));
    emit_intrinsic_state_set_local(function, context, 1, cursor);
    function
        .instruction(&Instruction::I64Const(3))
        .instruction(&Instruction::LocalSet(status));
    emit_intrinsic_state_set_local(function, context, 0, status);
    function.instruction(&Instruction::End);

    compile_expr(function, args[0], context);
    function.instruction(&Instruction::LocalSet(wanted_size));
    compile_expr(function, args[1], context);
    function
        .instruction(&Instruction::StructGet {
            struct_type_index: context.gc.standard_index(StdlibTypeId::MemoryRangeAccess),
            field_index: 0,
        })
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::I64Const(2))
        .instruction(&Instruction::Else);
    compile_expr(function, args[1], context);
    function
        .instruction(&Instruction::StructGet {
            struct_type_index: context.gc.standard_index(StdlibTypeId::MemoryRangeAccess),
            field_index: 0,
        })
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::I64Const(6))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(wanted_flags))
        .instruction(&Instruction::I64Const(MEMORY_RANGE_ENTRIES_PER_POLL))
        .instruction(&Instruction::LocalSet(budget))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty));

    emit_intrinsic_state_get(function, context, 1);
    function
        .instruction(&Instruction::LocalTee(cursor))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        // The current snapshot was exhausted. Leave the future pending and
        // refresh the range count on the next host update.
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(cursor));
    emit_intrinsic_state_set_local(function, context, 0, cursor);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalTee(cursor));
    emit_intrinsic_state_set_local(function, context, 1, cursor);

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeAddress),
        ))
        .instruction(&Instruction::LocalSet(address));
    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeSize),
        ))
        .instruction(&Instruction::LocalSet(size));
    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeFlags),
        ))
        .instruction(&Instruction::LocalSet(flags))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::LocalGet(size));
    function
        .instruction(&Instruction::LocalGet(wanted_size))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::LocalGet(flags))
        .instruction(&Instruction::LocalGet(wanted_flags))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::LocalGet(wanted_flags))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_local(function, context, 2, address);
    emit_intrinsic_state_set_local(function, context, 3, size);
    emit_intrinsic_state_set_local(function, context, 4, flags);
    function
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::LocalSet(status));
    emit_intrinsic_state_set_local(function, context, 0, status);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(status))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(budget))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalTee(budget))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
}

/// Multi-pattern counterpart of `emit_cooperative_process_scan`. One poll
/// checks one signature in one bounded window. This keeps latency independent
/// of both mapped-range size and the number of fallback signatures.
fn emit_cooperative_process_scan_any(
    function: &mut Function,
    target: &wasm_ir::CallTarget,
    signatures_expression: ExprId,
    signature_array: crate::ast::ArrayTypeId,
    scratch: &[u32],
    abi: &Abi,
    context: &ExprContext<'_>,
) {
    let range_cursor = scratch[0];
    let offset = scratch[1];
    let index = scratch[2];
    let address = scratch[3];
    let remaining = scratch[4];
    let matched = scratch[5];
    let signature = scratch[6];
    let signature_index = scratch[7];

    // Preserve both parts of the match across one mandatory cooperative
    // boundary. This keeps a fallback scan to one signature/window attempt per
    // host poll even when its caller immediately awaits another scan.
    emit_intrinsic_state_get(function, context, 3);
    function
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));

    emit_intrinsic_state_get(function, context, 0);
    function.instruction(&Instruction::LocalSet(range_cursor));
    emit_intrinsic_state_get(function, context, 1);
    function.instruction(&Instruction::LocalSet(offset));
    emit_intrinsic_state_get(function, context, 2);
    function.instruction(&Instruction::LocalSet(signature_index));

    emit_signature_array_length(function, signatures_expression, context);
    function
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
    compile_expr(function, signatures_expression, context);
    super::array_value::emit_backing(function, context.gc, signature_array);
    let signature_storage =
        super::array_value::storage_id(signature_array, context.arrays, context.semantics);
    function
        .instruction(&Instruction::LocalGet(signature_index))
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::ArrayGet(
            context.gc.index(Type::ArrayStorage(signature_storage)),
        ))
        .instruction(&Instruction::LocalSet(signature));

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeCount),
        ))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64GtU)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalSet(range_cursor))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(offset))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(signature_index))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_zero(function, context, 0);
    emit_intrinsic_state_set_zero(function, context, 1);
    emit_intrinsic_state_set_zero(function, context, 2);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalSet(index));

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeFlags),
        ))
        .instruction(&Instruction::I64Const(1 << 1))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_any_advance_range(function, context, index);
    function.instruction(&Instruction::End);

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeAddress),
        ))
        .instruction(&Instruction::LocalSet(address));
    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeSize),
        ))
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_any_advance_range(function, context, index);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalTee(remaining));
    emit_signature_length_i64(function, signature);
    function
        .instruction(&Instruction::I64LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_any_advance_range(function, context, index);
    function.instruction(&Instruction::End);

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(remaining));
    emit_signature_window_limit(function, signature);
    function
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::Else);
    emit_signature_window_limit(function, signature);
    function.instruction(&Instruction::End);
    emit_signature_arguments(function, signature);
    function
        .instruction(&Instruction::Call(
            context
                .runtime_helpers
                .function(RuntimeHelperId::ScanProcessRange),
        ))
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        // Try the next signature against this same range window.
        .instruction(&Instruction::LocalGet(signature_index))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalTee(signature_index));
    emit_signature_array_length(function, signatures_expression, context);
    function
        .instruction(&Instruction::I64LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_local(function, context, 0, range_cursor);
    emit_intrinsic_state_set_local(function, context, 1, offset);
    emit_intrinsic_state_set_local(function, context, 2, signature_index);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        // Every signature missed this window; move to the next window/range.
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(signature_index))
        .instruction(&Instruction::LocalGet(remaining));
    emit_signature_window_limit(function, signature);
    function
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_any_advance_range(function, context, index);
    function.instruction(&Instruction::Else);
    emit_intrinsic_state_set_local(function, context, 0, range_cursor);
    let (frame, offset_field, _) = intrinsic_state(context, 1);
    frame.emit(function);
    function
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Const(SIGNATURE_SCAN_CANDIDATES_PER_POLL))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::StructSet {
            struct_type_index: frame.struct_type,
            field_index: offset_field,
        });
    emit_intrinsic_state_set_zero(function, context, 2);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);

    emit_intrinsic_state_set_local(function, context, 3, matched);
    emit_intrinsic_state_set_local(function, context, 4, signature_index);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
    emit_intrinsic_state_get(function, context, 4);
    function.instruction(&Instruction::LocalSet(signature_index));
    emit_intrinsic_state_set_zero(function, context, 0);
    emit_intrinsic_state_set_zero(function, context, 1);
    emit_intrinsic_state_set_zero(function, context, 2);
    emit_intrinsic_state_set_zero(function, context, 3);
    emit_intrinsic_state_set_zero(function, context, 4);
}

/// Polls one bounded window while discovering live instances of a managed
/// class. The attachment preparation has already resolved the runtime-specific
/// object-header discriminator, so this path is backend-neutral.
fn emit_managed_instances_poll(
    function: &mut Function,
    class: crate::ast::ManagedClassId,
    destination: wasm_ir::SuspensionDestination,
    layout: &AsyncFrameLayout,
    abi: &Abi,
    context: &ExprContext<'_>,
) {
    const MATCH_LIMIT_PER_POLL: i32 = 1024;
    let cursor = 1;
    let range_address = 2;
    let range_size = 3;
    let search_address = 4;
    let remaining = 5;
    let matched = 6;
    let pointer_size = 7;
    let match_count = 8;

    let (_, _, Type::Array(array)) = intrinsic_state(context, 2) else {
        unreachable!("managed instance discovery stores its result array in leaf state")
    };
    let storage = super::array_value::storage_id(array, context.arrays, context.semantics);

    // A zero cursor is the uninitialized state. Store count + 1 so an empty
    // process map remains distinguishable from an uninitialized scan.
    emit_intrinsic_state_get(function, context, 0);
    function
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    let (frame, array_field, _) = intrinsic_state(context, 2);
    frame.emit(function);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::ArrayNewDefault(
            context.gc.index(Type::ArrayStorage(storage)),
        ));
    super::array_value::emit_wrap(function, context.gc, array, 0);
    function.instruction(&Instruction::StructSet {
        struct_type_index: frame.struct_type,
        field_index: array_field,
    });
    function
        .instruction(&Instruction::GlobalGet(context.runtime_globals.process))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeCount),
        ))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(cursor));
    emit_intrinsic_state_set_local(function, context, 0, cursor);
    function.instruction(&Instruction::End);

    // Cursor 1 means every range has been consumed and the snapshot is ready.
    emit_intrinsic_state_get(function, context, 0);
    function
        .instruction(&Instruction::LocalTee(cursor))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::If(BlockType::Empty));
    if let Some((destination_field, destination_type)) = layout.field(destination) {
        debug_assert_eq!(destination_type, Type::Array(array));
        context.locals.frame().emit(function);
        emit_intrinsic_state_get(function, context, 2);
        function.instruction(&Instruction::StructSet {
            struct_type_index: context.locals.frame().struct_type,
            field_index: destination_field,
        });
    }
    emit_intrinsic_state_set_zero(function, context, 0);
    emit_intrinsic_state_set_zero(function, context, 1);
    let (state_frame, state_field, state_type) = intrinsic_state(context, 2);
    state_frame.emit(function);
    emit_default(function, state_type, context.gc);
    function.instruction(&Instruction::StructSet {
        struct_type_index: state_frame.struct_type,
        field_index: state_field,
    });
    mark_future_complete(function, context.bare_return);
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    // The reverse cursor maps count + 1 to the next zero-based range index.
    function
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::I64Const(2))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalSet(cursor));
    function
        .instruction(&Instruction::GlobalGet(context.runtime_globals.process))
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeAddress),
        ))
        .instruction(&Instruction::LocalSet(range_address))
        .instruction(&Instruction::GlobalGet(context.runtime_globals.process))
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeSize),
        ))
        .instruction(&Instruction::LocalSet(range_size))
        .instruction(&Instruction::GlobalGet(context.runtime_globals.process))
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeFlags),
        ))
        .instruction(&Instruction::LocalSet(matched));
    emit_managed_binding_field(function, MANAGED_POINTER_SIZE_FIELD, context);
    function.instruction(&Instruction::LocalSet(pointer_size));

    // Heap objects live in writable data ranges. Excluding executable pages
    // removes code-address coincidences without assuming a platform layout.
    function
        .instruction(&Instruction::LocalGet(range_address))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::LocalGet(range_size))
        .instruction(&Instruction::LocalGet(pointer_size))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::LocalGet(matched))
        .instruction(&Instruction::I64Const(6))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::I64Const(6))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::LocalGet(matched))
        .instruction(&Instruction::I64Const(8))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty));

    emit_intrinsic_state_get(function, context, 1);
    function
        .instruction(&Instruction::LocalTee(search_address))
        .instruction(&Instruction::LocalGet(range_size))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_get(function, context, 0);
    function
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalSet(cursor));
    emit_intrinsic_state_set_local(function, context, 0, cursor);
    emit_intrinsic_state_set_zero(function, context, 1);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    function
        .instruction(&Instruction::LocalGet(range_address))
        .instruction(&Instruction::LocalGet(search_address))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(search_address))
        .instruction(&Instruction::LocalGet(range_size));
    emit_intrinsic_state_get(function, context, 1);
    function
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::I64Const(SIGNATURE_SCAN_CANDIDATES_PER_POLL))
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::LocalGet(range_size));
    emit_intrinsic_state_get(function, context, 1);
    function
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I64Const(SIGNATURE_SCAN_CANDIDATES_PER_POLL))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(match_count))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(matched))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(match_count))
        .instruction(&Instruction::I32Const(MATCH_LIMIT_PER_POLL))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::GlobalGet(context.runtime_globals.process))
        .instruction(&Instruction::LocalGet(search_address))
        .instruction(&Instruction::LocalGet(remaining));
    emit_managed_binding_field(
        function,
        &managed_instance_header_name(class.index()),
        context,
    );
    function
        .instruction(&Instruction::LocalGet(pointer_size))
        .instruction(&Instruction::Call(
            context
                .runtime_helpers
                .function(RuntimeHelperId::ScanAlignedPointerRange),
        ))
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(1));

    emit_intrinsic_state_get(function, context, 2);
    function
        .instruction(&Instruction::LocalGet(matched))
        .instruction(&Instruction::Call(context.array_functions.push(array)))
        // Advance past this header before asking for the next match.
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(matched))
        .instruction(&Instruction::LocalGet(search_address))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalGet(pointer_size))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::LocalGet(matched))
        .instruction(&Instruction::LocalGet(pointer_size))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(search_address))
        .instruction(&Instruction::LocalGet(match_count))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(match_count))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);

    // Exhausting a window consumes its remaining bytes. Hitting the match cap
    // instead resumes immediately after the last returned object header.
    function
        .instruction(&Instruction::LocalGet(search_address))
        .instruction(&Instruction::LocalGet(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(range_address))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalSet(search_address));
    emit_intrinsic_state_set_local(function, context, 1, search_address);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::Else);

    // Skip non-data ranges without ever scanning them.
    emit_intrinsic_state_get(function, context, 0);
    function
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalSet(cursor));
    emit_intrinsic_state_set_local(function, context, 0, cursor);
    emit_intrinsic_state_set_zero(function, context, 1);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
}

#[allow(clippy::too_many_arguments)]
fn compile_suspension_poll(
    function: &mut Function,
    mode: SuspensionMode,
    destination: wasm_ir::SuspensionDestination,
    value: ExprId,
    abi: &Abi,
    strings: &StringPool,
    layout: &AsyncFrameLayout,
    context: &ExprContext<'_>,
) {
    if mode == SuspensionMode::Retry {
        compile_retry_poll(function, destination, value, layout, context);
        return;
    }
    let value_expression = context
        .wasm_ir
        .expression(value)
        .expect("await value belongs to Wasm IR");
    let stateful_leaf = match &value_expression.kind {
        wasm_ir::ExpressionKind::Call {
            target: wasm_ir::CallTarget::Intrinsic { intrinsic, .. },
            ..
        } => !crate::intrinsic_registry::contract(*intrinsic)
            .async_state
            .is_empty(),
        wasm_ir::ExpressionKind::Call {
            target: wasm_ir::CallTarget::ManagedInstances { .. },
            ..
        } => true,
        _ => false,
    };
    if !matches!(
        &value_expression.kind,
        wasm_ir::ExpressionKind::Call {
            target: wasm_ir::CallTarget::Intrinsic { .. }
                | wasm_ir::CallTarget::ManagedInstances { .. },
            ..
        }
    ) || (stateful_leaf && context.intrinsic_capture.is_none())
    {
        compile_source_future_poll(function, destination, value, layout, context);
        return;
    }
    let wasm_ir::ExpressionKind::Call {
        target,
        arguments: args,
    } = &value_expression.kind
    else {
        unreachable!();
    };
    let scratch = context
        .matches
        .intrinsic_temps
        .get(&value)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let primary_scratch = scratch.first().copied().unwrap_or(u32::MAX);
    let module_address_local = primary_scratch;
    let module_size_local = scratch.get(1).copied().unwrap_or(u32::MAX);
    if let wasm_ir::CallTarget::ManagedInstances { class } = target {
        emit_managed_instances_poll(function, *class, destination, layout, abi, context);
        return;
    }
    match resolved_intrinsic(target) {
        Some(IntrinsicId::NextTick) => {}
        Some(IntrinsicId::FutureRace) => {
            emit_future_race_poll(function, args[0], destination, layout, scratch, context);
        }
        Some(IntrinsicId::FutureTimeout) => {
            emit_future_timeout_poll(
                function,
                value,
                args[0],
                args[1],
                destination,
                layout,
                scratch,
                context,
            );
        }
        Some(IntrinsicId::ProcessClosed) => {
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::Drop)
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return);
        }
        Some(IntrinsicId::ProcessMainModule | IntrinsicId::ProcessModule) => {
            let module_name = if resolved_intrinsic(target) == Some(IntrinsicId::ProcessModule) {
                let wasm_ir::ExpressionKind::String(name) = &context
                    .wasm_ir
                    .expression(args[0])
                    .expect("module name belongs to Wasm IR")
                    .kind
                else {
                    unreachable!();
                };
                Some(name.as_str())
            } else {
                None
            };
            emit_process_module_query(
                function,
                target,
                module_name,
                AbiImportId::ProcessGetModuleAddress,
                abi,
                strings,
                context,
            );
            function
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            emit_process_module_query(
                function,
                target,
                module_name,
                AbiImportId::ProcessGetModuleSize,
                abi,
                strings,
                context,
            );
            function
                .instruction(&Instruction::LocalTee(module_size_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::LocalGet(module_size_local));
                emit_module_name_value(function, module_name, context);
                function
                    .instruction(&Instruction::StructNew(
                        context.gc.standard_index(StdlibTypeId::Module),
                    ))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ProcessFindMemoryRange) => {
            emit_process_find_memory_range(
                function,
                target,
                args,
                destination,
                layout,
                scratch,
                abi,
                context,
            );
        }
        Some(IntrinsicId::ProcessRead) => {
            let read_type_id = match target {
                wasm_ir::CallTarget::Intrinsic { type_arguments, .. } => {
                    context.type_id(type_arguments[0])
                }
                _ => unreachable!("process.read must resolve to its standard-library item"),
            };
            let read_type = semantic_type(read_type_id, context.semantics);
            if let Some((_, stored_type)) = layout.field(destination) {
                context.locals.frame().emit(function);
                debug_assert_eq!(stored_type, read_type);
            }
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            emit_native_memory_read_destination_and_size(
                function,
                read_type_id,
                context.abi_read,
                context.memory,
                context.semantics,
                context.runtime_globals.process_pointer_size,
            );
            function
                .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
                .instruction(&Instruction::I32Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if read_type == Type::Address {
                emit_native_memory_value(
                    function,
                    read_type_id,
                    context.abi_read,
                    0,
                    context.memory,
                    context.semantics,
                    context.gc,
                    context.runtime_globals.process_pointer_size,
                    MemoryByteOrder::Little,
                );
                function
                    .instruction(&Instruction::I64Eqz)
                    .instruction(&Instruction::If(BlockType::Empty))
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::Return)
                    .instruction(&Instruction::End);
            }
            if emit_native_memory_value_is_valid(
                function,
                read_type_id,
                context.abi_read,
                0,
                context.memory,
                context.semantics,
                context.runtime_globals.process_pointer_size,
                MemoryByteOrder::Little,
            ) {
                function
                    .instruction(&Instruction::I32Eqz)
                    .instruction(&Instruction::If(BlockType::Empty))
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::Return)
                    .instruction(&Instruction::End);
            }
            if let Some((field, _)) = layout.field(destination) {
                emit_native_memory_value(
                    function,
                    read_type_id,
                    context.abi_read,
                    0,
                    context.memory,
                    context.semantics,
                    context.gc,
                    context.runtime_globals.process_pointer_size,
                    MemoryByteOrder::Little,
                );
                function.instruction(&Instruction::StructSet {
                    struct_type_index: context.locals.frame().struct_type,
                    field_index: field,
                });
            }
        }
        Some(IntrinsicId::ProcessFollow) => {
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            compile_expr(function, args[1], context);
            function.instruction(&Instruction::GlobalGet(
                context
                    .runtime_globals
                    .process_pointer_size
                    .expect("process.follow plans pointer-width storage"),
            ));
            function
                .instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::FollowAddress),
                ))
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ProcessScan) => {
            compile_expr(function, args[0], context);
            function.instruction(&Instruction::LocalSet(scratch[1]));
            compile_expr(function, args[1], context);
            function.instruction(&Instruction::LocalSet(scratch[2]));
            compile_expr(function, args[2], context);
            function.instruction(&Instruction::LocalSet(scratch[4]));
            emit_cooperative_range_scan(function, target, scratch, false, None, false, context);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(scratch[3]))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ProcessScanOnce) => {
            compile_expr(function, args[0], context);
            function.instruction(&Instruction::LocalSet(scratch[1]));
            compile_expr(function, args[1], context);
            function.instruction(&Instruction::LocalSet(scratch[2]));
            compile_expr(function, args[2], context);
            function.instruction(&Instruction::LocalSet(scratch[4]));
            emit_cooperative_range_scan(function, target, scratch, false, None, true, context);
            if let Some((field, Type::Option(option))) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(scratch[3]))
                    .instruction(&Instruction::I64Eqz)
                    .instruction(&Instruction::If(BlockType::Result(
                        context.gc.val_type(Type::Option(option)),
                    )))
                    .instruction(&Instruction::RefNull(HeapType::Concrete(
                        context.gc.index(Type::Option(option)),
                    )))
                    .instruction(&Instruction::Else)
                    .instruction(&Instruction::LocalGet(scratch[3]))
                    .instruction(&Instruction::StructNew(
                        context.gc.index(Type::Option(option)),
                    ))
                    .instruction(&Instruction::End)
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ModuleScanRelative32Target) => {
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
                    field_index: context
                        .gc
                        .standard_field_index(StdlibFieldId::ModuleAddress),
                })
                .instruction(&Instruction::LocalSet(scratch[1]));
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
                    field_index: context.gc.standard_field_index(StdlibFieldId::ModuleSize),
                })
                .instruction(&Instruction::LocalSet(scratch[2]));
            compile_expr(function, args[0], context);
            function.instruction(&Instruction::LocalSet(scratch[4]));
            compile_expr(function, args[1], context);
            function.instruction(&Instruction::LocalSet(scratch[5]));
            compile_expr(function, args[2], context);
            function.instruction(&Instruction::LocalSet(scratch[6]));
            emit_cooperative_range_scan(
                function,
                target,
                scratch,
                true,
                Some((scratch[5], scratch[6])),
                false,
                context,
            );
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(scratch[3]))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ProcessScanMemory) => {
            compile_expr(function, args[0], context);
            function.instruction(&Instruction::LocalSet(scratch[6]));
            emit_cooperative_process_scan(function, target, scratch, abi, context);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(scratch[5]))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ProcessScanMemoryAny) => {
            let Type::Array(signature_array) = context.expression_type(args[0]) else {
                unreachable!("scanMemoryAny accepts a signature array")
            };
            emit_cooperative_process_scan_any(
                function,
                target,
                args[0],
                signature_array,
                scratch,
                abi,
                context,
            );
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(scratch[5]))
                    .instruction(&Instruction::LocalGet(scratch[7]))
                    .instruction(&Instruction::I32WrapI64)
                    .instruction(&Instruction::StructNew(
                        context.gc.standard_index(StdlibTypeId::SignatureMatch),
                    ))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ProcessReadRelative32) => {
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::ReadRelative32),
                ))
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ModuleScan) => {
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
                    field_index: context
                        .gc
                        .standard_field_index(StdlibFieldId::ModuleAddress),
                })
                .instruction(&Instruction::LocalSet(scratch[1]));
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
                    field_index: context.gc.standard_field_index(StdlibFieldId::ModuleSize),
                })
                .instruction(&Instruction::LocalSet(scratch[2]));
            compile_expr(function, args[0], context);
            function.instruction(&Instruction::LocalSet(scratch[4]));
            emit_cooperative_range_scan(function, target, scratch, true, None, false, context);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(scratch[3]))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ModuleMd5) => {
            let [
                status,
                initialized,
                offset,
                size,
                mtime,
                packed_ab,
                packed_cd,
                path,
                hash,
                ..,
            ] = scratch
            else {
                unreachable!("Module.md5 reserves poll state and string scratch locals")
            };
            let Some((field, Type::Result(result))) = layout.field(destination) else {
                unreachable!("Module.md5 completes through String!")
            };

            function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
                    field_index: context.gc.standard_field_index(StdlibFieldId::ModuleName),
                })
                .instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::ModulePath),
                ))
                .instruction(&Instruction::LocalTee(*path))
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::If(BlockType::Empty));
            context.locals.frame().emit(function);
            emit_result_error(
                function,
                result,
                Type::Standard(StdlibTypeId::String),
                "module path could not be read",
                context.gc,
                context.failure_payloads,
            );
            function
                .instruction(&Instruction::StructSet {
                    struct_type_index: context.locals.frame().struct_type,
                    field_index: field,
                })
                .instruction(&Instruction::Else)
                .instruction(&Instruction::LocalGet(*path));
            for slot in 0..6 {
                emit_intrinsic_state_get(function, context, slot);
            }
            function
                .instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::ModuleMd5Poll),
                ))
                .instruction(&Instruction::LocalSet(*hash))
                .instruction(&Instruction::LocalSet(*packed_cd))
                .instruction(&Instruction::LocalSet(*packed_ab))
                .instruction(&Instruction::LocalSet(*mtime))
                .instruction(&Instruction::LocalSet(*size))
                .instruction(&Instruction::LocalSet(*offset))
                .instruction(&Instruction::LocalSet(*initialized))
                .instruction(&Instruction::LocalSet(*status));
            for (slot, local) in [*initialized, *offset, *size, *mtime, *packed_ab, *packed_cd]
                .into_iter()
                .enumerate()
            {
                emit_intrinsic_state_set_local(function, context, slot, local);
            }
            function
                .instruction(&Instruction::LocalGet(*status))
                .instruction(&Instruction::I32Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            context.locals.frame().emit(function);
            function
                .instruction(&Instruction::LocalGet(*status))
                .instruction(&Instruction::I32Const(2))
                .instruction(&Instruction::I32Eq)
                .instruction(&Instruction::If(BlockType::Result(
                    context.gc.val_type(Type::Result(result)),
                )))
                .instruction(&Instruction::LocalGet(*hash));
            emit_result_success(function, result, context.gc);
            function.instruction(&Instruction::Else);
            emit_result_error(
                function,
                result,
                Type::Standard(StdlibTypeId::String),
                "module file could not be hashed",
                context.gc,
                context.failure_payloads,
            );
            function
                .instruction(&Instruction::End)
                .instruction(&Instruction::StructSet {
                    struct_type_index: context.locals.frame().struct_type,
                    field_index: field,
                })
                .instruction(&Instruction::End);
        }
        Some(IntrinsicId::ModuleScanAny) => {
            emit_cooperative_module_scan_any(
                function,
                target,
                args[0],
                destination,
                layout,
                scratch,
                context,
            );
        }
        _ => unreachable!("type checking only permits awaitable builtins"),
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_future_timeout_poll(
    function: &mut Function,
    timeout: ExprId,
    operation: ExprId,
    duration: ExprId,
    destination: wasm_ir::SuspensionDestination,
    layout: &AsyncFrameLayout,
    scratch: &[u32],
    context: &ExprContext<'_>,
) {
    let [
        operation_value,
        now,
        duration_seconds,
        elapsed,
        duration_nanoseconds,
        ..,
    ] = scratch
    else {
        unreachable!("future.timeout reserves operation and duration scratch locals")
    };
    let TypeKind::Async {
        value: operation_completion,
        ..
    } = context
        .semantics
        .types()
        .kind(context.expression_type_id(operation))
    else {
        unreachable!("future.timeout accepts an async operation")
    };
    let operation_completion = semantic_type(*operation_completion, context.semantics);
    let TypeKind::Async {
        value: timeout_completion,
        ..
    } = context
        .semantics
        .types()
        .kind(context.expression_type_id(timeout))
    else {
        unreachable!("future.timeout produces an async result")
    };
    let TypeKind::Result {
        layout: timeout_result,
        value: timeout_value,
    } = context.semantics.types().kind(*timeout_completion)
    else {
        unreachable!("future.timeout completes through a Result")
    };
    let timeout_value = semantic_type(*timeout_value, context.semantics);
    let operation_is_already_fallible = operation_completion == Type::Result(*timeout_result);

    let poll_destination = if operation_completion.has_runtime_value() {
        FuturePollDestination::Local {
            local: *operation_value,
            ty: operation_completion,
        }
    } else {
        FuturePollDestination::Discard
    };
    emit_future_poll_status(
        function,
        context.expression_type(operation),
        poll_destination,
        context
            .exact_runtime
            .expression_continuation(context.function_instance, operation),
        context,
        |function| compile_expr(function, operation, context),
    );
    function.instruction(&Instruction::If(BlockType::Empty));

    // A completed fallible operation already has the exact result layout used
    // by timeout. Infallible values are lifted into that one shared channel.
    if let Some((field, destination_type)) = layout.field(destination) {
        debug_assert_eq!(destination_type, Type::Result(*timeout_result));
        context.locals.frame().emit(function);
        if operation_is_already_fallible {
            function.instruction(&Instruction::LocalGet(*operation_value));
        } else {
            if operation_completion.has_runtime_value() {
                function.instruction(&Instruction::LocalGet(*operation_value));
            } else {
                emit_default(function, operation_completion, context.gc);
            }
            emit_result_success(function, *timeout_result, context.gc);
        }
        function.instruction(&Instruction::StructSet {
            struct_type_index: context.locals.frame().struct_type,
            field_index: field,
        });
    }

    function.instruction(&Instruction::Else);

    // Start the monotonic deadline on the first poll that actually observes a
    // pending operation. Immediate completion therefore needs no clock read.
    let clock_destination = context.abi_read.destination(8);
    emit_monotonic_nanoseconds(function, context.abi, clock_destination);
    function.instruction(&Instruction::LocalSet(*now));
    emit_intrinsic_state_get(function, context, 0);
    function
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_local(function, context, 1, *now);
    let (frame, initialized_field, _) = intrinsic_state(context, 0);
    frame.emit(function);
    function
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::StructSet {
            struct_type_index: frame.struct_type,
            field_index: initialized_field,
        })
        .instruction(&Instruction::End);

    compile_expr(function, duration, context);
    function
        .instruction(&Instruction::StructGet {
            struct_type_index: context.gc.standard_index(StdlibTypeId::Duration),
            field_index: context
                .gc
                .standard_field_index(StdlibFieldId::DurationSeconds),
        })
        .instruction(&Instruction::LocalSet(*duration_seconds));
    compile_expr(function, duration, context);
    function
        .instruction(&Instruction::StructGet {
            struct_type_index: context.gc.standard_index(StdlibTypeId::Duration),
            field_index: context
                .gc
                .standard_field_index(StdlibFieldId::DurationNanoseconds),
        })
        .instruction(&Instruction::LocalSet(*duration_nanoseconds));

    function.instruction(&Instruction::LocalGet(*now));
    emit_intrinsic_state_get(function, context, 1);
    function
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalSet(*elapsed));

    // Non-positive durations expire after the operation's first immediate
    // poll. Positive durations compare normalized seconds/nanoseconds without
    // multiplying an i64 duration by one billion and risking overflow.
    function
        .instruction(&Instruction::LocalGet(*duration_seconds))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::I64LtS)
        .instruction(&Instruction::LocalGet(*duration_seconds))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::LocalGet(*duration_nanoseconds))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32LeS)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(*elapsed))
        .instruction(&Instruction::I64Const(1_000_000_000))
        .instruction(&Instruction::I64DivU)
        .instruction(&Instruction::LocalGet(*duration_seconds))
        .instruction(&Instruction::I64GtU)
        .instruction(&Instruction::LocalGet(*elapsed))
        .instruction(&Instruction::I64Const(1_000_000_000))
        .instruction(&Instruction::I64DivU)
        .instruction(&Instruction::LocalGet(*duration_seconds))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::LocalGet(*elapsed))
        .instruction(&Instruction::I64Const(1_000_000_000))
        .instruction(&Instruction::I64RemU)
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::LocalGet(*duration_nanoseconds))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::End)
        .instruction(&Instruction::If(BlockType::Empty));
    if let Some((field, destination_type)) = layout.field(destination) {
        debug_assert_eq!(destination_type, Type::Result(*timeout_result));
        context.locals.frame().emit(function);
        emit_result_error(
            function,
            *timeout_result,
            timeout_value,
            "future timed out",
            context.gc,
            context.failure_payloads,
        );
        function.instruction(&Instruction::StructSet {
            struct_type_index: context.locals.frame().struct_type,
            field_index: field,
        });
    }
    function.instruction(&Instruction::Else);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
}

fn emit_future_race_poll(
    function: &mut Function,
    operations: ExprId,
    destination: wasm_ir::SuspensionDestination,
    layout: &AsyncFrameLayout,
    scratch: &[u32],
    context: &ExprContext<'_>,
) {
    let Type::Array(array) = context.expression_type(operations) else {
        unreachable!("future.race accepts an array of futures")
    };
    let future_type = array_element_type(array, context.semantics);
    let Type::Async(_) = future_type else {
        unreachable!("future.race accepts async array elements")
    };
    let [index, length, version, ..] = scratch else {
        unreachable!("future.race reserves index, length, and version scratch locals")
    };

    compile_expr(function, operations, context);
    super::array_value::emit_length(function, context.gc, array);
    function.instruction(&Instruction::LocalSet(*length));
    compile_expr(function, operations, context);
    super::array_value::emit_version(function, context.gc, array);
    function
        .instruction(&Instruction::LocalSet(*version))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(*index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(*index))
        .instruction(&Instruction::LocalGet(*length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        // An empty array and an all-pending array have the same result: this
        // race remains pending until a later host update.
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    // A child is allowed to share the operations array through its captures.
    // If it structurally mutates the array while being polled, stop this pass
    // before indexing stale backing storage and observe the new shape next
    // update.
    compile_expr(function, operations, context);
    super::array_value::emit_version(function, context.gc, array);
    function
        .instruction(&Instruction::LocalGet(*version))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    let storage = Type::ArrayStorage(super::array_value::storage_id(
        array,
        context.arrays,
        context.semantics,
    ));
    emit_future_poll_status(
        function,
        future_type,
        FuturePollDestination::Frame {
            destination,
            layout,
        },
        None,
        context,
        |function| {
            compile_expr(function, operations, context);
            super::array_value::emit_backing(function, context.gc, array);
            function.instruction(&Instruction::LocalGet(*index));
            emit_array_get(function, context.gc.index(storage), future_type, context.gc);
        },
    );
    function
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Br(2))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(*index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(*index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
}

fn compile_source_future_poll(
    function: &mut Function,
    destination: wasm_ir::SuspensionDestination,
    expression: ExprId,
    parent_layout: &AsyncFrameLayout,
    context: &ExprContext<'_>,
) {
    let (child_field, child_type) = parent_layout.children[&expression];
    let Type::Async(child_future) = child_type else {
        unreachable!("source async calls produce future values")
    };
    let parent = context.locals.frame();

    parent.emit(function);
    function
        .instruction(&Instruction::StructGet {
            struct_type_index: parent.struct_type,
            field_index: child_field,
        })
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Empty));
    parent.emit(function);
    compile_expr(function, expression, context);
    function
        .instruction(&Instruction::StructSet {
            struct_type_index: parent.struct_type,
            field_index: child_field,
        })
        .instruction(&Instruction::End);

    emit_future_poll_status(
        function,
        child_type,
        FuturePollDestination::Frame {
            destination,
            layout: parent_layout,
        },
        context
            .exact_runtime
            .expression_continuation(context.function_instance, expression),
        context,
        |function| {
            parent.emit(function);
            function.instruction(&Instruction::StructGet {
                struct_type_index: parent.struct_type,
                field_index: child_field,
            });
        },
    );
    function
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    clear_child_future(function, parent, child_field, child_future, context);
}

/// Polls one erased first-class future and leaves `0` for pending or `1` for
/// ready on the stack. All producer families share this dispatch path, so
/// combinators cannot accidentally support only source, closure, or intrinsic
/// futures. A pending handle already polled during this host update is not
/// advanced again through another alias.
#[derive(Clone, Copy)]
enum FuturePollDestination<'a> {
    Frame {
        destination: wasm_ir::SuspensionDestination,
        layout: &'a AsyncFrameLayout,
    },
    Local {
        local: u32,
        ty: Type,
    },
    Discard,
}

fn emit_future_poll_status(
    function: &mut Function,
    future_type: Type,
    destination: FuturePollDestination<'_>,
    exact: Option<&super::exact_runtime::ContinuationProducer>,
    context: &ExprContext<'_>,
    mut emit_future: impl FnMut(&mut Function),
) {
    let Type::Async(_) = future_type else {
        unreachable!("future dispatch requires an async value")
    };
    let erased_frame = context.gc.index(future_type);
    let parent = context.locals.frame();

    let source_candidates = context
        .async_frames
        .functions()
        .filter(|(candidate, _)| {
            let result = context.semantics.specialize_type(
                candidate,
                context
                    .semantics
                    .function_result(candidate.function)
                    .expect("checked functions have result types"),
            );
            semantic_type(result, context.semantics) == future_type
        })
        .map(|(candidate, layout)| {
            (
                super::exact_runtime::ContinuationProducer::Function((*candidate).clone()),
                context.gc.function_frame_index(candidate),
                context.gc.function_frame_tag(candidate),
                context.functions[candidate]
                    .resume
                    .expect("async source functions have poll entries"),
                layout.completion,
            )
        });
    let closure_candidates = context
        .async_frames
        .closures()
        .filter(|(instance, _)| {
            let ty = context
                .wasm_ir
                .expression(instance.expression)
                .expect("reachable closure expressions belong to Wasm IR")
                .ty;
            let ty = instance
                .owner
                .as_ref()
                .map_or(ty, |owner| context.semantics.specialize_type(owner, ty));
            let TypeKind::Callable { result, .. } = context.semantics.types().kind(ty) else {
                unreachable!("checked closure expressions have callable types")
            };
            semantic_type(*result, context.semantics) == future_type
        })
        .map(|(instance, layout)| {
            (
                super::exact_runtime::ContinuationProducer::Closure((*instance).clone()),
                context.gc.closure_frame_index(instance),
                context.gc.closure_frame_tag(instance),
                context.closure_resumes[instance],
                layout.completion,
            )
        });
    let leaf_candidates = context
        .async_frames
        .leaves()
        .filter(|(_, layout)| layout.future == future_type)
        .map(|(instance, layout)| {
            (
                super::exact_runtime::ContinuationProducer::Leaf((*instance).clone()),
                context.gc.leaf_frame_index(instance),
                context.gc.leaf_frame_tag(instance),
                context.leaf_futures[instance],
                layout.completion,
            )
        });
    let candidates = source_candidates
        .chain(closure_candidates)
        .chain(leaf_candidates)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        // This is reachable for a statically empty `future.race([])`: there is
        // no concrete frame to dispatch because the program never constructs
        // an operation of the inferred future type. The caller's length check
        // keeps this path pending at runtime.
        function.instruction(&Instruction::I32Const(0));
        return;
    }

    function.instruction(&Instruction::Block(BlockType::Result(ValType::I32)));

    // A future that is still pending after another alias polled it during this
    // update reports pending without running its state machine twice. Completed
    // futures still flow through producer dispatch so their typed result can be
    // copied into the caller's destination.
    emit_future(function);
    function
        .instruction(&Instruction::StructGet {
            struct_type_index: erased_frame,
            field_index: CONTINUATION_POLL_EPOCH_FIELD,
        })
        .instruction(&Instruction::GlobalGet(
            context.runtime_globals.future_poll_epoch,
        ))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_future(function);
    function
        .instruction(&Instruction::StructGet {
            struct_type_index: erased_frame,
            field_index: CONTINUATION_STATE_FIELD,
        })
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Br(2))
        .instruction(&Instruction::End)
        .instruction(&Instruction::Else);
    emit_future(function);
    function
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::GlobalGet(
            context.runtime_globals.future_poll_epoch,
        ))
        .instruction(&Instruction::StructSet {
            struct_type_index: erased_frame,
            field_index: CONTINUATION_POLL_EPOCH_FIELD,
        })
        .instruction(&Instruction::End);

    if let Some(exact) = exact {
        let (_, frame_type, _, poll, completion) = candidates
            .iter()
            .find(|(producer, ..)| producer == exact)
            .expect("exact future producers have a reachable poll implementation");
        emit_future(function);
        function
            .instruction(&Instruction::RefCastNonNull(HeapType::Concrete(
                *frame_type,
            )))
            .instruction(&Instruction::Call(*poll))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::Br(1))
            .instruction(&Instruction::End);

        let mut emit_completion = |function: &mut Function| {
            if let Some((completion_field, completion_type)) = completion {
                emit_future(function);
                function.instruction(&Instruction::RefCastNonNull(HeapType::Concrete(
                    *frame_type,
                )));
                emit_frame_typed_struct_get(
                    function,
                    *frame_type,
                    *completion_field,
                    *completion_type,
                    context.gc,
                );
                *completion_type
            } else {
                emit_default(function, Type::None, context.gc);
                Type::None
            }
        };
        match destination {
            FuturePollDestination::Frame {
                destination,
                layout,
            } => {
                if let Some((destination_field, destination_type)) = layout.field(destination) {
                    parent.emit(function);
                    let completion_type = emit_completion(function);
                    debug_assert_eq!(destination_type, completion_type);
                    function.instruction(&Instruction::StructSet {
                        struct_type_index: parent.struct_type,
                        field_index: destination_field,
                    });
                }
            }
            FuturePollDestination::Local { local, ty } => {
                let completion_type = emit_completion(function);
                debug_assert_eq!(ty, completion_type);
                function.instruction(&Instruction::LocalSet(local));
            }
            FuturePollDestination::Discard => {}
        }
        function
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::End);
        return;
    }

    for (_, frame_type, tag, poll, completion) in candidates {
        emit_future(function);
        function
            .instruction(&Instruction::StructGet {
                struct_type_index: erased_frame,
                field_index: CONTINUATION_TAG_FIELD,
            })
            .instruction(&Instruction::I32Const(tag as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_future(function);
        function
            .instruction(&Instruction::RefCastNonNull(HeapType::Concrete(frame_type)))
            .instruction(&Instruction::Call(poll))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::Br(2))
            .instruction(&Instruction::End);

        let mut emit_completion = |function: &mut Function| {
            if let Some((completion_field, completion_type)) = completion {
                emit_future(function);
                function.instruction(&Instruction::RefCastNonNull(HeapType::Concrete(frame_type)));
                emit_frame_typed_struct_get(
                    function,
                    frame_type,
                    completion_field,
                    completion_type,
                    context.gc,
                );
                completion_type
            } else {
                emit_default(function, Type::None, context.gc);
                Type::None
            }
        };
        match destination {
            FuturePollDestination::Frame {
                destination,
                layout,
            } => {
                if let Some((destination_field, destination_type)) = layout.field(destination) {
                    parent.emit(function);
                    let completion_type = emit_completion(function);
                    debug_assert_eq!(destination_type, completion_type);
                    function.instruction(&Instruction::StructSet {
                        struct_type_index: parent.struct_type,
                        field_index: destination_field,
                    });
                }
            }
            FuturePollDestination::Local { local, ty } => {
                let completion_type = emit_completion(function);
                debug_assert_eq!(ty, completion_type);
                function.instruction(&Instruction::LocalSet(local));
            }
            FuturePollDestination::Discard => {}
        }

        function
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::Br(1))
            .instruction(&Instruction::End);
    }
    function
        .instruction(&Instruction::Unreachable)
        .instruction(&Instruction::End);
}

fn clear_child_future(
    function: &mut Function,
    parent: AsyncFrameRef,
    field: u32,
    future: crate::ast::AsyncTypeId,
    context: &ExprContext<'_>,
) {
    parent.emit(function);
    function
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            context.gc.index(Type::Async(future)),
        )))
        .instruction(&Instruction::StructSet {
            struct_type_index: parent.struct_type,
            field_index: field,
        });
}

fn emit_module_name_value(
    function: &mut Function,
    module_name: Option<&str>,
    context: &ExprContext<'_>,
) {
    if let Some(name) = module_name {
        emit_string_literal(function, name, context.gc);
        return;
    }

    let names = provider_process_names(context);
    debug_assert!(!names.is_empty());
    let string_type = context.gc.val_type(Type::Standard(StdlibTypeId::String));
    for (index, name) in names.iter().enumerate() {
        function
            .instruction(&Instruction::GlobalGet(
                context.runtime_globals.process_name,
            ))
            .instruction(&Instruction::I32Const(index as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Result(string_type)));
        emit_string_literal(function, name, context.gc);
        function.instruction(&Instruction::Else);
    }
    function.instruction(&Instruction::Unreachable);
    for _ in names {
        function.instruction(&Instruction::End);
    }
}

fn emit_process_module_query(
    function: &mut Function,
    target: &wasm_ir::CallTarget,
    module_name: Option<&str>,
    import: AbiImportId,
    abi: &Abi,
    strings: &StringPool,
    context: &ExprContext<'_>,
) {
    if let Some(name) = module_name {
        let (ptr, len) = strings.get(name);
        compile_receiver(function, target, context);
        function
            .instruction(&Instruction::I32Const(ptr as i32))
            .instruction(&Instruction::I32Const(len as i32))
            .instruction(&Instruction::Call(abi.function(import)));
        return;
    }

    let names = provider_process_names(context);
    debug_assert!(!names.is_empty());
    for (index, name) in names.iter().enumerate() {
        function
            .instruction(&Instruction::GlobalGet(
                context.runtime_globals.process_name,
            ))
            .instruction(&Instruction::I32Const(index as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Result(ValType::I64)));
        let (ptr, len) = strings.get(name);
        compile_receiver(function, target, context);
        function
            .instruction(&Instruction::I32Const(ptr as i32))
            .instruction(&Instruction::I32Const(len as i32))
            .instruction(&Instruction::Call(abi.function(import)))
            .instruction(&Instruction::Else);
    }
    function.instruction(&Instruction::Unreachable);
    for _ in names {
        function.instruction(&Instruction::End);
    }
}

fn provider_process_names<'a>(context: &'a ExprContext<'_>) -> Vec<&'a str> {
    context.process_names.to_vec()
}

fn compile_retry_poll(
    function: &mut Function,
    destination: wasm_ir::SuspensionDestination,
    expression: ExprId,
    frame: &AsyncFrameLayout,
    context: &ExprContext<'_>,
) {
    let expression_type = context
        .wasm_ir
        .expression(expression)
        .expect("retried expression belongs to Wasm IR")
        .ty;
    let TypeKind::Result {
        layout: result,
        value: result_value,
    } = context.semantics.types().kind(expression_type)
    else {
        unreachable!("type checking only permits retrying Result expressions")
    };
    let result_local = context.matches.suspension_temps[&expression];

    compile_expr(function, expression, context);
    function
        .instruction(&Instruction::LocalTee(result_local))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(
        function,
        context.gc.index(Type::Result(*result)),
        1,
        Type::I32,
    );
    function
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    if let Some((field, stored_type)) = frame.field(destination) {
        debug_assert_eq!(stored_type, semantic_type(*result_value, context.semantics));
        context.locals.frame().emit(function);
        function
            .instruction(&Instruction::LocalGet(result_local))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(
            function,
            context.gc.index(Type::Result(*result)),
            0,
            stored_type,
        );
        function.instruction(&Instruction::StructSet {
            struct_type_index: context.locals.frame().struct_type,
            field_index: field,
        });
    }
}

#[derive(Clone, Copy)]
enum AsyncState<'a> {
    Block {
        block: &'a wasm_ir::Block,
        loop_targets: Option<AsyncLoopTargets>,
        resume_source: Option<crate::ast::Span>,
    },
    ForHeader {
        binding: ValueId,
        pattern: &'a wasm_ir::LoweredPattern,
        iterable_value: ValueId,
        index_value: ValueId,
        version_value: ValueId,
        iterator_step: Option<ExprId>,
        body: &'a wasm_ir::Block,
        header_state: wasm_ir::AsyncStateId,
        exit_state: wasm_ir::AsyncStateId,
    },
    Poll {
        mode: SuspensionMode,
        destination: wasm_ir::SuspensionDestination,
        value: ExprId,
        resume_state: wasm_ir::AsyncStateId,
        cancellation: Option<wasm_ir::CancellationRegion>,
        source: Option<crate::ast::Span>,
    },
}

#[derive(Clone, Copy)]
struct AsyncLoopTargets {
    break_state: wasm_ir::AsyncStateId,
    continue_state: wasm_ir::AsyncStateId,
    break_destination: Option<wasm_ir::TemporaryId>,
}

impl AsyncLoopTargets {
    fn control(self, dispatcher_depth: u32) -> LoopControl {
        LoopControl::Async {
            break_state: self.break_state,
            continue_state: self.continue_state,
            dispatcher_depth,
            break_destination: self.break_destination,
        }
    }
}

fn collect_async_states<'a>(
    block: &'a wasm_ir::Block,
    states: &mut [Option<AsyncState<'a>>],
    loop_targets: Option<AsyncLoopTargets>,
) {
    for statement in &block.statements {
        match statement {
            wasm_ir::Statement::If {
                then_block,
                else_block,
                ..
            } => {
                collect_async_states(then_block, states, loop_targets);
                collect_async_states(else_block, states, loop_targets);
            }
            wasm_ir::Statement::Match { arms, .. } => {
                for arm in arms {
                    collect_async_states(&arm.block, states, loop_targets);
                }
            }
            wasm_ir::Statement::Fallback {
                fallback_block,
                success_block,
                ..
            } => {
                collect_async_states(fallback_block, states, loop_targets);
                collect_async_states(success_block, states, loop_targets);
            }
            wasm_ir::Statement::While { body, .. } => {
                collect_async_states(body, states, loop_targets)
            }
            wasm_ir::Statement::For { body, .. } => {
                collect_async_states(body, states, loop_targets)
            }
            wasm_ir::Statement::Store { .. }
            | wasm_ir::Statement::BindPattern { .. }
            | wasm_ir::Statement::StateStore { .. }
            | wasm_ir::Statement::DebugLocation(_)
            | wasm_ir::Statement::StoreTemporary { .. }
            | wasm_ir::Statement::IndexStore { .. }
            | wasm_ir::Statement::Evaluate { .. }
            | wasm_ir::Statement::ForInit { .. } => {}
        }
    }
    match &block.terminator {
        wasm_ir::Terminator::Yield {
            resume_state,
            continuation,
            ..
        } => {
            states[resume_state.index() as usize] = Some(AsyncState::Block {
                block: continuation,
                loop_targets,
                resume_source: None,
            });
            collect_async_states(continuation, states, loop_targets);
        }
        wasm_ir::Terminator::Suspend {
            mode,
            destination,
            value,
            poll_state,
            resume_state,
            cancellation,
            source,
            continuation,
            ..
        } => {
            states[poll_state.index() as usize] = Some(AsyncState::Poll {
                mode: *mode,
                destination: *destination,
                value: *value,
                resume_state: *resume_state,
                cancellation: *cancellation,
                source: *source,
            });
            states[resume_state.index() as usize] = Some(AsyncState::Block {
                block: continuation,
                loop_targets,
                resume_source: *source,
            });
            collect_async_states(continuation, states, loop_targets);
        }
        wasm_ir::Terminator::Retry {
            attempt,
            continuation,
            source,
            poll_state,
            resume_state,
            ..
        } => {
            states[poll_state.index() as usize] = Some(AsyncState::Block {
                block: attempt,
                loop_targets,
                resume_source: *source,
            });
            states[resume_state.index() as usize] = Some(AsyncState::Block {
                block: continuation,
                loop_targets,
                resume_source: *source,
            });
            collect_async_states(attempt, states, loop_targets);
            collect_async_states(continuation, states, loop_targets);
        }
        wasm_ir::Terminator::AsyncWhile {
            header,
            continuation,
            header_state,
            exit_state,
            result,
        } => {
            let inner_targets = AsyncLoopTargets {
                break_state: *exit_state,
                continue_state: *header_state,
                break_destination: *result,
            };
            states[header_state.index() as usize] = Some(AsyncState::Block {
                block: header,
                loop_targets: Some(inner_targets),
                resume_source: None,
            });
            states[exit_state.index() as usize] = Some(AsyncState::Block {
                block: continuation,
                loop_targets,
                resume_source: None,
            });
            collect_async_states(header, states, Some(inner_targets));
            collect_async_states(continuation, states, loop_targets);
        }
        wasm_ir::Terminator::AsyncWhileCondition { body, .. } => {
            collect_async_states(body, states, loop_targets);
        }
        wasm_ir::Terminator::AsyncFor {
            binding,
            pattern,
            iterable_value,
            index_value,
            version_value,
            iterator_step,
            body,
            continuation,
            header_state,
            exit_state,
        } => {
            let inner_targets = AsyncLoopTargets {
                break_state: *exit_state,
                continue_state: *header_state,
                break_destination: None,
            };
            states[header_state.index() as usize] = Some(AsyncState::ForHeader {
                binding: *binding,
                pattern,
                iterable_value: *iterable_value,
                index_value: *index_value,
                version_value: *version_value,
                iterator_step: *iterator_step,
                body,
                header_state: *header_state,
                exit_state: *exit_state,
            });
            states[exit_state.index() as usize] = Some(AsyncState::Block {
                block: continuation,
                loop_targets,
                resume_source: None,
            });
            collect_async_states(body, states, Some(inner_targets));
            collect_async_states(continuation, states, loop_targets);
        }
        wasm_ir::Terminator::Fallthrough
        | wasm_ir::Terminator::Break(_)
        | wasm_ir::Terminator::Continue
        | wasm_ir::Terminator::Return(_)
        | wasm_ir::Terminator::RetryComplete { .. }
        | wasm_ir::Terminator::Throw { .. } => {}
    }
}

fn set_async_state(function: &mut Function, state: wasm_ir::AsyncStateId, frame: AsyncFrameRef) {
    frame.emit(function);
    function
        .instruction(&Instruction::I32Const(state.index() as i32))
        .instruction(&Instruction::StructSet {
            struct_type_index: frame.struct_type,
            field_index: 0,
        });
}

/// Returns whether execution may reach the end of the emitted block. This is
/// tracked during emission so callers can omit dead completion/loop-back code.
#[must_use]
#[allow(clippy::too_many_arguments)]
fn compile_async_flow(
    function: &mut Function,
    block: &wasm_ir::Block,
    loop_depth: u32,
    loop_control: Option<LoopControl>,
    result_global: Option<u32>,
    cancellation_region: Option<wasm_ir::CancellationRegion>,
    layout: &AsyncFrameLayout,
    context: &ExprContext<'_>,
) -> bool {
    let expression_context = ExprContext {
        loop_control,
        ..*context
    };
    let context = &expression_context;
    for statement in &block.statements {
        match statement {
            wasm_ir::Statement::DebugLocation(source) => {
                if let Some(debug) = context.debug {
                    debug.mark(function, Some(*source));
                }
            }
            wasm_ir::Statement::Store {
                target,
                declaration,
                operation,
                value,
            } => {
                compile_assignment(
                    function,
                    *target,
                    *declaration,
                    operation.as_ref(),
                    *value,
                    context,
                );
            }
            wasm_ir::Statement::BindPattern { value, pattern } => {
                compile_irrefutable_pattern(function, pattern, *value, context);
            }
            wasm_ir::Statement::StateStore {
                target,
                operation,
                value,
            } => super::expression::compile_state_assignment(
                function,
                *target,
                operation.as_ref(),
                *value,
                context,
            ),
            wasm_ir::Statement::StoreTemporary { target, value } => {
                compile_temporary_set(function, *target, *value, context);
            }
            wasm_ir::Statement::IndexStore {
                target,
                operation,
                value,
            } => super::expression::compile_index_assignment(
                function, *target, operation, *value, context,
            ),
            wasm_ir::Statement::If {
                condition,
                then_block,
                else_block,
            } => {
                compile_expr(function, *condition, context);
                function.instruction(&Instruction::If(BlockType::Empty));
                let then_falls_through = compile_async_flow(
                    function,
                    then_block,
                    loop_depth + 1,
                    loop_control.map(|control| control.nested(1)),
                    result_global,
                    cancellation_region,
                    layout,
                    context,
                );
                function.instruction(&Instruction::Else);
                let else_falls_through = compile_async_flow(
                    function,
                    else_block,
                    loop_depth + 1,
                    loop_control.map(|control| control.nested(1)),
                    result_global,
                    cancellation_region,
                    layout,
                    context,
                );
                function.instruction(&Instruction::End);
                if !then_falls_through && !else_falls_through {
                    return false;
                }
            }
            wasm_ir::Statement::Match {
                expression,
                value,
                arms,
            } => {
                let value_local = context.matches.values[expression];
                let value_type = context.expression_type(*value);
                compile_expr(function, *value, context);
                function.instruction(&Instruction::LocalSet(value_local));
                let mut falls_through = false;
                for (arm_index, arm) in arms.iter().enumerate() {
                    let bindings = compile_statement_pattern(
                        function,
                        &arm.pattern,
                        value_local,
                        value_type,
                        context,
                    );
                    let arm_context = ExprContext {
                        loop_control: loop_control
                            .map(|control| control.nested(arm_index as u32 + 1)),
                        ..*context
                    };
                    if !bindings.is_empty() || arm.guard.is_some() {
                        function.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
                        for binding in bindings {
                            store_match_binding(function, binding, &arm_context);
                        }
                        if let Some(guard) = arm.guard {
                            compile_expr(function, guard, &arm_context);
                        } else {
                            function.instruction(&Instruction::I32Const(1));
                        }
                        function
                            .instruction(&Instruction::Else)
                            .instruction(&Instruction::I32Const(0))
                            .instruction(&Instruction::End);
                    }
                    function.instruction(&Instruction::If(BlockType::Empty));
                    falls_through |= compile_async_flow(
                        function,
                        &arm.block,
                        loop_depth + arm_index as u32 + 1,
                        arm_context.loop_control,
                        result_global,
                        cancellation_region,
                        layout,
                        &arm_context,
                    );
                    function.instruction(&Instruction::Else);
                }
                function.instruction(&Instruction::Unreachable);
                for _ in arms {
                    function.instruction(&Instruction::End);
                }
                if !falls_through {
                    return false;
                }
            }
            wasm_ir::Statement::Fallback {
                expression,
                value,
                fallback_block,
                success_block,
            } => {
                compile_fallback_condition(function, *expression, *value, context);
                function.instruction(&Instruction::If(BlockType::Empty));
                let fallback_falls_through = compile_async_flow(
                    function,
                    fallback_block,
                    loop_depth + 1,
                    loop_control.map(|control| control.nested(1)),
                    result_global,
                    cancellation_region,
                    layout,
                    context,
                );
                function.instruction(&Instruction::Else);
                let success_falls_through = compile_async_flow(
                    function,
                    success_block,
                    loop_depth + 1,
                    loop_control.map(|control| control.nested(1)),
                    result_global,
                    cancellation_region,
                    layout,
                    context,
                );
                function.instruction(&Instruction::End);
                if !fallback_falls_through && !success_falls_through {
                    return false;
                }
            }
            wasm_ir::Statement::While {
                condition,
                body,
                result,
            } => {
                function
                    .instruction(&Instruction::Block(BlockType::Empty))
                    .instruction(&Instruction::Loop(BlockType::Empty));
                compile_expr(function, *condition, context);
                function
                    .instruction(&Instruction::I32Eqz)
                    .instruction(&Instruction::BrIf(1));
                let body_falls_through = compile_async_flow(
                    function,
                    body,
                    loop_depth + 2,
                    Some(LoopControl::Branch {
                        break_depth: 1,
                        continue_depth: 0,
                        break_destination: *result,
                    }),
                    result_global,
                    cancellation_region,
                    layout,
                    context,
                );
                if body_falls_through {
                    function.instruction(&Instruction::Br(0));
                }
                function
                    .instruction(&Instruction::End)
                    .instruction(&Instruction::End);
            }
            wasm_ir::Statement::For {
                binding,
                pattern,
                iterable_value,
                index_value,
                version_value,
                iterable,
                iterator_step,
                body,
            } => {
                compile_for_init(
                    function,
                    *iterable_value,
                    *index_value,
                    *version_value,
                    *iterable,
                    context,
                );
                function
                    .instruction(&Instruction::Block(BlockType::Empty))
                    .instruction(&Instruction::Loop(BlockType::Empty));
                compile_for_has_next(
                    function,
                    *iterable_value,
                    *index_value,
                    *version_value,
                    *iterator_step,
                    context,
                );
                function
                    .instruction(&Instruction::I32Eqz)
                    .instruction(&Instruction::BrIf(1));
                compile_for_bind_and_advance(
                    function,
                    *binding,
                    *iterable_value,
                    *index_value,
                    *version_value,
                    context,
                );
                compile_irrefutable_pattern(function, pattern, *binding, context);
                let body_falls_through = compile_async_flow(
                    function,
                    body,
                    loop_depth + 2,
                    Some(LoopControl::Branch {
                        break_depth: 1,
                        continue_depth: 0,
                        break_destination: None,
                    }),
                    result_global,
                    cancellation_region,
                    layout,
                    context,
                );
                if body_falls_through {
                    function.instruction(&Instruction::Br(0));
                }
                function
                    .instruction(&Instruction::End)
                    .instruction(&Instruction::End);
            }
            wasm_ir::Statement::ForInit {
                iterable_value,
                index_value,
                version_value,
                iterable,
                ..
            } => compile_for_init(
                function,
                *iterable_value,
                *index_value,
                *version_value,
                *iterable,
                context,
            ),
            wasm_ir::Statement::Evaluate {
                expression,
                discard_result,
            } => {
                compile_expr(function, *expression, context);
                if *discard_result && context.expression_type(*expression) != Type::Never {
                    function.instruction(&Instruction::Drop);
                }
            }
        }
    }
    match &block.terminator {
        wasm_ir::Terminator::Fallthrough => return true,
        wasm_ir::Terminator::Break(value) => {
            let control = loop_control.expect("checked break expressions belong to loops");
            if let Some(value) = value {
                if let Some(destination) = control.break_destination() {
                    compile_temporary_set(function, destination, *value, context);
                } else {
                    compile_expr(function, *value, &context.erasing_none());
                    if context.expression_type(*value).has_runtime_value() {
                        function.instruction(&Instruction::Drop);
                    }
                }
            }
            control.emit_break(function, context.locals.continuation_frame());
        }
        wasm_ir::Terminator::Continue => {
            loop_control
                .expect("checked continue expressions belong to loops")
                .emit_continue(function, context.locals.continuation_frame());
        }
        wasm_ir::Terminator::AsyncWhile { header_state, .. } => {
            set_async_state(function, *header_state, context.locals.frame());
            function.instruction(&Instruction::Br(loop_depth));
        }
        wasm_ir::Terminator::AsyncWhileCondition {
            condition,
            body,
            exit_state,
            ..
        } => {
            compile_expr(function, *condition, context);
            function.instruction(&Instruction::If(BlockType::Empty));
            let body_falls_through = compile_async_flow(
                function,
                body,
                loop_depth + 1,
                loop_control.map(|control| control.nested(1)),
                result_global,
                cancellation_region,
                layout,
                context,
            );
            function.instruction(&Instruction::Else);
            set_async_state(function, *exit_state, context.locals.frame());
            function
                .instruction(&Instruction::Br(loop_depth + 1))
                .instruction(&Instruction::End);
            return body_falls_through;
        }
        wasm_ir::Terminator::AsyncFor { header_state, .. } => {
            set_async_state(function, *header_state, context.locals.frame());
            function.instruction(&Instruction::Br(loop_depth));
        }
        wasm_ir::Terminator::Return(value) => {
            match context.bare_return {
                BareReturn::AsyncFuture { frame, completion } => {
                    if let Some(value) = value {
                        if let Some((field, _)) = completion {
                            frame.emit(function);
                            compile_expr(function, *value, context);
                            function.instruction(&Instruction::StructSet {
                                struct_type_index: frame.struct_type,
                                field_index: field,
                            });
                        } else {
                            compile_expr(function, *value, &context.erasing_none());
                        }
                    }
                }
                BareReturn::AsyncAction {
                    action,
                    result_global: _,
                } => {
                    if let Some(global) = result_global {
                        if let Some(value) = value {
                            compile_expr(function, *value, context);
                        } else {
                            super::script_functions::emit_action_default(
                                function,
                                action,
                                context.semantics,
                                context.gc,
                            );
                        }
                        function.instruction(&Instruction::GlobalSet(global));
                    } else {
                        debug_assert!(value.is_none());
                    }
                }
                BareReturn::Generator { .. } => {
                    if let Some(value) = value {
                        debug_assert_eq!(
                            context.expression_type(*value),
                            Type::None,
                            "generator returns cannot carry values"
                        );
                        compile_expr(function, *value, &context.erasing_none());
                    }
                }
                BareReturn::None | BareReturn::Action(_) => {
                    unreachable!("direct bodies do not use the async state emitter")
                }
            }
            // Explicit action returns already stored either their value or the
            // action's source-level bare-return default above. Only lexical
            // fallthrough applies the continuation default here; doing so a
            // second time would overwrite `return false` in `whileAttached`.
            emit_continuation_return(function, context.bare_return, context.gc);
        }
        wasm_ir::Terminator::Yield {
            value,
            resume_state,
            ..
        } => {
            let BareReturn::Generator { step, .. } = context.bare_return else {
                unreachable!("yield terminators only occur in generator continuations")
            };
            set_async_state(function, *resume_state, context.locals.frame());
            compile_expr(function, *value, context);
            function
                .instruction(&Instruction::StructNew(context.gc.index(step)))
                .instruction(&Instruction::Return);
        }
        wasm_ir::Terminator::Suspend {
            mode,
            value,
            poll_state,
            cancellation,
            source,
            ..
        } => {
            if call_target(context.wasm_ir, *value)
                .and_then(resolved_intrinsic)
                .is_some()
            {
                assert_eq!(
                    *cancellation, cancellation_region,
                    "awaited standard-library operation must participate in its body's cancellation region"
                );
            }
            if let Some(debug) = context.debug {
                debug.mark_suspend(function, *source);
            }
            set_async_state(function, *poll_state, context.locals.frame());
            if *mode == SuspensionMode::Await
                && call_target(context.wasm_ir, *value).and_then(resolved_intrinsic)
                    == Some(IntrinsicId::NextTick)
            {
                function
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::Return);
            } else {
                // The dispatcher owns the one canonical copy of every poll.
                // Redispatch immediately for a first poll in this tick rather
                // than inlining the same operation here and again in its poll
                // state for later ticks.
                function.instruction(&Instruction::Br(loop_depth));
            }
        }
        wasm_ir::Terminator::Retry {
            poll_state,
            cancellation,
            source,
            ..
        } => {
            assert_eq!(
                *cancellation, cancellation_region,
                "retry must participate in its body's cancellation region"
            );
            if let Some(debug) = context.debug {
                debug.mark_suspend(function, *source);
            }
            set_async_state(function, *poll_state, context.locals.frame());
            // Retry attempts are ordinary dispatcher states. Enter that state
            // in the current tick instead of duplicating its complete block at
            // every syntactic retry boundary.
            function.instruction(&Instruction::Br(loop_depth));
        }
        wasm_ir::Terminator::RetryComplete {
            value,
            destination,
            resume_state,
        } => {
            compile_retry_poll(function, *destination, *value, layout, context);
            set_async_state(function, *resume_state, context.locals.frame());
            function.instruction(&Instruction::Br(loop_depth));
        }
        wasm_ir::Terminator::Throw { error, target } => match target {
            crate::hir::FailureTarget::Return(target) => {
                let Type::Result(target_result) = context.ty(*target) else {
                    unreachable!("throw targets are result values")
                };
                emit_failure_return(
                    function,
                    target_result,
                    context,
                    error_may_have_effects(*error, context),
                    |function| {
                        compile_expr(function, *error, context);
                    },
                );
            }
            crate::hir::FailureTarget::Retry { .. } => {
                compile_expr(function, *error, context);
                function
                    .instruction(&Instruction::Drop)
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::Return);
            }
        },
    }
    false
}

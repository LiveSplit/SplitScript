//! Per-tick process, snapshot, lifecycle-action, and timer runtime emission.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, HeapType, Instruction, RefType, ValType};

use crate::{
    abi::AbiImportId,
    ast::{ActionKind, Program, StateField, ValueId},
    semantic::SemanticModel,
    stdlib::{
        CoreTypeId, RuntimeRepresentation, StdlibFieldId, StdlibStateProvider,
        StdlibStateProviderId, StdlibTypeId,
    },
    wasm_ir,
};

use super::{
    GcLayout, STATE_TYPE, Type,
    data_plan::StringPool,
    emit_result_error, emit_typed_struct_get,
    global_plan::{ATTACH_PREPARED, ATTACH_READY, ATTACH_REJECTED, RuntimeGlobals},
    imports::Abi,
    managed_state_reads::ManagedStateReadCache,
    memarg,
    memory_plan::RuntimeScratch,
    pointer_prefixes::{
        PointerPrefixPlan, PrefixEmissionContext, PrefixEmissionState, PrefixLocals,
    },
    semantic_type, state_storage_index, struct_field_type, value_type,
};

/// Per-tick runtime view of the completed backend plans.
pub(super) struct UpdateContext<'a> {
    pub standard_library: &'a crate::stdlib::StandardLibrary,
    pub abi: &'a Abi,
    pub gc: &'a GcLayout,
    pub failure_payloads: &'a super::failure_payload::FailurePayloadDemand,
    pub runtime_helpers: &'a super::RuntimeHelperPlan,
    pub runtime_globals: RuntimeGlobals,
    pub provider_values: &'a HashMap<StdlibStateProviderId, u32>,
    pub semantics: &'a crate::semantic::SemanticModel,
    pub managed: &'a crate::managed::ManagedBindingPlan,
    pub managed_state_reads: &'a ManagedStateReadCache,
    pub pointer_prefixes: &'a PointerPrefixPlan,
    pub scratch: RuntimeScratch,
    pub explicit_shape_selection: bool,
    pub globals: &'a HashMap<ValueId, u32>,
    pub global_types: &'a HashMap<ValueId, Type>,
    pub attachment_globals: &'a [ValueId],
    pub attempt_globals: &'a [ValueId],
    pub scoped_globals: &'a crate::ScopedGlobalAnalysis,
    pub process_names: &'a [&'a str],
    pub provider_attach: Option<ProviderAttach>,
    pub provider_alternatives: &'a [ProviderAlternative<'a>],
    pub provider_preparation: Option<ProviderPreparation>,
}

pub(super) struct ProviderAlternative<'a> {
    pub provider: StdlibStateProviderId,
    pub declaration: &'a StdlibStateProvider,
    pub source_processes: &'a [String],
    pub attachment: Option<ProviderAttach>,
}

#[derive(Clone, Copy)]
pub(super) struct ProviderAttach {
    pub init: u32,
    pub poll: u32,
    pub frame_global: u32,
    pub frame_type: u32,
    pub completion_field: u32,
    /// Synchronous provider-owned mapping validation. When it returns false,
    /// the logical attachment ends and ordinary discovery starts again while
    /// retaining the still-open host process.
    pub validation: Option<u32>,
}

#[derive(Clone, Copy)]
struct ProviderValidation {
    function: u32,
    provider_global: u32,
}

#[derive(Clone, Copy)]
pub(super) struct ProviderPreparation {
    pub init: u32,
    pub poll: u32,
    pub frame_global: u32,
    pub frame_type: u32,
    pub completion_field: u32,
    pub value_global: u32,
    pub value_type: Type,
    pub ready_global: u32,
}

pub(super) struct StatePollFunctions<'a> {
    pub reads: &'a [u32],
    pub transforms: &'a [Option<u32>],
}

#[derive(Clone, Copy)]
pub(super) enum PredicateState {
    Unavailable,
    Local(u32),
    Global(u32),
}

#[derive(Clone, Copy)]
struct StateFieldPoll {
    field: ValueId,
    read_function: u32,
    transform_function: Option<u32>,
    poll_result_local: u32,
}

struct SnapshotPollContext<'a> {
    program: &'a Program,
    candidate_state: u32,
    poll_result_locals: HashMap<ValueId, u32>,
    pointer_prefix_locals: &'a PrefixLocals,
    pointer_emission: PrefixEmissionContext<'a>,
    lowering: &'a UpdateContext<'a>,
}

#[derive(Clone, Copy)]
struct ProcessSelectionLocals {
    result: u32,
    capacity: u32,
    count: u32,
    index: u32,
    required_pages: u32,
    pid: u32,
}

fn emit_process_attachment(
    function: &mut Function,
    strings: &StringPool,
    actions: &HashMap<ActionKind, u32>,
    selection_locals: Option<ProcessSelectionLocals>,
    newly_attached: u32,
    lowering: &UpdateContext<'_>,
) {
    let Some(selector) = actions.get(&ActionKind::SelectProcess).copied() else {
        emit_default_process_attachment(function, strings, newly_attached, lowering);
        return;
    };
    emit_selected_process_attachment(
        function,
        strings,
        selector,
        selection_locals.expect("selectProcess has selection locals"),
        newly_attached,
        lowering,
    );
}

fn emit_default_process_attachment(
    function: &mut Function,
    strings: &StringPool,
    newly_attached: u32,
    lowering: &UpdateContext<'_>,
) {
    let globals = lowering.runtime_globals;
    function
        .instruction(&Instruction::GlobalGet(globals.process))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    for (process_index, process) in lowering.process_names.iter().enumerate() {
        let (process_ptr, process_len) = strings.get(process);
        function
            .instruction(&Instruction::GlobalGet(globals.process))
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(process_ptr as i32))
            .instruction(&Instruction::I32Const(process_len as i32))
            .instruction(&Instruction::Call(
                lowering.abi.function(AbiImportId::ProcessAttach),
            ))
            .instruction(&Instruction::GlobalSet(globals.process))
            .instruction(&Instruction::GlobalGet(globals.process))
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Else)
            .instruction(&Instruction::I32Const(process_index as i32))
            .instruction(&Instruction::GlobalSet(globals.process_name))
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::LocalSet(newly_attached))
            .instruction(&Instruction::End)
            .instruction(&Instruction::End);
    }
    function.instruction(&Instruction::End);
}

fn emit_selected_process_attachment(
    function: &mut Function,
    strings: &StringPool,
    selector: u32,
    locals: ProcessSelectionLocals,
    newly_attached: u32,
    lowering: &UpdateContext<'_>,
) {
    let globals = lowering.runtime_globals;
    let abi = lowering.abi;
    let length_pointer = lowering.scratch.settings_length.start();
    let list_pointer = lowering.scratch.host_strings_start;
    // The zero-capacity sizing call still receives a non-null aligned
    // one-past pointer, matching Rust's empty-slice validity requirements.
    let empty_list_pointer = list_pointer;
    // Reserve enough headroom for the page-rounding calculation below.
    let maximum_count = (u32::MAX - list_pointer as u32) / 8;
    let result = lowering
        .semantics
        .action_result(ActionKind::SelectProcess)
        .expect("checked selectProcess has a result type");
    let crate::types::TypeKind::Result { layout, .. } = lowering.semantics.types().kind(result)
    else {
        unreachable!("selectProcess has a fallible boolean ABI result")
    };
    let result_struct = lowering.gc.index(Type::Result(*layout));

    function
        .instruction(&Instruction::GlobalGet(globals.process))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    for (process_index, process) in lowering.process_names.iter().enumerate() {
        let (process_ptr, process_len) = strings.get(process);
        function
            .instruction(&Instruction::GlobalGet(globals.process))
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            // First query only obtains the complete candidate count.
            .instruction(&Instruction::I32Const(length_pointer))
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::I32Store(memarg()))
            .instruction(&Instruction::I32Const(process_ptr as i32))
            .instruction(&Instruction::I32Const(process_len as i32))
            .instruction(&Instruction::I32Const(empty_list_pointer))
            .instruction(&Instruction::I32Const(length_pointer))
            .instruction(&Instruction::Call(
                abi.function(AbiImportId::ProcessListByName),
            ))
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(length_pointer))
            .instruction(&Instruction::I32Load(memarg()))
            .instruction(&Instruction::LocalTee(locals.capacity))
            .instruction(&Instruction::I32Const(maximum_count as i32))
            .instruction(&Instruction::I32LeU)
            .instruction(&Instruction::If(BlockType::Empty))
            // Grow the unbounded host staging area to hold every returned PID.
            .instruction(&Instruction::LocalGet(locals.capacity))
            .instruction(&Instruction::I32Const(3))
            .instruction(&Instruction::I32Shl)
            .instruction(&Instruction::I32Const(list_pointer))
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::I32Sub)
            .instruction(&Instruction::I32Const(16))
            .instruction(&Instruction::I32ShrU)
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::LocalTee(locals.required_pages))
            .instruction(&Instruction::MemorySize(0))
            .instruction(&Instruction::I32GtU)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::LocalGet(locals.required_pages))
            .instruction(&Instruction::MemorySize(0))
            .instruction(&Instruction::I32Sub)
            .instruction(&Instruction::MemoryGrow(0))
            .instruction(&Instruction::I32Const(-1))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::End)
            // Re-query into the correctly sized buffer. If the process set
            // grew between calls, ignore the partial list until the next tick.
            .instruction(&Instruction::I32Const(length_pointer))
            .instruction(&Instruction::LocalGet(locals.capacity))
            .instruction(&Instruction::I32Store(memarg()))
            .instruction(&Instruction::I32Const(process_ptr as i32))
            .instruction(&Instruction::I32Const(process_len as i32))
            .instruction(&Instruction::I32Const(list_pointer))
            .instruction(&Instruction::I32Const(length_pointer))
            .instruction(&Instruction::Call(
                abi.function(AbiImportId::ProcessListByName),
            ))
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(length_pointer))
            .instruction(&Instruction::I32Load(memarg()))
            .instruction(&Instruction::LocalTee(locals.count))
            .instruction(&Instruction::LocalGet(locals.capacity))
            .instruction(&Instruction::I32LeU)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::LocalSet(locals.index))
            .instruction(&Instruction::Block(BlockType::Empty))
            .instruction(&Instruction::Loop(BlockType::Empty))
            .instruction(&Instruction::GlobalGet(globals.process))
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::BrIf(1))
            .instruction(&Instruction::LocalGet(locals.index))
            .instruction(&Instruction::LocalGet(locals.count))
            .instruction(&Instruction::I32GeU)
            .instruction(&Instruction::BrIf(1))
            .instruction(&Instruction::I32Const(list_pointer))
            .instruction(&Instruction::LocalGet(locals.index))
            .instruction(&Instruction::I32Const(3))
            .instruction(&Instruction::I32Shl)
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::I64Load(memarg()))
            .instruction(&Instruction::LocalSet(locals.pid))
            .instruction(&Instruction::LocalGet(locals.pid))
            .instruction(&Instruction::Call(
                abi.function(AbiImportId::ProcessAttachByPid),
            ))
            .instruction(&Instruction::GlobalSet(globals.process))
            .instruction(&Instruction::GlobalGet(globals.process))
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Else)
            .instruction(&Instruction::I32Const(process_index as i32))
            .instruction(&Instruction::GlobalSet(globals.process_name))
            .instruction(&Instruction::Call(selector))
            .instruction(&Instruction::LocalSet(locals.result))
            // Both an uncaught error and `false` reject only this candidate.
            .instruction(&Instruction::LocalGet(locals.result))
            .instruction(&Instruction::StructGet {
                struct_type_index: result_struct,
                field_index: 1,
            })
            .instruction(&Instruction::If(BlockType::Empty));
        emit_reject_process_candidate(function, lowering);
        function
            .instruction(&Instruction::Else)
            .instruction(&Instruction::LocalGet(locals.result))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(function, result_struct, 0, Type::Bool);
        function
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::LocalSet(newly_attached))
            .instruction(&Instruction::Else);
        emit_reject_process_candidate(function, lowering);
        function
            .instruction(&Instruction::End)
            .instruction(&Instruction::End)
            .instruction(&Instruction::End)
            .instruction(&Instruction::LocalGet(locals.index))
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::LocalSet(locals.index))
            .instruction(&Instruction::Br(0))
            .instruction(&Instruction::End)
            .instruction(&Instruction::End)
            .instruction(&Instruction::End)
            .instruction(&Instruction::End)
            .instruction(&Instruction::End)
            .instruction(&Instruction::End)
            .instruction(&Instruction::End);
    }
    function.instruction(&Instruction::End);
}

fn emit_reject_process_candidate(function: &mut Function, lowering: &UpdateContext<'_>) {
    let globals = lowering.runtime_globals;
    function
        .instruction(&Instruction::GlobalGet(globals.process))
        .instruction(&Instruction::Call(
            lowering.abi.function(AbiImportId::ProcessDetach),
        ))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::GlobalSet(globals.process))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::GlobalSet(globals.process_name));
}

fn emit_attached_tick_rate(
    function: &mut Function,
    program: &Program,
    lowering: &UpdateContext<'_>,
) {
    function
        .instruction(&Instruction::F64Const(program.attached_tick_rate().into()))
        .instruction(&Instruction::Call(
            lowering.abi.function(AbiImportId::RuntimeSetTickRate),
        ));
}

/// Ends one source-visible state-provider attachment. Emulator providers can
/// lose their guest-memory mapping while the host process remains open, so
/// process-handle teardown is deliberately separate from attachment-owned
/// state, continuations, lifecycle events, and polling policy.
fn emit_attachment_teardown(
    function: &mut Function,
    program: &Program,
    actions: &HashMap<ActionKind, u32>,
    cancellation_region: Option<wasm_ir::CancellationRegion>,
    attachment_transition: u32,
    detach_process: bool,
    lowering: &UpdateContext<'_>,
) {
    let globals = lowering.runtime_globals;
    let state = program.state.as_ref().expect("update requires state");

    if detach_process {
        function
            .instruction(&Instruction::GlobalGet(globals.process))
            .instruction(&Instruction::Call(
                lowering.abi.function(AbiImportId::ProcessDetach),
            ))
            .instruction(&Instruction::I64Const(0))
            .instruction(&Instruction::GlobalSet(globals.process))
            .instruction(&Instruction::I32Const(-1))
            .instruction(&Instruction::GlobalSet(globals.process_name));
        if let Some(pointer_size) = globals.process_pointer_size {
            function
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::GlobalSet(pointer_size));
        }
    }

    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::GlobalSet(globals.state_ready))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::GlobalSet(globals.attach_ready));
    if let Some(provider_global) = globals.provider_value {
        let provider_type = lowering
            .semantics
            .state_provider()
            .map(|provider| {
                lowering
                    .standard_library
                    .state_provider(provider)
                    .process_type
            })
            .expect("provider storage requires a resolved provider");
        emit_provider_default(function, provider_type, lowering);
        function.instruction(&Instruction::GlobalSet(provider_global));
    }
    for alternative in lowering.provider_alternatives {
        let Some(provider_global) = lowering.provider_values.get(&alternative.provider).copied()
        else {
            continue;
        };
        emit_provider_default(function, alternative.declaration.process_type, lowering);
        function.instruction(&Instruction::GlobalSet(provider_global));
    }
    if let (Some(frame_global), Some(ProviderAttach { frame_type, .. })) =
        (globals.provider_attachment_frame, lowering.provider_attach)
    {
        function
            .instruction(&Instruction::RefNull(HeapType::Concrete(frame_type)))
            .instruction(&Instruction::GlobalSet(frame_global));
    }
    for attachment in lowering
        .provider_alternatives
        .iter()
        .filter_map(|alternative| alternative.attachment)
    {
        function
            .instruction(&Instruction::RefNull(HeapType::Concrete(
                attachment.frame_type,
            )))
            .instruction(&Instruction::GlobalSet(attachment.frame_global));
    }
    if let Some(preparation) = lowering.provider_preparation {
        function
            .instruction(&Instruction::RefNull(HeapType::Concrete(
                preparation.frame_type,
            )))
            .instruction(&Instruction::GlobalSet(preparation.frame_global));
        emit_storage_default(function, lowering.gc.val_type(preparation.value_type));
        function
            .instruction(&Instruction::GlobalSet(preparation.value_global))
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::GlobalSet(preparation.ready_global));
    }
    if let (Some(selected), Some(provider_value)) =
        (globals.selected_provider, state.provider_value)
    {
        let provider_type = lowering.global_types[&provider_value];
        emit_storage_default(function, lowering.gc.val_type(provider_type));
        function.instruction(&Instruction::GlobalSet(selected));
    }
    for value in lowering.attachment_globals {
        let ty = lowering.global_types[value];
        if !ty.has_runtime_value() {
            continue;
        }
        emit_storage_default(function, lowering.gc.val_type(ty));
        function.instruction(&Instruction::GlobalSet(lowering.globals[value]));
    }
    if let Some(region) = cancellation_region {
        emit_cancel_region(function, region, lowering.gc, globals);
    }
    function
        .instruction(&Instruction::F64Const(program.detached_tick_rate().into()))
        .instruction(&Instruction::Call(
            lowering.abi.function(AbiImportId::RuntimeSetTickRate),
        ));
    if let Some(detach) = actions.get(&ActionKind::OnDetach) {
        function
            .instruction(&Instruction::LocalGet(attachment_transition))
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Call(*detach))
            .instruction(&Instruction::End);
    }
}

pub(super) fn compile_update(
    program: &Program,
    strings: &StringPool,
    state_functions: StatePollFunctions<'_>,
    actions: &HashMap<ActionKind, u32>,
    refresh_settings: Option<u32>,
    cancellation_region: Option<wasm_ir::CancellationRegion>,
    lowering: &UpdateContext<'_>,
) -> Function {
    let abi = lowering.abi;
    let globals = lowering.runtime_globals;
    let semantics = lowering.semantics;
    let has_game_time = actions.contains_key(&ActionKind::GameTime);
    let has_timer_lifecycle = globals.observed_timer_state.is_some();
    let has_timer_decisions = actions.keys().any(|action| {
        matches!(
            action,
            ActionKind::Start
                | ActionKind::Split
                | ActionKind::Reset
                | ActionKind::IsLoading
                | ActionKind::GameTime
        )
    });
    let timer_state = 0;
    let nullable_bool = 1;
    let attachment_transition = 2;
    let duration_local = 3;
    let candidate_state = if has_game_time { 4 } else { 3 };
    let first_poll_result = candidate_state + 1;
    let mut locals = vec![(3, ValType::I32)];
    if has_game_time {
        locals.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(lowering.gc.standard_index(StdlibTypeId::Duration)),
            }),
        ));
    }
    locals.push((
        1,
        ValType::Ref(RefType {
            nullable: true,
            heap_type: HeapType::Concrete(STATE_TYPE),
        }),
    ));
    let state = program.state.as_ref().unwrap();
    let all_fields = state.all_fields().collect::<Vec<_>>();
    let read_indices = all_fields
        .iter()
        .enumerate()
        .map(|(index, field)| (field.id, index))
        .collect::<HashMap<_, _>>();
    let poll_result_locals = read_indices
        .iter()
        .map(|(field, index)| (*field, first_poll_result + *index as u32))
        .collect();
    for field in &all_fields {
        let poll_result = semantic_type(
            semantics
                .state_poll_result(field.id)
                .expect("checked state fields have poll-result types"),
            semantics,
        );
        locals.push((1, lowering.gc.val_type(poll_result)));
    }
    let first_selection_local = first_poll_result + all_fields.len() as u32;
    let selection_locals = actions.contains_key(&ActionKind::SelectProcess).then(|| {
        let result = semantics
            .action_result(ActionKind::SelectProcess)
            .expect("checked selectProcess has a result type");
        locals.push((
            1,
            lowering
                .gc
                .val_type(semantic_type(result, lowering.semantics)),
        ));
        locals.push((4, ValType::I32));
        locals.push((1, ValType::I64));
        ProcessSelectionLocals {
            result: first_selection_local,
            capacity: first_selection_local + 1,
            count: first_selection_local + 2,
            index: first_selection_local + 3,
            required_pages: first_selection_local + 4,
            pid: first_selection_local + 5,
        }
    });
    let first_prefix_local = first_selection_local + u32::from(selection_locals.is_some()) * 6;
    let (prefix_local_declarations, pointer_prefix_locals) = lowering
        .pointer_prefixes
        .allocate_locals(first_prefix_local);
    locals.extend(prefix_local_declarations);
    let mut function = Function::new(locals);
    let snapshot_poll = SnapshotPollContext {
        program,
        candidate_state,
        poll_result_locals,
        pointer_prefix_locals: &pointer_prefix_locals,
        pointer_emission: PrefixEmissionContext {
            plan: lowering.pointer_prefixes,
            strings,
            abi: lowering.abi,
            process_global: lowering.runtime_globals.process,
            process_pointer_size: lowering.runtime_globals.process_pointer_size,
            abi_read: lowering.scratch.abi_read,
        },
        lowering,
    };

    function
        .instruction(&Instruction::GlobalGet(globals.future_poll_epoch))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::GlobalSet(globals.future_poll_epoch));

    if let Some(refresh_settings) = refresh_settings {
        function.instruction(&Instruction::Call(refresh_settings));
    }
    if let Some(observed_timer_state) = globals.observed_timer_state {
        emit_timer_lifecycle_events(
            &mut function,
            timer_state,
            observed_timer_state,
            actions,
            lowering,
        );
    }
    emit_process_attachment(
        &mut function,
        strings,
        actions,
        selection_locals,
        attachment_transition,
        lowering,
    );
    function
        .instruction(&Instruction::GlobalGet(globals.process))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
    if lowering.provider_alternatives.is_empty() && lowering.provider_attach.is_none() {
        function
            .instruction(&Instruction::LocalGet(attachment_transition))
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::F64Const(program.attached_tick_rate().into()))
            .instruction(&Instruction::Call(
                abi.function(AbiImportId::RuntimeSetTickRate),
            ))
            .instruction(&Instruction::End);
    }
    function
        .instruction(&Instruction::GlobalGet(globals.process))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessIsOpen)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        // Only a process whose complete attachment initialization succeeded
        // owns an onDetach event. Pending or rejected initialization is merely
        // cancelled and cleared when that process closes.
        .instruction(&Instruction::GlobalGet(globals.attach_ready))
        .instruction(&Instruction::I32Const(ATTACH_READY))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalSet(attachment_transition));
    emit_attachment_teardown(
        &mut function,
        program,
        actions,
        cancellation_region,
        attachment_transition,
        true,
        lowering,
    );
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    emit_native_pointer_size_detection(&mut function, strings, lowering);

    if !lowering.provider_alternatives.is_empty() {
        let selected = globals
            .selected_provider
            .expect("multi-provider state has selected-provider storage");
        let enumeration = state
            .provider_enum
            .as_ref()
            .expect("multi-provider state generates StateProvider");
        for (variant_index, alternative) in lowering.provider_alternatives.iter().enumerate() {
            function
                .instruction(&Instruction::GlobalGet(selected))
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::If(BlockType::Empty));
            let accepted = match alternative.declaration.processes {
                crate::stdlib::StateProviderProcesses::Declared(names) => names.to_vec(),
                crate::stdlib::StateProviderProcesses::SourceState => alternative
                    .source_processes
                    .iter()
                    .map(String::as_str)
                    .collect(),
            };
            let mut emitted_process = false;
            for (index, name) in lowering.process_names.iter().enumerate() {
                if !accepted.iter().any(|accepted| accepted == name) {
                    continue;
                }
                function
                    .instruction(&Instruction::GlobalGet(globals.process_name))
                    .instruction(&Instruction::I32Const(index as i32))
                    .instruction(&Instruction::I32Eq);
                if emitted_process {
                    function.instruction(&Instruction::I32Or);
                }
                emitted_process = true;
            }
            debug_assert!(emitted_process, "every provider contributes a process name");
            function.instruction(&Instruction::If(BlockType::Empty));
            if let Some(attachment) = alternative.attachment {
                function
                    .instruction(&Instruction::GlobalGet(attachment.frame_global))
                    .instruction(&Instruction::RefIsNull)
                    .instruction(&Instruction::If(BlockType::Empty));
                // Callable providers discover their guest mapping cooperatively.
                // Use the active cadence before the first poll: range discovery
                // and signature scans intentionally perform bounded work per
                // update and would otherwise take prohibitively long at the
                // detached cadence. Mapping validation still owns the logical
                // attachment boundary and restores the detached rate first.
                emit_attached_tick_rate(&mut function, program, lowering);
                function
                    .instruction(&Instruction::Call(attachment.init))
                    .instruction(&Instruction::GlobalSet(attachment.frame_global))
                    .instruction(&Instruction::End)
                    .instruction(&Instruction::GlobalGet(attachment.frame_global))
                    .instruction(&Instruction::RefAsNonNull)
                    .instruction(&Instruction::Call(attachment.poll))
                    .instruction(&Instruction::If(BlockType::Empty))
                    .instruction(&Instruction::GlobalGet(attachment.frame_global))
                    .instruction(&Instruction::StructGet {
                        struct_type_index: attachment.frame_type,
                        field_index: attachment.completion_field,
                    })
                    .instruction(&Instruction::GlobalSet(
                        lowering.provider_values[&alternative.provider],
                    ))
                    .instruction(&Instruction::RefNull(HeapType::Concrete(
                        attachment.frame_type,
                    )))
                    .instruction(&Instruction::GlobalSet(attachment.frame_global));
                emit_provider_selection(
                    &mut function,
                    variant_index,
                    enumeration,
                    selected,
                    lowering,
                );
                function.instruction(&Instruction::End);
            } else {
                debug_assert_eq!(
                    alternative.declaration.attachment,
                    crate::stdlib::StateProviderAttachment::Identity
                );
                emit_attached_tick_rate(&mut function, program, lowering);
                emit_provider_selection(
                    &mut function,
                    variant_index,
                    enumeration,
                    selected,
                    lowering,
                );
            }
            function
                .instruction(&Instruction::End)
                .instruction(&Instruction::End);
        }
        function
            .instruction(&Instruction::GlobalGet(selected))
            .instruction(&Instruction::RefIsNull)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);

        // The selected alternative owns the logical attachment. Losing its
        // private mapping ends that attachment even if the shared host process
        // remains open; ordinary provider selection runs again next update.
        for (variant_index, alternative) in lowering.provider_alternatives.iter().enumerate() {
            let Some(attachment) = alternative.attachment else {
                continue;
            };
            let Some(validation) = attachment.validation else {
                continue;
            };
            function
                .instruction(&Instruction::GlobalGet(selected))
                .instruction(&Instruction::StructGet {
                    struct_type_index: lowering.gc.index(Type::Enum(enumeration.id)),
                    field_index: 0,
                })
                .instruction(&Instruction::I32Const(variant_index as i32))
                .instruction(&Instruction::I32Eq)
                .instruction(&Instruction::If(BlockType::Empty));
            emit_provider_validation(
                &mut function,
                program,
                actions,
                cancellation_region,
                ProviderValidation {
                    function: validation,
                    provider_global: lowering.provider_values[&alternative.provider],
                },
                attachment_transition,
                lowering,
            );
            function.instruction(&Instruction::End);
        }
    }

    if let (Some(provider_global), Some(provider_attach)) =
        (globals.provider_value, lowering.provider_attach)
    {
        let provider_type = semantics
            .state_provider()
            .map(|provider| {
                lowering
                    .standard_library
                    .state_provider(provider)
                    .process_type
            })
            .expect("provider storage requires a resolved provider");
        emit_provider_unavailable(&mut function, provider_global, provider_type, lowering);
        function.instruction(&Instruction::If(BlockType::Empty));
        let ProviderAttach {
            init,
            poll,
            frame_global,
            frame_type,
            completion_field,
            ..
        } = provider_attach;
        function
            .instruction(&Instruction::GlobalGet(frame_global))
            .instruction(&Instruction::RefIsNull)
            .instruction(&Instruction::If(BlockType::Empty));
        // Provider acquisition is cooperative and may scan only one bounded
        // window or mapped range per host update. Raise the cadence before the
        // future starts, both initially and after mapping invalidation.
        emit_attached_tick_rate(&mut function, program, lowering);
        function
            .instruction(&Instruction::Call(init))
            .instruction(&Instruction::GlobalSet(frame_global))
            .instruction(&Instruction::End)
            .instruction(&Instruction::GlobalGet(frame_global))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::Call(poll))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::GlobalGet(frame_global))
            .instruction(&Instruction::StructGet {
                struct_type_index: frame_type,
                field_index: completion_field,
            })
            .instruction(&Instruction::GlobalSet(provider_global))
            .instruction(&Instruction::RefNull(HeapType::Concrete(frame_type)))
            .instruction(&Instruction::GlobalSet(frame_global));
        function.instruction(&Instruction::End);
        emit_provider_unavailable(&mut function, provider_global, provider_type, lowering);
        function
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
        if let Some(validation) = provider_attach.validation {
            emit_provider_validation(
                &mut function,
                program,
                actions,
                cancellation_region,
                ProviderValidation {
                    function: validation,
                    provider_global,
                },
                attachment_transition,
                lowering,
            );
        }
    }

    if let Some(preparation) = lowering.provider_preparation {
        function
            .instruction(&Instruction::GlobalGet(preparation.ready_global))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::GlobalGet(preparation.frame_global))
            .instruction(&Instruction::RefIsNull)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Call(preparation.init))
            .instruction(&Instruction::GlobalSet(preparation.frame_global))
            .instruction(&Instruction::End)
            .instruction(&Instruction::GlobalGet(preparation.frame_global))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::Call(preparation.poll))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::GlobalGet(preparation.frame_global))
            .instruction(&Instruction::StructGet {
                struct_type_index: preparation.frame_type,
                field_index: preparation.completion_field,
            })
            .instruction(&Instruction::GlobalSet(preparation.value_global))
            .instruction(&Instruction::RefNull(HeapType::Concrete(
                preparation.frame_type,
            )))
            .instruction(&Instruction::GlobalSet(preparation.frame_global))
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::GlobalSet(preparation.ready_global))
            .instruction(&Instruction::End);
    }

    let automatic_shape = if lowering.explicit_shape_selection {
        None
    } else {
        lowering.managed.automatic_shape.as_ref().filter(|plan| {
            plan.evidence_fields.is_empty()
                || semantics.state_provider() == Some(crate::stdlib::StdlibStateProviderId::Unity)
        })
    };
    if let Some(plan) = automatic_shape {
        function
            .instruction(&Instruction::GlobalGet(globals.attach_ready))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_automatic_shape_selection(&mut function, program, plan, lowering);
        function
            .instruction(&Instruction::GlobalGet(globals.attach_ready))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_automatic_shape_failure_report(&mut function, strings, program, plan, lowering);
        function
            .instruction(&Instruction::I32Const(ATTACH_REJECTED))
            .instruction(&Instruction::GlobalSet(globals.attach_ready))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::I32Const(
                if actions.contains_key(&ActionKind::OnAttach) {
                    ATTACH_PREPARED
                } else {
                    ATTACH_READY
                },
            ))
            .instruction(&Instruction::GlobalSet(globals.attach_ready))
            .instruction(&Instruction::End);
        if let Some(on_attach) = actions.get(&ActionKind::OnAttach) {
            function
                .instruction(&Instruction::GlobalGet(globals.attach_ready))
                .instruction(&Instruction::I32Const(ATTACH_PREPARED))
                .instruction(&Instruction::I32Eq)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::GlobalGet(globals.process))
                .instruction(&Instruction::Call(*on_attach))
                .instruction(&Instruction::I32Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            emit_return_if_attachment_rejected(&mut function, globals);
            if globals.while_attached_result.is_some() {
                emit_reset_async_action_frame(&mut function, lowering.gc, globals);
            }
            function
                .instruction(&Instruction::I32Const(ATTACH_READY))
                .instruction(&Instruction::GlobalSet(globals.attach_ready))
                .instruction(&Instruction::End);
        }
    } else if let Some(on_attach) = actions.get(&ActionKind::OnAttach) {
        function
            .instruction(&Instruction::GlobalGet(globals.attach_ready))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::GlobalGet(globals.process))
            .instruction(&Instruction::Call(*on_attach))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
        emit_return_if_attachment_rejected(&mut function, globals);
        emit_managed_field_presence_validation(&mut function, program, lowering);
        if globals.while_attached_result.is_some() {
            emit_reset_async_action_frame(&mut function, lowering.gc, globals);
        }
        function
            .instruction(&Instruction::I32Const(ATTACH_READY))
            .instruction(&Instruction::GlobalSet(globals.attach_ready))
            .instruction(&Instruction::End);
    } else {
        // With no user initializer, reaching this point means provider
        // preparation (if any) completed and the attachment is fully ready.
        function
            .instruction(&Instruction::I32Const(ATTACH_READY))
            .instruction(&Instruction::GlobalSet(globals.attach_ready));
    }

    // A rejected attachment can come from managed metadata validation or the
    // implicit failure boundary around `onAttach`. Retain the selected process
    // until it closes so a live process is not selected again every tick.
    function
        .instruction(&Instruction::GlobalGet(globals.attach_ready))
        .instruction(&Instruction::I32Const(ATTACH_REJECTED))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    // Managed static fields are attachment metadata roots, but their values
    // (notably Unity singleton instances) may be replaced while the process
    // remains alive. Share each read only within this snapshot attempt and
    // clear the transaction before any field is polled.
    if let Some(active) = lowering.managed_state_reads.active() {
        function
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::GlobalSet(active));
    }
    for storage in lowering.managed_state_reads.entries() {
        function
            .instruction(&Instruction::RefNull(HeapType::Concrete(
                lowering.gc.index(Type::Result(storage.result)),
            )))
            .instruction(&Instruction::GlobalSet(storage.global));
    }

    function
        .instruction(&Instruction::StructNewDefault(STATE_TYPE))
        .instruction(&Instruction::LocalSet(candidate_state));
    if !state.has_provider_alternatives() {
        let mut prefix_emission = PrefixEmissionState::default();
        for field in state_dependency_order(&all_fields, semantics) {
            let read_index = read_indices[&field.id];
            let predicate = semantics.state_field_shape_predicate(field.id);
            if let Some(predicate) = predicate {
                emit_shape_predicate(
                    &mut function,
                    program,
                    predicate,
                    lowering.semantics,
                    lowering.gc,
                    lowering.globals,
                    PredicateState::Local(candidate_state),
                );
                function.instruction(&Instruction::If(BlockType::Empty));
            }
            emit_state_field_poll(
                &mut function,
                StateFieldPoll {
                    field: field.id,
                    read_function: state_functions.reads[read_index],
                    transform_function: state_functions.transforms[read_index],
                    poll_result_local: first_poll_result + read_index as u32,
                },
                &mut prefix_emission,
                predicate,
                &snapshot_poll,
            );
            if predicate.is_some() {
                function.instruction(&Instruction::End);
            }
        }
    } else {
        let selected = globals
            .selected_provider
            .expect("provider alternatives have discriminator storage");
        let enumeration = state
            .provider_enum
            .as_ref()
            .expect("provider alternatives generate a typed enum");
        for (provider_index, (_, fields)) in state.provider_variant_fields().enumerate() {
            let mut prefix_emission = PrefixEmissionState::default();
            function
                .instruction(&Instruction::GlobalGet(selected))
                .instruction(&Instruction::StructGet {
                    struct_type_index: lowering.gc.index(Type::Enum(enumeration.id)),
                    field_index: 0,
                })
                .instruction(&Instruction::I32Const(provider_index as i32))
                .instruction(&Instruction::I32Eq)
                .instruction(&Instruction::If(BlockType::Empty));
            let provider_fields = fields.iter().collect::<Vec<_>>();
            for field in state_dependency_order(&provider_fields, semantics) {
                let read_index = read_indices[&field.id];
                emit_state_field_poll(
                    &mut function,
                    StateFieldPoll {
                        field: field.id,
                        read_function: state_functions.reads[read_index],
                        transform_function: state_functions.transforms[read_index],
                        poll_result_local: first_poll_result + read_index as u32,
                    },
                    &mut prefix_emission,
                    None,
                    &snapshot_poll,
                );
            }
            function.instruction(&Instruction::End);
        }
    }
    emit_dynamic_shape_transition_seeding(&mut function, program, candidate_state, lowering);
    if let Some(active) = lowering.managed_state_reads.active() {
        function
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::GlobalSet(active));
    }
    function
        .instruction(&Instruction::GlobalGet(globals.state_ready))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(candidate_state))
        .instruction(&Instruction::GlobalSet(globals.current))
        .instruction(&Instruction::LocalGet(candidate_state))
        .instruction(&Instruction::GlobalSet(globals.old))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::GlobalSet(globals.state_ready));
    if let Some(on_state_ready) = actions.get(&ActionKind::OnStateReady) {
        emit_action_args(&mut function, globals);
        function.instruction(&Instruction::Call(*on_state_ready));
    }
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::GlobalGet(globals.current))
        .instruction(&Instruction::GlobalSet(globals.old))
        .instruction(&Instruction::LocalGet(candidate_state))
        .instruction(&Instruction::GlobalSet(globals.current));

    if let Some(update) = actions.get(&ActionKind::WhileAttached) {
        emit_action_args(&mut function, globals);
        function.instruction(&Instruction::Call(*update));
        if let Some(result) = globals.while_attached_result {
            // The action's return is readiness while its Boolean lifecycle
            // gate is stored separately. A pending invocation owns this
            // attachment's single continuation and suppresses timer decisions.
            function
                .instruction(&Instruction::I32Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            emit_reset_async_action_frame(&mut function, lowering.gc, globals);
            function.instruction(&Instruction::GlobalGet(result));
        }
        function
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
    }
    if has_timer_decisions {
        if !has_timer_lifecycle {
            function
                .instruction(&Instruction::Call(abi.function(AbiImportId::TimerGetState)))
                .instruction(&Instruction::LocalSet(timer_state));
        }

        if let Some(start) = actions.get(&ActionKind::Start) {
            function
                .instruction(&Instruction::LocalGet(timer_state))
                .instruction(&Instruction::I32Eqz)
                .instruction(&Instruction::If(BlockType::Empty));
            emit_action_args(&mut function, globals);
            function
                .instruction(&Instruction::Call(*start))
                .instruction(&Instruction::I32Const(1))
                .instruction(&Instruction::I32Eq)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::Call(abi.function(AbiImportId::TimerStart)))
                .instruction(&Instruction::End)
                .instruction(&Instruction::End);
        }

        function
            .instruction(&Instruction::LocalGet(timer_state))
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::LocalGet(timer_state))
            .instruction(&Instruction::I32Const(2))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::I32Or)
            .instruction(&Instruction::If(BlockType::Empty));

        if let Some(is_loading) = actions.get(&ActionKind::IsLoading) {
            emit_action_result(
                &mut function,
                *is_loading,
                ActionKind::IsLoading,
                ValType::I32,
                |function| {
                    function.instruction(&Instruction::I32Const(-1));
                },
                lowering,
            );
            function
                .instruction(&Instruction::LocalTee(nullable_bool))
                .instruction(&Instruction::I32Const(-1))
                .instruction(&Instruction::I32Ne)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::LocalGet(nullable_bool))
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::Call(
                    abi.function(AbiImportId::TimerPauseGameTime),
                ))
                .instruction(&Instruction::Else)
                .instruction(&Instruction::Call(
                    abi.function(AbiImportId::TimerResumeGameTime),
                ))
                .instruction(&Instruction::End)
                .instruction(&Instruction::End);
        }
        if let Some(game_time) = actions.get(&ActionKind::GameTime) {
            emit_action_result(
                &mut function,
                *game_time,
                ActionKind::GameTime,
                ValType::Ref(RefType {
                    nullable: true,
                    heap_type: HeapType::Concrete(
                        lowering.gc.standard_index(StdlibTypeId::Duration),
                    ),
                }),
                |function| {
                    function.instruction(&Instruction::RefNull(HeapType::Concrete(
                        lowering.gc.standard_index(StdlibTypeId::Duration),
                    )));
                },
                lowering,
            );
            function
                .instruction(&Instruction::LocalTee(duration_local))
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::Else)
                .instruction(&Instruction::LocalGet(duration_local))
                .instruction(&Instruction::StructGet {
                    struct_type_index: lowering.gc.standard_index(StdlibTypeId::Duration),
                    field_index: lowering
                        .gc
                        .standard_field_index(StdlibFieldId::DurationSeconds),
                })
                .instruction(&Instruction::LocalGet(duration_local))
                .instruction(&Instruction::StructGet {
                    struct_type_index: lowering.gc.standard_index(StdlibTypeId::Duration),
                    field_index: lowering
                        .gc
                        .standard_field_index(StdlibFieldId::DurationNanoseconds),
                })
                .instruction(&Instruction::Call(
                    abi.function(AbiImportId::TimerSetGameTime),
                ))
                .instruction(&Instruction::End);
        }

        if let Some(reset) = actions.get(&ActionKind::Reset) {
            emit_action_bool_result(&mut function, *reset, ActionKind::Reset, lowering);
            function
                .instruction(&Instruction::I32Const(1))
                .instruction(&Instruction::I32Eq)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::Call(abi.function(AbiImportId::TimerReset)));
            if let Some(split) = actions.get(&ActionKind::Split) {
                function.instruction(&Instruction::Else);
                emit_split(&mut function, *split, abi, lowering);
            }
            function.instruction(&Instruction::End);
        } else if let Some(split) = actions.get(&ActionKind::Split) {
            emit_split(&mut function, *split, abi, lowering);
        }

        function.instruction(&Instruction::End);
    }

    function.instruction(&Instruction::End);
    function
}

fn emit_native_pointer_size_detection(
    function: &mut Function,
    strings: &StringPool,
    lowering: &UpdateContext<'_>,
) {
    let Some(pointer_size) = lowering.runtime_globals.process_pointer_size else {
        return;
    };
    function
        .instruction(&Instruction::GlobalGet(pointer_size))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    for (index, name) in lowering.process_names.iter().enumerate() {
        let (name_ptr, name_len) = strings.get(name);
        function
            .instruction(&Instruction::GlobalGet(
                lowering.runtime_globals.process_name,
            ))
            .instruction(&Instruction::I32Const(index as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::GlobalGet(lowering.runtime_globals.process))
            .instruction(&Instruction::GlobalGet(lowering.runtime_globals.process))
            .instruction(&Instruction::I32Const(name_ptr as i32))
            .instruction(&Instruction::I32Const(name_len as i32))
            .instruction(&Instruction::Call(
                lowering.abi.function(AbiImportId::ProcessGetModuleAddress),
            ))
            .instruction(&Instruction::Call(lowering.runtime_helpers.function(
                crate::intrinsic_registry::RuntimeHelperId::DetectProcessPointerSize,
            )))
            .instruction(&Instruction::GlobalSet(pointer_size))
            .instruction(&Instruction::End);
    }
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::GlobalGet(pointer_size))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
}

fn emit_return_if_attachment_rejected(function: &mut Function, globals: RuntimeGlobals) {
    function
        .instruction(&Instruction::GlobalGet(globals.attach_ready))
        .instruction(&Instruction::I32Const(ATTACH_REJECTED))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
}

fn emit_automatic_shape_failure_report(
    function: &mut Function,
    strings: &StringPool,
    program: &Program,
    plan: &crate::shape_selection::ShapeSelectionPlan,
    lowering: &UpdateContext<'_>,
) {
    let report = plan.failure_report(program);
    emit_runtime_message(function, strings, &report.header, lowering.abi);
    let Some(bindings_global) = lowering.runtime_globals.provider_preparation_value else {
        return;
    };
    let bindings = program
        .structs
        .iter()
        .find(|structure| structure.name == crate::stdlib::PROVIDER_BINDINGS_TYPE)
        .expect("managed shape evidence has generated bindings");
    emit_runtime_message(function, strings, &report.observed_present, lowering.abi);
    for evidence in &report.evidence {
        emit_layout_evidence_condition(
            function,
            bindings_global,
            bindings,
            evidence.field,
            true,
            lowering,
        );
        function.instruction(&Instruction::If(BlockType::Empty));
        emit_runtime_message(function, strings, &evidence.label, lowering.abi);
        function.instruction(&Instruction::End);
    }
    emit_runtime_message(function, strings, &report.observed_absent, lowering.abi);
    for evidence in &report.evidence {
        emit_layout_evidence_condition(
            function,
            bindings_global,
            bindings,
            evidence.field,
            false,
            lowering,
        );
        function.instruction(&Instruction::If(BlockType::Empty));
        emit_runtime_message(function, strings, &evidence.label, lowering.abi);
        function.instruction(&Instruction::End);
    }
    for candidate in &report.candidates {
        emit_runtime_message(function, strings, &candidate.label, lowering.abi);
        emit_runtime_message(function, strings, &report.expected_present, lowering.abi);
        for evidence in &report.evidence {
            if candidate.present_fields.contains(&evidence.field) {
                emit_runtime_message(function, strings, &evidence.label, lowering.abi);
            }
        }
        emit_runtime_message(function, strings, &report.expected_absent, lowering.abi);
        for evidence in &report.evidence {
            if !candidate.present_fields.contains(&evidence.field) {
                emit_runtime_message(function, strings, &evidence.label, lowering.abi);
            }
        }
    }
}

fn emit_layout_evidence_condition(
    function: &mut Function,
    bindings_global: u32,
    bindings: &crate::ast::StructDecl,
    field: crate::ast::ManagedFieldId,
    expected_present: bool,
    lowering: &UpdateContext<'_>,
) {
    let name = crate::stdlib::managed_field_presence_name(field.index());
    let (field_index, declaration) = bindings
        .fields
        .iter()
        .enumerate()
        .find(|(_, candidate)| candidate.name == name)
        .expect("shape evidence has generated presence storage");
    let field_type = struct_field_type(declaration.id, lowering.semantics);
    function
        .instruction(&Instruction::GlobalGet(bindings_global))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(
        function,
        lowering.gc.index(Type::Struct(bindings.id)),
        field_index as u32,
        field_type,
    );
    if !expected_present {
        function.instruction(&Instruction::I32Eqz);
    }
}

fn emit_runtime_message(function: &mut Function, strings: &StringPool, message: &str, abi: &Abi) {
    let (pointer, length) = strings.get(message);
    function
        .instruction(&Instruction::I32Const(pointer as i32))
        .instruction(&Instruction::I32Const(length as i32))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::RuntimePrintMessage),
        ));
}

fn emit_timer_lifecycle_events(
    function: &mut Function,
    timer_state: u32,
    observed_timer_state: u32,
    actions: &HashMap<ActionKind, u32>,
    lowering: &UpdateContext<'_>,
) {
    let abi = lowering.abi;
    function
        .instruction(&Instruction::Call(abi.function(AbiImportId::TimerGetState)))
        .instruction(&Instruction::LocalSet(timer_state))
        .instruction(&Instruction::GlobalGet(observed_timer_state))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(timer_state))
        .instruction(&Instruction::GlobalSet(observed_timer_state))
        .instruction(&Instruction::Else);

    if let Some(on_start) = actions.get(&ActionKind::OnStart) {
        function
            .instruction(&Instruction::GlobalGet(observed_timer_state))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::LocalGet(timer_state))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::I32And)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Call(*on_start));
        if let Some(attempt_ready) = lowering.runtime_globals.attempt_ready {
            function
                .instruction(&Instruction::I32Const(1))
                .instruction(&Instruction::GlobalSet(attempt_ready));
        }
        function.instruction(&Instruction::End);
    }
    if actions.contains_key(&ActionKind::OnReset) || !lowering.attempt_globals.is_empty() {
        function
            .instruction(&Instruction::GlobalGet(observed_timer_state))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::LocalGet(timer_state))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::I32And)
            .instruction(&Instruction::If(BlockType::Empty));
        if let Some(on_reset) = actions.get(&ActionKind::OnReset) {
            if lowering
                .scoped_globals
                .action_requires_attempt(ActionKind::OnReset)
            {
                let ready = lowering
                    .runtime_globals
                    .attempt_ready
                    .expect("attempt-dependent onReset has readiness storage");
                function
                    .instruction(&Instruction::GlobalGet(ready))
                    .instruction(&Instruction::If(BlockType::Empty))
                    .instruction(&Instruction::Call(*on_reset))
                    .instruction(&Instruction::End);
            } else {
                function.instruction(&Instruction::Call(*on_reset));
            }
        }
        for value in lowering.attempt_globals {
            let ty = lowering.global_types[value];
            if !ty.has_runtime_value() {
                continue;
            }
            emit_storage_default(function, lowering.gc.val_type(ty));
            function.instruction(&Instruction::GlobalSet(lowering.globals[value]));
        }
        if let Some(attempt_ready) = lowering.runtime_globals.attempt_ready {
            function
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::GlobalSet(attempt_ready));
        }
        function.instruction(&Instruction::End);
    }
    function
        .instruction(&Instruction::LocalGet(timer_state))
        .instruction(&Instruction::GlobalSet(observed_timer_state))
        .instruction(&Instruction::End);
}

pub(super) fn emit_shape_predicate(
    function: &mut Function,
    program: &Program,
    predicate: &crate::semantic::ResolvedShapePredicate,
    semantics: &SemanticModel,
    gc: &GcLayout,
    globals: &HashMap<ValueId, u32>,
    state: PredicateState,
) {
    for (alternative_index, alternative) in predicate.alternatives.iter().enumerate() {
        emit_shape_constraints(
            function,
            program,
            alternative,
            semantics,
            gc,
            globals,
            state,
        );
        if alternative_index != 0 {
            function.instruction(&Instruction::I32Or);
        }
    }
    if predicate.alternatives.is_empty() {
        function.instruction(&Instruction::I32Const(0));
    }
}

fn emit_shape_constraints(
    function: &mut Function,
    program: &Program,
    constraints: &[crate::semantic::ResolvedShapeConstraint],
    semantics: &SemanticModel,
    gc: &GcLayout,
    globals: &HashMap<ValueId, u32>,
    state_snapshot: PredicateState,
) {
    if constraints.is_empty() {
        function.instruction(&Instruction::I32Const(1));
        return;
    }
    for (index, constraint) in constraints.iter().enumerate() {
        let field_type = match constraint.dimension {
            crate::semantic::ResolvedShapeDimension::Global(value) => value_type(value, semantics),
            crate::semantic::ResolvedShapeDimension::StateField(value) => {
                value_type(value, semantics)
            }
        };
        let Type::Enum(enumeration) = field_type else {
            unreachable!("validated shape dimensions are source enums")
        };
        let enumeration_decl = program
            .enum_declarations()
            .find(|candidate| candidate.id == enumeration)
            .expect("shape dimension enums belong to the source program");
        let variant_index = enumeration_decl
            .variants
            .iter()
            .position(|variant| variant.id == constraint.variant)
            .expect("shape constraints refer to variants of their dimension")
            as i32;

        match constraint.dimension {
            crate::semantic::ResolvedShapeDimension::Global(value) => {
                function.instruction(&Instruction::GlobalGet(globals[&value]));
            }
            crate::semantic::ResolvedShapeDimension::StateField(value) => {
                match state_snapshot {
                    PredicateState::Local(snapshot) => {
                        function.instruction(&Instruction::LocalGet(snapshot));
                    }
                    PredicateState::Global(snapshot) => {
                        function.instruction(&Instruction::GlobalGet(snapshot));
                    }
                    PredicateState::Unavailable => unreachable!(
                        "dynamic state predicates are only emitted with a state snapshot"
                    ),
                }
                function.instruction(&Instruction::RefAsNonNull);
                let (field_index, _) = state_storage_index(value, semantics);
                emit_typed_struct_get(function, STATE_TYPE, field_index, field_type);
            }
        }
        function
            .instruction(&Instruction::StructGet {
                struct_type_index: gc.index(field_type),
                field_index: 0,
            })
            .instruction(&Instruction::I32Const(variant_index))
            .instruction(&Instruction::I32Eq);
        if index != 0 {
            function.instruction(&Instruction::I32And);
        }
    }
}

fn emit_managed_field_presence_validation(
    function: &mut Function,
    program: &Program,
    lowering: &UpdateContext<'_>,
) {
    use crate::stdlib::{PROVIDER_BINDINGS_TYPE, managed_field_presence_name};

    let Some(bindings_global) = lowering.runtime_globals.provider_preparation_value else {
        return;
    };
    let Some(bindings) = program
        .structs
        .iter()
        .find(|structure| structure.name == PROVIDER_BINDINGS_TYPE)
    else {
        return;
    };

    for class in &lowering.managed.classes {
        for group in &class.conditional_fields {
            let present_fields = group
                .fields
                .iter()
                .filter_map(|field| {
                    let name = managed_field_presence_name(field.id.index());
                    bindings
                        .fields
                        .iter()
                        .enumerate()
                        .find(|(_, candidate)| candidate.name == name)
                })
                .collect::<Vec<_>>();
            if present_fields.is_empty() {
                continue;
            }
            emit_shape_predicate(
                function,
                program,
                &group.predicate,
                lowering.semantics,
                lowering.gc,
                lowering.globals,
                PredicateState::Unavailable,
            );
            function.instruction(&Instruction::If(BlockType::Empty));
            for (field_index, presence) in present_fields {
                let presence_type = struct_field_type(presence.id, lowering.semantics);
                function
                    .instruction(&Instruction::GlobalGet(bindings_global))
                    .instruction(&Instruction::RefAsNonNull);
                emit_typed_struct_get(
                    function,
                    lowering.gc.index(Type::Struct(bindings.id)),
                    field_index as u32,
                    presence_type,
                );
                function
                    .instruction(&Instruction::I32Eqz)
                    .instruction(&Instruction::If(BlockType::Empty))
                    .instruction(&Instruction::I32Const(ATTACH_REJECTED))
                    .instruction(&Instruction::GlobalSet(
                        lowering.runtime_globals.attach_ready,
                    ))
                    .instruction(&Instruction::Return)
                    .instruction(&Instruction::End);
            }
            function.instruction(&Instruction::End);
        }
    }
}

fn emit_automatic_shape_selection(
    function: &mut Function,
    program: &Program,
    plan: &crate::shape_selection::ShapeSelectionPlan,
    lowering: &UpdateContext<'_>,
) {
    use crate::stdlib::{PROVIDER_BINDINGS_TYPE, managed_field_presence_name};

    let bindings = (!plan.evidence_fields.is_empty()).then(|| {
        let global = lowering
            .runtime_globals
            .provider_preparation_value
            .expect("managed shape evidence has provider preparation storage");
        let structure = program
            .structs
            .iter()
            .find(|structure| structure.name == PROVIDER_BINDINGS_TYPE)
            .expect("managed shape evidence has generated bindings");
        (global, structure)
    });
    for candidate in &plan.candidates {
        if let Some((bindings_global, bindings_struct)) = bindings {
            for (index, field) in plan.evidence_fields.iter().enumerate() {
                let name = managed_field_presence_name(field.index());
                let (field_index, declaration) = bindings_struct
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, candidate)| candidate.name == name)
                    .expect("shape evidence has generated presence storage");
                let field_type = struct_field_type(declaration.id, lowering.semantics);
                function
                    .instruction(&Instruction::GlobalGet(bindings_global))
                    .instruction(&Instruction::RefAsNonNull);
                emit_typed_struct_get(
                    function,
                    lowering.gc.index(Type::Struct(bindings_struct.id)),
                    field_index as u32,
                    field_type,
                );
                if !candidate.present_fields.contains(field) {
                    function.instruction(&Instruction::I32Eqz);
                }
                if index != 0 {
                    function.instruction(&Instruction::I32And);
                }
            }
        } else {
            function.instruction(&Instruction::I32Const(1));
        }
        function.instruction(&Instruction::If(BlockType::Empty));
        for (dimension, variant) in plan.dimensions.iter().zip(&candidate.variants) {
            let enumeration = program
                .enum_declaration(dimension.enumeration)
                .expect("shape dimension enum belongs to the source program");
            let variant_index = enumeration
                .variants
                .iter()
                .position(|candidate| candidate.id == *variant)
                .expect("shape candidate uses a declared variant");
            function.instruction(&Instruction::I32Const(variant_index as i32));
            for _ in &enumeration.variants {
                function.instruction(&Instruction::I32Const(0));
            }
            function.instruction(&Instruction::StructNew(
                lowering.gc.index(Type::Enum(enumeration.id)),
            ));
            if let crate::semantic::ResolvedShapeDimension::Global(value) = dimension.dimension {
                function.instruction(&Instruction::GlobalSet(lowering.globals[&value]));
            }
        }
        function
            .instruction(&Instruction::I32Const(ATTACH_PREPARED))
            .instruction(&Instruction::GlobalSet(
                lowering.runtime_globals.attach_ready,
            ))
            .instruction(&Instruction::End);
    }
}

/// Clears attachment-owned storage without constructing source defaults.
/// Reference-typed globals are nullable at the storage boundary precisely so
/// a detached runtime cannot retain GC objects from the previous process.
fn emit_storage_default(function: &mut Function, ty: ValType) {
    match ty {
        ValType::I32 => function.instruction(&Instruction::I32Const(0)),
        ValType::I64 => function.instruction(&Instruction::I64Const(0)),
        ValType::F32 => function.instruction(&Instruction::F32Const(0.0.into())),
        ValType::F64 => function.instruction(&Instruction::F64Const(0.0.into())),
        ValType::V128 => function.instruction(&Instruction::V128Const(0)),
        ValType::Ref(reference) => function.instruction(&Instruction::RefNull(reference.heap_type)),
    };
}

fn emit_provider_selection(
    function: &mut Function,
    variant_index: usize,
    enumeration: &crate::ast::EnumDecl,
    selected: u32,
    lowering: &UpdateContext<'_>,
) {
    function.instruction(&Instruction::I32Const(variant_index as i32));
    for _ in &enumeration.variants {
        function.instruction(&Instruction::I32Const(0));
    }
    function
        .instruction(&Instruction::StructNew(
            lowering.gc.index(Type::Enum(enumeration.id)),
        ))
        .instruction(&Instruction::GlobalSet(selected));
}

fn predicate_uses_state(predicate: &crate::semantic::ResolvedShapePredicate) -> bool {
    predicate.alternatives.iter().flatten().any(|constraint| {
        matches!(
            constraint.dimension,
            crate::semantic::ResolvedShapeDimension::StateField(_)
        )
    })
}

/// A newly activated conditional group has no meaningful previous value.
/// Once all of its fields have been read transactionally, seed those slots in
/// the accepted snapshot so the first visible `(old, current)` pair does not
/// manufacture a transition from backend defaults.
fn emit_dynamic_shape_transition_seeding(
    function: &mut Function,
    program: &Program,
    candidate_state: u32,
    lowering: &UpdateContext<'_>,
) {
    let Some(state) = &program.state else {
        return;
    };
    let fields = state
        .conditional_fields
        .iter()
        .flat_map(|group| &group.fields)
        .filter_map(|field| {
            let predicate = lowering.semantics.state_field_shape_predicate(field.id)?;
            predicate_uses_state(predicate).then_some((field, predicate))
        })
        .collect::<Vec<_>>();
    if fields.is_empty() {
        return;
    }

    function
        .instruction(&Instruction::GlobalGet(
            lowering.runtime_globals.state_ready,
        ))
        .instruction(&Instruction::If(BlockType::Empty));
    for (field, predicate) in fields {
        emit_shape_predicate(
            function,
            program,
            predicate,
            lowering.semantics,
            lowering.gc,
            lowering.globals,
            PredicateState::Local(candidate_state),
        );
        emit_shape_predicate(
            function,
            program,
            predicate,
            lowering.semantics,
            lowering.gc,
            lowering.globals,
            PredicateState::Global(lowering.runtime_globals.current),
        );
        function
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::I32And)
            .instruction(&Instruction::If(BlockType::Empty));
        let field_type = value_type(field.id, lowering.semantics);
        let (field_index, _) = state_storage_index(field.id, lowering.semantics);
        function
            .instruction(&Instruction::GlobalGet(lowering.runtime_globals.current))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::LocalGet(candidate_state))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(function, STATE_TYPE, field_index, field_type);
        function
            .instruction(&Instruction::StructSet {
                struct_type_index: STATE_TYPE,
                field_index,
            })
            .instruction(&Instruction::End);
    }
    function.instruction(&Instruction::End);
}

/// Stable topological order for the physical fields active in one state
/// shape. Independent declarations retain source order; dependencies are
/// emitted before the candidate-state reads that consume them.
fn state_dependency_order<'a>(
    fields: &[&'a StateField],
    semantics: &SemanticModel,
) -> Vec<&'a StateField> {
    fn visit<'a>(
        field: &'a StateField,
        by_id: &HashMap<ValueId, &'a StateField>,
        semantics: &SemanticModel,
        visiting: &mut std::collections::HashSet<ValueId>,
        visited: &mut std::collections::HashSet<ValueId>,
        output: &mut Vec<&'a StateField>,
    ) {
        if visited.contains(&field.id) || !visiting.insert(field.id) {
            return;
        }
        for dependency in semantics.state_dependencies(field.id) {
            if let Some(dependency) = by_id.get(dependency) {
                visit(dependency, by_id, semantics, visiting, visited, output);
            }
        }
        visiting.remove(&field.id);
        visited.insert(field.id);
        output.push(field);
    }

    let by_id = fields
        .iter()
        .map(|field| (field.id, *field))
        .collect::<HashMap<_, _>>();
    let mut visiting = std::collections::HashSet::new();
    let mut visited = std::collections::HashSet::new();
    let mut output = Vec::with_capacity(fields.len());
    for field in fields {
        visit(
            field,
            &by_id,
            semantics,
            &mut visiting,
            &mut visited,
            &mut output,
        );
    }
    output
}

fn emit_state_field_poll(
    function: &mut Function,
    poll: StateFieldPoll,
    prefix_emission: &mut PrefixEmissionState,
    predicate: Option<&crate::semantic::ResolvedShapePredicate>,
    context: &SnapshotPollContext<'_>,
) {
    let StateFieldPoll {
        field,
        read_function,
        transform_function,
        poll_result_local,
    } = poll;
    let lowering = context.lowering;
    let semantics = lowering.semantics;
    let Type::Result(result_type) = semantic_type(
        semantics
            .state_poll_result(field)
            .expect("checked state fields have poll-result types"),
        semantics,
    ) else {
        unreachable!("state poll-result types are Result layouts")
    };
    let (field_index, _) = state_storage_index(field, semantics);
    let field_type = value_type(field, semantics);
    let prefix = lowering.pointer_prefixes.field(field);
    if let Some(prefix) = prefix {
        context.pointer_prefix_locals.ensure_field_prefix(
            function,
            prefix,
            &context.pointer_emission,
            prefix_emission,
            predicate.is_some(),
        );
    }
    let dependencies = semantics.state_dependencies(field);
    if dependencies.is_empty() {
        emit_state_read_call(
            function,
            field,
            read_function,
            poll_result_local,
            prefix,
            context,
        );
    } else {
        for (index, dependency) in dependencies.iter().enumerate() {
            let dependency_result = semantic_type(
                semantics
                    .state_poll_result(*dependency)
                    .expect("state dependencies have poll-result types"),
                semantics,
            );
            let Type::Result(dependency_result) = dependency_result else {
                unreachable!("state dependency poll-result types are Result layouts")
            };
            function
                .instruction(&Instruction::LocalGet(
                    context.poll_result_locals[dependency],
                ))
                .instruction(&Instruction::StructGet {
                    struct_type_index: lowering.gc.index(Type::Result(dependency_result)),
                    field_index: 1,
                });
            if index != 0 {
                function.instruction(&Instruction::I32Or);
            }
        }
        function.instruction(&Instruction::If(BlockType::Empty));
        emit_result_error(
            function,
            result_type,
            field_type,
            "state field dependency was unavailable",
            lowering.gc,
            lowering.failure_payloads,
        );
        function
            .instruction(&Instruction::LocalSet(poll_result_local))
            .instruction(&Instruction::Else);
        emit_state_read_call(
            function,
            field,
            read_function,
            poll_result_local,
            prefix,
            context,
        );
        function.instruction(&Instruction::End);
    }

    // A filter is itself fallible. A failed raw read bypasses it so the
    // original error reaches the field's acceptance boundary unchanged.
    if let Some(transform) = transform_function {
        function
            .instruction(&Instruction::LocalGet(poll_result_local))
            .instruction(&Instruction::StructGet {
                struct_type_index: lowering.gc.index(Type::Result(result_type)),
                field_index: 1,
            })
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Else);
        emit_poll_value(
            function,
            poll_result_local,
            result_type,
            field_type,
            lowering,
        );
        if !dependencies.is_empty() {
            function
                .instruction(&Instruction::LocalGet(context.candidate_state))
                .instruction(&Instruction::RefAsNonNull);
        }
        function
            .instruction(&Instruction::Call(transform))
            .instruction(&Instruction::LocalSet(poll_result_local))
            .instruction(&Instruction::End);
    }

    // Before initialization every required field must succeed in the same
    // poll. Afterwards, an error retains this field's accepted value while
    // successful sibling fields continue building the candidate snapshot.
    function
        .instruction(&Instruction::LocalGet(poll_result_local))
        .instruction(&Instruction::StructGet {
            struct_type_index: lowering.gc.index(Type::Result(result_type)),
            field_index: 1,
        })
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::GlobalGet(
            lowering.runtime_globals.state_ready,
        ))
        .instruction(&Instruction::I32Eqz);
    if let Some(predicate) = predicate.filter(|predicate| predicate_uses_state(predicate)) {
        emit_shape_predicate(
            function,
            context.program,
            predicate,
            lowering.semantics,
            lowering.gc,
            lowering.globals,
            PredicateState::Global(lowering.runtime_globals.current),
        );
        function
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::I32Or);
    }
    function.instruction(&Instruction::If(BlockType::Empty));
    if let Some(active) = lowering.managed_state_reads.active() {
        function
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::GlobalSet(active));
    }
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(context.candidate_state))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::GlobalGet(lowering.runtime_globals.current))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(function, STATE_TYPE, field_index, field_type);
    function
        .instruction(&Instruction::StructSet {
            struct_type_index: STATE_TYPE,
            field_index,
        })
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(context.candidate_state))
        .instruction(&Instruction::RefAsNonNull);
    emit_poll_value(
        function,
        poll_result_local,
        result_type,
        field_type,
        lowering,
    );
    function
        .instruction(&Instruction::StructSet {
            struct_type_index: STATE_TYPE,
            field_index,
        })
        .instruction(&Instruction::End);
}

fn emit_state_read_call(
    function: &mut Function,
    field: ValueId,
    read_function: u32,
    poll_result_local: u32,
    prefix: Option<crate::codegen::pointer_prefixes::FieldPrefix>,
    context: &SnapshotPollContext<'_>,
) {
    function.instruction(&Instruction::GlobalGet(
        context.lowering.runtime_globals.process,
    ));
    if !context
        .lowering
        .semantics
        .state_dependencies(field)
        .is_empty()
    {
        function
            .instruction(&Instruction::LocalGet(context.candidate_state))
            .instruction(&Instruction::RefAsNonNull);
    }
    if let Some(prefix) = prefix {
        context
            .pointer_prefix_locals
            .emit_field_prefix_values(function, prefix);
    }
    function
        .instruction(&Instruction::Call(read_function))
        .instruction(&Instruction::LocalSet(poll_result_local));
}

fn emit_poll_value(
    function: &mut Function,
    poll_result_local: u32,
    result_type: crate::ast::ResultTypeId,
    field_type: Type,
    lowering: &UpdateContext<'_>,
) {
    function
        .instruction(&Instruction::LocalGet(poll_result_local))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(
        function,
        lowering.gc.index(Type::Result(result_type)),
        0,
        field_type,
    );
}

/// Treats a provider-owned mapping as the source-visible attachment boundary.
/// A failed probe tears down that logical attachment while deliberately
/// retaining the still-open emulator process; normal discovery and lifecycle
/// initialization start again on the following update.
fn emit_provider_validation(
    function: &mut Function,
    program: &Program,
    actions: &HashMap<ActionKind, u32>,
    cancellation_region: Option<wasm_ir::CancellationRegion>,
    validation: ProviderValidation,
    attachment_transition: u32,
    lowering: &UpdateContext<'_>,
) {
    // A rejected onAttach remains inert until the host process closes. Every
    // other accepted or initializing provider is abandoned if its backing
    // mapping disappears.
    function
        .instruction(&Instruction::GlobalGet(
            lowering.runtime_globals.attach_ready,
        ))
        .instruction(&Instruction::I32Const(ATTACH_REJECTED))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::GlobalGet(validation.provider_global))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::Call(validation.function))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::GlobalGet(
            lowering.runtime_globals.attach_ready,
        ))
        .instruction(&Instruction::I32Const(ATTACH_READY))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalSet(attachment_transition));
    emit_attachment_teardown(
        function,
        program,
        actions,
        cancellation_region,
        attachment_transition,
        false,
        lowering,
    );
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
}

fn emit_provider_default(function: &mut Function, ty: StdlibTypeId, context: &UpdateContext<'_>) {
    match context.standard_library.type_decl(ty).representation {
        RuntimeRepresentation::Scalar {
            storage: CoreTypeId::I64,
        } => {
            function.instruction(&Instruction::I64Const(0));
        }
        RuntimeRepresentation::GcStruct { .. }
        | RuntimeRepresentation::GcArray { .. }
        | RuntimeRepresentation::Enum { .. } => {
            function.instruction(&Instruction::RefNull(HeapType::Concrete(
                context.gc.standard_index(ty),
            )));
        }
        representation => {
            unreachable!("unsupported state-provider representation: {representation:?}")
        }
    }
}

fn emit_provider_unavailable(
    function: &mut Function,
    global: u32,
    ty: StdlibTypeId,
    context: &UpdateContext<'_>,
) {
    function.instruction(&Instruction::GlobalGet(global));
    match context.standard_library.type_decl(ty).representation {
        RuntimeRepresentation::Scalar {
            storage: CoreTypeId::I64,
        } => {
            function.instruction(&Instruction::I64Eqz);
        }
        RuntimeRepresentation::GcStruct { .. }
        | RuntimeRepresentation::GcArray { .. }
        | RuntimeRepresentation::Enum { .. } => {
            function.instruction(&Instruction::RefIsNull);
        }
        representation => {
            unreachable!("unsupported state-provider representation: {representation:?}")
        }
    }
}

fn emit_cancel_region(
    function: &mut Function,
    region: wasm_ir::CancellationRegion,
    gc: &GcLayout,
    globals: RuntimeGlobals,
) {
    match region {
        wasm_ir::CancellationRegion::AttachmentLifetime => {
            function
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::GlobalSet(globals.attach_ready))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::GlobalSet(globals.state_ready));
            emit_reset_async_action_frame(function, gc, globals);
        }
    }
}

fn emit_reset_async_action_frame(function: &mut Function, gc: &GcLayout, globals: RuntimeGlobals) {
    function
        .instruction(&Instruction::StructNewDefault(gc.async_frame_index()))
        .instruction(&Instruction::GlobalSet(globals.async_frame));
}

fn emit_split(function: &mut Function, split: u32, abi: &Abi, lowering: &UpdateContext<'_>) {
    emit_action_bool_result(function, split, ActionKind::Split, lowering);
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Call(abi.function(AbiImportId::TimerSplit)))
        .instruction(&Instruction::End);
}

fn emit_action_bool_result(
    function: &mut Function,
    action: u32,
    kind: ActionKind,
    lowering: &UpdateContext<'_>,
) {
    emit_action_result(
        function,
        action,
        kind,
        ValType::I32,
        |function| {
            function.instruction(&Instruction::I32Const(0));
        },
        lowering,
    );
}

fn emit_action_result(
    function: &mut Function,
    action: u32,
    kind: ActionKind,
    result: ValType,
    emit_unavailable: impl FnOnce(&mut Function),
    lowering: &UpdateContext<'_>,
) {
    if lowering.scoped_globals.action_requires_attempt(kind) {
        let ready = lowering
            .runtime_globals
            .attempt_ready
            .expect("attempt-dependent action has readiness storage");
        function
            .instruction(&Instruction::GlobalGet(ready))
            .instruction(&Instruction::If(BlockType::Result(result)));
        emit_action_args(function, lowering.runtime_globals);
        function
            .instruction(&Instruction::Call(action))
            .instruction(&Instruction::Else);
        emit_unavailable(function);
        function.instruction(&Instruction::End);
    } else {
        emit_action_args(function, lowering.runtime_globals);
        function.instruction(&Instruction::Call(action));
    }
}

fn emit_action_args(function: &mut Function, globals: RuntimeGlobals) {
    function
        .instruction(&Instruction::GlobalGet(globals.current))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::GlobalGet(globals.old))
        .instruction(&Instruction::RefAsNonNull);
}

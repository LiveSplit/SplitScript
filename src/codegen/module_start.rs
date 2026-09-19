//! One-time module initialization after all backend plans are fixed.

use std::collections::HashMap;

use wasm_encoder::{Function, Instruction};

use crate::{
    abi::AbiImportId,
    ast::{Program, StructDecl, StructId, ValueId},
    semantic::SemanticModel,
    types::{TypeId, TypeKind},
};

use super::{
    GcLayout, STATE_TYPE, SettingStorage, Type,
    context::EmissionContext,
    data_plan::StringPool,
    debug_artifacts::DebugEmission,
    emit_default,
    expression::{BareReturn, ExprContext, LocalStorage, MatchLayout, compile_block},
    is_wasm_global_constant,
    script_functions::{LocalPlanOptions, plan_wasm_locals},
    semantic_type,
    settings::{SettingsContext, emit_setting_registration},
    value_type,
};

pub(super) struct StartFunctions {
    pub(super) start: u32,
    pub(super) refresh_settings: Option<u32>,
    pub(super) setup: Option<u32>,
}

pub(super) fn compile_start(
    program: &Program,
    settings_context: &SettingsContext<'_>,
    emission: &EmissionContext<'_>,
    strings: &StringPool,
    settings: &HashMap<ValueId, SettingStorage>,
    start_functions: StartFunctions,
    has_async_frame: bool,
) -> Function {
    let semantics = settings_context.semantics;
    let mut locals = HashMap::new();
    let mut matches = MatchLayout::default();
    let mut local_types = Vec::new();
    for initializer in emission
        .wasm_ir
        .global_initializer_plans()
        .filter(|initializer| {
            let ty = value_type(initializer.value, semantics);
            ty.has_runtime_value()
                && (initializer.destructures
                    || !is_wasm_global_constant(initializer.expression, emission.wasm_ir))
        })
    {
        plan_wasm_locals(
            &initializer.locals,
            &mut locals,
            &mut matches,
            &mut local_types,
            LocalPlanOptions {
                parameter_count: 0,
                semantics,
                wasm_ir: emission.wasm_ir,
                gc: settings_context.gc,
                reachability: emission.reachability,
                instance: None,
                include_values: true,
            },
        );
    }
    let mut function = Function::new_with_locals_types(local_types);
    let debug = emission.debug_emission(start_functions.start);
    emit_runtime_global_initializers(&mut function, emission, &locals, &matches, debug);
    emit_initial_state(&mut function, program, semantics, settings_context.gc);
    function.instruction(&Instruction::GlobalSet(
        settings_context.runtime_globals.current,
    ));
    emit_initial_state(&mut function, program, semantics, settings_context.gc);
    function.instruction(&Instruction::GlobalSet(
        settings_context.runtime_globals.old,
    ));
    if has_async_frame {
        function
            .instruction(&Instruction::StructNewDefault(
                settings_context.gc.async_frame_index(),
            ))
            .instruction(&Instruction::GlobalSet(
                settings_context.runtime_globals.async_frame,
            ));
    }
    for setting in &program.settings {
        emit_setting_registration(
            &mut function,
            setting,
            strings,
            settings.get(&setting.id).copied(),
            settings_context,
        );
    }
    if let Some(refresh_settings) = start_functions.refresh_settings {
        function.instruction(&Instruction::Call(refresh_settings));
    }
    function
        .instruction(&Instruction::F64Const(program.detached_tick_rate().into()))
        .instruction(&Instruction::Call(
            emission.abi.function(AbiImportId::RuntimeSetTickRate),
        ));
    if let Some(setup) = start_functions.setup {
        function.instruction(&Instruction::Call(setup));
    }
    function.instruction(&Instruction::End);
    function
}

/// Materializes non-Wasm-constant source initializers once in the module start
/// function.
///
/// Wasm constant expressions cannot call pure helpers or construct GC values.
/// Their globals therefore begin with backend defaults and are populated before
/// any exported script entry point can observe them.
fn emit_runtime_global_initializers(
    function: &mut Function,
    lowering: &EmissionContext<'_>,
    locals: &HashMap<ValueId, (u32, Type)>,
    matches: &MatchLayout,
    debug: Option<DebugEmission<'_>>,
) {
    let context = ExprContext {
        standard_library: lowering.standard_library,
        reachability: lowering.reachability,
        failure_payloads: lowering.failure_payloads,
        abi: lowering.abi,
        locals: LocalStorage::Wasm {
            values: locals,
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
        enums: lowering.enums,
        arrays: lowering.arrays,
        memory: lowering.memory,
        abi_read: lowering.abi_read,
        runtime_scratch: lowering.runtime_scratch,
        signatures: lowering.signatures,
        matches,
        semantics: lowering.semantics,
        wasm_ir: lowering.wasm_ir,
        gc: lowering.gc,
        async_frames: lowering.async_frames,
        exact_runtime: lowering.exact_runtime,
        intrinsic_capture: None,
        debug,
        function_instance: None,
        loop_control: None,
        bare_return: BareReturn::None,
        materialize_none: true,
    };

    for initializer in lowering.wasm_ir.global_initializer_plans() {
        let ty = value_type(initializer.value, lowering.semantics);
        if !ty.has_runtime_value()
            || (!initializer.destructures
                && is_wasm_global_constant(initializer.expression, lowering.wasm_ir))
        {
            continue;
        }
        compile_block(function, &initializer.entry, &context, None);
    }
}

/// Constructs source-language state defaults instead of relying on Wasm's
/// `structure.new_default`, because nested structs are non-null source values.
fn emit_initial_state(
    function: &mut Function,
    program: &Program,
    semantics: &SemanticModel,
    gc: &GcLayout,
) {
    for field in semantics.state_storage_fields() {
        let ty = semantics
            .value_type(*field)
            .expect("checked state fields have semantic types");
        emit_source_default(
            function,
            ty,
            &program.structs,
            semantics,
            gc,
            &mut Vec::new(),
        );
    }
    function.instruction(&Instruction::StructNew(STATE_TYPE));
}

fn emit_source_default(
    function: &mut Function,
    ty: TypeId,
    structs: &[StructDecl],
    semantics: &SemanticModel,
    gc: &GcLayout,
    visiting: &mut Vec<StructId>,
) {
    let TypeKind::Struct(structure) = semantics.types().kind(ty) else {
        emit_default(function, semantic_type(ty, semantics), gc);
        return;
    };

    if visiting.contains(structure) {
        emit_default(function, Type::Struct(*structure), gc);
        return;
    }

    visiting.push(*structure);
    let declaration = structs
        .iter()
        .find(|declaration| declaration.id == *structure)
        .expect("semantic struct types belong to source declarations");
    for field in &declaration.fields {
        let field_type = semantics
            .struct_field_type(field.id)
            .expect("checked struct fields have semantic types");
        emit_source_default(function, field_type, structs, semantics, gc, visiting);
    }
    visiting.pop();
    function.instruction(&Instruction::StructNew(gc.index(Type::Struct(*structure))));
}

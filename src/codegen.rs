//! Emit unary GC reads (`struct.get*` and `array.len`) without a preceding
//! null assertion: the read itself traps on null. Keep assertions needed by
//! non-null result types or before evaluating further operands of writes and
//! indexed reads, where deferring a trap could change observable effects.

use wasm_encoder::{
    AbstractHeapType, BlockType, ConstExpr, Function, HeapType, Instruction, MemArg, RefType,
    ValType,
};

use crate::ast::{
    ActionKind, ArrayTypeId, EnumDecl, EnumVariantId, ExprId, OptionTypeId, Program, ResultTypeId,
    StructFieldId, ValueId,
};
use crate::equality::EqualityCapabilities;
use crate::memory::{MemoryAddressWidth, MemoryLayouts, MemoryTypeLayout};
use crate::semantic::{FunctionInstance, ResolvedReceiver, SemanticModel};
use crate::stdlib::{
    Implementation, IntrinsicId, StandardLibrary, StateProviderAttachment, StateProviderProcesses,
    StdlibCapabilityId, StdlibItemId, StdlibTypeId,
};
use crate::types::{
    ResolvedArrayType, ResolvedOptionType, ResolvedRangeType, ResolvedResultType, TypeId, TypeKind,
};
use crate::wasm_ir::{self, BodyOwner};

mod array_functions;
mod array_value;
mod async_frame;
mod async_state;
mod backend_type;
mod code_bodies;
mod context;
mod data_plan;
mod debug_artifacts;
mod dependencies;
mod display;
mod display_plan;
mod equality_plan;
mod exact_runtime;
mod expression;
mod failure_payload;
mod function_plan;
mod function_types;
mod gc_layout;
mod gc_types;
mod global_plan;
mod imports;
mod managed_decoders;
mod managed_demand;
mod managed_freezers;
mod managed_references;
mod managed_snapshots;
mod managed_state_reads;
mod memory_plan;
mod module_assembly;
mod module_start;
mod pointer_prefixes;
mod reachability;
mod runtime_helper_registry;
mod runtime_helpers;
mod script_functions;
mod set_functions;
mod settings;
mod specialization;
mod update;

use self::array_functions::ArrayFunctions;
use self::async_frame::AsyncFrameLayouts;
use self::async_state::{
    compile_async_action, compile_async_closure_poll, compile_async_function_poll,
    compile_generator_closure_next, compile_generator_function_next, compile_leaf_future_poll,
};
use self::backend_type::Type;
use self::context::{AttachContext, EmissionContext};
use self::data_plan::StaticData;
use self::dependencies::BackendDependencies;
use self::display_plan::DisplayFunctions;
use self::equality_plan::EqualityFunctions;
use self::exact_runtime::ExactRuntimeRepresentations;
use self::failure_payload::FailurePayloadDemand;
use self::gc_layout::GcLayout;
use self::global_plan::SettingStorage;
use self::imports::Abi;
use self::module_start::compile_start;
use self::runtime_helper_registry::RuntimeHelperPlan;
use self::script_functions::{
    LocalPlanOptions, compile_action, compile_async_closure_init, compile_async_function_init,
    compile_closure, compile_function_value_adapter, compile_read, compile_state_transform,
    compile_user_function, plan_wasm_locals,
};
use self::set_functions::SetFunctions;
use self::update::{
    ProviderAlternative, ProviderAttach, ProviderPreparation, StatePollFunctions, compile_update,
};
use crate::intrinsic_registry::RuntimeHelperId;

const STATE_TYPE: u32 = 0;

/// The null-only subtype of every unshared GC reference. Unlike a concrete
/// type index, its heap-type encoding is always one byte. Source callables
/// are GC wrapper structs too; raw function and extern references must use
/// their own null types instead.
const GC_NULL_HEAP_TYPE: HeapType = HeapType::Abstract {
    shared: false,
    ty: AbstractHeapType::None,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoryByteOrder {
    Little,
    Big,
}

impl From<crate::intrinsic_registry::ProviderByteOrder> for MemoryByteOrder {
    fn from(value: crate::intrinsic_registry::ProviderByteOrder) -> Self {
        match value {
            crate::intrinsic_registry::ProviderByteOrder::Little => Self::Little,
            crate::intrinsic_registry::ProviderByteOrder::Big => Self::Big,
        }
    }
}

fn standard_display_function(
    source: TypeId,
    program: &Program,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
) -> Option<(StdlibTypeId, FunctionInstance)> {
    let TypeKind::Standard(standard) = semantics.types().kind(source) else {
        return None;
    };
    let item = standard_library.display_implementation(*standard)?;
    let Implementation::LibraryBody { function_name, .. } = item.implementation else {
        unreachable!("validated custom Display implementations have source bodies")
    };
    let function = program
        .functions
        .iter()
        .find(|function| function.name == function_name)
        .expect("custom Display bodies are injected into the program");
    let string = semantics.types().id_for_standard(StdlibTypeId::String);
    Some((
        *standard,
        semantics.function_instance(function.id, vec![source, string]),
    ))
}

fn display_function(
    source: TypeId,
    program: &Program,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
    capabilities: &crate::capabilities::CapabilityAnalysis,
) -> Option<(TypeId, FunctionInstance)> {
    if let Some((_, function)) =
        standard_display_function(source, program, semantics, standard_library)
    {
        return Some((source, function));
    }
    let function = capabilities.method_implementation(
        source,
        StdlibCapabilityId::Display,
        StdlibItemId::DisplayToString,
        semantics,
    )?;
    let signature = semantics
        .function_parameter_types(function)
        .iter()
        .copied()
        .chain(semantics.function_result(function))
        .collect();
    Some((source, semantics.function_instance(function, signature)))
}

fn debug_function(
    source: TypeId,
    semantics: &SemanticModel,
    capabilities: &crate::capabilities::CapabilityAnalysis,
) -> Option<(TypeId, FunctionInstance)> {
    let function = capabilities.method_implementation(
        source,
        StdlibCapabilityId::Debug,
        StdlibItemId::DebugDebugString,
        semantics,
    )?;
    let signature = semantics
        .function_parameter_types(function)
        .iter()
        .copied()
        .chain(semantics.function_result(function))
        .collect();
    Some((source, semantics.function_instance(function, signature)))
}

fn provider_attachment_function(
    provider: &crate::stdlib::StdlibStateProvider,
    program: &Program,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
) -> Option<FunctionInstance> {
    let StateProviderAttachment::Callable(item) = provider.attachment else {
        return None;
    };
    let Implementation::LibraryBody { function_name, .. } =
        standard_library.item(item).implementation
    else {
        return None;
    };
    let function = program
        .functions
        .iter()
        .find(|function| function.name == function_name)
        .expect("source-defined provider attachments are injected into the program");
    let signature = semantics
        .function_parameter_types(function.id)
        .iter()
        .copied()
        .chain(semantics.function_result(function.id))
        .collect();
    Some(semantics.function_instance(function.id, signature))
}

fn provider_validation_function(
    provider: &crate::stdlib::StdlibStateProvider,
    program: &Program,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
) -> Option<FunctionInstance> {
    let item = provider.validation?;
    let Implementation::LibraryBody { function_name, .. } =
        standard_library.item(item).implementation
    else {
        return None;
    };
    let function = program
        .functions
        .iter()
        .find(|function| function.name == function_name)
        .expect("source-defined provider mapping validation is injected into the program");
    let signature = semantics
        .function_parameter_types(function.id)
        .iter()
        .copied()
        .chain(semantics.function_result(function.id))
        .collect();
    Some(semantics.function_instance(function.id, signature))
}

fn provider_preparation_function(
    program: &Program,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
) -> Option<FunctionInstance> {
    standard_library.state_provider_preparation(
        semantics.state_provider()?,
        semantics.state_provider_selector(),
    )?;
    let function = program
        .functions
        .iter()
        .find(|function| function.name == crate::stdlib::PROVIDER_PREPARATION_FUNCTION)
        .expect("selected provider preparation is injected into the program");
    let signature = semantics
        .function_parameter_types(function.id)
        .iter()
        .copied()
        .chain(semantics.function_result(function.id))
        .collect();
    Some(semantics.function_instance(function.id, signature))
}

struct ConstructedTypes {
    enums: Vec<EnumDecl>,
    arrays: Vec<ResolvedArrayType>,
    options: Vec<ResolvedOptionType>,
    results: Vec<ResolvedResultType>,
    asyncs: Vec<crate::types::ResolvedAsyncType>,
    iterators: Vec<crate::types::ResolvedIteratorType>,
    callables: Vec<crate::types::ResolvedCallableType>,
    ranges: Vec<ResolvedRangeType>,
    sets: Vec<crate::types::ResolvedSetType>,
    applications: Vec<crate::types::ResolvedApplicationType>,
}

/// Complete, immutable input to backend planning and Wasm encoding.
///
/// Earlier compiler products are gathered once when Wasm IR is lowered. The
/// encoder therefore cannot be called with a mismatched syntax tree, semantic
/// model, constructed-type table, or capability analysis.
pub struct BackendProgram<'a> {
    standard_library: StandardLibrary,
    program: std::borrow::Cow<'a, Program>,
    semantics: SemanticModel,
    wasm_ir: wasm_ir::Program,
    constructed_types: ConstructedTypes,
    memory_layouts: &'a MemoryLayouts,
    equality: &'a EqualityCapabilities,
    capabilities: &'a crate::capabilities::CapabilityAnalysis,
    scoped_globals: &'a crate::ScopedGlobalAnalysis,
    source_name: &'a str,
    source: &'a str,
}

impl<'a> BackendProgram<'a> {
    pub(crate) fn new(checked: &'a crate::CheckedProgram, mut wasm_ir: wasm_ir::Program) -> Self {
        let mut semantics = checked.semantics.clone();
        let mut constructed_types = ConstructedTypes {
            enums: checked.enum_types.clone(),
            arrays: checked.array_types.clone(),
            options: checked.option_types.clone(),
            results: checked.result_types.clone(),
            asyncs: checked.async_types.clone(),
            iterators: checked.iterator_types.clone(),
            callables: checked.callable_types.clone(),
            ranges: checked.range_types.clone(),
            sets: checked.set_types.clone(),
            applications: checked.application_types.clone(),
        };
        specialization::materialize(
            &wasm_ir,
            &checked.compilation_syntax,
            &checked.capabilities,
            &mut semantics,
            &mut constructed_types.arrays,
            &mut constructed_types.options,
            &mut constructed_types.results,
            &mut constructed_types.asyncs,
            &mut constructed_types.iterators,
            &mut constructed_types.callables,
            &mut constructed_types.ranges,
            &mut constructed_types.sets,
            &mut constructed_types.applications,
        );
        let program = if let Some(program) = managed_demand::prune(
            &checked.compilation_syntax,
            &mut semantics,
            &wasm_ir,
            &checked.capabilities,
        ) {
            // Rebuild control flow after pruning so discarded awaits never
            // acquire frame slots or async states. Checked tooling products
            // retain the complete schema and original generated declarations.
            let original = checked
                .compilation_syntax
                .functions
                .iter()
                .find(|function| function.name == crate::stdlib::PROVIDER_PREPARATION_FUNCTION)
                .expect("pruned bindings have a preparation function");
            let replacement = program
                .functions
                .iter()
                .find(|function| function.id == original.id)
                .unwrap();
            let hir =
                checked
                    .hir
                    .rebuild_pruned_function(original, replacement, &program, &semantics);
            wasm_ir = wasm_ir::Program::lower(
                &hir,
                &semantics,
                &checked.effects,
                &checked.capabilities,
                &checked.scoped_globals,
                wasm_ir.profile(),
            );
            std::borrow::Cow::Owned(program)
        } else {
            std::borrow::Cow::Borrowed(checked.compilation_syntax.as_ref())
        };
        wasm_ir.plan_suspension_liveness();
        Self {
            standard_library: checked.context.standard_library(),
            program,
            semantics,
            wasm_ir,
            constructed_types,
            memory_layouts: checked.capabilities.memory(),
            equality: checked.capabilities.equality(),
            capabilities: &checked.capabilities,
            scoped_globals: &checked.scoped_globals,
            source_name: checked.source_name(),
            source: checked.source_document().source(),
        }
    }

    pub fn wasm_ir(&self) -> &wasm_ir::Program {
        &self.wasm_ir
    }
}

impl std::ops::Deref for BackendProgram<'_> {
    type Target = wasm_ir::Program;

    fn deref(&self) -> &Self::Target {
        self.wasm_ir()
    }
}

pub fn compile(inputs: BackendProgram<'_>) -> Vec<u8> {
    compile_internal(inputs, None)
}

/// Sidecar information from the same plan that emitted the module. Requesting
/// this report never adds names, custom sections, or helpers to the Wasm.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CodegenReport {
    pub functions: Vec<(u32, String)>,
    pub runtime_helpers: Vec<String>,
    /// Reserved scratch banks, excluding padding before immutable data.
    pub scratch_bytes: u64,
    pub abi_read_capacity: u32,
    pub static_data_start: u32,
    pub static_data_end: u64,
    pub minimum_memory_pages: u64,
}

pub(crate) fn compile_with_report(inputs: BackendProgram<'_>) -> (Vec<u8>, CodegenReport) {
    let mut report = CodegenReport::default();
    let wasm = compile_internal(inputs, Some(&mut report));
    (wasm, report)
}

fn compile_internal(inputs: BackendProgram<'_>, report: Option<&mut CodegenReport>) -> Vec<u8> {
    // These contracts describe immutable compiler-owned tables, independent of
    // the source, profile, or injected standard-library graph.
    static VALIDATED_CONTRACTS: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    VALIDATED_CONTRACTS.get_or_init(|| {
        let intrinsic_effect_errors = runtime_helper_registry::validate_intrinsic_effects();
        assert!(
            intrinsic_effect_errors.is_empty(),
            "invalid trusted intrinsic implementation contracts: {}",
            intrinsic_effect_errors.join("; ")
        );
    });
    let BackendProgram {
        standard_library,
        program,
        semantics,
        wasm_ir,
        constructed_types,
        memory_layouts,
        equality,
        capabilities,
        scoped_globals,
        source_name,
        source,
    } = inputs;
    let program = program.as_ref();
    let ConstructedTypes {
        enums,
        arrays: array_types,
        options: option_types,
        results: result_types,
        asyncs: async_types,
        iterators: iterator_types,
        callables: callable_types,
        ranges: range_types,
        sets: set_types,
        applications: application_types,
    } = constructed_types;
    let semantics = &semantics;
    let enums = &enums;
    let array_types = &array_types;
    let option_types = &option_types;
    let result_types = &result_types;
    let iterator_types = &iterator_types;
    let callable_types = &callable_types;
    let set_types = &set_types;
    let application_types = &application_types;
    let wasm_ir = &wasm_ir;
    let state = program.state.as_ref().unwrap();
    let provider = semantics
        .state_provider()
        .map(|provider| standard_library.state_provider(provider));
    let mut process_name_values = Vec::<String>::new();
    let mut append_process_names =
        |provider: &crate::stdlib::StdlibStateProvider, source_processes: &[String]| {
            let names: Vec<&str> = match provider.processes {
                StateProviderProcesses::Declared(processes) => processes.to_vec(),
                StateProviderProcesses::SourceState => {
                    source_processes.iter().map(String::as_str).collect()
                }
            };
            for name in names {
                if !process_name_values.iter().any(|existing| existing == name) {
                    process_name_values.push(name.to_owned());
                }
            }
        };
    if let Some(provider) = provider {
        append_process_names(provider, &state.processes);
    }
    let provider_declarations = state
        .provider_alternatives
        .iter()
        .filter_map(|alternative| {
            let (provider, _) = semantics.state_provider_alternative(alternative.variant)?;
            Some((
                alternative.variant,
                provider,
                standard_library.state_provider(provider),
                alternative.processes.as_slice(),
            ))
        })
        .collect::<Vec<_>>();
    for (_, _, declaration, processes) in &provider_declarations {
        append_process_names(declaration, processes);
    }
    let process_names = process_name_values
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let cancellation_region = [ActionKind::OnAttach, ActionKind::WhileAttached]
        .into_iter()
        .filter_map(|action| {
            wasm_ir
                .body(BodyOwner::Action(action))
                .and_then(|body| body.cancellation_region)
        })
        .next();
    let provider_attachment = provider.and_then(|provider| {
        provider_attachment_function(provider, program, semantics, &standard_library)
    });
    let provider_validation = provider.and_then(|provider| {
        provider_validation_function(provider, program, semantics, &standard_library)
    });
    let provider_alternatives = provider_declarations
        .iter()
        .filter_map(|(variant, provider, declaration, _)| {
            provider_attachment_function(declaration, program, semantics, &standard_library)
                .map(|function| (*variant, *provider, function))
        })
        .collect::<Vec<_>>();
    let provider_alternative_validations = provider_declarations
        .iter()
        .filter_map(|(variant, provider, declaration, _)| {
            provider_validation_function(declaration, program, semantics, &standard_library)
                .map(|function| (*variant, *provider, function))
        })
        .collect::<Vec<_>>();
    let provider_preparation = provider_preparation_function(program, semantics, &standard_library);
    let mut reachability = reachability::Reachability::analyze(
        program,
        semantics,
        wasm_ir,
        &standard_library,
        capabilities,
        provider_attachment
            .clone()
            .into_iter()
            .chain(
                provider_alternatives
                    .iter()
                    .map(|(_, _, function)| function.clone()),
            )
            .chain(provider_validation.clone())
            .chain(
                provider_alternative_validations
                    .iter()
                    .map(|(_, _, function)| function.clone()),
            )
            .chain(provider_preparation.clone()),
    );
    let async_frames = AsyncFrameLayouts::plan(program, wasm_ir, semantics, &reachability);
    let exact_runtime = ExactRuntimeRepresentations::analyze(
        program,
        wasm_ir,
        semantics,
        &reachability,
        &async_frames,
    );
    let managed = crate::managed::ManagedBindingPlan::build(program, semantics);
    let explicit_shape_selection = crate::shape_selection::has_explicit_shape_selection(program);
    let automatic_shape = if explicit_shape_selection {
        None
    } else {
        managed.automatic_shape.as_ref().filter(|plan| {
            plan.evidence_fields.is_empty()
                || semantics.state_provider() == Some(crate::stdlib::StdlibStateProviderId::Unity)
        })
    };
    let mut dependencies = BackendDependencies::analyze(
        program,
        semantics,
        wasm_ir,
        &reachability,
        capabilities,
        automatic_shape,
    );
    reachability.require_runtime_helper_types(
        &dependencies,
        array_types,
        program,
        semantics,
        &standard_library,
        capabilities,
    );
    let failure_payloads =
        FailurePayloadDemand::analyze(semantics, wasm_ir, &reachability, &managed, capabilities);
    if dependencies.require_managed_error_paths(
        program,
        semantics,
        &reachability,
        capabilities,
        &failure_payloads,
    ) {
        reachability.require_runtime_helper_types(
            &dependencies,
            array_types,
            program,
            semantics,
            &standard_library,
            capabilities,
        );
    }
    let static_data = StaticData::collect(
        program,
        &process_names,
        automatic_shape,
        wasm_ir,
        &reachability,
        &dependencies,
    );
    let strings = &static_data.strings;
    let signatures = &static_data.signatures;

    let gc_types::EncodedTypes {
        section: mut types,
        next_type_index: first_import_type,
        layout: mut gc,
    } = gc_types::encode(gc_types::Inputs {
        standard_library: &standard_library,
        program,
        wasm_ir,
        semantics,
        async_frames: &async_frames,
        enums,
        array_types,
        option_types,
        result_types,
        async_types: &async_types,
        iterator_types,
        callable_types,
        set_types,
        application_types,
        range_types: &range_types,
        reachability: &reachability,
    });
    if wasm_ir.profile() == crate::BuildProfile::Release {
        gc.string_literals.enable(static_data.segment_count());
    }
    let mut function_types = function_types::FunctionTypes::new(first_import_type);
    let imports::EncodedImports {
        section: imports,
        abi,
        function_count: imported_functions,
    } = imports::encode(&mut types, &mut function_types, &dependencies);

    let pointer_prefixes =
        pointer_prefixes::PointerPrefixPlan::build(program, semantics, &standard_library);
    let global_plan::GlobalPlan {
        section: globals,
        runtime: runtime_globals,
        variables: global_indices,
        variable_types: global_types,
        settings: setting_indices,
        managed_state_reads,
        provider_values,
        provider_attachment_frames,
    } = global_plan::encode(global_plan::Inputs {
        program,
        semantics,
        gc: &gc,
        wasm_ir,
        managed: &managed,
        reachability: &reachability,
        provider_attachment: provider_attachment.as_ref(),
        provider_alternatives: &provider_alternatives,
        provider_preparation: provider_preparation.as_ref(),
        needs_native_pointer_size: dependencies.needs_native_pointer_size(),
    });

    let function_plan::FunctionPlan {
        section: functions,
        runtime_helpers,
        equality: equality_functions,
        managed_equality: managed_equality_functions,
        array_functions,
        sets: set_functions,
        users: user_functions,
        closures: closure_functions,
        function_values: function_value_functions,
        closure_resumes,
        leaf_futures,
        displays: display_functions,
        managed_state_reads: managed_state_read_functions,
        managed_snapshots: managed_snapshot_functions,
        managed_decoders: managed_decoder_functions,
        managed_references: managed_reference_functions,
        managed_contracts: managed_contract_functions,
        managed_freezers,
        reads: read_functions,
        transforms: transform_functions,
        actions: action_functions,
        start: start_function,
        update: update_function,
        arrays: helper_arrays,
        debug_names: function_debug_names,
    } = function_plan::encode(
        &mut types,
        &mut function_types,
        imported_functions,
        report.is_some(),
        function_plan::Inputs {
            standard_library: &standard_library,
            program,
            semantics,
            enums,
            arrays: array_types,
            options: option_types,
            results: result_types,
            sets: set_types,
            equality,
            capabilities,
            structural: capabilities.structural_types(),
            dependencies: &dependencies,
            reachability: &reachability,
            gc: &gc,
            wasm_ir,
            async_frames: &async_frames,
            managed_state_reads: &managed_state_reads,
            pointer_prefixes: &pointer_prefixes,
        },
    );
    let debug_recorder = (wasm_ir.profile() == crate::BuildProfile::Debug)
        .then(debug_artifacts::DebugRecorder::default);
    let mut codes = code_bodies::CodeBodies::new(
        imported_functions,
        functions.len() as usize,
        debug_recorder.as_ref(),
    );
    let lowering = EmissionContext {
        program,
        standard_library: &standard_library,
        reachability: &reachability,
        failure_payloads: &failure_payloads,
        capabilities,
        abi: &abi,
        state,
        globals: &global_indices,
        global_types: &global_types,
        settings: &setting_indices,
        runtime_globals,
        provider_values: &provider_values,
        process_names: &process_names,
        runtime_helpers: &runtime_helpers,
        functions: &user_functions,
        closures: &closure_functions,
        function_values: &function_value_functions,
        closure_resumes: &closure_resumes,
        leaf_futures: &leaf_futures,
        display_functions: &display_functions,
        equality_functions: &equality_functions,
        managed_equality_functions: &managed_equality_functions,
        array_functions: &array_functions,
        set_functions: &set_functions,
        structs: &program.structs,
        managed: &managed,
        managed_state_reads: &managed_state_reads,
        managed_state_read_functions: &managed_state_read_functions,
        managed_snapshot_functions: &managed_snapshot_functions,
        managed_decoder_functions: &managed_decoder_functions,
        managed_reference_functions: &managed_reference_functions,
        managed_contract_functions: &managed_contract_functions,
        managed_freezers: &managed_freezers,
        enums,
        arrays: array_types,
        memory: memory_layouts,
        abi_read: static_data.layout().scratch().abi_read,
        runtime_scratch: static_data.layout().scratch(),
        signatures,
        semantics,
        wasm_ir,
        gc: &gc,
        async_frames: &async_frames,
        exact_runtime: &exact_runtime,
        explicit_shape_selection,
        debug: debug_recorder.as_ref(),
    };
    let runtime = AttachContext {
        abi: &abi,
        strings,
        lowering: &lowering,
    };
    let settings_context = settings::SettingsContext {
        abi: &abi,
        enums,
        gc: &gc,
        runtime_globals,
        semantics,
    };
    let provider_attach = provider_attachment.as_ref().map(|instance| {
        let plan = user_functions[instance];
        let layout = async_frames
            .function(instance)
            .expect("source provider attachments must suspend");
        let (completion_field, completion_type) = layout
            .completion
            .expect("source provider attachments return their provider value");
        debug_assert_eq!(
            completion_type,
            Type::Standard(provider.unwrap().process_type)
        );
        ProviderAttach {
            init: plan.call,
            poll: plan.resume.expect("source provider attachments are async"),
            frame_global: runtime_globals
                .provider_attachment_frame
                .expect("source provider attachments have frame storage"),
            frame_type: gc.function_frame_index(instance),
            completion_field,
            validation: provider_validation
                .as_ref()
                .map(|instance| user_functions[instance].call),
        }
    });
    let runtime_provider_alternatives = provider_declarations
        .iter()
        .map(|(variant, provider, declaration, source_processes)| {
            let attachment = provider_alternatives
                .iter()
                .find(|(candidate, _, _)| candidate == variant)
                .map(|(_, _, instance)| {
                    let plan = user_functions[instance];
                    let layout = async_frames
                        .function(instance)
                        .expect("source provider attachments must suspend");
                    let (completion_field, completion_type) = layout
                        .completion
                        .expect("source provider attachments return their provider value");
                    debug_assert_eq!(completion_type, Type::Standard(declaration.process_type));
                    ProviderAttach {
                        init: plan.call,
                        poll: plan.resume.expect("source provider attachments are async"),
                        frame_global: provider_attachment_frames[variant],
                        frame_type: gc.function_frame_index(instance),
                        completion_field,
                        validation: provider_alternative_validations
                            .iter()
                            .find(|(candidate, _, _)| candidate == variant)
                            .map(|(_, _, instance)| user_functions[instance].call),
                    }
                });
            ProviderAlternative {
                provider: *provider,
                declaration,
                source_processes,
                attachment,
            }
        })
        .collect::<Vec<_>>();
    let provider_preparation = provider_preparation.as_ref().map(|instance| {
        let plan = user_functions[instance];
        let layout = async_frames
            .function(instance)
            .expect("source provider preparations must suspend");
        let (completion_field, completion_type) = layout
            .completion
            .expect("source provider preparations return a runtime context");
        ProviderPreparation {
            init: plan.call,
            poll: plan.resume.expect("source provider preparations are async"),
            frame_global: runtime_globals
                .provider_preparation_frame
                .expect("source provider preparations have frame storage"),
            frame_type: gc.function_frame_index(instance),
            completion_field,
            value_global: runtime_globals
                .provider_preparation_value
                .expect("source provider preparations have result storage"),
            value_type: completion_type,
            ready_global: runtime_globals
                .provider_prepared
                .expect("source provider preparations have readiness storage"),
        }
    });
    let attachment_globals = wasm_ir.attachment_globals().collect::<Vec<_>>();
    let attempt_globals = wasm_ir.attempt_globals().collect::<Vec<_>>();
    let update_context = update::UpdateContext {
        standard_library: &standard_library,
        abi: &abi,
        gc: &gc,
        failure_payloads: &failure_payloads,
        runtime_helpers: &runtime_helpers,
        runtime_globals,
        provider_values: &provider_values,
        semantics,
        managed: &managed,
        managed_state_reads: &managed_state_reads,
        pointer_prefixes: &pointer_prefixes,
        scratch: static_data.layout().scratch(),
        explicit_shape_selection,
        globals: &global_indices,
        global_types: &global_types,
        attachment_globals: &attachment_globals,
        attempt_globals: &attempt_globals,
        scoped_globals,
        process_names: &process_names,
        provider_attach,
        provider_alternatives: &runtime_provider_alternatives,
        provider_preparation,
    };

    let helper_inputs = runtime_helpers::RuntimeHelperInputs {
        abi: &abi,
        strings,
        plan: &runtime_helpers,
        arrays: helper_arrays,
        program,
        semantics,
        settings: &settings_context,
        settings_map: &setting_indices,
        gc: &gc,
        failure_payloads: &failure_payloads,
        memory: static_data.layout(),
        float_format: static_data.float_format.as_ref(),
    };
    let helper_bodies = runtime_helpers::compile_runtime(&runtime_helpers, &helper_inputs);
    let equality_bodies = runtime_helpers::compile_equality(
        &runtime_helpers,
        capabilities.structural_types(),
        array_types,
        option_types,
        result_types,
        semantics,
        &equality_functions,
        &gc,
    );
    let display_bodies = display::compile(&display::DisplayInputs {
        program,
        structural: capabilities.structural_types(),
        arrays: array_types,
        semantics,
        displays: &display_functions,
        users: &user_functions,
        helpers: &runtime_helpers,
        debug_depth: runtime_globals.debug_depth,
        globals: &global_indices,
        selected_provider: runtime_globals.selected_provider,
        gc: &gc,
    });
    let array_bodies = array_functions::compile(array_types, &array_functions, semantics, &gc);
    let frozen_sets = reachability
        .managed_decoders()
        .filter_map(|source| {
            let output = capabilities.managed_decoder(source).unwrap().output;
            match semantics.types().kind(output) {
                crate::types::TypeKind::Set { layout, .. } => Some(*layout),
                _ => None,
            }
        })
        .collect();
    let set_bodies = set_functions::compile(
        set_types,
        &set_functions,
        semantics,
        &equality_functions,
        runtime_helpers
            .optional_function(RuntimeHelperId::StringEquality)
            .unwrap_or(0),
        &gc,
        &frozen_sets,
    );
    for body in helper_bodies {
        codes.push(&body);
    }
    let refresh_settings = runtime_helpers.optional_function(RuntimeHelperId::RefreshSettings);
    for body in equality_bodies {
        codes.push(&body);
    }
    for ty in reachability.managed_equality() {
        if let Some(body) = runtime_helpers::compile_managed_equality(ty, &lowering) {
            codes.push(&body);
        }
    }
    for body in display_bodies {
        codes.push(&body);
    }
    for body in array_bodies {
        codes.push(&body);
    }
    for body in set_bodies {
        codes.push(&body);
    }
    for storage in managed_state_reads.entries() {
        let body = expression::compile_managed_static_read(storage, &lowering);
        codes.push(&body);
    }
    for class in reachability.managed_snapshots() {
        let body = managed_snapshots::compile(class, &lowering);
        codes.push(&body);
    }
    for value in reachability.managed_references() {
        codes.push(&managed_references::compile(value, &lowering));
    }
    for value in reachability.managed_decoders() {
        codes.push(&managed_decoders::compile(value, capabilities, &lowering));
    }
    for source in
        crate::managed_read::schema_contracts(reachability.managed_decoders(), capabilities)
    {
        codes.push(&managed_decoders::contracts::compile(source, &lowering));
    }
    for value in managed_freezers::required(&reachability, capabilities, semantics) {
        codes.push(&managed_freezers::compile(value, &lowering));
    }
    for instance in reachability.functions() {
        let function = program
            .functions
            .iter()
            .find(|function| function.id == instance.function)
            .expect("reachable function instances have source declarations");
        if let Some(layout) = async_frames.function(instance) {
            let plan = user_functions
                .get(instance)
                .expect("reachable functions have final function plans");
            let body = compile_async_function_init(function, instance, layout, &lowering);
            codes.push(&body);
            match wasm_ir
                .body(wasm_ir::BodyOwner::Function(instance.clone()))
                .expect("reachable functions have Wasm IR bodies")
                .abi
            {
                wasm_ir::BodyAbi::AsyncFunction(_) => {
                    let body = compile_async_function_poll(
                        instance,
                        plan.resume.expect("async functions have poll entry points"),
                        layout,
                        &runtime,
                    );
                    codes.push(&body);
                }
                wasm_ir::BodyAbi::Generator(_) => {
                    if let Some(next) = plan.resume {
                        let body =
                            compile_generator_function_next(instance, next, layout, &runtime);
                        codes.push(&body);
                    }
                }
                wasm_ir::BodyAbi::Direct | wasm_ir::BodyAbi::AsyncAction => {
                    unreachable!("planned continuation frames have continuation bodies")
                }
            }
        } else {
            let function_index = user_functions[instance].call;
            let body = compile_user_function(function, instance, function_index, &lowering);
            codes.push(&body);
        }
    }
    for (instance, layout) in async_frames.leaves() {
        let function_index = leaf_futures[instance];
        let body = compile_leaf_future_poll(instance, function_index, layout, &runtime);
        codes.push(&body);
    }
    for (field_index, field) in state.all_fields().enumerate() {
        let body = compile_read(
            field,
            read_functions[field_index],
            pointer_prefixes.field(field.id),
            &abi,
            strings,
            &lowering,
        );
        codes.push(&body);
        if field.transform.is_some() {
            let function_index = transform_functions[field_index]
                .expect("filtered state fields have transform functions");
            let body = compile_state_transform(field, function_index, &lowering);
            codes.push(&body);
        }
    }
    for action in &program.actions {
        if let Some(layout) = async_frames.action(action.kind) {
            let body =
                compile_async_action(action, action_functions[&action.kind], layout, &runtime);
            codes.push(&body);
        } else {
            let body = compile_action(action, action_functions[&action.kind], &lowering);
            codes.push(&body);
        }
    }
    let body = compile_start(
        program,
        &settings_context,
        &lowering,
        strings,
        &setting_indices,
        module_start::StartFunctions {
            start: start_function,
            refresh_settings,
            setup: action_functions.get(&ActionKind::Setup).copied(),
        },
        async_frames.actions().len() != 0,
    );
    codes.push(&body);
    let body = compile_update(
        program,
        strings,
        StatePollFunctions {
            reads: &read_functions,
            transforms: &transform_functions,
        },
        &action_functions,
        refresh_settings,
        cancellation_region,
        &update_context,
    );
    codes.push(&body);
    for (instance, layout) in async_frames.closures() {
        let Some(&resume) = closure_resumes.get(instance) else {
            continue;
        };
        let closure = wasm_ir
            .closure(instance.expression)
            .expect("continuation-backed closure instances have bodies");
        let body = match closure.abi {
            wasm_ir::BodyAbi::AsyncFunction(_) => {
                compile_async_closure_poll(instance, closure, resume, layout, &runtime)
            }
            wasm_ir::BodyAbi::Generator(_) => {
                compile_generator_closure_next(instance, closure, resume, layout, &runtime)
            }
            wasm_ir::BodyAbi::Direct | wasm_ir::BodyAbi::AsyncAction => {
                unreachable!("planned closure frames have continuation bodies")
            }
        };
        codes.push(&body);
    }
    for instance in reachability.closure_instances() {
        let closure = wasm_ir
            .closure(instance.expression)
            .expect("reachable closure instances have bodies");
        if let Some(layout) = async_frames.closure(instance) {
            let body = compile_async_closure_init(instance, closure, layout, &lowering);
            codes.push(&body);
        } else {
            let body = compile_closure(instance, closure, closure_functions[instance], &lowering);
            codes.push(&body);
        }
    }
    for instance in reachability.function_value_instances() {
        let body = compile_function_value_adapter(instance, &lowering);
        codes.push(&body);
    }

    let debug_artifacts = debug_recorder.as_ref().map(|recorder| {
        debug_artifacts::DebugArtifactPlan::new(debug_artifacts::DebugArtifactInputs {
            abi: &abi,
            defined_functions: &function_debug_names,
            recorder,
            global_indices: &global_indices,
            global_types: &global_types,
            program,
            source_name,
            source,
        })
    });

    if let Some(report) = report {
        let memory = static_data.layout();
        *report = CodegenReport {
            functions: function_debug_names,
            runtime_helpers: runtime_helpers
                .entries()
                .map(|helper| format!("{helper:?}"))
                .collect(),
            scratch_bytes: memory.scratch_bytes(),
            abi_read_capacity: memory.scratch().abi_read.capacity() as u32,
            static_data_start: memory.static_data_start(),
            static_data_end: memory.static_data_end(),
            minimum_memory_pages: memory.minimum_pages(),
        };
    }

    module_assembly::finish(
        module_assembly::Sections {
            types,
            imports,
            functions,
            globals,
            referenced_functions: {
                let mut functions = closure_functions
                    .values()
                    .chain(function_value_functions.values())
                    .chain(managed_contract_functions.values())
                    .copied()
                    .collect::<Vec<_>>();
                functions.sort_unstable();
                functions
            },
            codes: codes.finish(),
        },
        &static_data,
        &gc.string_literals,
        start_function,
        update_function,
        debug_artifacts.as_ref(),
    )
}

fn resolved_intrinsic(target: &wasm_ir::CallTarget) -> Option<IntrinsicId> {
    match target {
        wasm_ir::CallTarget::Intrinsic { intrinsic, .. } => Some(*intrinsic),
        wasm_ir::CallTarget::UserFunction { .. }
        | wasm_ir::CallTarget::UserMethod { .. }
        | wasm_ir::CallTarget::LibraryOverload { .. }
        | wasm_ir::CallTarget::CapabilityRequirement { .. }
        | wasm_ir::CallTarget::DefaultFormatting { .. }
        | wasm_ir::CallTarget::GeneratorNext { .. }
        | wasm_ir::CallTarget::IteratorIdentity { .. }
        | wasm_ir::CallTarget::ManagedSnapshot { .. }
        | wasm_ir::CallTarget::ManagedComponent { .. }
        | wasm_ir::CallTarget::ManagedInstances { .. }
        | wasm_ir::CallTarget::ResultError { .. }
        | wasm_ir::CallTarget::OptionSome { .. }
        | wasm_ir::CallTarget::IteratorItem { .. }
        | wasm_ir::CallTarget::ResultSuccess { .. } => None,
    }
}

fn call_target(wasm_ir: &wasm_ir::Program, expression: ExprId) -> Option<&wasm_ir::CallTarget> {
    let wasm_ir::ExpressionKind::Call { target, .. } = &wasm_ir
        .expression(expression)
        .expect("checked call belongs to Wasm IR")
        .kind
    else {
        return None;
    };
    Some(target)
}

fn semantic_type(id: TypeId, semantics: &SemanticModel) -> Type {
    match semantics.types().kind(id) {
        TypeKind::Error => unreachable!("failed inference reached code generation"),
        TypeKind::Builtin(builtin) => Type::from_core(*builtin),
        TypeKind::Standard(standard) => Type::Standard(*standard),
        TypeKind::StateSnapshot => Type::StateSnapshot,
        TypeKind::SettingsView => Type::SettingsView,
        TypeKind::Struct(structure) => Type::Struct(*structure),
        TypeKind::Enum(enumeration) => Type::Enum(*enumeration),
        TypeKind::ManagedClass(class) => Type::ManagedClass(*class),
        TypeKind::ManagedReference(_) => Type::Address,
        TypeKind::GenericParameter { .. } => {
            unreachable!("generic template types must be substituted before code generation")
        }
        TypeKind::Array { layout, .. } => Type::Array(*layout),
        TypeKind::Option { layout, .. } => Type::Option(*layout),
        TypeKind::Result { layout, .. } => Type::Result(*layout),
        TypeKind::Async { layout, .. } => Type::Async(*layout),
        TypeKind::Iterator { layout, .. } => Type::Iterator(*layout),
        TypeKind::Callable { layout, .. } => Type::Callable(*layout),
        TypeKind::Set { layout, .. } => Type::Set(*layout),
        TypeKind::Range { layout, .. } => Type::Range(*layout),
        TypeKind::Application { layout, .. } => Type::Application(*layout),
    }
}

fn application_type_argument(
    application: crate::ast::TypeApplicationId,
    expected_constructor: crate::stdlib::StdlibTypeConstructorId,
    semantics: &SemanticModel,
) -> Type {
    semantics
        .types()
        .iter()
        .find_map(|(_, kind)| match kind {
            TypeKind::Application {
                layout,
                constructor,
                arguments,
            } if *layout == application && *constructor == expected_constructor => {
                Some(semantic_type(arguments[0], semantics))
            }
            _ => None,
        })
        .expect("checked named application has its declared constructor and argument")
}

fn range_bound_type(range: crate::ast::RangeTypeId, semantics: &SemanticModel) -> Type {
    semantics
        .types()
        .iter()
        .find_map(|(_, kind)| match kind {
            TypeKind::Range { layout, bound, .. } if *layout == range => {
                Some(semantic_type(*bound, semantics))
            }
            _ => None,
        })
        .expect("checked range has a concrete bound type")
}

fn value_type(value: ValueId, semantics: &SemanticModel) -> Type {
    semantic_type(
        semantics
            .value_type(value)
            .expect("checked value declarations have semantic types"),
        semantics,
    )
}

fn state_storage_index(field: ValueId, semantics: &SemanticModel) -> (u32, ValueId) {
    let storage = semantics
        .state_storage_field(field)
        .expect("checked state fields have physical storage");
    let index = semantics
        .state_storage_fields()
        .iter()
        .position(|candidate| *candidate == storage)
        .expect("physical state field belongs to the snapshot layout");
    (index as u32, storage)
}

fn struct_field_type(field: StructFieldId, semantics: &SemanticModel) -> Type {
    semantic_type(
        semantics
            .struct_field_type(field)
            .expect("checked struct fields have semantic types"),
        semantics,
    )
}

fn managed_snapshot_field_type(
    field: crate::ast::ManagedFieldId,
    semantics: &SemanticModel,
) -> Type {
    semantic_type(
        semantics
            .managed_field_snapshot_type(field)
            .expect("checked managed fields have semantic value types"),
        semantics,
    )
}

fn standard_field_type(field: crate::stdlib::StdlibFieldId, semantics: &SemanticModel) -> Type {
    semantic_type(
        semantics
            .standard_field_type(field)
            .expect("checked standard fields have semantic types"),
        semantics,
    )
}

fn enum_variant_payload(variant: EnumVariantId, semantics: &SemanticModel) -> Option<Type> {
    semantics
        .enum_variant_payload(variant)
        .map(|payload| semantic_type(payload, semantics))
}

fn array_element_type(array: ArrayTypeId, semantics: &SemanticModel) -> Type {
    try_array_element_type(array, semantics)
        .expect("concrete array layouts have backend-representable element types")
}

fn set_element_type(set: crate::ast::TypeApplicationId, semantics: &SemanticModel) -> Type {
    semantics
        .types()
        .iter()
        .find_map(|(_, kind)| match kind {
            TypeKind::Set {
                layout, element, ..
            } if *layout == set => Some(semantic_type(*element, semantics)),
            _ => None,
        })
        .expect("checked set layouts have semantic element types")
}

fn try_array_element_type(array: ArrayTypeId, semantics: &SemanticModel) -> Option<Type> {
    let element = semantics
        .array_element_type(array)
        .expect("checked array layouts have semantic element types");
    (!matches!(
        semantics.types().kind(element),
        TypeKind::GenericParameter { .. }
    ))
    .then(|| semantic_type(element, semantics))
}

fn option_value_type(option: OptionTypeId, semantics: &SemanticModel) -> Type {
    semantics
        .types()
        .iter()
        .find_map(|(_, kind)| match kind {
            TypeKind::Option { layout, value } if *layout == option => {
                Some(semantic_type(*value, semantics))
            }
            _ => None,
        })
        .expect("checked option layouts have semantic value types")
}

fn result_value_type(result: ResultTypeId, semantics: &SemanticModel) -> Type {
    semantics
        .types()
        .iter()
        .find_map(|(_, kind)| match kind {
            TypeKind::Result { layout, value } if *layout == result => {
                Some(semantic_type(*value, semantics))
            }
            _ => None,
        })
        .expect("checked result layouts have semantic value types")
}

fn emit_struct_get(function: &mut Function, field_index: u32, ty: Type) {
    emit_typed_struct_get(function, STATE_TYPE, field_index, ty);
}

fn emit_typed_struct_get(
    function: &mut Function,
    struct_type_index: u32,
    field_index: u32,
    ty: Type,
) {
    let instruction = match ty {
        Type::Bool | Type::U8 | Type::U16 => Instruction::StructGetU {
            struct_type_index,
            field_index,
        },
        Type::I8 | Type::I16 => Instruction::StructGetS {
            struct_type_index,
            field_index,
        },
        _ => Instruction::StructGet {
            struct_type_index,
            field_index,
        },
    };
    function.instruction(&instruction);
}

fn emit_frame_typed_struct_get(
    function: &mut Function,
    struct_type_index: u32,
    field_index: u32,
    ty: Type,
    gc: &GcLayout,
) {
    emit_typed_struct_get(function, struct_type_index, field_index, ty);
    if matches!(
        gc.val_type(ty),
        ValType::Ref(RefType {
            nullable: false,
            ..
        })
    ) {
        function.instruction(&Instruction::RefAsNonNull);
    }
}

fn emit_array_get(function: &mut Function, array_type_index: u32, element: Type, gc: &GcLayout) {
    function.instruction(&match element {
        Type::Bool | Type::U8 | Type::U16 => Instruction::ArrayGetU(array_type_index),
        Type::I8 | Type::I16 => Instruction::ArrayGetS(array_type_index),
        _ => Instruction::ArrayGet(array_type_index),
    });
    if matches!(
        gc.val_type(element),
        ValType::Ref(RefType {
            nullable: false,
            ..
        })
    ) {
        function.instruction(&Instruction::RefAsNonNull);
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_memory_value(
    function: &mut Function,
    ty: TypeId,
    scratch: memory_plan::AbiReadScratch,
    offset: u32,
    memory: &MemoryLayouts,
    semantics: &SemanticModel,
    gc: &GcLayout,
    address_width: MemoryAddressWidth,
    byte_order: MemoryByteOrder,
) {
    match memory
        .layout(ty, semantics, address_width)
        .expect("checked memory values are MemoryReadable")
    {
        MemoryTypeLayout::Scalar { .. } => {
            emit_memory_load(
                function,
                semantic_type(ty, semantics),
                scratch.at(offset),
                address_width,
                byte_order,
            );
        }
        MemoryTypeLayout::Enum(layout) => {
            let TypeKind::Enum(enumeration) = semantics.types().kind(layout.ty) else {
                unreachable!("source enum memory layouts have source enum types")
            };
            debug_assert!(!layout.variants.is_empty());
            for (index, variant) in layout
                .variants
                .iter()
                .take(layout.variants.len().saturating_sub(1))
                .enumerate()
            {
                emit_memory_load(
                    function,
                    Type::from_core(layout.representation),
                    scratch.at(offset),
                    address_width,
                    byte_order,
                );
                emit_enum_discriminant(function, layout.representation, variant.value);
                function
                    .instruction(&enum_discriminant_equality(layout.representation))
                    .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
                    .instruction(&Instruction::I32Const(index as i32))
                    .instruction(&Instruction::Else);
            }
            function.instruction(&Instruction::I32Const(
                layout.variants.len().saturating_sub(1) as i32,
            ));
            for _ in 0..layout.variants.len().saturating_sub(1) {
                function.instruction(&Instruction::End);
            }
            for _ in &layout.variants {
                function.instruction(&Instruction::I32Const(0));
            }
            function.instruction(&Instruction::StructNew(gc.index(Type::Enum(*enumeration))));
        }
        MemoryTypeLayout::Struct(layout) => {
            for field in &layout.fields {
                emit_memory_value(
                    function,
                    field.ty,
                    scratch,
                    offset + field.offset,
                    memory,
                    semantics,
                    gc,
                    address_width,
                    byte_order,
                );
            }
            function.instruction(&Instruction::StructNew(
                gc.index(semantic_type(layout.ty, semantics)),
            ));
        }
        MemoryTypeLayout::FixedArray(layout) => {
            for index in 0..layout.length {
                emit_memory_value(
                    function,
                    layout.element,
                    scratch,
                    offset + index * layout.stride,
                    memory,
                    semantics,
                    gc,
                    address_width,
                    byte_order,
                );
            }
            let Type::Array(array) = semantic_type(layout.ty, semantics) else {
                unreachable!("fixed memory array layouts have array types")
            };
            array_value::emit_new_fixed(function, gc, array, layout.length);
        }
    }
}

/// Emits a boolean only when `ty` contains a represented enum whose raw
/// discriminant needs validation. The caller can therefore avoid generating
/// a branch for ordinary scalar/aggregate reads.
#[allow(clippy::too_many_arguments)]
fn emit_memory_value_is_valid(
    function: &mut Function,
    ty: TypeId,
    scratch: memory_plan::AbiReadScratch,
    offset: u32,
    memory: &MemoryLayouts,
    semantics: &SemanticModel,
    address_width: MemoryAddressWidth,
    byte_order: MemoryByteOrder,
) -> bool {
    match memory
        .layout(ty, semantics, address_width)
        .expect("checked memory values are MemoryReadable")
    {
        MemoryTypeLayout::Scalar { .. } => false,
        MemoryTypeLayout::Enum(layout) => {
            if layout.variants.is_empty() {
                function.instruction(&Instruction::I32Const(0));
                return true;
            }
            for (index, variant) in layout.variants.iter().enumerate() {
                emit_memory_load(
                    function,
                    Type::from_core(layout.representation),
                    scratch.at(offset),
                    address_width,
                    byte_order,
                );
                emit_enum_discriminant(function, layout.representation, variant.value);
                function.instruction(&enum_discriminant_equality(layout.representation));
                if index != 0 {
                    function.instruction(&Instruction::I32Or);
                }
            }
            true
        }
        MemoryTypeLayout::Struct(layout) => {
            let mut emitted = false;
            for field in &layout.fields {
                if emit_memory_value_is_valid(
                    function,
                    field.ty,
                    scratch,
                    offset + field.offset,
                    memory,
                    semantics,
                    address_width,
                    byte_order,
                ) {
                    if emitted {
                        function.instruction(&Instruction::I32And);
                    }
                    emitted = true;
                }
            }
            emitted
        }
        MemoryTypeLayout::FixedArray(layout) => {
            let mut emitted = false;
            for index in 0..layout.length {
                if emit_memory_value_is_valid(
                    function,
                    layout.element,
                    scratch,
                    offset + index * layout.stride,
                    memory,
                    semantics,
                    address_width,
                    byte_order,
                ) {
                    if emitted {
                        function.instruction(&Instruction::I32And);
                    }
                    emitted = true;
                }
            }
            emitted
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_memory_value_result(
    function: &mut Function,
    ty: TypeId,
    result: ResultTypeId,
    error: &str,
    scratch: memory_plan::AbiReadScratch,
    offset: u32,
    memory: &MemoryLayouts,
    semantics: &SemanticModel,
    gc: &GcLayout,
    failure_payloads: &failure_payload::FailurePayloadDemand,
    address_width: MemoryAddressWidth,
    byte_order: MemoryByteOrder,
) {
    if emit_memory_value_is_valid(
        function,
        ty,
        scratch,
        offset,
        memory,
        semantics,
        address_width,
        byte_order,
    ) {
        function.instruction(&Instruction::If(BlockType::Result(
            gc.val_type(Type::Result(result)),
        )));
        emit_memory_value(
            function,
            ty,
            scratch,
            offset,
            memory,
            semantics,
            gc,
            address_width,
            byte_order,
        );
        emit_result_success(function, result, gc);
        function.instruction(&Instruction::Else);
        emit_result_error(
            function,
            result,
            semantic_type(ty, semantics),
            error,
            gc,
            failure_payloads,
        );
        function.instruction(&Instruction::End);
    } else {
        emit_memory_value(
            function,
            ty,
            scratch,
            offset,
            memory,
            semantics,
            gc,
            address_width,
            byte_order,
        );
        emit_result_success(function, result, gc);
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_native_memory_value(
    function: &mut Function,
    ty: TypeId,
    scratch: memory_plan::AbiReadScratch,
    offset: u32,
    memory: &MemoryLayouts,
    semantics: &SemanticModel,
    gc: &GcLayout,
    pointer_size: Option<u32>,
    byte_order: MemoryByteOrder,
) {
    if !memory.depends_on_address_width(ty, semantics) {
        emit_memory_value(
            function,
            ty,
            scratch,
            offset,
            memory,
            semantics,
            gc,
            MemoryAddressWidth::Bit64,
            byte_order,
        );
        return;
    }
    let pointer_size = pointer_size.expect("pointer-dependent reads plan native-width storage");
    function
        .instruction(&Instruction::GlobalGet(pointer_size))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Result(
            gc.val_type(semantic_type(ty, semantics)),
        )));
    emit_memory_value(
        function,
        ty,
        scratch,
        offset,
        memory,
        semantics,
        gc,
        MemoryAddressWidth::Bit32,
        byte_order,
    );
    function.instruction(&Instruction::Else);
    emit_memory_value(
        function,
        ty,
        scratch,
        offset,
        memory,
        semantics,
        gc,
        MemoryAddressWidth::Bit64,
        byte_order,
    );
    function.instruction(&Instruction::End);
}

#[allow(clippy::too_many_arguments)]
fn emit_native_memory_value_is_valid(
    function: &mut Function,
    ty: TypeId,
    scratch: memory_plan::AbiReadScratch,
    offset: u32,
    memory: &MemoryLayouts,
    semantics: &SemanticModel,
    pointer_size: Option<u32>,
    byte_order: MemoryByteOrder,
) -> bool {
    if !memory.depends_on_address_width(ty, semantics) {
        return emit_memory_value_is_valid(
            function,
            ty,
            scratch,
            offset,
            memory,
            semantics,
            MemoryAddressWidth::Bit64,
            byte_order,
        );
    }
    // Whether validation is required depends on represented enums, not on
    // pointer width, so both runtime branches have the same stack shape.
    if !memory.requires_validation(ty, semantics) {
        return false;
    }
    let pointer_size = pointer_size.expect("pointer-dependent reads plan native-width storage");
    function
        .instruction(&Instruction::GlobalGet(pointer_size))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    emit_memory_value_is_valid(
        function,
        ty,
        scratch,
        offset,
        memory,
        semantics,
        MemoryAddressWidth::Bit32,
        byte_order,
    );
    function.instruction(&Instruction::Else);
    emit_memory_value_is_valid(
        function,
        ty,
        scratch,
        offset,
        memory,
        semantics,
        MemoryAddressWidth::Bit64,
        byte_order,
    );
    function.instruction(&Instruction::End);
    true
}

#[allow(clippy::too_many_arguments)]
fn emit_native_memory_value_result(
    function: &mut Function,
    ty: TypeId,
    result: ResultTypeId,
    error: &str,
    scratch: memory_plan::AbiReadScratch,
    offset: u32,
    memory: &MemoryLayouts,
    semantics: &SemanticModel,
    gc: &GcLayout,
    failure_payloads: &failure_payload::FailurePayloadDemand,
    pointer_size: Option<u32>,
    byte_order: MemoryByteOrder,
) {
    if emit_native_memory_value_is_valid(
        function,
        ty,
        scratch,
        offset,
        memory,
        semantics,
        pointer_size,
        byte_order,
    ) {
        function.instruction(&Instruction::If(BlockType::Result(
            gc.val_type(Type::Result(result)),
        )));
        emit_native_memory_value(
            function,
            ty,
            scratch,
            offset,
            memory,
            semantics,
            gc,
            pointer_size,
            byte_order,
        );
        emit_result_success(function, result, gc);
        function.instruction(&Instruction::Else);
        emit_result_error(
            function,
            result,
            semantic_type(ty, semantics),
            error,
            gc,
            failure_payloads,
        );
        function.instruction(&Instruction::End);
    } else {
        emit_native_memory_value(
            function,
            ty,
            scratch,
            offset,
            memory,
            semantics,
            gc,
            pointer_size,
            byte_order,
        );
        emit_result_success(function, result, gc);
    }
}

fn emit_native_memory_read_destination_and_size(
    function: &mut Function,
    ty: TypeId,
    scratch: memory_plan::AbiReadScratch,
    memory: &MemoryLayouts,
    semantics: &SemanticModel,
    pointer_size: Option<u32>,
) {
    let bit32 = memory
        .layout(ty, semantics, MemoryAddressWidth::Bit32)
        .expect("checked native reads have a 32-bit layout")
        .size();
    let bit64 = memory
        .layout(ty, semantics, MemoryAddressWidth::Bit64)
        .expect("checked native reads have a 64-bit layout")
        .size();
    function.instruction(&Instruction::I32Const(
        scratch.destination(bit32.max(bit64)),
    ));
    if memory.depends_on_address_width(ty, semantics) {
        let pointer_size = pointer_size.expect("pointer-dependent reads plan native-width storage");
        function
            .instruction(&Instruction::GlobalGet(pointer_size))
            .instruction(&Instruction::I32Const(4))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
            .instruction(&Instruction::I32Const(bit32 as i32))
            .instruction(&Instruction::Else)
            .instruction(&Instruction::I32Const(bit64 as i32))
            .instruction(&Instruction::End);
    } else {
        function.instruction(&Instruction::I32Const(bit64 as i32));
    }
}

fn emit_enum_discriminant(
    function: &mut Function,
    representation: crate::types::BuiltinType,
    value: i128,
) {
    if matches!(
        representation,
        crate::types::BuiltinType::I64 | crate::types::BuiltinType::U64
    ) {
        function.instruction(&Instruction::I64Const(value as i64));
    } else {
        function.instruction(&Instruction::I32Const(value as i32));
    }
}

fn enum_discriminant_equality(representation: crate::types::BuiltinType) -> Instruction<'static> {
    if matches!(
        representation,
        crate::types::BuiltinType::I64 | crate::types::BuiltinType::U64
    ) {
        Instruction::I64Eq
    } else {
        Instruction::I32Eq
    }
}

fn emit_memory_load(
    function: &mut Function,
    ty: Type,
    address: i32,
    address_width: MemoryAddressWidth,
    byte_order: MemoryByteOrder,
) {
    if byte_order == MemoryByteOrder::Big && !matches!(ty, Type::Bool | Type::U8 | Type::I8) {
        emit_big_endian_memory_load(function, ty, address, address_width);
        return;
    }
    function.instruction(&Instruction::I32Const(address));
    function.instruction(&match ty {
        Type::Bool | Type::U8 => Instruction::I32Load8U(memarg()),
        Type::I8 => Instruction::I32Load8S(memarg()),
        Type::U16 => Instruction::I32Load16U(memarg()),
        Type::I16 => Instruction::I32Load16S(memarg()),
        Type::I32 | Type::U32 => Instruction::I32Load(memarg()),
        Type::Address if address_width == MemoryAddressWidth::Bit32 => {
            function
                .instruction(&Instruction::I32Load(memarg()))
                .instruction(&Instruction::I64ExtendI32U);
            return;
        }
        Type::I64 | Type::U64 | Type::Address => Instruction::I64Load(memarg()),
        Type::F32 => Instruction::F32Load(memarg()),
        Type::F64 => Instruction::F64Load(memarg()),
        _ => unreachable!(),
    });
}

fn emit_big_endian_memory_load(
    function: &mut Function,
    ty: Type,
    address: i32,
    address_width: MemoryAddressWidth,
) {
    let (bytes, wide) = match ty {
        Type::U16 | Type::I16 => (2, false),
        Type::I32 | Type::U32 | Type::F32 => (4, false),
        Type::Address => (address_width.bytes(), true),
        Type::I64 | Type::U64 | Type::F64 => (8, true),
        _ => unreachable!("non-scalar or byte-sized values do not need big-endian assembly"),
    };

    for byte in 0..bytes {
        function
            .instruction(&Instruction::I32Const(address))
            .instruction(&Instruction::I32Load8U(MemArg {
                offset: u64::from(byte),
                align: 0,
                memory_index: 0,
            }));
        let shift = (bytes - byte - 1) * 8;
        if wide {
            function.instruction(&Instruction::I64ExtendI32U);
            if shift != 0 {
                function
                    .instruction(&Instruction::I64Const(shift as i64))
                    .instruction(&Instruction::I64Shl);
            }
            if byte != 0 {
                function.instruction(&Instruction::I64Or);
            }
        } else {
            if shift != 0 {
                function
                    .instruction(&Instruction::I32Const(shift as i32))
                    .instruction(&Instruction::I32Shl);
            }
            if byte != 0 {
                function.instruction(&Instruction::I32Or);
            }
        }
    }

    match ty {
        Type::I16 => {
            function.instruction(&Instruction::I32Extend16S);
        }
        Type::F32 => {
            function.instruction(&Instruction::F32ReinterpretI32);
        }
        Type::F64 => {
            function.instruction(&Instruction::F64ReinterpretI64);
        }
        _ => {}
    }
}

fn emit_default(function: &mut Function, ty: Type, gc: &GcLayout) {
    function.instruction(&match gc.val_type(ty) {
        ValType::I32 => Instruction::I32Const(0),
        ValType::I64 => Instruction::I64Const(0),
        ValType::F32 => Instruction::F32Const(0.0.into()),
        ValType::F64 => Instruction::F64Const(0.0.into()),
        ValType::Ref(_) => Instruction::RefNull(GC_NULL_HEAP_TYPE),
        ValType::V128 => unreachable!(),
    });
}

/// Reads the WASI monotonic clock and leaves its nanosecond timestamp on the
/// operand stack. Keeping this host boundary shared prevents `Instant.now()`
/// and future combinators from drifting to different clock IDs or precision.
fn emit_monotonic_nanoseconds(function: &mut Function, abi: &Abi, destination: i32) {
    function
        // WASI clock ID 1 is the monotonic clock. A precision of one requests
        // the finest available nanosecond reading.
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I32Const(destination))
        .instruction(&Instruction::Call(
            abi.function(crate::abi::AbiImportId::WasiClockTimeGet),
        ))
        .instruction(&Instruction::If(wasm_encoder::BlockType::Empty))
        .instruction(&Instruction::Unreachable)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(destination))
        .instruction(&Instruction::I64Load(memarg()));
}

/// Wraps the value already on the operand stack in a successful `T!`.
fn emit_result_success(function: &mut Function, result: ResultTypeId, gc: &GcLayout) {
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
        .instruction(&Instruction::StructNew(gc.index(Type::Result(result))));
}

fn emit_result_error(
    function: &mut Function,
    result: ResultTypeId,
    value_type: Type,
    message: &str,
    gc: &GcLayout,
    failure_payloads: &FailurePayloadDemand,
) {
    emit_default(function, value_type, gc);
    function.instruction(&Instruction::I32Const(1));
    if failure_payloads.is_demanded(result) {
        emit_string_literal(function, message, gc);
    } else {
        function.instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE));
    }
    function.instruction(&Instruction::StructNew(gc.index(Type::Result(result))));
}

/// Constructs an error for the nearest compiled failure boundary.
///
/// Both explicit `throw` and postfix `?` use this Result construction. The
/// caller transfers it to a direct return or an asynchronous completion slot.
fn emit_failure_value(
    function: &mut Function,
    target: ResultTypeId,
    target_value: Type,
    gc: &GcLayout,
    materialize_payload: bool,
    preserve_discarded_payload: bool,
    emit_error: impl FnOnce(&mut Function),
) {
    emit_default(function, target_value, gc);
    function.instruction(&Instruction::I32Const(1));
    if materialize_payload {
        emit_error(function);
    } else {
        if preserve_discarded_payload {
            emit_error(function);
            function.instruction(&Instruction::Drop);
        }
        function.instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE));
    }
    function.instruction(&Instruction::StructNew(gc.index(Type::Result(target))));
}

fn emit_int(function: &mut Function, value: u64, ty: Type) {
    match ty {
        Type::I64 | Type::U64 | Type::Address => {
            function.instruction(&Instruction::I64Const(value as i64));
        }
        Type::F32 => {
            function.instruction(&Instruction::F32Const((value as f32).into()));
        }
        Type::F64 => {
            function.instruction(&Instruction::F64Const((value as f64).into()));
        }
        _ => {
            function.instruction(&Instruction::I32Const(value as i32));
        }
    }
}

fn emit_integer_literal(function: &mut Function, value: u64, negative: bool, ty: Type) {
    match ty {
        Type::F32 => {
            let value = value as f32;
            function.instruction(&Instruction::F32Const(
                (if negative { -value } else { value }).into(),
            ));
        }
        Type::F64 => {
            let value = value as f64;
            function.instruction(&Instruction::F64Const(
                (if negative { -value } else { value }).into(),
            ));
        }
        _ => emit_int(
            function,
            if negative {
                value.wrapping_neg()
            } else {
                value
            },
            ty,
        ),
    }
}

fn emit_string_literal(function: &mut Function, value: &str, gc: &GcLayout) {
    if let Some((segment, offset)) = gc.string_literals.intern(value) {
        function
            .instruction(&Instruction::I32Const(offset as i32))
            .instruction(&Instruction::I32Const(value.len() as i32))
            .instruction(&Instruction::ArrayNewData {
                array_type_index: gc.standard_index(StdlibTypeId::String),
                array_data_index: segment,
            });
        return;
    }
    for byte in value.bytes() {
        function.instruction(&Instruction::I32Const(byte as i32));
    }
    function.instruction(&Instruction::ArrayNewFixed {
        array_type_index: gc.standard_index(StdlibTypeId::String),
        array_size: value.len() as u32,
    });
}

fn constant(expression: ExprId, wasm_ir: &wasm_ir::Program, ty: Type) -> ConstExpr {
    let expression = wasm_ir
        .expression(expression)
        .expect("global initializer belongs to Wasm IR");
    let (operator, inner) = match &expression.kind {
        wasm_ir::ExpressionKind::Call {
            target:
                wasm_ir::CallTarget::Intrinsic {
                    intrinsic:
                        intrinsic @ (IntrinsicId::SignedNegate
                        | IntrinsicId::BoolNot
                        | IntrinsicId::IntegerBitNot),
                    receiver:
                        Some(ResolvedReceiver::Expression {
                            expression: inner,
                            members,
                        }),
                    ..
                },
            arguments,
        } if members.is_empty() && arguments.is_empty() => (
            Some(*intrinsic),
            wasm_ir
                .expression(*inner)
                .expect("global initializer operand belongs to Wasm IR"),
        ),
        _ => (None, expression),
    };
    let outer_negative = operator == Some(IntrinsicId::SignedNegate);
    let inverted = operator == Some(IntrinsicId::BoolNot);
    let complemented = operator == Some(IntrinsicId::IntegerBitNot);
    match &inner.kind {
        wasm_ir::ExpressionKind::None => ConstExpr::ref_null(HeapType::Abstract {
            shared: false,
            ty: AbstractHeapType::None,
        }),
        wasm_ir::ExpressionKind::Bool(value) => ConstExpr::i32_const((*value ^ inverted) as i32),
        wasm_ir::ExpressionKind::Char(value) => ConstExpr::i32_const(*value as i32),
        wasm_ir::ExpressionKind::Int { value, negative } if ty == Type::F32 => {
            ConstExpr::f32_const(
                (if *negative ^ outer_negative {
                    -(*value as f32)
                } else {
                    *value as f32
                })
                .into(),
            )
        }
        wasm_ir::ExpressionKind::Int { value, negative } if ty == Type::F64 => {
            ConstExpr::f64_const(
                (if *negative ^ outer_negative {
                    -(*value as f64)
                } else {
                    *value as f64
                })
                .into(),
            )
        }
        wasm_ir::ExpressionKind::Int { value, negative }
            if matches!(ty, Type::I64 | Type::U64 | Type::Address) =>
        {
            let value = if *negative {
                value.wrapping_neg()
            } else {
                *value
            };
            let value = if outer_negative {
                value.wrapping_neg()
            } else {
                value
            };
            ConstExpr::i64_const(if complemented {
                !(value as i64)
            } else {
                value as i64
            })
        }
        wasm_ir::ExpressionKind::Int { value, negative } => {
            let value = if *negative {
                value.wrapping_neg()
            } else {
                *value
            };
            let value = if outer_negative {
                value.wrapping_neg()
            } else {
                value
            } as i32;
            ConstExpr::i32_const(if complemented {
                match ty {
                    Type::U8 => !value & 0xff,
                    Type::U16 => !value & 0xffff,
                    _ => !value,
                }
            } else {
                value
            })
        }
        wasm_ir::ExpressionKind::Float(literal) if ty == Type::F32 => {
            let value = literal
                .normalized
                .parse::<f32>()
                .expect("checked f32 literals fit their target");
            ConstExpr::f32_const((if outer_negative { -value } else { value }).into())
        }
        wasm_ir::ExpressionKind::Float(literal) => {
            let value = literal.value;
            ConstExpr::f64_const((if outer_negative { -value } else { value }).into())
        }
        _ => unreachable!(),
    }
}

/// Whether an initializer can remain in the Wasm global section instead of
/// executing in the module start body. Keep this deliberately narrower than
/// source-level purity: Wasm constant expressions cannot call even a pure
/// helper or allocate a GC aggregate.
fn is_wasm_global_constant(expression: ExprId, wasm_ir: &wasm_ir::Program) -> bool {
    let Some(expression) = wasm_ir.expression(expression) else {
        return false;
    };
    if matches!(
        expression.kind,
        wasm_ir::ExpressionKind::None
            | wasm_ir::ExpressionKind::Bool(_)
            | wasm_ir::ExpressionKind::Char(_)
            | wasm_ir::ExpressionKind::Int { .. }
            | wasm_ir::ExpressionKind::Float(_)
    ) {
        return true;
    }
    let wasm_ir::ExpressionKind::Call {
        target:
            wasm_ir::CallTarget::Intrinsic {
                intrinsic:
                    IntrinsicId::SignedNegate | IntrinsicId::BoolNot | IntrinsicId::IntegerBitNot,
                receiver:
                    Some(ResolvedReceiver::Expression {
                        expression: inner,
                        members,
                    }),
                ..
            },
        arguments,
    } = &expression.kind
    else {
        return false;
    };
    members.is_empty()
        && arguments.is_empty()
        && wasm_ir.expression(*inner).is_some_and(|inner| {
            matches!(
                inner.kind,
                wasm_ir::ExpressionKind::Bool(_)
                    | wasm_ir::ExpressionKind::Int { .. }
                    | wasm_ir::ExpressionKind::Float(_)
            )
        })
}

fn action_result_val_type(action: ActionKind, semantics: &SemanticModel, gc: &GcLayout) -> ValType {
    if action == ActionKind::GameTime {
        return ValType::Ref(RefType {
            nullable: true,
            heap_type: HeapType::Concrete(gc.standard_index(StdlibTypeId::Duration)),
        });
    }
    if action != ActionKind::SelectProcess {
        return ValType::I32;
    }
    let result = semantics
        .action_result(action)
        .expect("checked actions have semantic result types");
    gc.val_type(semantic_type(result, semantics))
}

fn memarg() -> MemArg {
    MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }
}

#[cfg(test)]
mod architecture_tests {
    use std::{fs, path::Path};

    use wasm_encoder::{
        CodeSection, ConstExpr, DataSection, ExportKind, ExportSection, FunctionSection,
        MemorySection, MemoryType, Module, TypeSection,
    };

    use super::*;

    #[test]
    fn big_endian_scalar_decoder_preserves_integer_float_and_sign_bits() {
        let mut types = TypeSection::new();
        types.ty().function([], [ValType::I32]);
        types.ty().function([], [ValType::F32]);
        let mut functions = FunctionSection::new();
        functions.function(0);
        functions.function(1);
        functions.function(0);
        let mut memories = MemorySection::new();
        memories.memory(MemoryType {
            minimum: 1,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        let mut exports = ExportSection::new();
        exports.export("integer", ExportKind::Func, 0);
        exports.export("float", ExportKind::Func, 1);
        exports.export("signed", ExportKind::Func, 2);
        let mut code = CodeSection::new();
        for (ty, address) in [(Type::U32, 0), (Type::F32, 4), (Type::I16, 8)] {
            let mut function = Function::new([]);
            emit_memory_load(
                &mut function,
                ty,
                address,
                MemoryAddressWidth::Bit64,
                MemoryByteOrder::Big,
            );
            function.instruction(&Instruction::End);
            code.function(&function);
        }
        let mut data = DataSection::new();
        data.active(
            0,
            &ConstExpr::i32_const(0),
            [0x12, 0x34, 0x56, 0x78, 0x3f, 0xc0, 0x00, 0x00, 0xff, 0xfe],
        );
        let mut module = Module::new();
        module.section(&types);
        module.section(&functions);
        module.section(&memories);
        module.section(&exports);
        module.section(&code);
        module.section(&data);

        let engine = wasmtime::Engine::default();
        let module = wasmtime::Module::new(&engine, module.finish()).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        assert_eq!(
            instance
                .get_typed_func::<(), i32>(&mut store, "integer")
                .unwrap()
                .call(&mut store, ())
                .unwrap() as u32,
            0x1234_5678
        );
        assert_eq!(
            instance
                .get_typed_func::<(), f32>(&mut store, "float")
                .unwrap()
                .call(&mut store, ())
                .unwrap(),
            1.5
        );
        assert_eq!(
            instance
                .get_typed_func::<(), i32>(&mut store, "signed")
                .unwrap()
                .call(&mut store, ())
                .unwrap(),
            -2
        );
    }

    fn visit_rust_files(path: &Path, check: &mut impl FnMut(&Path)) {
        for entry in fs::read_dir(path).expect("codegen source directory should be readable") {
            let path = entry
                .expect("codegen source entry should be readable")
                .path();
            if path.is_dir() {
                visit_rust_files(&path, check);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                check(&path);
            }
        }
    }

    #[test]
    fn codegen_modules_do_not_reintroduce_parent_wildcard_imports() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/codegen");
        visit_rust_files(&root, &mut |path| {
            let source = fs::read_to_string(path).expect("codegen source should be UTF-8");
            assert!(
                !source.lines().any(|line| line.trim() == "use super::*;"),
                "{} reintroduced an implicit parent-module prelude",
                path.display()
            );
        });
    }
}

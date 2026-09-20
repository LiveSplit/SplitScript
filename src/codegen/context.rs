//! Immutable context shared by Wasm body emitters after all plans are fixed.

use std::collections::HashMap;

use crate::{
    ast::{EnumDecl, ManagedClassId, ManagedFieldId, StructDecl, ValueId},
    managed::ManagedBindingPlan,
    memory::MemoryLayouts,
    semantic::{FunctionInstance, SemanticModel},
    stdlib::{StandardLibrary, StdlibStateProviderId},
    types::ResolvedArrayType,
    wasm_ir,
};

use super::{
    ArrayFunctions, DisplayFunctions, EqualityFunctions, GcLayout, RuntimeHelperPlan, SetFunctions,
    SettingStorage, Type,
    data_plan::{SignaturePool, StringPool},
    debug_artifacts::DebugRecorder,
    global_plan::RuntimeGlobals,
    imports::Abi,
    managed_state_reads::ManagedStateReadCache,
    memory_plan::{AbiReadScratch, RuntimeScratch},
};

/// Shared immutable inputs needed while emitting script-owned bodies.
///
/// This is intentionally constructed only after type, function, global,
/// memory, helper, and GC plans are complete. Individual emitters should take
/// a narrower input when they do not need this complete view.
pub(super) struct EmissionContext<'a> {
    pub program: &'a crate::ast::Program,
    pub standard_library: &'a StandardLibrary,
    pub reachability: &'a super::reachability::Reachability,
    pub failure_payloads: &'a super::failure_payload::FailurePayloadDemand,
    pub capabilities: &'a crate::capabilities::CapabilityAnalysis,
    pub abi: &'a Abi,
    pub state: &'a crate::ast::StateDecl,
    pub globals: &'a HashMap<ValueId, u32>,
    pub global_types: &'a HashMap<ValueId, Type>,
    pub settings: &'a HashMap<ValueId, SettingStorage>,
    pub runtime_globals: RuntimeGlobals,
    pub provider_values: &'a HashMap<StdlibStateProviderId, u32>,
    pub process_names: &'a [&'a str],
    pub runtime_helpers: &'a RuntimeHelperPlan,
    pub functions: &'a HashMap<FunctionInstance, super::function_plan::UserFunctionPlan>,
    pub closures: &'a HashMap<crate::semantic::ClosureInstance, u32>,
    pub function_values: &'a HashMap<crate::semantic::FunctionValueInstance, u32>,
    pub closure_resumes: &'a HashMap<crate::semantic::ClosureInstance, u32>,
    pub leaf_futures: &'a HashMap<super::async_frame::LeafFutureInstance, u32>,
    pub display_functions: &'a DisplayFunctions,
    pub equality_functions: &'a EqualityFunctions,
    pub managed_equality_functions: &'a EqualityFunctions,
    pub array_functions: &'a ArrayFunctions,
    pub set_functions: &'a SetFunctions,
    pub structs: &'a [StructDecl],
    pub managed: &'a ManagedBindingPlan,
    pub managed_state_reads: &'a ManagedStateReadCache,
    pub managed_state_read_functions: &'a HashMap<ManagedFieldId, u32>,
    pub managed_freezers: &'a HashMap<crate::types::TypeId, u32>,
    pub managed_contract_functions: &'a HashMap<crate::types::TypeId, u32>,
    pub managed_decoder_functions: &'a HashMap<crate::types::TypeId, u32>,
    pub managed_reference_functions: &'a HashMap<crate::types::TypeId, u32>,
    pub managed_snapshot_functions: &'a HashMap<ManagedClassId, u32>,
    pub enums: &'a [EnumDecl],
    pub arrays: &'a [ResolvedArrayType],
    pub memory: &'a MemoryLayouts,
    pub abi_read: AbiReadScratch,
    pub runtime_scratch: RuntimeScratch,
    pub signatures: &'a SignaturePool,
    pub semantics: &'a SemanticModel,
    pub wasm_ir: &'a wasm_ir::Program,
    pub gc: &'a GcLayout,
    pub async_frames: &'a super::async_frame::AsyncFrameLayouts,
    pub exact_runtime: &'a super::exact_runtime::ExactRuntimeRepresentations,
    pub explicit_shape_selection: bool,
    pub debug: Option<&'a DebugRecorder>,
}

impl<'a> EmissionContext<'a> {
    pub fn debug_emission(
        &self,
        function: u32,
    ) -> Option<super::debug_artifacts::DebugEmission<'a>> {
        self.debug.map(|recorder| recorder.emission(function))
    }
}

/// Extra pools required only by the async attachment state machine.
pub(super) struct AttachContext<'a> {
    pub abi: &'a Abi,
    pub strings: &'a StringPool,
    pub lowering: &'a EmissionContext<'a>,
}

use std::collections::BTreeSet;

use crate::{
    abi::AbiImportId,
    ast::{ActionKind, ManagedFieldId, Program, SettingFileFilter, SettingKind, StateSource},
    intrinsic_registry::{self, DependencyRoot, RuntimeHelperId},
    semantic::{ResolvedMember, ResolvedValue, SemanticModel},
    stdlib::{CoreTypeId, Implementation, IntrinsicId, StdlibItemId, StdlibTypeId},
    types::{TypeId, TypeKind},
    wasm_ir,
};

use super::runtime_helper_registry;

#[derive(Debug, Default)]
pub(super) struct BackendDependencies {
    stdlib_items: BTreeSet<StdlibItemId>,
    helpers: BTreeSet<RuntimeHelperId>,
    host_imports: BTreeSet<AbiImportId>,
    needs_native_pointer_size: bool,
    memory_read_capacity: u32,
}

impl BackendDependencies {
    pub fn analyze(
        program: &Program,
        semantics: &SemanticModel,
        wasm_ir: &wasm_ir::Program,
        reachability: &super::reachability::Reachability,
        capabilities: &crate::capabilities::CapabilityAnalysis,
        automatic_shape: Option<&crate::shape_selection::ShapeSelectionPlan>,
    ) -> Self {
        let mut dependencies = Self::default();
        if program
            .actions
            .iter()
            .any(|action| action.kind == ActionKind::SelectProcess)
        {
            dependencies.require_import(AbiImportId::ProcessAttachByPid);
            dependencies.require_import(AbiImportId::ProcessListByName);
        } else {
            dependencies.require_import(AbiImportId::ProcessAttach);
        }
        dependencies.require_import(AbiImportId::ProcessDetach);
        dependencies.require_import(AbiImportId::ProcessIsOpen);
        // Polling rates are lifecycle policy even when source never calls the
        // dynamic setTickRate API directly.
        dependencies.require_import(AbiImportId::RuntimeSetTickRate);
        if automatic_shape.is_some() {
            dependencies.require_import(AbiImportId::RuntimePrintMessage);
        }

        // Snapshot readers materialize every instance field of a reachable
        // class. Direct field paths below cover static and live-reference
        // reads. Keeping these roots separate prevents an unused declaration
        // from retaining the managed-string decoder.
        for class in reachability.managed_snapshots() {
            let declaration = program
                .managed_class(class)
                .expect("reachable managed classes belong to the program");
            if declaration
                .all_fields()
                .filter(|field| !field.is_static)
                .any(|field| {
                    semantics.managed_field_snapshot_type(field.id)
                        != semantics.managed_field_value_type(field.id)
                        || semantics
                            .managed_field_value_type(field.id)
                            .is_some_and(|value| {
                                matches!(
                                    crate::managed::ManagedFieldRead::for_type(value, semantics),
                                    crate::managed::ManagedFieldRead::ManagedString { .. }
                                )
                            })
                })
                || declaration
                    .all_fields()
                    .filter(|field| !field.is_static)
                    .count()
                    > crate::managed_read::MAX_SNAPSHOT_WORK as usize
            {
                dependencies.require(RuntimeHelperId::EnterManagedObject);
                dependencies.require(RuntimeHelperId::ChargeManagedWork);
            }
            for field in declaration.all_fields().filter(|field| !field.is_static) {
                dependencies.require_managed_field_reader(
                    field.id,
                    program,
                    semantics,
                    capabilities,
                );
            }
        }

        if let Some(state) = &program.state {
            let pointer_providers = state
                .all_fields()
                .filter(|field| matches!(field.source, StateSource::Pointer(_)))
                .filter_map(|field| semantics.state_field_provider(field.id))
                .collect::<BTreeSet<_>>();
            for provider in pointer_providers {
                let provider = wasm_ir.standard_library().state_provider(provider);
                let Implementation::Intrinsic(direct_read) = wasm_ir
                    .standard_library()
                    .item(provider.direct_read)
                    .implementation
                else {
                    unreachable!("validated state-provider reads are intrinsic")
                };
                dependencies.require_intrinsic(direct_read);
            }
            for field in state.all_fields() {
                if let StateSource::Pointer(path) = &field.source {
                    dependencies.require_import(AbiImportId::ProcessRead);
                    if let Some(decoder) = path.decoder {
                        dependencies.require(match decoder {
                            crate::ast::StateMemoryDecoder::Utf8 { .. } => {
                                RuntimeHelperId::ReadUtf8String
                            }
                            crate::ast::StateMemoryDecoder::Utf16Le { .. } => {
                                RuntimeHelperId::ReadUtf16LeString
                            }
                        });
                    }
                    if matches!(path.base, crate::ast::PointerPathBase::Module { .. }) {
                        dependencies.require_import(AbiImportId::ProcessGetModuleAddress);
                    }
                    let direct_read = semantics.state_field_provider(field.id).map(|provider| {
                        let Implementation::Intrinsic(intrinsic) = wasm_ir
                            .standard_library()
                            .item(
                                wasm_ir
                                    .standard_library()
                                    .state_provider(provider)
                                    .direct_read,
                            )
                            .implementation
                        else {
                            unreachable!("provider reads are intrinsic")
                        };
                        intrinsic
                    });
                    let native = direct_read == Some(IntrinsicId::ProcessRead);
                    let value =
                        semantics
                            .value_type(field.id)
                            .map(|ty| match semantics.types().kind(ty) {
                                TypeKind::Option { value, .. } => *value,
                                _ => ty,
                            });
                    if path.decoder.is_none() {
                        dependencies.require_memory_read(
                            value.expect("checked pointer fields have value types"),
                            direct_read
                                .and_then(intrinsic_registry::provider_read_contract)
                                .map(|contract| contract.address_width),
                            semantics,
                            capabilities,
                        );
                    }
                    if native
                        && (!path.offsets.is_empty()
                            || value.is_some_and(|ty| {
                                capabilities
                                    .memory()
                                    .depends_on_address_width(ty, semantics)
                            }))
                    {
                        dependencies.needs_native_pointer_size = true;
                    }
                }
            }
        }
        for action in &program.actions {
            match action.kind {
                ActionKind::OnStart | ActionKind::OnReset => {
                    dependencies.require_import(AbiImportId::TimerGetState);
                }
                ActionKind::Start => {
                    dependencies.require_import(AbiImportId::TimerGetState);
                    dependencies.require_import(AbiImportId::TimerStart);
                }
                ActionKind::Split => {
                    dependencies.require_import(AbiImportId::TimerGetState);
                    dependencies.require_import(AbiImportId::TimerSplit);
                }
                ActionKind::Reset => {
                    dependencies.require_import(AbiImportId::TimerGetState);
                    dependencies.require_import(AbiImportId::TimerReset);
                }
                ActionKind::IsLoading => {
                    dependencies.require_import(AbiImportId::TimerGetState);
                    dependencies.require_import(AbiImportId::TimerPauseGameTime);
                    dependencies.require_import(AbiImportId::TimerResumeGameTime);
                }
                ActionKind::GameTime => {
                    dependencies.require_import(AbiImportId::TimerGetState);
                    dependencies.require_import(AbiImportId::TimerSetGameTime);
                }
                ActionKind::Setup
                | ActionKind::SelectProcess
                | ActionKind::OnAttach
                | ActionKind::OnDetach
                | ActionKind::OnStateReady
                | ActionKind::WhileAttached => {}
            }
        }

        for (owner, expression_id) in reachability.expression_instances() {
            let expression = wasm_ir
                .expression(expression_id)
                .expect("reachable expressions belong to Wasm IR");
            let specialize = |ty| {
                owner
                    .as_ref()
                    .map_or(ty, |instance| semantics.specialize_type(instance, ty))
            };
            match &expression.kind {
                wasm_ir::ExpressionKind::Path { root, members } => {
                    if let Some(ResolvedValue::ManagedStatic { field, .. }) = root {
                        dependencies.require_managed_field_reader(
                            *field,
                            program,
                            semantics,
                            capabilities,
                        );
                    }
                    for member in members {
                        if let ResolvedMember::ManagedField(field) = member {
                            dependencies.require_managed_field_reader(
                                *field,
                                program,
                                semantics,
                                capabilities,
                            );
                        }
                    }
                }
                wasm_ir::ExpressionKind::Member { members, .. } => {
                    for member in members {
                        if let ResolvedMember::ManagedField(field) = member {
                            dependencies.require_managed_field_reader(
                                *field,
                                program,
                                semantics,
                                capabilities,
                            );
                        }
                    }
                }
                _ => {}
            }
            match &expression.kind {
                wasm_ir::ExpressionKind::Call { target, .. }
                    if matches!(
                        reachability.resolved_call_target(owner.as_ref(), expression.id, target),
                        wasm_ir::CallTarget::ManagedInstances { .. }
                    ) =>
                {
                    dependencies.require(RuntimeHelperId::ScanAlignedPointerRange);
                    dependencies.require_import(AbiImportId::ProcessGetMemoryRangeCount);
                    dependencies.require_import(AbiImportId::ProcessGetMemoryRangeAddress);
                    dependencies.require_import(AbiImportId::ProcessGetMemoryRangeSize);
                    dependencies.require_import(AbiImportId::ProcessGetMemoryRangeFlags);
                }
                wasm_ir::ExpressionKind::Call { target, .. }
                    if matches!(
                        reachability.resolved_call_target(owner.as_ref(), expression.id, target),
                        wasm_ir::CallTarget::DefaultFormatting { .. }
                    ) =>
                {
                    let wasm_ir::CallTarget::DefaultFormatting {
                        mode,
                        receiver_type,
                        ..
                    } = reachability.resolved_call_target(owner.as_ref(), expression.id, target)
                    else {
                        unreachable!()
                    };
                    let receiver_type = specialize(*receiver_type);
                    if *mode == wasm_ir::FormattingMode::Debug
                        && matches!(
                            semantics.types().kind(receiver_type),
                            TypeKind::Standard(StdlibTypeId::String)
                                | TypeKind::Builtin(CoreTypeId::Char)
                        )
                    {
                        dependencies.require(RuntimeHelperId::QuoteDebugString);
                    }
                    dependencies.require_display_helpers(
                        receiver_type,
                        semantics,
                        reachability,
                        capabilities,
                    );
                }
                wasm_ir::ExpressionKind::Call {
                    target, arguments, ..
                } if matches!(
                    reachability.resolved_call_target(owner.as_ref(), expression.id, target),
                    wasm_ir::CallTarget::Intrinsic { .. }
                ) =>
                {
                    let wasm_ir::CallTarget::Intrinsic {
                        item,
                        intrinsic,
                        receiver_type,
                        type_arguments,
                        ..
                    } = reachability.resolved_call_target(owner.as_ref(), expression.id, target)
                    else {
                        unreachable!()
                    };
                    dependencies.stdlib_items.insert(*item);
                    dependencies.require_intrinsic(*intrinsic);
                    if *intrinsic == IntrinsicId::ProcessRead
                        || intrinsic_registry::provider_read_contract(*intrinsic).is_some()
                    {
                        dependencies.require_memory_read(
                            specialize(type_arguments[0]),
                            intrinsic_registry::provider_read_contract(*intrinsic)
                                .map(|contract| contract.address_width),
                            semantics,
                            capabilities,
                        );
                    }
                    if *intrinsic == IntrinsicId::ProcessFollow
                        || (*intrinsic == IntrinsicId::ProcessRead
                            && type_arguments.first().is_some_and(|ty| {
                                capabilities
                                    .memory()
                                    .depends_on_address_width(specialize(*ty), semantics)
                            }))
                    {
                        dependencies.needs_native_pointer_size = true;
                    }
                    if matches!(
                        intrinsic,
                        IntrinsicId::MemoryReaderReadUtf8 | IntrinsicId::MemoryReaderReadUtf16Le
                    ) {
                        let receiver_type = specialize(
                            receiver_type.expect("MemoryReader intrinsics always have a receiver"),
                        );
                        let TypeKind::Standard(reader) = semantics.types().kind(receiver_type)
                        else {
                            unreachable!("concrete MemoryReader receivers are standard types")
                        };
                        let reads_utf8 = *intrinsic == IntrinsicId::MemoryReaderReadUtf8;
                        match intrinsic_registry::memory_reader_backend(*reader)
                            .expect("catalog MemoryReader implementations have a backend")
                        {
                            intrinsic_registry::MemoryReaderBackend::Process => {
                                dependencies.require(if reads_utf8 {
                                    RuntimeHelperId::ReadUtf8String
                                } else {
                                    RuntimeHelperId::ReadUtf16LeString
                                });
                            }
                            intrinsic_registry::MemoryReaderBackend::Provider(contract) => {
                                dependencies.require(contract.reader);
                                dependencies.require(if reads_utf8 {
                                    RuntimeHelperId::Utf8StringFromMemory
                                } else {
                                    RuntimeHelperId::Utf16LeStringFromMemory
                                });
                            }
                        }
                    }
                    for displayed in intrinsic_registry::contract(*intrinsic)
                        .dependency_roots
                        .iter()
                        .filter_map(|root| match root {
                            DependencyRoot::DisplayArgument(index) => {
                                arguments.get(usize::from(*index))
                            }
                            DependencyRoot::Helper(_)
                            | DependencyRoot::HostImport(_)
                            | DependencyRoot::MemoryReader => None,
                        })
                    {
                        let ty = wasm_ir
                            .expression(*displayed)
                            .expect("intrinsic arguments belong to Wasm IR")
                            .ty;
                        dependencies.require_display_helpers(
                            specialize(ty),
                            semantics,
                            reachability,
                            capabilities,
                        );
                    }
                }
                wasm_ir::ExpressionKind::InterpolatedString(parts) => {
                    dependencies.require(RuntimeHelperId::JoinStrings);
                    for source in parts.iter().filter_map(|part| match part {
                        wasm_ir::InterpolatedPart::Expression {
                            string_conversion_source,
                            ..
                        } => *string_conversion_source,
                        wasm_ir::InterpolatedPart::Text(_) => None,
                    }) {
                        dependencies.require_display_helpers(
                            specialize(source),
                            semantics,
                            reachability,
                            capabilities,
                        );
                    }
                }
                wasm_ir::ExpressionKind::Cast { value }
                    if matches!(
                        semantics.types().kind(specialize(expression.ty)),
                        TypeKind::Standard(StdlibTypeId::String)
                    ) =>
                {
                    let source = wasm_ir
                        .expression(*value)
                        .expect("cast operand belongs to Wasm IR")
                        .ty;
                    let source = specialize(source);
                    dependencies.require_display_helpers(
                        source,
                        semantics,
                        reachability,
                        capabilities,
                    );
                }
                _ => {}
            }
        }

        if reachability.requires_string_equality() {
            dependencies.require(RuntimeHelperId::StringEquality);
        }

        if reachability.derived_debugs().any(|ty| {
            capabilities.derived_debug_kind(ty, semantics)
                == Some(crate::capabilities::DerivedDebugKind::Structural)
        }) {
            dependencies.require(RuntimeHelperId::JoinStrings);
            dependencies.require(RuntimeHelperId::IndentDisplay);
            dependencies.require(RuntimeHelperId::WrapDebugEntry);
            dependencies.require(RuntimeHelperId::WrapDebugVariant);
            dependencies.require(RuntimeHelperId::QuoteDebugString);
        }
        for ty in reachability.derived_debugs() {
            dependencies.require_display_helpers(ty, semantics, reachability, capabilities);
        }

        if !program.settings.is_empty() {
            let has_values = program
                .settings
                .iter()
                .any(|setting| !matches!(&setting.kind, SettingKind::Title { .. }));
            let has_string_values = program.settings.iter().any(|setting| {
                matches!(
                    &setting.kind,
                    SettingKind::Text { .. }
                        | SettingKind::Choice { .. }
                        | SettingKind::File { .. }
                )
            });
            if has_values {
                dependencies.require(RuntimeHelperId::RefreshSettings);
                dependencies.require_import(AbiImportId::SettingsMapLoad);
                dependencies.require_import(AbiImportId::SettingsMapFree);
                dependencies.require_import(AbiImportId::SettingsMapGet);
                dependencies.require_import(AbiImportId::SettingValueFree);
            }
            if has_string_values {
                dependencies.require(RuntimeHelperId::StringFromMemory);
            }
            if program
                .settings
                .iter()
                .any(|setting| matches!(&setting.kind, SettingKind::Choice { .. }))
            {
                dependencies.require(RuntimeHelperId::StringEquality);
            }
            for setting in &program.settings {
                if setting.tooltip.is_some() {
                    dependencies.require_import(AbiImportId::UserSettingsSetTooltip);
                }
                match &setting.kind {
                    SettingKind::Bool { .. } => {
                        dependencies.require_import(AbiImportId::UserSettingsAddBool);
                        dependencies.require_import(AbiImportId::SettingValueGetBool);
                    }
                    SettingKind::Text { .. } => {
                        dependencies.require_import(AbiImportId::UserSettingsAddTextInput);
                        dependencies.require_import(AbiImportId::SettingValueGetString);
                    }
                    SettingKind::Title { .. } => {
                        dependencies.require_import(AbiImportId::UserSettingsAddTitle);
                    }
                    SettingKind::Choice { .. } => {
                        dependencies.require_import(AbiImportId::UserSettingsAddChoice);
                        dependencies.require_import(AbiImportId::UserSettingsAddChoiceOption);
                        dependencies.require_import(AbiImportId::SettingValueGetString);
                    }
                    SettingKind::File { filters, .. } => {
                        dependencies.require_import(AbiImportId::UserSettingsAddFileSelect);
                        dependencies.require_import(AbiImportId::SettingValueGetString);
                        for filter in filters {
                            dependencies.require_import(match filter {
                                SettingFileFilter::Name { .. } => {
                                    AbiImportId::UserSettingsAddFileSelectNameFilter
                                }
                                SettingFileFilter::Mime { .. } => {
                                    AbiImportId::UserSettingsAddFileSelectMimeFilter
                                }
                            });
                        }
                    }
                }
            }
        }

        if dependencies.needs_native_pointer_size {
            dependencies.require(RuntimeHelperId::DetectProcessPointerSize);
            dependencies.require_import(AbiImportId::ProcessGetModuleAddress);
        }
        dependencies
    }

    fn require_managed_field_reader(
        &mut self,
        field: ManagedFieldId,
        program: &Program,
        semantics: &SemanticModel,
        capabilities: &crate::capabilities::CapabilityAnalysis,
    ) {
        let declaration = program
            .managed_class_declarations()
            .into_iter()
            .flat_map(|class| class.all_fields())
            .find(|candidate| candidate.id == field)
            .expect("resolved managed fields belong to the program");
        let value = semantics
            .managed_field_value_type(field)
            .expect("checked managed fields have semantic value types");
        if !declaration.is_static {
            self.require(RuntimeHelperId::ManagedFieldAddress);
        }
        if capabilities
            .memory()
            .depends_on_address_width(value, semantics)
        {
            self.needs_native_pointer_size = true;
        }
        if crate::managed::ManagedFieldRead::for_type(value, semantics)
            == crate::managed::ManagedFieldRead::Fixed
        {
            self.require(RuntimeHelperId::ReadManagedMemory);
            self.require_memory_read(value, None, semantics, capabilities);
            return;
        }
        self.require(
            if matches!(semantics.types().kind(value), TypeKind::Option { .. }) {
                RuntimeHelperId::ReadOptionalManagedStringField
            } else {
                RuntimeHelperId::ReadManagedStringField
            },
        );
    }

    pub fn uses_helper(&self, helper: RuntimeHelperId) -> bool {
        self.helpers.contains(&helper)
    }

    fn require_memory_read(
        &mut self,
        mut ty: TypeId,
        address_width: Option<crate::memory::MemoryAddressWidth>,
        semantics: &SemanticModel,
        capabilities: &crate::capabilities::CapabilityAnalysis,
    ) {
        if let TypeKind::Option { value, .. } = semantics.types().kind(ty) {
            ty = *value;
        }
        if matches!(semantics.types().kind(ty), TypeKind::ManagedReference(_)) {
            self.memory_read_capacity = self.memory_read_capacity.max(8);
            return;
        }
        for width in [
            crate::memory::MemoryAddressWidth::Bit32,
            crate::memory::MemoryAddressWidth::Bit64,
        ] {
            if address_width.is_some_and(|selected| selected != width) {
                continue;
            }
            let size = capabilities
                .memory()
                .layout(ty, semantics, width)
                .expect("reachable fixed-layout reads are MemoryReadable")
                .size();
            self.memory_read_capacity = self.memory_read_capacity.max(size);
        }
    }

    pub fn scratch_requirements(
        &self,
        maximum_signature_len: u32,
    ) -> super::memory_plan::ScratchRequirements {
        use super::memory_plan::{ScratchRequirements, ScratchRole};
        let mut requirements = ScratchRequirements {
            maximum_signature_len,
            read_padding: u32::from(self.uses_helper(RuntimeHelperId::GenesisReadMemory)) * 2,
            roles: self
                .helpers()
                .flat_map(|helper| {
                    runtime_helper_registry::descriptor(helper)
                        .scratch
                        .iter()
                        .copied()
                })
                .collect(),
            ..Default::default()
        };
        // Fixed helper/ABI records need at most 16 bytes; native pointer-width
        // detection reads a 64-byte executable header. Dynamic-size process
        // reads are bounded by their reachable types, not all declared types.
        if self.host_imports().any(|import| {
            matches!(
                import,
                AbiImportId::ProcessRead
                    | AbiImportId::ProcessGetModulePath
                    | AbiImportId::ProcessGetPath
                    | AbiImportId::RuntimeGetOs
                    | AbiImportId::RuntimeGetArch
                    | AbiImportId::WasiClockTimeGet
                    | AbiImportId::WasiFdPrestatGet
                    | AbiImportId::WasiPathOpen
                    | AbiImportId::WasiFdRead
                    | AbiImportId::WasiFdSeek
            )
        }) {
            // Word-swapped emulator reads can need a byte at either end.
            requirements.abi_read_capacity = self
                .memory_read_capacity
                .saturating_add(requirements.read_padding)
                .max(if self.needs_native_pointer_size {
                    64
                } else {
                    16
                });
        }
        if self
            .host_imports
            .contains(&AbiImportId::SettingValueGetString)
        {
            requirements.roles.insert(ScratchRole::SettingsString);
            requirements.roles.insert(ScratchRole::SettingsLength);
        }
        if self
            .host_imports
            .contains(&AbiImportId::SettingValueGetBool)
            || self.host_imports.contains(&AbiImportId::ProcessListByName)
        {
            requirements.roles.insert(ScratchRole::SettingsLength);
        }
        requirements
    }

    pub fn uses_float_format(&self) -> bool {
        self.uses_helper(RuntimeHelperId::FormatF32) || self.uses_helper(RuntimeHelperId::FormatF64)
    }

    pub fn helpers(&self) -> impl Iterator<Item = RuntimeHelperId> + '_ {
        runtime_helper_registry::DESCRIPTORS
            .iter()
            .map(|descriptor| descriptor.id)
            .filter(|helper| self.uses_helper(*helper))
    }

    pub fn host_imports(&self) -> impl Iterator<Item = AbiImportId> + '_ {
        self.host_imports.iter().copied()
    }

    pub fn needs_native_pointer_size(&self) -> bool {
        self.needs_native_pointer_size
    }

    fn require_intrinsic(&mut self, intrinsic: IntrinsicId) {
        for root in intrinsic_registry::contract(intrinsic).dependency_roots {
            match root {
                DependencyRoot::Helper(helper) => self.require(*helper),
                DependencyRoot::HostImport(import) => self.require_import(*import),
                DependencyRoot::DisplayArgument(_) => {}
                DependencyRoot::MemoryReader => {}
            }
        }
    }

    fn require_display_helpers(
        &mut self,
        ty: TypeId,
        semantics: &SemanticModel,
        reachability: &super::reachability::Reachability,
        capabilities: &crate::capabilities::CapabilityAnalysis,
    ) {
        let mut pending = vec![ty];
        let mut visited = BTreeSet::new();
        while let Some(ty) = pending.pop() {
            if !visited.insert(ty) {
                continue;
            }
            // A custom Display/Debug implementation is an opaque formatting
            // boundary. Its own reachable body contributes any helpers it
            // actually uses; deriving through the source type here would keep
            // an additional formatter that can never be called.
            if reachability.has_custom_formatting(ty) {
                continue;
            }
            match semantics.types().kind(ty) {
                TypeKind::Builtin(CoreTypeId::F32) => self.require(RuntimeHelperId::FormatF32),
                TypeKind::Builtin(CoreTypeId::F64) => self.require(RuntimeHelperId::FormatF64),
                TypeKind::Builtin(CoreTypeId::Char) => self.require(RuntimeHelperId::FormatChar),
                TypeKind::Builtin(
                    CoreTypeId::I8
                    | CoreTypeId::U8
                    | CoreTypeId::I16
                    | CoreTypeId::U16
                    | CoreTypeId::I32
                    | CoreTypeId::U32
                    | CoreTypeId::I64
                    | CoreTypeId::U64
                    | CoreTypeId::Address,
                ) => self.require(RuntimeHelperId::FormatI64),
                _ => pending.extend(capabilities.debug_dependency_types(ty, semantics)),
            }
        }
    }

    fn require(&mut self, helper: RuntimeHelperId) {
        if !self.helpers.insert(helper) {
            return;
        }
        let descriptor = runtime_helper_registry::descriptor(helper);
        for import in descriptor.host_imports {
            self.require_import(*import);
        }
        for dependency in descriptor.dependencies {
            self.require(*dependency);
        }
    }

    fn require_import(&mut self, import: AbiImportId) {
        self.host_imports.insert(import);
    }

    #[cfg(test)]
    pub fn with_host_imports(imports: impl IntoIterator<Item = AbiImportId>) -> Self {
        let mut dependencies = Self::default();
        dependencies.host_imports.extend(imports);
        dependencies
    }
}

#[cfg(test)]
mod tests {
    use crate::intrinsic_registry::RuntimeHelperId;

    use super::BackendDependencies;

    #[test]
    fn helper_dependencies_are_closed_transitively() {
        let mut dependencies = BackendDependencies::default();
        dependencies.require(RuntimeHelperId::ReadUtf8String);
        assert!(dependencies.uses_helper(RuntimeHelperId::ReadUtf8String));
        assert!(dependencies.uses_helper(RuntimeHelperId::Utf8StringFromMemory));
        assert!(dependencies.uses_helper(RuntimeHelperId::StringFromMemory));
    }
}

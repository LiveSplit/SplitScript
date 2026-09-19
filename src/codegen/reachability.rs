use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ast::{
        ArrayTypeId, AsyncTypeId, CallableTypeId, EnumId, ExprId, IteratorTypeId, ManagedClassId,
        OptionTypeId, Program, ResultTypeId, StructId, TypeApplicationId,
    },
    semantic::{ClosureInstance, FunctionInstance, FunctionValueInstance, SemanticModel},
    stdlib::{
        IntrinsicId, RuntimeRepresentation, StandardLibrary, StdlibCapabilityId, StdlibTypeId,
    },
    types::{ResolvedArrayType, TypeId, TypeKind},
    wasm_ir::{self, BodyOwner, Visitor},
};

#[derive(Debug, Default)]
pub(super) struct Reachability {
    functions: BTreeSet<FunctionInstance>,
    expressions: BTreeSet<ExprId>,
    closures: BTreeSet<ClosureInstance>,
    function_values: BTreeSet<FunctionValueInstance>,
    expression_instances: BTreeSet<(Option<FunctionInstance>, ExprId)>,
    equality_structs: BTreeSet<StructId>,
    equality_standard_structs: BTreeSet<StdlibTypeId>,
    equality_enums: BTreeSet<EnumId>,
    equality_arrays: BTreeSet<ArrayTypeId>,
    equality_options: BTreeSet<OptionTypeId>,
    equality_results: BTreeSet<ResultTypeId>,
    string_equality: bool,
    gc_standard: BTreeSet<StdlibTypeId>,
    gc_structs: BTreeSet<StructId>,
    gc_managed_classes: BTreeSet<ManagedClassId>,
    managed_snapshots: BTreeSet<ManagedClassId>,
    managed_decoders: BTreeSet<TypeId>,
    managed_instances: BTreeSet<ManagedClassId>,
    gc_enums: BTreeSet<EnumId>,
    gc_arrays: BTreeSet<ArrayTypeId>,
    gc_array_storage: BTreeSet<ArrayTypeId>,
    array_sets: BTreeSet<ArrayTypeId>,
    array_pushes: BTreeSet<ArrayTypeId>,
    array_removals: BTreeSet<ArrayTypeId>,
    array_clears: BTreeSet<ArrayTypeId>,
    gc_options: BTreeSet<OptionTypeId>,
    gc_results: BTreeSet<ResultTypeId>,
    gc_asyncs: BTreeSet<AsyncTypeId>,
    gc_iterators: BTreeSet<IteratorTypeId>,
    gc_callables: BTreeSet<CallableTypeId>,
    gc_sets: BTreeSet<TypeApplicationId>,
    set_operations: BTreeSet<(TypeApplicationId, IntrinsicId)>,
    gc_applications: BTreeSet<TypeApplicationId>,
    display_functions: BTreeMap<TypeId, FunctionInstance>,
    debug_functions: BTreeMap<TypeId, FunctionInstance>,
    derived_debugs: BTreeSet<TypeId>,
    capability_calls: BTreeMap<(Option<FunctionInstance>, ExprId), wasm_ir::CallTarget>,
}

impl Reachability {
    pub fn analyze(
        program: &Program,
        semantics: &SemanticModel,
        wasm_ir: &wasm_ir::Program,
        standard_library: &StandardLibrary,
        capabilities: &crate::capabilities::CapabilityAnalysis,
        provider_functions: impl IntoIterator<Item = FunctionInstance>,
    ) -> Self {
        let mut pending = Vec::new();
        let mut pending_functions = provider_functions
            .into_iter()
            .map(|function| (None, function))
            .collect::<Vec<_>>();
        for body in wasm_ir.bodies() {
            if matches!(body.owner, BodyOwner::Action(_)) {
                collect_block_expression_roots(&body.entry, wasm_ir, None, &mut pending);
                collect_assignment_function_roots(
                    &body.entry,
                    wasm_ir,
                    None,
                    &mut pending_functions,
                );
            }
        }
        for expression in wasm_ir.state_expressions() {
            collect_block_expression_roots(&expression.entry, wasm_ir, None, &mut pending);
            collect_assignment_function_roots(
                &expression.entry,
                wasm_ir,
                None,
                &mut pending_functions,
            );
        }
        for transform in wasm_ir.state_transforms() {
            collect_block_expression_roots(&transform.entry, wasm_ir, None, &mut pending);
            collect_assignment_function_roots(
                &transform.entry,
                wasm_ir,
                None,
                &mut pending_functions,
            );
        }
        for initializer in wasm_ir.global_initializer_plans() {
            collect_block_expression_roots(&initializer.entry, wasm_ir, None, &mut pending);
            collect_assignment_function_roots(
                &initializer.entry,
                wasm_ir,
                None,
                &mut pending_functions,
            );
        }

        let mut reachable = Self::default();
        loop {
            while let Some((owner, function)) = pending_functions.pop() {
                let function = owner.as_ref().map_or(function.clone(), |owner| {
                    semantics.specialize_function_instance(owner, &function)
                });
                if reachable.functions.insert(function.clone()) {
                    let body = wasm_ir
                        .body(BodyOwner::Function(function.clone()))
                        .expect("resolved user functions have Wasm IR bodies");
                    collect_block_expression_roots(
                        &body.entry,
                        wasm_ir,
                        Some(function.clone()),
                        &mut pending,
                    );
                    collect_assignment_function_roots(
                        &body.entry,
                        wasm_ir,
                        Some(function),
                        &mut pending_functions,
                    );
                }
            }
            let Some((owner, id)) = pending.pop() else {
                break;
            };
            if !reachable.expression_instances.insert((owner.clone(), id)) {
                continue;
            }
            reachable.expressions.insert(id);
            let expression = wasm_ir
                .expression(id)
                .expect("reachable expressions belong to Wasm IR");
            wasm_ir::visit_expression_children(&expression.kind, |child| {
                pending.push((owner.clone(), child))
            });
            if let wasm_ir::ExpressionKind::Match { arms, .. } = &expression.kind
                && arms.iter().any(|arm| arm.pattern.contains_string())
            {
                reachable.string_equality = true;
            }
            if let wasm_ir::ExpressionKind::Closure { closure, .. } = expression.kind {
                let instance = ClosureInstance::new(owner.clone(), closure);
                if reachable.closures.insert(instance) {
                    let body = wasm_ir
                        .closure(closure)
                        .expect("reachable closures have lowered bodies");
                    collect_block_expression_roots(
                        &body.entry,
                        wasm_ir,
                        owner.clone(),
                        &mut pending,
                    );
                    collect_assignment_function_roots(
                        &body.entry,
                        wasm_ir,
                        owner.clone(),
                        &mut pending_functions,
                    );
                }
            }
            if let wasm_ir::ExpressionKind::FunctionValue { function } = &expression.kind {
                let function = owner.as_ref().map_or(function.clone(), |owner| {
                    semantics.specialize_function_instance(owner, function)
                });
                let ty = owner.as_ref().map_or(expression.ty, |owner| {
                    semantics.specialize_type(owner, expression.ty)
                });
                let TypeKind::Callable { .. } = semantics.types().kind(ty) else {
                    unreachable!("checked function values have callable types")
                };
                reachable.function_values.insert(FunctionValueInstance {
                    function: function.clone(),
                    ty,
                });
                pending_functions.push((None, function));
            }
            for constant in constant_roots(&expression.kind) {
                let function = wasm_ir
                    .constant_function(constant)
                    .expect("resolved constants have hidden function bodies")
                    .clone();
                pending_functions.push((owner.clone(), function));
            }
            if let wasm_ir::ExpressionKind::Call { target, .. } = &expression.kind {
                let capability_call =
                    matches!(target, wasm_ir::CallTarget::CapabilityRequirement { .. });
                let resolved_target = if capability_call {
                    Some(
                        wasm_ir::resolve_capability_requirement(
                            target,
                            owner.as_ref(),
                            program,
                            semantics,
                            standard_library,
                            capabilities,
                        )
                        .expect("validated capability calls have concrete implementations"),
                    )
                } else {
                    None
                };
                if let Some(resolved) = resolved_target.clone() {
                    reachable
                        .capability_calls
                        .insert((owner.clone(), id), resolved);
                }
                let target = resolved_target.as_ref().unwrap_or(target);
                if let wasm_ir::CallTarget::Intrinsic {
                    intrinsic: IntrinsicId::EquatableEquals | IntrinsicId::EquatableNotEquals,
                    receiver_type: Some(receiver),
                    ..
                } = target
                {
                    let receiver = owner.as_ref().map_or(*receiver, |owner| {
                        semantics.specialize_type(owner, *receiver)
                    });
                    reachable.require_equality(receiver, semantics, standard_library, capabilities);
                }
                if let wasm_ir::CallTarget::Intrinsic {
                    intrinsic: IntrinsicId::ArraySet,
                    receiver_type: Some(receiver),
                    ..
                } = target
                {
                    let receiver = owner.as_ref().map_or(*receiver, |owner| {
                        semantics.specialize_type(owner, *receiver)
                    });
                    let TypeKind::Array { layout, .. } = semantics.types().kind(receiver) else {
                        unreachable!()
                    };
                    reachable.array_sets.insert(*layout);
                }
                if let wasm_ir::CallTarget::Intrinsic {
                    intrinsic: IntrinsicId::ArrayPush,
                    receiver_type: Some(receiver),
                    ..
                } = target
                {
                    let receiver = owner.as_ref().map_or(*receiver, |owner| {
                        semantics.specialize_type(owner, *receiver)
                    });
                    let TypeKind::Array { layout, length, .. } = semantics.types().kind(receiver)
                    else {
                        unreachable!("checked array push calls have array receivers")
                    };
                    debug_assert!(length.is_none());
                    reachable.array_pushes.insert(*layout);
                }
                if let wasm_ir::CallTarget::Intrinsic {
                    intrinsic: IntrinsicId::ArrayRemoveAt,
                    receiver_type: Some(receiver),
                    ..
                } = target
                {
                    let receiver = owner.as_ref().map_or(*receiver, |owner| {
                        semantics.specialize_type(owner, *receiver)
                    });
                    let TypeKind::Array { layout, length, .. } = semantics.types().kind(receiver)
                    else {
                        unreachable!("checked array removeAt calls have array receivers")
                    };
                    debug_assert!(length.is_none());
                    reachable.array_removals.insert(*layout);
                }
                if let wasm_ir::CallTarget::Intrinsic {
                    intrinsic: IntrinsicId::ArrayClear,
                    receiver_type: Some(receiver),
                    ..
                } = target
                {
                    let receiver = owner.as_ref().map_or(*receiver, |owner| {
                        semantics.specialize_type(owner, *receiver)
                    });
                    let TypeKind::Array { layout, length, .. } = semantics.types().kind(receiver)
                    else {
                        unreachable!("checked array clear calls have array receivers")
                    };
                    debug_assert!(length.is_none());
                    reachable.array_clears.insert(*layout);
                }
                if let wasm_ir::CallTarget::Intrinsic {
                    intrinsic:
                        intrinsic @ (IntrinsicId::SetNew
                        | IntrinsicId::SetLength
                        | IntrinsicId::SetContains
                        | IntrinsicId::SetInsert
                        | IntrinsicId::SetRemove
                        | IntrinsicId::SetClear),
                    receiver_type,
                    ..
                } = target
                {
                    let set_type = if *intrinsic == IntrinsicId::SetNew {
                        expression.ty
                    } else {
                        receiver_type.expect("checked set methods have receivers")
                    };
                    let set_type = owner
                        .as_ref()
                        .map_or(set_type, |owner| semantics.specialize_type(owner, set_type));
                    let TypeKind::Set {
                        layout, element, ..
                    } = semantics.types().kind(set_type)
                    else {
                        unreachable!("checked set operations use concrete Set types")
                    };
                    reachable.set_operations.insert((*layout, *intrinsic));
                    if *intrinsic == IntrinsicId::SetInsert {
                        // Insertion calls contains to reject duplicate elements.
                        reachable
                            .set_operations
                            .insert((*layout, IntrinsicId::SetContains));
                    }
                    if matches!(
                        intrinsic,
                        IntrinsicId::SetContains | IntrinsicId::SetInsert | IntrinsicId::SetRemove
                    ) {
                        reachable.require_equality(
                            *element,
                            semantics,
                            standard_library,
                            capabilities,
                        );
                    }
                }
                if let wasm_ir::CallTarget::ManagedSnapshot { class, .. } = target {
                    reachable.managed_snapshots.insert(*class);
                }
                if let wasm_ir::CallTarget::ManagedInstances { class } = target {
                    reachable.managed_instances.insert(*class);
                    let future = owner.as_ref().map_or(expression.ty, |owner| {
                        semantics.specialize_type(owner, expression.ty)
                    });
                    let TypeKind::Async { value, .. } = semantics.types().kind(future) else {
                        unreachable!("managed instances calls produce async arrays")
                    };
                    let TypeKind::Array { layout, .. } = semantics.types().kind(*value) else {
                        unreachable!("managed instances futures complete with arrays")
                    };
                    reachable.array_pushes.insert(*layout);
                }
                let function = match target {
                    wasm_ir::CallTarget::UserFunction { function }
                    | wasm_ir::CallTarget::UserMethod { function, .. } => Some(function.clone()),
                    wasm_ir::CallTarget::ManagedComponent { helper, .. } => Some(helper.clone()),
                    wasm_ir::CallTarget::LibraryOverload { .. } => {
                        wasm_ir::resolve_library_overload(
                            target,
                            owner.as_ref(),
                            semantics,
                            standard_library,
                        )
                    }
                    wasm_ir::CallTarget::Intrinsic { .. }
                    | wasm_ir::CallTarget::CapabilityRequirement { .. }
                    | wasm_ir::CallTarget::DefaultFormatting { .. }
                    | wasm_ir::CallTarget::GeneratorNext { .. }
                    | wasm_ir::CallTarget::IteratorIdentity { .. }
                    | wasm_ir::CallTarget::ManagedSnapshot { .. }
                    | wasm_ir::CallTarget::ManagedInstances { .. }
                    | wasm_ir::CallTarget::ResultError { .. }
                    | wasm_ir::CallTarget::OptionSome { .. }
                    | wasm_ir::CallTarget::IteratorItem { .. }
                    | wasm_ir::CallTarget::ResultSuccess { .. } => None,
                };
                let function = function.map(|function| {
                    if capability_call {
                        return function;
                    }
                    if matches!(target, wasm_ir::CallTarget::LibraryOverload { .. }) {
                        return function;
                    }
                    owner.as_ref().map_or(function.clone(), |owner| {
                        semantics.specialize_function_instance(owner, &function)
                    })
                });
                if let Some(function) = function {
                    pending_functions.push((None, function));
                }
            }

            let specialize = |ty| {
                owner
                    .as_ref()
                    .map_or(ty, |owner| semantics.specialize_type(owner, ty))
            };
            let mut display_sources = Vec::new();
            match &expression.kind {
                wasm_ir::ExpressionKind::Cast { value }
                    if matches!(
                        semantics.types().kind(specialize(expression.ty)),
                        TypeKind::Standard(StdlibTypeId::String)
                    ) =>
                {
                    display_sources.push((
                        wasm_ir
                            .expression(*value)
                            .expect("cast operands belong to Wasm IR")
                            .ty,
                        wasm_ir::FormattingMode::Display,
                    ));
                }
                wasm_ir::ExpressionKind::InterpolatedString(parts) => {
                    display_sources.extend(parts.iter().filter_map(|part| {
                        match part {
                            wasm_ir::InterpolatedPart::Expression {
                                string_conversion_source,
                                ..
                            } => string_conversion_source
                                .map(|source| (source, wasm_ir::FormattingMode::Display)),
                            wasm_ir::InterpolatedPart::Text(_) => None,
                        }
                    }));
                }
                wasm_ir::ExpressionKind::Call { target, arguments } => {
                    let target = reachable.resolved_call_target(owner.as_ref(), id, target);
                    let converted = match target {
                        wasm_ir::CallTarget::Intrinsic {
                            intrinsic: IntrinsicId::Print,
                            ..
                        } => arguments.first(),
                        wasm_ir::CallTarget::Intrinsic {
                            intrinsic: IntrinsicId::TimerSetVariable,
                            ..
                        } => arguments.get(1),
                        _ => None,
                    };
                    if let wasm_ir::CallTarget::DefaultFormatting {
                        mode,
                        receiver_type,
                        ..
                    } = target
                    {
                        display_sources.push((*receiver_type, *mode));
                    }
                    if let Some(argument) = converted {
                        display_sources.push((
                            wasm_ir
                                .expression(*argument)
                                .expect("call arguments belong to Wasm IR")
                                .ty,
                            wasm_ir::FormattingMode::Display,
                        ));
                    }
                }
                _ => {}
            }
            for (source, mode) in display_sources {
                let source = specialize(source);
                match mode {
                    wasm_ir::FormattingMode::Display => reachable.require_display(
                        source,
                        program,
                        semantics,
                        standard_library,
                        capabilities,
                        &mut pending_functions,
                    ),
                    wasm_ir::FormattingMode::Debug => reachable.require_debug(
                        source,
                        program,
                        semantics,
                        standard_library,
                        capabilities,
                        &mut pending_functions,
                    ),
                }
            }
        }

        // Type reachability includes every value shape referenced by emitted
        // storage or signatures, not only the result types of live expressions.
        let mut type_roots = Vec::new();
        if let Some(state) = &program.state {
            if let Some(layout) = state.provider_value {
                type_roots.push(
                    semantics
                        .value_type(layout)
                        .expect("checked layout values have types"),
                );
            }
            for field in state.all_fields() {
                type_roots.push(
                    semantics
                        .value_type(field.id)
                        .expect("checked state fields have types"),
                );
                type_roots.push(
                    semantics
                        .state_poll_result(field.id)
                        .expect("checked state fields have poll-result types"),
                );
            }
        }
        type_roots.extend(wasm_ir.global_initializers().map(|(global, _)| {
            semantics
                .value_type(global)
                .expect("checked globals have types")
        }));
        // Bare globals initialized by lifecycle or provider-generated code do
        // not necessarily have a Wasm-IR initializer expression. They still
        // own runtime storage, so their types must participate in GC layout
        // reachability just like source-initialized globals do.
        for declaration in &program.globals {
            declaration.binding.visit_bindings(&mut |binding| {
                if wasm_ir.contains_global(binding.id) {
                    type_roots.push(
                        semantics
                            .value_type(binding.id)
                            .expect("checked globals have types"),
                    );
                }
            });
        }
        type_roots.extend(
            program
                .settings
                .iter()
                .filter_map(|setting| semantics.value_type(setting.id)),
        );
        // Lifecycle ABI results are emitted even when the source body falls
        // through without constructing that value explicitly.
        type_roots.extend(
            program
                .actions
                .iter()
                .filter_map(|action| semantics.action_result(action.kind)),
        );

        for body in wasm_ir
            .bodies()
            .filter(|body| matches!(body.owner, BodyOwner::Action(_)))
        {
            type_roots.extend(body.locals.iter().map(|local| local.ty));
        }
        for instance in &reachable.functions {
            let body = wasm_ir
                .body(BodyOwner::Function(instance.clone()))
                .expect("reachable functions have template bodies");
            type_roots.extend(
                body.locals
                    .iter()
                    .map(|local| semantics.specialize_type(instance, local.ty)),
            );
        }
        for expression in wasm_ir.state_expressions() {
            type_roots.extend(expression.locals.iter().map(|local| local.ty));
        }
        for transform in wasm_ir.state_transforms() {
            type_roots.extend(transform.locals.iter().map(|local| local.ty));
        }
        for initializer in wasm_ir.global_initializer_plans() {
            type_roots.extend(initializer.locals.iter().map(|local| local.ty));
        }
        for instance in &reachable.functions {
            let function = program
                .functions
                .iter()
                .find(|function| function.id == instance.function)
                .expect("reachable functions have declarations");
            type_roots.extend(function.params.iter().map(|parameter| {
                semantics.specialize_type(
                    instance,
                    semantics
                        .value_type(parameter.id)
                        .expect("checked function parameters have types"),
                )
            }));
            type_roots.push(
                semantics.specialize_type(
                    instance,
                    semantics
                        .function_result(function.id)
                        .expect("checked functions have result types"),
                ),
            );
        }
        for (owner, id) in &reachable.expression_instances {
            let expression = wasm_ir
                .expression(*id)
                .expect("reachable expressions exist");
            let specialize = |ty| {
                owner
                    .as_ref()
                    .map_or(ty, |owner| semantics.specialize_type(owner, ty))
            };
            type_roots.push(specialize(expression.ty));
            if let Some(conversion) = expression.conversion {
                type_roots.extend([specialize(conversion.source), specialize(conversion.target)]);
            }
            match &expression.kind {
                wasm_ir::ExpressionKind::Call { target, .. } => {
                    match reachable.resolved_call_target(owner.as_ref(), *id, target) {
                        wasm_ir::CallTarget::UserMethod { receiver_type, .. } => {
                            type_roots.push(specialize(*receiver_type));
                        }
                        wasm_ir::CallTarget::Intrinsic {
                            intrinsic,
                            type_arguments,
                            receiver_type,
                            ..
                        } => {
                            type_roots.extend(type_arguments.iter().copied().map(specialize));
                            type_roots.extend(receiver_type.map(specialize));
                            let contract = crate::intrinsic_registry::contract(*intrinsic);
                            for policy in contract
                                .async_scratch
                                .iter()
                                .chain(contract.async_state)
                                .chain(contract.synchronous_scratch)
                            {
                                if let crate::intrinsic_registry::ScratchType::Standard(standard) =
                                    policy.ty
                                {
                                    type_roots.push(semantics.types().id_for_standard(standard));
                                }
                            }
                        }
                        wasm_ir::CallTarget::LibraryOverload {
                            dispatch_type,
                            receiver_type,
                            ..
                        } => {
                            type_roots.push(specialize(*dispatch_type));
                            type_roots.extend(receiver_type.map(specialize));
                        }
                        wasm_ir::CallTarget::DefaultFormatting { receiver_type, .. } => {
                            type_roots.push(specialize(*receiver_type));
                        }
                        wasm_ir::CallTarget::GeneratorNext { receiver_type, .. }
                        | wasm_ir::CallTarget::IteratorIdentity { receiver_type, .. } => {
                            type_roots.push(specialize(*receiver_type));
                        }
                        wasm_ir::CallTarget::ManagedSnapshot { receiver_type, .. } => {
                            type_roots.push(specialize(*receiver_type));
                        }
                        wasm_ir::CallTarget::ManagedComponent {
                            receiver_type,
                            helper_result,
                            ..
                        } => {
                            type_roots.push(specialize(*receiver_type));
                            type_roots.push(specialize(*helper_result));
                        }
                        wasm_ir::CallTarget::UserFunction { .. }
                        | wasm_ir::CallTarget::ManagedInstances { .. }
                        | wasm_ir::CallTarget::CapabilityRequirement { .. }
                        | wasm_ir::CallTarget::ResultError { .. }
                        | wasm_ir::CallTarget::OptionSome { .. }
                        | wasm_ir::CallTarget::IteratorItem { .. }
                        | wasm_ir::CallTarget::ResultSuccess { .. } => {}
                    }
                }
                wasm_ir::ExpressionKind::Propagate { target, .. } => {
                    type_roots.push(specialize(target.result()));
                }
                _ => {}
            }
        }
        reachable.string_equality |= wasm_ir.bodies().any(|body| {
            let reachable_body = match &body.owner {
                BodyOwner::Action(_) => true,
                BodyOwner::Function(function) => reachable.functions.contains(function),
            };
            reachable_body && block_uses_string_match_pattern(&body.entry, wasm_ir)
        });
        reachable.string_equality |= wasm_ir
            .state_expressions()
            .any(|expression| block_uses_string_match_pattern(&expression.entry, wasm_ir));
        reachable.string_equality |= wasm_ir
            .state_transforms()
            .any(|transform| block_uses_string_match_pattern(&transform.entry, wasm_ir));
        // Settings refresh owns a temporary string slot even for boolean-only
        // settings. Derived formatters return strings outside source bodies.
        if !program.settings.is_empty() || !reachable.derived_debugs.is_empty() {
            type_roots.push(semantics.types().id_for_standard(StdlibTypeId::String));
        }
        let mut field_reads = BTreeSet::new();
        for (owner, id) in reachable.expression_instances() {
            let expression = wasm_ir.expression(id).unwrap();
            let members = match &expression.kind {
                wasm_ir::ExpressionKind::Path { root, members } => {
                    if let Some(crate::semantic::ResolvedValue::ManagedStatic { field, .. }) = root
                    {
                        field_reads.insert(*field);
                    }
                    members.as_slice()
                }
                wasm_ir::ExpressionKind::Member { members, .. } => members.as_slice(),
                wasm_ir::ExpressionKind::Call { target, .. } => {
                    let target = reachable.resolved_call_target(owner.as_ref(), id, target);
                    if let Some((receiver, _)) = target.receiver_with_type() {
                        if let Some((
                            crate::semantic::ResolvedValue::ManagedStatic { field, .. },
                            _,
                        )) = receiver.path()
                        {
                            field_reads.insert(field);
                        }
                        receiver.members()
                    } else {
                        &[]
                    }
                }
                _ => &[],
            };
            for member in members {
                if let crate::semantic::ResolvedMember::ManagedField(field) = member {
                    field_reads.insert(*field);
                }
            }
        }
        let mut pending_classes = reachable
            .managed_snapshots
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let mut pending_decoders = Vec::new();
        for field in field_reads {
            let value = semantics.managed_field_value_type(field).unwrap();
            if crate::managed::ManagedFieldRead::for_type(value, semantics)
                == crate::managed::ManagedFieldRead::Recursive
                || super::managed_freezers::contains_array(value, capabilities.memory(), semantics)
            {
                pending_decoders.push(semantics.managed_field_type(field).unwrap());
            }
        }
        // Interleave class and decoder nodes to close recursive schemas without
        // retaining readers merely because their types or declarations exist.
        while !pending_classes.is_empty() || !pending_decoders.is_empty() {
            while let Some(class) = pending_classes.pop() {
                for field in program
                    .managed_class(class)
                    .unwrap()
                    .all_fields()
                    .filter(|field| !field.is_static)
                {
                    let ty = semantics.managed_field_snapshot_type(field.id).unwrap();
                    let child_type = match semantics.types().kind(ty) {
                        TypeKind::Option { value, .. } => *value,
                        _ => ty,
                    };
                    if let TypeKind::ManagedClass(child) = semantics.types().kind(child_type)
                        && reachable.managed_snapshots.insert(*child)
                    {
                        pending_classes.push(*child);
                        type_roots.push(ty);
                    }
                    if crate::managed::ManagedFieldRead::for_type(ty, semantics)
                        == crate::managed::ManagedFieldRead::Recursive
                        || super::managed_freezers::contains_array(
                            ty,
                            capabilities.memory(),
                            semantics,
                        )
                    {
                        pending_decoders.push(semantics.managed_field_type(field.id).unwrap());
                    }
                }
            }
            while let Some(ty) = pending_decoders.pop() {
                if !reachable.managed_decoders.insert(ty) {
                    continue;
                }
                let plan = capabilities
                    .managed_decoder(ty)
                    .expect("reachable managed values have decoder nodes");
                type_roots.push(plan.output);
                type_roots.extend(semantics.types().iter().find_map(|(id, kind)| {
                    matches!(kind, TypeKind::Result { value, .. } if *value == plan.output)
                        .then_some(id)
                }));
                pending_decoders.extend(plan.kind.children());
                if let crate::managed_read::ManagedDecoderKind::Map { key, .. }
                | crate::managed_read::ManagedDecoderKind::Set { element: key } = plan.kind
                {
                    let key = capabilities.managed_decoder(key).unwrap().output;
                    reachable.require_equality(key, semantics, standard_library, capabilities);
                }
                match plan.kind {
                    crate::managed_read::ManagedDecoderKind::Class { class }
                        if reachable.managed_snapshots.insert(class) =>
                    {
                        pending_classes.push(class);
                    }
                    _ => {}
                }
            }
        }
        reachable.require_types(
            type_roots,
            program,
            semantics,
            standard_library,
            capabilities,
        );
        // Every emitted Set layout currently owns its complete method suite,
        // including contains/insert/remove. Those bodies require element
        // equality even when the source only constructs or displays the set.
        // Keep this dependency paired with the emitted body family rather
        // than relying on an incidental call site to pull it in.
        let set_elements = semantics
            .types()
            .iter()
            .filter_map(|(_, kind)| match kind {
                TypeKind::Set {
                    layout, element, ..
                } if reachable.gc_sets.contains(layout) => Some(*element),
                _ => None,
            })
            .collect::<Vec<_>>();
        for element in set_elements {
            reachable.require_equality(element, semantics, standard_library, capabilities);
        }
        reachable
    }

    /// Retains constructed GC layouts referenced by the signatures of the
    /// runtime helpers selected after expression reachability is known.
    pub fn require_runtime_helper_types(
        &mut self,
        dependencies: &super::dependencies::BackendDependencies,
        arrays: &[ResolvedArrayType],
        program: &Program,
        semantics: &SemanticModel,
        standard_library: &StandardLibrary,
        capabilities: &crate::capabilities::CapabilityAnalysis,
    ) {
        let required = super::runtime_helper_registry::required_array_layouts(
            dependencies.helpers(),
            arrays,
            semantics,
        )
        .collect::<Vec<_>>();
        self.gc_arrays.extend(required.iter().copied());
        self.gc_array_storage.extend(required);
        use super::runtime_helper_registry::{HelperValueType, descriptor};
        let roots = dependencies
            .helpers()
            .flat_map(|helper| {
                let signature = descriptor(helper).signature;
                signature.params.iter().chain(signature.results)
            })
            .filter_map(|ty| match ty {
                HelperValueType::Standard(standard) => Some(*standard),
                HelperValueType::String
                | HelperValueType::StringArray
                | HelperValueType::StringResult
                | HelperValueType::OptionalStringResult => Some(StdlibTypeId::String),
                _ => None,
            })
            .map(|standard| semantics.types().id_for_standard(standard))
            .collect::<Vec<_>>();
        self.require_types(roots, program, semantics, standard_library, capabilities);
    }

    pub fn functions(&self) -> impl Iterator<Item = &FunctionInstance> {
        self.functions.iter()
    }

    pub fn expression_instances(
        &self,
    ) -> impl Iterator<Item = (Option<FunctionInstance>, ExprId)> + '_ {
        self.expression_instances.iter().cloned()
    }

    pub fn display_functions(&self) -> impl Iterator<Item = (TypeId, &FunctionInstance)> {
        self.display_functions
            .iter()
            .map(|(ty, function)| (*ty, function))
    }

    pub fn debug_functions(&self) -> impl Iterator<Item = (TypeId, &FunctionInstance)> {
        self.debug_functions
            .iter()
            .map(|(ty, function)| (*ty, function))
    }

    pub fn derived_debugs(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.derived_debugs.iter().copied()
    }

    /// Whether formatting this type dispatches to a source-defined body.
    ///
    /// Backend dependency discovery uses this to stop structural helper
    /// traversal at the same boundary as actual formatting emission. The
    /// custom body's reachable expressions own their dependencies; walking
    /// through the type as well would retain the unused derived formatter.
    pub(super) fn has_custom_formatting(&self, ty: TypeId) -> bool {
        self.display_functions.contains_key(&ty) || self.debug_functions.contains_key(&ty)
    }

    pub fn contains_expression(&self, expression: ExprId) -> bool {
        self.expressions.contains(&expression)
    }

    pub(super) fn resolved_call_target<'a>(
        &'a self,
        owner: Option<&FunctionInstance>,
        expression: ExprId,
        original: &'a wasm_ir::CallTarget,
    ) -> &'a wasm_ir::CallTarget {
        self.capability_calls
            .get(&(owner.cloned(), expression))
            .unwrap_or(original)
    }

    pub fn closure_instances(&self) -> impl Iterator<Item = &ClosureInstance> {
        self.closures.iter()
    }

    pub fn function_value_instances(&self) -> impl Iterator<Item = &FunctionValueInstance> {
        self.function_values.iter()
    }

    pub fn requires_struct_equality(&self, structure: StructId) -> bool {
        self.equality_structs.contains(&structure)
    }

    pub fn requires_standard_struct_equality(&self, structure: StdlibTypeId) -> bool {
        self.equality_standard_structs.contains(&structure)
    }

    pub fn requires_enum_equality(&self, enumeration: EnumId) -> bool {
        self.equality_enums.contains(&enumeration)
    }

    pub fn requires_array_equality(&self, array: ArrayTypeId) -> bool {
        self.equality_arrays.contains(&array)
    }

    pub fn requires_option_equality(&self, option: OptionTypeId) -> bool {
        self.equality_options.contains(&option)
    }

    pub fn requires_result_equality(&self, result: ResultTypeId) -> bool {
        self.equality_results.contains(&result)
    }

    pub fn requires_string_equality(&self) -> bool {
        self.string_equality
    }

    pub fn contains_struct_type(&self, structure: StructId) -> bool {
        self.gc_structs.contains(&structure)
    }

    pub fn contains_managed_class_type(&self, class: ManagedClassId) -> bool {
        self.gc_managed_classes.contains(&class)
    }

    pub fn managed_decoders(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.managed_decoders.iter().copied()
    }

    pub fn managed_snapshots(&self) -> impl Iterator<Item = ManagedClassId> + '_ {
        self.managed_snapshots.iter().copied()
    }

    pub fn contains_enum_type(&self, enumeration: EnumId) -> bool {
        self.gc_enums.contains(&enumeration)
    }

    pub fn contains_array_type(&self, array: ArrayTypeId) -> bool {
        self.gc_arrays.contains(&array)
    }

    pub fn contains_array_storage(&self, array: ArrayTypeId) -> bool {
        self.gc_array_storage.contains(&array)
    }

    pub fn requires_array_set(&self, array: ArrayTypeId) -> bool {
        !self.managed_decoders.is_empty() && self.array_sets.contains(&array)
    }

    pub fn requires_array_push(&self, array: ArrayTypeId) -> bool {
        self.array_pushes.contains(&array)
    }

    pub fn requires_array_clear(&self, array: ArrayTypeId) -> bool {
        self.array_clears.contains(&array)
    }

    pub fn requires_array_remove_at(&self, array: ArrayTypeId) -> bool {
        self.array_removals.contains(&array)
    }

    pub fn contains_option_type(&self, option: OptionTypeId) -> bool {
        self.gc_options.contains(&option)
    }

    pub fn contains_result_type(&self, result: ResultTypeId) -> bool {
        self.gc_results.contains(&result)
    }

    pub fn contains_async_type(&self, future: AsyncTypeId) -> bool {
        self.gc_asyncs.contains(&future)
    }

    pub fn contains_iterator_type(&self, iterator: IteratorTypeId) -> bool {
        self.gc_iterators.contains(&iterator)
    }

    pub fn contains_callable_type(&self, callable: CallableTypeId) -> bool {
        self.gc_callables.contains(&callable)
    }

    pub fn contains_set_type(&self, set: TypeApplicationId) -> bool {
        self.gc_sets.contains(&set)
    }

    pub fn requires_set_operation(&self, set: TypeApplicationId, intrinsic: IntrinsicId) -> bool {
        self.set_operations.contains(&(set, intrinsic))
    }

    pub fn contains_application_type(&self, application: TypeApplicationId) -> bool {
        self.gc_applications.contains(&application)
    }

    pub fn contains_standard_type(&self, standard: StdlibTypeId) -> bool {
        self.gc_standard.contains(&standard)
    }

    fn require_types(
        &mut self,
        roots: impl IntoIterator<Item = TypeId>,
        program: &Program,
        semantics: &SemanticModel,
        standard_library: &StandardLibrary,
        capabilities: &crate::capabilities::CapabilityAnalysis,
    ) {
        let mut pending = roots.into_iter().collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        while let Some(ty) = pending.pop() {
            if !visited.insert(ty) {
                continue;
            }
            match semantics.types().kind(ty) {
                TypeKind::Error => {
                    unreachable!("failed inference reached code-generation reachability")
                }
                TypeKind::Builtin(_)
                | TypeKind::ManagedReference(_)
                | TypeKind::GenericParameter { .. } => {}
                TypeKind::ManagedClass(class) => {
                    self.gc_managed_classes.insert(*class);
                    if self.managed_snapshots.contains(class) {
                        pending.extend(semantics.types().iter().find_map(|(id, kind)| {
                            matches!(kind, TypeKind::Result { value, .. } if *value == ty)
                                .then_some(id)
                        }));
                    }
                    let declaration = program
                        .managed_class(*class)
                        .expect("semantic managed classes belong to source declarations");
                    for field in declaration.all_fields().filter(|field| !field.is_static) {
                        let value = semantics
                            .managed_field_snapshot_type(field.id)
                            .expect("checked managed fields have semantic value types");
                        pending.push(value);
                        if self.managed_snapshots.contains(class) {
                            // Live pointer readers supply child object addresses
                            // before the owned snapshot decoder is invoked.
                            let live = semantics.managed_field_value_type(field.id).unwrap();
                            if live != value {
                                pending.push(live);
                                pending.extend(semantics.types().iter().find_map(|(id, kind)| {
                                    matches!(kind, TypeKind::Result { value: candidate, .. } if *candidate == live).then_some(id)
                                }));
                            }
                            let result = semantics
                                .types()
                                .iter()
                                .find_map(|(id, kind)| match kind {
                                    TypeKind::Result {
                                        value: candidate, ..
                                    } if *candidate == value => Some(id),
                                    _ => None,
                                })
                                .expect("managed snapshot fields have Result types");
                            pending.push(result);
                        }
                    }
                }
                TypeKind::StateSnapshot => {
                    pending.extend(
                        program
                            .state
                            .as_ref()
                            .expect("checked programs have state declarations")
                            .all_fields()
                            .map(|field| {
                                semantics
                                    .value_type(field.id)
                                    .expect("checked state fields have semantic types")
                            }),
                    );
                }
                TypeKind::SettingsView => {
                    pending.extend(
                        program
                            .settings
                            .iter()
                            .filter_map(|setting| semantics.value_type(setting.id)),
                    );
                }
                TypeKind::Standard(standard) => {
                    self.gc_standard.insert(*standard);
                    if matches!(
                        standard_library.type_decl(*standard).representation,
                        RuntimeRepresentation::GcStruct { .. }
                    ) {
                        pending.extend(standard_library.fields_of(*standard).map(|field| {
                            semantics
                                .standard_field_type(field.id)
                                .expect("checked standard fields have semantic types")
                        }));
                    }
                }
                TypeKind::Struct(structure) => {
                    self.gc_structs.insert(*structure);
                    // Generated attachment structs may have had unused
                    // metadata fields removed after capability checking.
                    // Follow the fields that will actually be emitted.
                    let declaration = program
                        .structs
                        .iter()
                        .find(|item| item.id == *structure)
                        .expect("reachable source structs have declarations");
                    pending.extend(declaration.fields.iter().map(|field| {
                        semantics
                            .struct_field_type(field.id)
                            .expect("checked struct fields have semantic types")
                    }));
                }
                TypeKind::Enum(enumeration) => {
                    self.gc_enums.insert(*enumeration);
                    pending.extend(capabilities.structural_dependency_types(ty));
                }
                TypeKind::Array {
                    layout, element, ..
                } => {
                    self.gc_arrays.insert(*layout);
                    self.gc_array_storage.insert(*layout);
                    pending.push(*element);
                }
                TypeKind::Option { layout, value } => {
                    self.gc_options.insert(*layout);
                    pending.push(*value);
                }
                TypeKind::Result { layout, value } => {
                    self.gc_results.insert(*layout);
                    pending.push(*value);
                    pending.push(semantics.types().id_for_standard(StdlibTypeId::String));
                }
                TypeKind::Async { layout, value } => {
                    self.gc_asyncs.insert(*layout);
                    pending.push(*value);
                }
                TypeKind::Iterator { layout, item } => {
                    self.gc_iterators.insert(*layout);
                    pending.push(*item);
                }
                TypeKind::Callable {
                    layout,
                    parameters,
                    result,
                } => {
                    self.gc_callables.insert(*layout);
                    pending.extend(parameters.iter().copied());
                    pending.push(*result);
                }
                TypeKind::Set {
                    layout,
                    element,
                    backing,
                } => {
                    self.gc_sets.insert(*layout);
                    self.gc_array_storage.insert(*backing);
                    pending.push(*element);
                }
                TypeKind::Range { bound, .. } => pending.push(*bound),
                TypeKind::Application {
                    layout,
                    constructor,
                    arguments,
                } => {
                    self.gc_applications.insert(*layout);
                    pending.extend(arguments.iter().copied());
                    let declaration = standard_library.type_constructor(*constructor);
                    let variables = declaration
                        .parameters
                        .iter()
                        .zip(arguments)
                        .map(|(parameter, argument)| (parameter.name, *argument))
                        .collect::<std::collections::HashMap<_, _>>();
                    pending.extend(standard_library.fields_of_constructor(*constructor).map(
                        |field| {
                            super::gc_types::instantiated_catalog_type(
                                field.ty, &variables, semantics,
                            )
                        },
                    ));
                }
            }
        }
    }

    fn require_equality(
        &mut self,
        root: TypeId,
        semantics: &SemanticModel,
        standard_library: &StandardLibrary,
        capabilities: &crate::capabilities::CapabilityAnalysis,
    ) {
        let mut pending = vec![root];
        let mut visited = BTreeSet::new();
        while let Some(ty) = pending.pop() {
            if !visited.insert(ty) {
                continue;
            }
            match semantics.types().kind(ty) {
                TypeKind::Error => {
                    unreachable!("failed inference reached equality reachability")
                }
                TypeKind::Standard(StdlibTypeId::String) => self.string_equality = true,
                TypeKind::Standard(standard) => {
                    let library = standard_library;
                    let declaration = library.type_decl(*standard);
                    if library.type_has_capability(*standard, StdlibCapabilityId::Equatable)
                        && matches!(
                            declaration.representation,
                            RuntimeRepresentation::GcStruct { .. }
                        )
                        && self.equality_standard_structs.insert(*standard)
                    {
                        pending.extend(library.fields_of(*standard).map(|field| {
                            semantics
                                .standard_field_type(field.id)
                                .expect("checked standard fields have semantic types")
                        }));
                    }
                }
                TypeKind::Builtin(_)
                | TypeKind::StateSnapshot
                | TypeKind::SettingsView
                | TypeKind::ManagedClass(_)
                | TypeKind::ManagedReference(_)
                | TypeKind::GenericParameter { .. } => {}
                TypeKind::Struct(structure) if self.equality_structs.insert(*structure) => {
                    pending.extend(capabilities.structural_dependency_types(ty));
                }
                TypeKind::Enum(enumeration) if self.equality_enums.insert(*enumeration) => {
                    pending.extend(capabilities.structural_dependency_types(ty));
                }
                TypeKind::Array {
                    layout, element, ..
                } if self.equality_arrays.insert(*layout) => {
                    pending.push(*element);
                }
                TypeKind::Option { layout, value } if self.equality_options.insert(*layout) => {
                    pending.push(*value);
                }
                TypeKind::Result { layout, value } if self.equality_results.insert(*layout) => {
                    self.string_equality = true;
                    pending.push(*value);
                }
                TypeKind::Struct(_)
                | TypeKind::Enum(_)
                | TypeKind::Array { .. }
                | TypeKind::Option { .. }
                | TypeKind::Result { .. }
                | TypeKind::Async { .. }
                | TypeKind::Iterator { .. }
                | TypeKind::Callable { .. }
                | TypeKind::Range { .. }
                | TypeKind::Set { .. } => {}
                TypeKind::Application { .. } => {}
            }
        }
    }

    fn require_display(
        &mut self,
        root: TypeId,
        program: &Program,
        semantics: &SemanticModel,
        standard_library: &StandardLibrary,
        capabilities: &crate::capabilities::CapabilityAnalysis,
        pending_functions: &mut Vec<(Option<FunctionInstance>, FunctionInstance)>,
    ) {
        if let Some((source, function)) =
            super::display_function(root, program, semantics, standard_library, capabilities)
        {
            self.display_functions.insert(source, function.clone());
            pending_functions.push((None, function));
            return;
        }
        if capabilities.has_derived_display(root, semantics) {
            self.require_debug(
                root,
                program,
                semantics,
                standard_library,
                capabilities,
                pending_functions,
            );
        }
    }

    fn require_debug(
        &mut self,
        root: TypeId,
        program: &Program,
        semantics: &SemanticModel,
        standard_library: &StandardLibrary,
        capabilities: &crate::capabilities::CapabilityAnalysis,
        pending_functions: &mut Vec<(Option<FunctionInstance>, FunctionInstance)>,
    ) {
        let mut pending = vec![root];
        let mut visited = BTreeSet::new();
        while let Some(ty) = pending.pop() {
            if !visited.insert(ty) {
                continue;
            }
            if let Some((source, function)) = super::debug_function(ty, semantics, capabilities) {
                self.debug_functions.insert(source, function.clone());
                pending_functions.push((None, function));
                continue;
            }
            if capabilities.has_derived_debug(ty, semantics) && self.derived_debugs.insert(ty) {
                pending.extend(capabilities.debug_dependency_types(ty, semantics));
                continue;
            }
            // Opaque standard-library types may intentionally share their
            // public display spelling with their nested debug spelling.
            if let Some((source, function)) =
                super::display_function(ty, program, semantics, standard_library, capabilities)
            {
                self.debug_functions.insert(source, function.clone());
                pending_functions.push((None, function));
            }
        }
    }
}

fn constant_roots(
    expression: &wasm_ir::ExpressionKind,
) -> impl Iterator<Item = crate::stdlib::StdlibItemId> + '_ {
    let direct = match expression {
        wasm_ir::ExpressionKind::Path {
            root: Some(crate::semantic::ResolvedValue::StandardLibraryConstant(item)),
            ..
        } => Some(*item),
        _ => None,
    };
    let receiver = match expression {
        wasm_ir::ExpressionKind::Call { target, .. } => match target {
            wasm_ir::CallTarget::UserMethod { receiver, .. }
            | wasm_ir::CallTarget::ManagedSnapshot { receiver, .. }
            | wasm_ir::CallTarget::ManagedComponent { receiver, .. }
            | wasm_ir::CallTarget::CapabilityRequirement { receiver, .. }
            | wasm_ir::CallTarget::DefaultFormatting { receiver, .. }
            | wasm_ir::CallTarget::GeneratorNext { receiver, .. }
            | wasm_ir::CallTarget::IteratorIdentity { receiver, .. } => Some(receiver),
            wasm_ir::CallTarget::Intrinsic { receiver, .. }
            | wasm_ir::CallTarget::LibraryOverload { receiver, .. } => receiver.as_ref(),
            wasm_ir::CallTarget::UserFunction { .. }
            | wasm_ir::CallTarget::ManagedInstances { .. }
            | wasm_ir::CallTarget::ResultError { .. }
            | wasm_ir::CallTarget::OptionSome { .. }
            | wasm_ir::CallTarget::IteratorItem { .. }
            | wasm_ir::CallTarget::ResultSuccess { .. } => None,
        },
        _ => None,
    }
    .and_then(|receiver| match receiver {
        crate::semantic::ResolvedReceiver::Path {
            root: crate::semantic::ResolvedValue::StandardLibraryConstant(item),
            ..
        } => Some(*item),
        crate::semantic::ResolvedReceiver::Path { .. }
        | crate::semantic::ResolvedReceiver::Expression { .. } => None,
    });
    direct.into_iter().chain(receiver)
}

fn block_uses_string_match_pattern(block: &wasm_ir::Block, program: &wasm_ir::Program) -> bool {
    #[derive(Default)]
    struct StringPatternFinder {
        found: bool,
    }

    impl Visitor for StringPatternFinder {
        fn visit_statement(&mut self, statement: &wasm_ir::Statement, program: &wasm_ir::Program) {
            if let wasm_ir::Statement::Match { arms, .. } = statement
                && arms.iter().any(|arm| arm.pattern.contains_string())
            {
                self.found = true;
            }
            wasm_ir::walk_statement(self, statement, program);
        }
    }

    let mut finder = StringPatternFinder::default();
    finder.visit_block(block, program);
    finder.found
}

fn collect_block_expression_roots(
    block: &wasm_ir::Block,
    program: &wasm_ir::Program,
    owner: Option<FunctionInstance>,
    output: &mut Vec<(Option<FunctionInstance>, ExprId)>,
) {
    struct RootCollector<'a> {
        owner: Option<FunctionInstance>,
        output: &'a mut Vec<(Option<FunctionInstance>, ExprId)>,
    }

    impl Visitor for RootCollector<'_> {
        fn visit_expression(
            &mut self,
            expression: &wasm_ir::Expression,
            _program: &wasm_ir::Program,
        ) {
            self.output.push((self.owner.clone(), expression.id));
        }
    }

    RootCollector { owner, output }.visit_block(block, program);
}

fn collect_assignment_function_roots(
    block: &wasm_ir::Block,
    program: &wasm_ir::Program,
    owner: Option<FunctionInstance>,
    output: &mut Vec<(Option<FunctionInstance>, FunctionInstance)>,
) {
    struct AssignmentCollector<'a> {
        owner: Option<FunctionInstance>,
        output: &'a mut Vec<(Option<FunctionInstance>, FunctionInstance)>,
    }

    impl Visitor for AssignmentCollector<'_> {
        fn visit_statement(&mut self, statement: &wasm_ir::Statement, program: &wasm_ir::Program) {
            let operation = match statement {
                wasm_ir::Statement::Store { operation, .. } => operation.as_ref(),
                wasm_ir::Statement::IndexStore { operation, .. } => Some(operation),
                _ => None,
            };
            if let Some(wasm_ir::AssignmentOperation::Call(wasm_ir::CallTarget::UserMethod {
                function,
                ..
            })) = operation
            {
                self.output.push((self.owner.clone(), function.clone()));
            }
            wasm_ir::walk_statement(self, statement, program);
        }
    }

    AssignmentCollector { owner, output }.visit_block(block, program);
}

#[cfg(test)]
mod tests {
    use super::Reachability;
    use crate::{stdlib::StdlibTypeId, types::TypeKind};

    #[test]
    fn explicit_unity_backends_exclude_opposite_runtime_types() {
        for (provider, excluded) in [
            (
                "Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64())",
                StdlibTypeId::MonoModule,
            ),
            ("Unity.mono(MonoVersion.V2)", StdlibTypeId::UnityModule),
        ] {
            let source = format!(
                "image \"Assembly-CSharp\" {{ class Probe {{ static i32 value; }} }}\n\
                 state {provider} [\"game.exe\"] {{ value = Probe.value?; }}"
            );
            let checked = crate::check(crate::parse(&source).unwrap()).unwrap();
            let backend = crate::lower_wasm_with_options(
                &checked,
                crate::CompilerOptions {
                    profile: crate::BuildProfile::Release,
                    ..Default::default()
                },
            );
            let reachable = Reachability::analyze(
                &backend.program,
                &backend.semantics,
                &backend.wasm_ir,
                &backend.standard_library,
                backend.capabilities,
                [],
            );
            assert!(!reachable.contains_standard_type(excluded), "{provider}");
        }
    }

    #[test]
    fn identity_types_and_their_fields_follow_reachable_reads() {
        for (source, pe, elf, guid) in [
            ("state \"game.exe\" {}", false, false, false),
            (
                "state \"game.exe\" {} fn unused(value: PeDebugId) { print(value.guid) }",
                false,
                false,
                false,
            ),
            (
                include_str!("../../tests/pe_debug_id.split"),
                true,
                false,
                true,
            ),
            (
                include_str!("../../tests/elf_build_id.split"),
                false,
                true,
                true,
            ),
            (
                include_str!("../../tests/mach_uuid.split"),
                false,
                false,
                true,
            ),
        ] {
            let checked = crate::check(crate::parse(source).unwrap()).unwrap();
            let backend = crate::lower_wasm_with_options(
                &checked,
                crate::CompilerOptions {
                    profile: crate::BuildProfile::Release,
                    ..Default::default()
                },
            );
            let reachable = Reachability::analyze(
                &backend.program,
                &backend.semantics,
                &backend.wasm_ir,
                &backend.standard_library,
                backend.capabilities,
                [],
            );
            assert_eq!(
                reachable.contains_standard_type(StdlibTypeId::PeDebugId),
                pe
            );
            assert_eq!(
                reachable.contains_standard_type(StdlibTypeId::ElfIdentitySegment),
                elf
            );
            let guid_type = backend
                .semantics
                .types()
                .iter()
                .find_map(|(_, kind)| match kind {
                    TypeKind::Array {
                        layout,
                        element,
                        length: Some(16),
                    } if *element
                        == backend
                            .semantics
                            .types()
                            .id_for_core(crate::stdlib::CoreTypeId::U8) =>
                    {
                        Some(*layout)
                    }
                    _ => None,
                })
                .expect("the catalog declares the GUID field layout");
            assert_eq!(reachable.contains_array_type(guid_type), guid);
        }
    }
}

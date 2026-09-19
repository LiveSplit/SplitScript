//! Top-level type-checking pass orchestration.

use crate::{
    ast::Program,
    inference::{
        ApplicationLayout, ArrayLayout, ArrayShape, AsyncLayout, CallableLayout,
        ConstructedLayouts, InferenceContext, OptionLayout, RangeLayout, Requirements,
        ResultLayout, SetLayout, Type,
    },
    resolution::ProgramResolutions,
    semantic::SemanticBuilder,
    stdlib::StandardLibrary,
    types::TypeStore,
};

use super::{
    Checker, RecoveringCheckOutput, body_pass,
    context::{
        CallableContext, DebugContext, ExpressionMode, FailureContext, LoopContext, NonePolicy,
    },
    declaration_pass,
    declarations::DeclarationEnvironment,
    finalization, syntax_type,
};

pub(super) fn check_recovering(
    program: &Program,
    resolutions: &ProgramResolutions,
    standard_library: StandardLibrary,
) -> RecoveringCheckOutput {
    let mut checker = initialize_checker(program, resolutions, standard_library);
    declaration_pass::collect(&mut checker, program);
    body_pass::check(&mut checker, program);
    finalization::finish(checker, program)
}

fn initialize_checker(
    program: &Program,
    resolutions: &ProgramResolutions,
    standard_library: StandardLibrary,
) -> Checker {
    let structs = program.structs.clone();
    let enums = program.enum_declarations().cloned().collect::<Vec<_>>();
    let managed_classes = program
        .managed_class_declarations()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let semantic_types =
        TypeStore::with_source_types(&standard_library, &structs, &enums, &managed_classes);
    let array_types = program
        .array_types
        .iter()
        .map(|array| ArrayLayout {
            id: array.id,
            element: syntax_type(array.element, &semantic_types, resolutions),
            shape: array.length.map_or(ArrayShape::Growable, ArrayShape::Exact),
        })
        .collect::<Vec<_>>();
    let option_types = program
        .option_types
        .iter()
        .map(|option| OptionLayout {
            id: option.id,
            value: syntax_type(option.value, &semantic_types, resolutions),
        })
        .collect::<Vec<_>>();
    let result_types = program
        .result_types
        .iter()
        .map(|result| ResultLayout {
            id: result.id,
            value: syntax_type(result.value, &semantic_types, resolutions),
        })
        .collect::<Vec<_>>();
    let async_types = program
        .async_types
        .iter()
        .map(|future| AsyncLayout {
            id: future.id,
            value: syntax_type(future.value, &semantic_types, resolutions),
        })
        .collect::<Vec<_>>();
    let iterator_types = program
        .iterator_types
        .iter()
        .map(|iterator| crate::inference::IteratorLayout {
            id: iterator.id,
            item: syntax_type(iterator.item, &semantic_types, resolutions),
        })
        .collect::<Vec<_>>();
    let callable_types = program
        .callable_types
        .iter()
        .map(|callable| CallableLayout {
            id: callable.id,
            parameters: callable
                .parameters
                .iter()
                .map(|parameter| syntax_type(*parameter, &semantic_types, resolutions))
                .collect(),
            result: syntax_type(callable.result, &semantic_types, resolutions),
        })
        .collect::<Vec<_>>();
    let range_types = program
        .range_types
        .iter()
        .map(|range| RangeLayout {
            id: range.id,
            lower: syntax_type(range.lower, &semantic_types, resolutions),
            upper: syntax_type(range.upper, &semantic_types, resolutions),
            kind: range.kind,
        })
        .collect::<Vec<_>>();
    let set_types = program
        .type_applications
        .iter()
        .filter(|application| {
            matches!(
                resolutions.type_ref(crate::ast::TypeRef::Application(application.id)),
                Some(crate::types::ResolvedTypeRef::Set(_))
            )
        })
        .map(|application| SetLayout {
            id: application.id,
            element: syntax_type(application.arguments[0], &semantic_types, resolutions),
            backing: None,
        })
        .collect::<Vec<_>>();
    let application_types = program
        .type_applications
        .iter()
        .filter_map(|application| {
            let Some(crate::types::ResolvedTypeRef::Application(_)) =
                resolutions.type_ref(crate::ast::TypeRef::Application(application.id))
            else {
                return None;
            };
            let name = program.type_name(application.constructor);
            let constructor =
                standard_library.named_type_constructor_by_name_including_private(name)?;
            Some(ApplicationLayout {
                id: application.id,
                constructor: constructor.id,
                arguments: application
                    .arguments
                    .iter()
                    .map(|argument| syntax_type(*argument, &semantic_types, resolutions))
                    .collect(),
            })
        })
        .collect::<Vec<_>>();
    let none_type = Type::Known(semantic_types.id_for_core(crate::stdlib::CoreTypeId::None));
    let inference = InferenceContext::new(
        standard_library.clone(),
        semantic_types,
        structs.len() as u32 + enums.len() as u32,
        ConstructedLayouts {
            arrays: array_types,
            options: option_types,
            results: result_types,
            asyncs: async_types,
            iterators: iterator_types,
            callables: callable_types,
            ranges: range_types,
            sets: set_types,
            applications: application_types,
        },
    );
    let provider_value = resolutions.state_provider().map(|provider| {
        let declaration = standard_library.state_provider(provider);
        (
            provider,
            Type::Known(
                inference
                    .type_store()
                    .id_for_standard(declaration.process_type),
            ),
        )
    });
    let provider_values = resolutions
        .state_providers()
        .map(|provider| {
            let declaration = standard_library.state_provider(provider);
            (
                provider,
                Type::Known(
                    inference
                        .type_store()
                        .id_for_standard(declaration.process_type),
                ),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let provider_alternatives = resolutions
        .state_provider_alternatives()
        .map(|(variant, alternative)| (variant, (alternative.provider, alternative.selector)))
        .collect();

    let mut checker = Checker {
        standard_library,
        resolutions: resolutions.clone(),
        errors: Vec::new(),
        declarations: DeclarationEnvironment::new(
            structs,
            enums,
            managed_classes,
            program
                .functions
                .iter()
                .filter(|function| function.debug_only)
                .map(|function| function.id)
                .collect(),
        ),
        inference,
        provider_value,
        provider_values,
        state_refinement_value: program
            .state
            .as_ref()
            .and_then(|state| state.provider_value),
        active_provider_variants: None,
        active_state_field: None,
        shape_dimensions: Vec::new(),
        active_shapes: None,
        scopes: Vec::new(),
        condition_flows: std::collections::HashMap::new(),
        active_condition_bindings: Vec::new(),
        conditional_binding_declarations: Vec::new(),
        return_ty: none_type,
        generator_item: None,
        callable: CallableContext::TopLevel,
        expression_mode: ExpressionMode::Normal,
        debug_context: DebugContext::Normal,
        loops: LoopContext::default(),
        failure: FailureContext::None,
        inferred_process_reads: Vec::new(),
        inferred_empty_collections: Vec::new(),
        deferred_member_paths: Vec::new(),
        none_policy: NonePolicy::OptionalOnly,
        semantics: SemanticBuilder::with_state_provider(
            resolutions.state_provider(),
            resolutions.state_provider_selector(),
            provider_alternatives,
        ),
        standard_field_types: std::collections::HashMap::new(),
        active_function_component: std::collections::HashSet::new(),
        expected_type_source: None,
        return_type_source: None,
    };
    for range in program.range_types.iter() {
        let lower = checker.syntax_type(range.lower);
        let upper = checker.syntax_type(range.upper);
        checker.unify(lower, upper, range.occurrences[0]);
        checker.require(
            lower,
            Requirements::capability(crate::stdlib::StdlibCapabilityId::Integer),
            range.occurrences[0],
        );
    }
    let fields = checker.standard_library.fields().to_vec();
    let variables = std::collections::HashMap::new();
    for field in fields {
        if !matches!(field.owner, crate::stdlib::StdlibOwner::Type(_)) {
            continue;
        }
        let ty = checker.catalog_type(field.ty, &variables);
        checker.standard_field_types.insert(field.id, ty);
        checker.semantics.resolve_standard_field_type(field.id, ty);
    }
    let managed_type_spans = program
        .managed_class_declarations()
        .into_iter()
        .flat_map(|class| class.all_fields().map(|field| field.type_span))
        .collect::<Vec<_>>();
    for application in &program.type_applications {
        let name = program.type_name(application.constructor);
        let Some(constructor) = checker
            .standard_library
            .named_type_constructor_by_name_including_private(name)
        else {
            continue;
        };
        if !matches!(
            checker
                .resolutions
                .type_ref(crate::ast::TypeRef::Application(application.id)),
            Some(
                crate::types::ResolvedTypeRef::Set(_)
                    | crate::types::ResolvedTypeRef::Application(_)
            )
        ) {
            continue;
        }
        // Interned syntax can occur both in a managed field and in an ordinary
        // annotation. Interpret each written use in its own storage context.
        for occurrence in &application.occurrences {
            let managed = managed_type_spans
                .iter()
                .any(|span| span.start <= occurrence.span.start && occurrence.span.end <= span.end);
            for (argument, parameter) in application.arguments.iter().zip(constructor.parameters) {
                if parameter.constraints.is_empty() {
                    continue;
                }
                let argument = checker.syntax_type(*argument);
                let argument = if managed {
                    checker.managed_owned_type(argument)
                } else {
                    argument
                };
                if checker
                    .require(
                        argument,
                        Requirements::capabilities(parameter.constraints.iter().copied()),
                        occurrence.constructor,
                    )
                    .is_some()
                {
                    for constraint in parameter.constraints {
                        checker.semantics.record_constructor_constraint(
                            argument,
                            *constraint,
                            occurrence.constructor,
                        );
                    }
                }
            }
        }
    }
    checker
}

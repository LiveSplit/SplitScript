//! Post-type-check semantic validation.
//!
//! Type inference establishes resolved types and calls. This stage derives
//! operational/capability facts and reports constraints that require the
//! complete typed program. Strict compilation and editor recovery consume the
//! same product.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::{
    Diagnostic, DiagnosticCode, DiagnosticFix, FixApplicability, TextEdit,
    ast::{self, EnumDecl, Program, StateSource},
    capabilities::CapabilityAnalysis,
    effects::{OperationAnalysis, StateSnapshotContext},
    hir::{
        self, ExpressionResolution, TypedBlock, TypedExpression, TypedExpressionKind,
        TypedMatchArm, TypedProgram, TypedStatementKind, TypedVisitor,
    },
    semantic::{
        DynamicCallCallee, FunctionInstance, ResolvedCall, ResolvedEnumVariantId, ResolvedMember,
        ResolvedValue, SemanticModel,
    },
    stdlib::{
        Implementation, StandardLibrary, StdlibCapabilityId, StdlibItemId, StdlibOwner,
        StdlibTypeConstructorId,
    },
    types::TypeKind,
    visit::{self, Visitor as SyntaxVisitor},
};

mod stdlib_bodies;

pub(crate) struct ValidationOutput {
    pub(crate) scoped_globals: crate::scoped_globals::ScopedGlobalAnalysis,
    pub(crate) capabilities: CapabilityAnalysis,
    pub(crate) effects: OperationAnalysis,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

pub(crate) fn validate(
    standard_library: StandardLibrary,
    syntax: &Program,
    hir: &TypedProgram,
    semantics: &SemanticModel,
    enum_types: &[EnumDecl],
) -> ValidationOutput {
    let (scoped_globals, scoped_global_diagnostics) =
        crate::scoped_globals::analyze(syntax, hir, semantics);
    let capabilities = CapabilityAnalysis::build(
        &syntax.structs,
        enum_types,
        &syntax.managed_class_declarations(),
        &syntax.functions,
        semantics,
        standard_library.clone(),
    );
    let effects = OperationAnalysis::infer(syntax, hir, semantics, &capabilities, &scoped_globals);
    let mut diagnostics = Vec::new();
    diagnostics.extend(scoped_global_diagnostics);
    diagnostics.extend(validate_enum_memory_layouts(
        syntax,
        semantics,
        &capabilities,
    ));
    diagnostics.extend(validate_global_initializers(syntax, hir, &effects));
    diagnostics.extend(stdlib_bodies::validate_signatures(
        &standard_library,
        syntax,
        hir,
        semantics,
    ));
    diagnostics.extend(validate_function_instances(
        syntax,
        hir,
        semantics,
        &standard_library,
        &capabilities,
    ));
    diagnostics.extend(validate_for_iterables(
        syntax,
        hir,
        semantics,
        &standard_library,
        &capabilities,
    ));
    diagnostics.extend(validate_future_storage(syntax, semantics, enum_types));
    diagnostics.extend(validate_must_use(&standard_library, hir, semantics));
    let unused_declarations = validate_unused_declarations(syntax, hir, semantics, &capabilities);
    diagnostics.extend(validate_unused_bindings(
        syntax,
        hir,
        &unused_declarations.function_profiles,
    ));
    diagnostics.extend(unused_declarations.diagnostics);
    diagnostics.extend(validate_static_setting_lookups(syntax, hir));
    diagnostics.extend(validate_struct_field_shorthand(syntax));
    diagnostics.extend(validate_empty_future_races(hir));
    diagnostics.extend(validate_irrefutable_is_patterns(syntax));
    diagnostics.extend(validate_async_function_results(
        &standard_library,
        syntax,
        hir,
        &effects,
    ));
    diagnostics.extend(validate_async_recursion(syntax, hir, &effects));
    diagnostics.extend(validate_suspending_calls(
        &standard_library,
        syntax,
        hir,
        &effects,
    ));

    // The standalone standard-library bootstrap is the authority for catalog
    // metadata. Every ordinary compilation rechecks the same injected bodies
    // and verifies that their complete typed call graph still agrees with the
    // cached, user-independent result.
    if standard_library.source_body_operations_are_initialized() {
        for item in standard_library.all_items() {
            if !matches!(
                item.implementation,
                Implementation::LibraryBody { .. } | Implementation::LibraryOverloads { .. }
            ) {
                continue;
            }
            let cataloged = standard_library.operation_metadata(item.id);
            let mut functions = hir.library_functions(item.id);
            // Signature-only tooling contexts deliberately omit catalog body
            // declarations. Their operation metadata was already derived by
            // the standalone bootstrap, so there is no local body to compare.
            let Some(function) = functions.next() else {
                continue;
            };
            let inferred = functions.fold(
                effects.function(function).metadata(),
                |combined, function| {
                    combined.conservative_union(effects.function(function).metadata())
                },
            );
            if inferred != cataloged {
                let span = syntax
                    .functions
                    .iter()
                    .find(|declaration| declaration.id == function)
                    .map(|declaration| declaration.span)
                    .unwrap_or_default();
                diagnostics.push(Diagnostic::semantic(
                    format!(
                        "standard-library body `{}` inferred operation metadata {:?}, but its standalone catalog analysis produced {:?}",
                        item.qualified_name, inferred, cataloged
                    ),
                    span,
                ));
            }
        }
    }

    for violation in effects.attached_process_violations(hir, semantics, &capabilities) {
        let name = violation
            .standard_library_name
            .map(str::to_owned)
            .or_else(|| {
                let function = violation.function?;
                syntax
                    .functions
                    .iter()
                    .find(|declaration| declaration.id == function)
                    .map(|declaration| declaration.name.clone())
            });
        diagnostics.push(Diagnostic::semantic(
            format!(
                "`{}` requires an attached process and is unavailable in `{}`",
                name.unwrap_or_else(|| "function".to_owned()),
                violation.action.name(),
            ),
            violation.expression_span,
        ));
    }

    for violation in effects.state_snapshot_violations(hir, semantics, &capabilities) {
        let name = violation
            .standard_library_name
            .map(str::to_owned)
            .or_else(|| {
                let function = violation.function?;
                syntax
                    .functions
                    .iter()
                    .find(|declaration| declaration.id == function)
                    .map(|declaration| declaration.name.clone())
            })
            .unwrap_or_else(|| "function".to_owned());
        let context = match violation.context {
            StateSnapshotContext::Action(action) => format!("`{}`", action.name()),
            StateSnapshotContext::StateSource => "a state field expression".to_owned(),
            StateSnapshotContext::StateTransform => "a state field filter".to_owned(),
        };
        diagnostics.push(
            Diagnostic::semantic(
                format!("`{name}` requires state snapshots and is unavailable in {context}"),
                violation.expression_span,
            )
            .with_migration_topic("asl.state.helper-snapshots"),
        );
    }

    for expression in hir.expressions() {
        if let TypedExpressionKind::Binary {
            op: ast::BinaryOp::Eq | ast::BinaryOp::Ne,
            left,
            ..
        } = expression.kind
        {
            let operand = hir
                .expression(left)
                .expect("binary operands belong to typed HIR");
            if let Err(error) =
                capabilities.require(operand.ty, StdlibCapabilityId::Equatable, semantics)
            {
                diagnostics.push(Diagnostic::semantic(error, expression.span));
            }
        }
        if let Some(ResolvedCall::StandardLibrary {
            item,
            type_arguments,
            ..
        }) = hir.call(expression.id)
        {
            let item = standard_library.item(*item);
            for (parameter, argument) in item.signature.type_parameters.iter().zip(type_arguments) {
                for constraint in parameter.constraints {
                    if let Err(error) = capabilities.require(*argument, *constraint, semantics) {
                        let capability = standard_library.capability(*constraint);
                        diagnostics.push(capability_diagnostic(
                            format!(
                                "`{}` requires capability `{}`: {error}",
                                item.qualified_name, capability.name,
                            ),
                            expression.span,
                            *argument,
                            *constraint,
                            syntax,
                            semantics,
                            &standard_library,
                            &capabilities,
                        ));
                    }
                }
            }
        }
        let display_source = match &expression.kind {
            TypedExpressionKind::InterpolatedString(parts) => {
                for part in parts {
                    let hir::TypedInterpolatedPart::Expression {
                        expression: value,
                        conversion: Some(hir::ImplicitConversion::ToString { source }),
                    } = part
                    else {
                        continue;
                    };
                    if let Err(error) =
                        capabilities.require(*source, StdlibCapabilityId::Display, semantics)
                    {
                        let span = hir
                            .expression(*value)
                            .expect("interpolation operands belong to typed HIR")
                            .span;
                        diagnostics.push(capability_diagnostic(
                            error,
                            span,
                            *source,
                            StdlibCapabilityId::Display,
                            syntax,
                            semantics,
                            &standard_library,
                            &capabilities,
                        ));
                    }
                }
                None
            }
            TypedExpressionKind::Cast {
                expression: value, ..
            } if matches!(
                semantics.types().kind(expression.ty),
                TypeKind::Standard(crate::stdlib::StdlibTypeId::String)
            ) =>
            {
                hir.expression(*value).map(|value| (value.ty, value.span))
            }
            _ => None,
        };
        if let Some((source, span)) = display_source
            && let Err(error) = capabilities.require(source, StdlibCapabilityId::Display, semantics)
        {
            diagnostics.push(capability_diagnostic(
                error,
                span,
                source,
                StdlibCapabilityId::Display,
                syntax,
                semantics,
                &standard_library,
                &capabilities,
            ));
        }
    }

    diagnostics.extend(validate_remote_memory_layouts(
        &standard_library,
        syntax,
        hir,
        semantics,
        &capabilities,
    ));

    ValidationOutput {
        scoped_globals,
        capabilities,
        effects,
        diagnostics,
    }
}

fn validate_for_iterables(
    syntax: &Program,
    hir: &TypedProgram,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
    capabilities: &CapabilityAnalysis,
) -> Vec<Diagnostic> {
    struct Collector<'a> {
        syntax: &'a Program,
        semantics: &'a SemanticModel,
        standard_library: &'a StandardLibrary,
        capabilities: &'a CapabilityAnalysis,
        diagnostics: Vec<Diagnostic>,
    }

    impl TypedVisitor for Collector<'_> {
        fn visit_statement(&mut self, statement: &hir::TypedStatement, program: &TypedProgram) {
            if let TypedStatementKind::For { iterable, .. } = statement.kind {
                let expression = program
                    .expression(iterable)
                    .expect("for iterables belong to typed HIR");
                let iterator = StdlibCapabilityId::Iterator;
                let iterable_capability = StdlibCapabilityId::Iterable;
                if self
                    .capabilities
                    .require(expression.ty, iterator, self.semantics)
                    .is_err()
                    && let Err(error) = self.capabilities.require(
                        expression.ty,
                        iterable_capability,
                        self.semantics,
                    )
                {
                    self.diagnostics.push(capability_diagnostic(
                        format!("`for ... in` requires capability `Iterable`: {error}"),
                        expression.span,
                        expression.ty,
                        iterable_capability,
                        self.syntax,
                        self.semantics,
                        self.standard_library,
                        self.capabilities,
                    ));
                }
            }
            hir::walk_typed_statement(self, statement, program);
        }
    }

    let mut collector = Collector {
        syntax,
        semantics,
        standard_library,
        capabilities,
        diagnostics: Vec::new(),
    };
    for body in hir.function_bodies() {
        collector.visit_block(&body.body, hir);
    }
    for body in hir.action_bodies() {
        collector.visit_block(&body.body, hir);
    }
    collector.diagnostics
}

fn validate_enum_memory_layouts(
    syntax: &Program,
    semantics: &SemanticModel,
    capabilities: &CapabilityAnalysis,
) -> Vec<Diagnostic> {
    syntax
        .enums
        .iter()
        .filter(|enumeration| enumeration.representation.is_some())
        .filter(|enumeration| {
            enumeration
                .variants
                .iter()
                .all(|variant| variant.payload.is_none())
        })
        .filter(|enumeration| {
            semantics
                .enum_representation(enumeration.id)
                .is_some_and(|ty| {
                    matches!(
                        semantics.types().kind(ty),
                        TypeKind::Builtin(
                            crate::types::BuiltinType::I8
                                | crate::types::BuiltinType::U8
                                | crate::types::BuiltinType::I16
                                | crate::types::BuiltinType::U16
                                | crate::types::BuiltinType::I32
                                | crate::types::BuiltinType::U32
                                | crate::types::BuiltinType::I64
                                | crate::types::BuiltinType::U64
                        )
                    )
                })
        })
        .filter_map(|enumeration| {
            capabilities
                .memory()
                .enumeration(enumeration.id)
                .err()
                .map(|error| {
                    Diagnostic::semantic(
                        error,
                        enumeration
                            .representation_span
                            .unwrap_or(enumeration.name_span),
                    )
                })
        })
        .collect()
}

fn validate_irrefutable_is_patterns(syntax: &Program) -> Vec<Diagnostic> {
    #[derive(Default)]
    struct Collector {
        diagnostics: Vec<Diagnostic>,
    }

    impl<'ast> SyntaxVisitor<'ast> for Collector {
        fn visit_expr(&mut self, expression: &'ast ast::Expr) {
            if let ast::ExprKind::Is { pattern, .. } = &expression.kind
                && syntactically_irrefutable_pattern(&pattern.kind)
            {
                self.diagnostics.push(
                    Diagnostic::warning(
                        DiagnosticCode::AlwaysMatchesPattern,
                        "this `is` pattern always matches",
                        pattern.span,
                    )
                    .with_primary_label("this pattern cannot produce `false`")
                    .with_note("use `let` when only binding the value is intended"),
                );
            }
            visit::walk_expr(self, expression);
        }
    }

    let mut collector = Collector::default();
    collector.visit_program(syntax);
    collector.diagnostics
}

fn syntactically_irrefutable_pattern(pattern: &ast::MatchPattern) -> bool {
    match pattern {
        ast::MatchPattern::Binding(_) | ast::MatchPattern::Wildcard => true,
        ast::MatchPattern::Struct { fields, .. } => fields
            .iter()
            .all(|field| syntactically_irrefutable_pattern(&field.pattern.kind)),
        ast::MatchPattern::Alternation(alternatives) => alternatives
            .iter()
            .any(|pattern| syntactically_irrefutable_pattern(&pattern.kind)),
        _ => false,
    }
}

/// Validates every source declaration whose runtime implementation performs a
/// fixed-layout process-memory read.
///
/// This is deliberately a post-inference semantic boundary shared by native
/// state fields and managed terminal fields. Code generation may rely on the
/// resulting invariant without rediscovering which source types are readable.
/// Managed references and managed strings have dedicated decoders and
/// therefore do not participate in the ordinary `MemoryReadable` contract.
fn validate_remote_memory_layouts(
    standard_library: &StandardLibrary,
    syntax: &Program,
    hir: &TypedProgram,
    semantics: &SemanticModel,
    capabilities: &CapabilityAnalysis,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    if let Some(state) = &syntax.state {
        for field in state.all_fields() {
            if !matches!(
                field.source,
                StateSource::Pointer(ref path) if path.decoder.is_none()
            ) {
                continue;
            }
            let ty = semantics
                .value_type(field.id)
                .expect("checked state fields have semantic types");
            let memory_ty = match semantics.types().kind(ty) {
                TypeKind::Option { value, .. } => *value,
                _ => ty,
            };
            if let Err(error) =
                capabilities.require(memory_ty, StdlibCapabilityId::MemoryReadable, semantics)
            {
                diagnostics.push(Diagnostic::semantic(error, field.span));
            }
        }
    }

    diagnostics.extend(validate_provider_guest_memory_ranges(
        standard_library,
        syntax,
        hir,
        semantics,
        capabilities,
    ));

    for class in syntax.managed_class_declarations() {
        for field in class.all_fields() {
            let ty = semantics
                .managed_field_value_type(field.id)
                .expect("checked managed fields have semantic types");
            let reference = match semantics.types().kind(ty) {
                TypeKind::ManagedReference(_) => true,
                TypeKind::Option { value, .. } => matches!(
                    semantics.types().kind(*value),
                    TypeKind::ManagedReference(_)
                ),
                _ => false,
            };
            if reference {
                continue;
            }
            let storage = semantics
                .managed_field_type(field.id)
                .expect("checked managed fields have storage types");
            let Err(error) =
                capabilities.require(storage, StdlibCapabilityId::ManagedReadable, semantics)
            else {
                continue;
            };
            let mut diagnostic = Diagnostic::semantic(
                format!(
                    "managed field `{}.{}` has no supported managed decoder",
                    class.name, field.name
                ),
                field.type_span,
            )
            .with_primary_label("this managed value must satisfy `ManagedReadable`")
            .with_note(error);
            if matches!(
                semantics.types().kind(ty),
                TypeKind::Array { length: None, .. }
            ) {
                diagnostic = diagnostic.with_note(
                    "every managed array element needs an implemented `ManagedReadable` decoder; live references cannot be retained as owned elements",
                );
            }
            diagnostics.push(diagnostic);
        }
    }

    for (expression, call) in semantics.calls() {
        if let crate::semantic::ResolvedCall::ManagedSnapshot { class, .. } = call {
            let ty = semantics.types().id_for_managed_class(*class);
            if let Err(error) =
                capabilities.require(ty, StdlibCapabilityId::ManagedReadable, semantics)
                && let Some(expression) = hir.expression(expression)
            {
                diagnostics.push(
                    Diagnostic::semantic(
                        "managed class snapshot contains a value without a managed decoder",
                        expression.span,
                    )
                    .with_note(error),
                );
            }
        }
    }
    diagnostics
}

fn validate_provider_guest_memory_ranges(
    standard_library: &StandardLibrary,
    syntax: &Program,
    hir: &TypedProgram,
    semantics: &SemanticModel,
    capabilities: &CapabilityAnalysis,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    if let Some(state) = &syntax.state {
        for field in state.all_fields() {
            let Some(provider_id) = semantics.state_field_provider(field.id) else {
                continue;
            };
            let provider = standard_library.state_provider(provider_id);
            if !provider.readable_ranges.is_empty() {
                let StateSource::Pointer(path) = &field.source else {
                    continue;
                };
                let crate::ast::PointerPathBase::Absolute(address) = path.base else {
                    continue;
                };
                let size = if path.offsets.is_empty() {
                    let crate::stdlib::Implementation::Intrinsic(direct_read) =
                        standard_library.item(provider.direct_read).implementation
                    else {
                        continue;
                    };
                    state_field_read_size(
                        field,
                        semantics,
                        capabilities,
                        crate::intrinsic_registry::provider_read_contract(direct_read)
                            .expect("guest-memory ranges belong to provider reads")
                            .address_width,
                    )
                } else {
                    // Provider pointer paths read a 32-bit guest pointer at
                    // every intermediate hop.
                    Some(4)
                };
                let Some(size) = size else {
                    continue;
                };
                if !provider
                    .readable_ranges
                    .iter()
                    .any(|range| range.contains(address, size))
                {
                    diagnostics.push(invalid_guest_read_diagnostic(
                        provider,
                        address,
                        size,
                        path.base_span,
                        format!("state field `{}`", field.name),
                    ));
                }
            }
        }
    }

    for expression in hir.expressions() {
        let Some(ResolvedCall::StandardLibrary {
            item,
            type_arguments,
            ..
        }) = hir.call(expression.id)
        else {
            continue;
        };
        let Some(provider) = standard_library
            .state_providers()
            .iter()
            .find(|provider| provider.direct_read == *item && !provider.readable_ranges.is_empty())
        else {
            continue;
        };
        let TypedExpressionKind::Call { arguments, .. } = &expression.kind else {
            continue;
        };
        let Some(address) = arguments
            .first()
            .and_then(|argument| hir.expression(*argument))
        else {
            continue;
        };
        let TypedExpressionKind::Int { value, .. } = address.kind else {
            continue;
        };
        let Some(memory_ty) = type_arguments.first().copied() else {
            continue;
        };
        let crate::stdlib::Implementation::Intrinsic(direct_read) =
            standard_library.item(provider.direct_read).implementation
        else {
            continue;
        };
        let address_width = crate::intrinsic_registry::provider_read_contract(direct_read)
            .expect("guest-memory ranges belong to provider reads")
            .address_width;
        let Ok(layout) = capabilities
            .memory()
            .layout(memory_ty, semantics, address_width)
        else {
            continue;
        };
        let size = layout.size();
        if !provider
            .readable_ranges
            .iter()
            .any(|range| range.contains(value, size))
        {
            diagnostics.push(invalid_guest_read_diagnostic(
                provider,
                value,
                size,
                address.span,
                format!("call to `{}`", standard_library.item(*item).qualified_name),
            ));
        }
    }

    diagnostics
}

fn state_field_read_size(
    field: &crate::ast::StateField,
    semantics: &SemanticModel,
    capabilities: &CapabilityAnalysis,
    address_width: crate::memory::MemoryAddressWidth,
) -> Option<u32> {
    let StateSource::Pointer(path) = &field.source else {
        return None;
    };
    match path.decoder {
        Some(crate::ast::StateMemoryDecoder::Utf8 { max_bytes, .. }) => Some(max_bytes),
        Some(crate::ast::StateMemoryDecoder::Utf16Le { max_units, .. }) => max_units.checked_mul(2),
        None => {
            let ty = semantics.value_type(field.id)?;
            let memory_ty = match semantics.types().kind(ty) {
                TypeKind::Option { value, .. } => *value,
                _ => ty,
            };
            capabilities
                .memory()
                .layout(memory_ty, semantics, address_width)
                .ok()
                .map(crate::memory::MemoryTypeLayout::size)
        }
    }
}

fn invalid_guest_read_diagnostic(
    provider: &crate::stdlib::StdlibStateProvider,
    address: u64,
    size: u32,
    span: crate::ast::Span,
    source: String,
) -> Diagnostic {
    let ranges = provider
        .readable_ranges
        .iter()
        .map(|range| format!("0x{:08x}..<0x{:08x}", range.start, range.end))
        .collect::<Vec<_>>()
        .join(" or ");
    Diagnostic::semantic(
        format!(
            "literal guest-memory read is outside `state {}`'s readable domain",
            provider.name
        ),
        span,
    )
    .with_primary_label(format!(
        "{source} reads {size} byte{} from 0x{address:08x}",
        if size == 1 { "" } else { "s" }
    ))
    .with_note(format!(
        "`{}` reads must lie entirely within {ranges}",
        provider.name
    ))
    .with_note(
        "computed guest addresses remain fallible and are checked by the provider at runtime",
    )
}

fn validate_struct_field_shorthand(syntax: &Program) -> Vec<Diagnostic> {
    #[derive(Default)]
    struct Collector {
        diagnostics: Vec<Diagnostic>,
    }

    impl<'ast> SyntaxVisitor<'ast> for Collector {
        fn visit_expr(&mut self, expression: &'ast ast::Expr) {
            if let ast::ExprKind::Struct { fields, .. } = &expression.kind {
                for field in fields {
                    if field.shorthand
                        || !matches!(
                            &field.value.kind,
                            ast::ExprKind::Path(path)
                                if path.as_slice() == [field.name.as_str()]
                        )
                    {
                        continue;
                    }
                    self.diagnostics.push(struct_field_shorthand_diagnostic(
                        &field.name,
                        field.name_span,
                        field.value.span,
                        "initializer",
                    ));
                }
            }
            visit::walk_expr(self, expression);
        }

        fn visit_pattern(&mut self, pattern: &'ast ast::MatchPattern) {
            if let ast::MatchPattern::Struct { fields, .. } = pattern {
                for field in fields {
                    let ast::MatchPattern::Binding(binding) = &field.pattern.kind else {
                        continue;
                    };
                    if field.shorthand || field.name != binding.name {
                        continue;
                    }
                    self.diagnostics.push(struct_field_shorthand_diagnostic(
                        &field.name,
                        field.name_span,
                        field.pattern.span,
                        "binding",
                    ));
                }
            }
            visit::walk_pattern(self, pattern);
        }
    }

    let mut collector = Collector::default();
    collector.visit_program(syntax);
    collector.diagnostics
}

fn struct_field_shorthand_diagnostic(
    name: &str,
    name_span: ast::Span,
    repeated_span: ast::Span,
    repeated_role: &str,
) -> Diagnostic {
    Diagnostic::warning(
        DiagnosticCode::StructFieldShorthand,
        format!("struct field `{name}` repeats its {repeated_role} name"),
        name_span.join(repeated_span),
    )
    .with_primary_label("use the shorthand field syntax")
    .with_fix(DiagnosticFix {
        title: format!("shorten to `{name}`"),
        applicability: FixApplicability::MachineApplicable,
        edits: vec![TextEdit {
            span: ast::Span {
                start: name_span.end,
                end: repeated_span.end,
            },
            replacement: String::new(),
        }],
    })
}

fn validate_empty_future_races(hir: &TypedProgram) -> Vec<Diagnostic> {
    hir.expressions()
        .filter_map(|expression| {
            let ResolvedCall::StandardLibrary { item, .. } = hir.call(expression.id)? else {
                return None;
            };
            if *item != StdlibItemId::FutureRace {
                return None;
            }
            let TypedExpressionKind::Call { arguments, .. } = &expression.kind else {
                return None;
            };
            let operations = hir.expression(*arguments.first()?)?;
            if !matches!(&operations.kind, TypedExpressionKind::Array(values) if values.is_empty()) {
                return None;
            }
            Some(
                Diagnostic::warning(
                    DiagnosticCode::EmptyFutureRace,
                    "an empty future race never completes",
                    operations.span,
                )
                .with_primary_label("this array contains no operation that could win")
                .with_note(
                    "an empty `future.race` is valid and remains pending forever; add an operation unless that is intentional",
                ),
            )
        })
        .collect()
}

fn validate_global_initializers(
    syntax: &Program,
    hir: &TypedProgram,
    effects: &OperationAnalysis,
) -> Vec<Diagnostic> {
    let mut declarations = HashMap::new();
    for global in &syntax.globals {
        declarations.insert(global.id, global);
        global
            .binding
            .visit_bindings(&mut |binding| _ = declarations.insert(binding.id, global));
    }
    let mut diagnostics = Vec::new();

    for initializer in hir.global_initializers() {
        let Some(operation) = effects.global_initializer(initializer.expression) else {
            continue;
        };
        let forbidden_effects = operation
            .effects
            .iter()
            .copied()
            .filter(|effect| {
                !matches!(
                    effect,
                    crate::stdlib::Effect::Pure | crate::stdlib::Effect::Allocates
                )
            })
            .collect::<Vec<_>>();
        if forbidden_effects.is_empty()
            && operation.global_reads.is_empty()
            && operation.global_writes.is_empty()
            && operation.availability == crate::stdlib::Availability::Everywhere
        {
            continue;
        }

        let Some(global) = declarations.get(&initializer.binding.value) else {
            continue;
        };
        let initializer_span = hir
            .expression(initializer.expression)
            .map_or(global.span, |expression| expression.span);
        let invalid_call = first_invalid_initializer_call(initializer.expression, hir, effects);
        let span = invalid_call
            .and_then(|(expression, _)| hir.expression(expression))
            .map_or(initializer_span, |expression| expression.span);
        let mut diagnostic = Diagnostic::semantic(
            format!(
                "global initializer for `{}` must be closed, synchronous, and pure",
                global.name
            ),
            span,
        )
        .with_primary_label("this expression runs once during module initialization");

        if let Some((_, function)) = invalid_call
            && let Some(declaration) = syntax
                .functions
                .iter()
                .find(|declaration| declaration.id == function)
        {
            diagnostic = diagnostic.with_secondary_label(
                declaration.name_span,
                "this helper carries the initializer dependency",
            );
        }

        for value in operation
            .global_reads
            .iter()
            .chain(&operation.global_writes)
        {
            if let Some(declaration) = declarations.get(value) {
                diagnostic = diagnostic.with_secondary_label(
                    declaration.name_span,
                    format!(
                        "the initializer transitively accesses global `{}`",
                        declaration.name
                    ),
                );
            }
        }
        if !forbidden_effects.is_empty() {
            diagnostic = diagnostic.with_note(format!(
                "the initializer transitively {}",
                forbidden_effects
                    .iter()
                    .map(|effect| effect.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if operation.availability != crate::stdlib::Availability::Everywhere {
            diagnostic = diagnostic
                .with_note("the initializer depends on values that only exist while attached");
        }
        diagnostics.push(
            diagnostic
                .with_note(
                    "global initializers may allocate and call synchronous pure helpers, but cannot access globals, settings, timer or process state, or suspend",
                )
                .with_note(
                    "move runtime-dependent initialization to the appropriate lifecycle block",
                ),
        );
    }

    diagnostics
}

fn initializer_operation_is_invalid(
    operation: &crate::effects::FunctionOperationSemantics,
) -> bool {
    operation.effects.iter().any(|effect| {
        !matches!(
            effect,
            crate::stdlib::Effect::Pure | crate::stdlib::Effect::Allocates
        )
    }) || !operation.global_reads.is_empty()
        || !operation.global_writes.is_empty()
        || operation.availability != crate::stdlib::Availability::Everywhere
}

fn first_invalid_initializer_call(
    root: ast::ExprId,
    hir: &TypedProgram,
    effects: &OperationAnalysis,
) -> Option<(ast::ExprId, ast::FunctionId)> {
    struct Finder<'a> {
        effects: &'a OperationAnalysis,
        found: Option<(ast::ExprId, ast::FunctionId)>,
    }

    impl TypedVisitor for Finder<'_> {
        fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
            if self.found.is_some() {
                return;
            }
            hir::walk_typed_expression(self, expression, program);
            if self.found.is_some()
                || !self
                    .effects
                    .call(expression.id)
                    .is_some_and(initializer_operation_is_invalid)
            {
                return;
            }
            let function = match program.call(expression.id) {
                Some(ResolvedCall::UserFunction { function, .. })
                | Some(ResolvedCall::UserMethod { function, .. }) => *function,
                _ => return,
            };
            self.found = Some((expression.id, function));
        }
    }

    let expression = hir.expression(root)?;
    let mut finder = Finder {
        effects,
        found: None,
    };
    finder.visit_expression(expression, hir);
    finder.found
}

#[allow(clippy::too_many_arguments)]
fn capability_diagnostic(
    message: String,
    span: ast::Span,
    ty: crate::types::TypeId,
    capability: StdlibCapabilityId,
    syntax: &Program,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
    capabilities: &CapabilityAnalysis,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::semantic(message, span);
    if standard_library.capability(capability).behavior
        != crate::stdlib::CapabilityBehavior::StructuralMethods
    {
        return diagnostic;
    }

    for &requirement in capabilities.structural_method_requirements(capability) {
        if let Some(function) = capabilities.method_candidate(ty, requirement)
            && let Some(declaration) = syntax
                .functions
                .iter()
                .find(|declaration| declaration.id == function)
        {
            return diagnostic.with_secondary_label(
                declaration.name_span,
                format!(
                    "this method was considered for `{}`",
                    standard_library.render_signature(requirement)
                ),
            );
        }
    }

    let declaration = match semantics.types().kind(ty) {
        TypeKind::Struct(id) => syntax
            .structs
            .iter()
            .find(|declaration| declaration.id == *id)
            .map(|declaration| declaration.name_span),
        TypeKind::Enum(id) => syntax
            .enum_declarations()
            .find(|declaration| declaration.id == *id)
            .map(|declaration| declaration.name_span),
        _ => None,
    };
    if let Some(declaration) = declaration {
        diagnostic = diagnostic.with_secondary_label(
            declaration,
            format!(
                "define the required method on this type to satisfy `{}`",
                standard_library.capability(capability).name
            ),
        );
    }
    diagnostic
}

fn validate_static_setting_lookups(syntax: &Program, hir: &TypedProgram) -> Vec<Diagnostic> {
    let settings = syntax
        .settings
        .iter()
        .map(|setting| (setting.runtime_key(), setting))
        .collect::<HashMap<_, _>>();
    let mut diagnostics = Vec::new();

    for expression in hir.expressions() {
        let Some(ResolvedCall::StandardLibrary { item, receiver, .. }) = hir.call(expression.id)
        else {
            continue;
        };
        if !matches!(
            *item,
            StdlibItemId::SettingsViewEnabled | StdlibItemId::SettingsViewContains
        ) {
            continue;
        }
        let TypedExpressionKind::Call { arguments, .. } = &expression.kind else {
            continue;
        };
        let Some(argument) = arguments
            .first()
            .and_then(|argument| hir.expression(*argument))
        else {
            continue;
        };
        let TypedExpressionKind::String(key) = &argument.kind else {
            continue;
        };
        let Some(setting) = settings.get(key.as_str()) else {
            continue;
        };
        if matches!(setting.kind, ast::SettingKind::Title { .. }) {
            continue;
        }

        let direct_root = receiver.as_ref().and_then(|receiver| match receiver {
            crate::semantic::ResolvedReceiver::Path { root, members } if members.is_empty() => {
                match root {
                    ResolvedValue::SettingsView => Some("settings"),
                    ResolvedValue::OldSettingsView => Some("oldSettings"),
                    _ => None,
                }
            }
            _ => None,
        });

        if *item == StdlibItemId::SettingsViewEnabled {
            if !matches!(setting.kind, ast::SettingKind::Bool { .. }) || !setting.source_visible {
                continue;
            }
            let Some(root) = direct_root else {
                continue;
            };
            let replacement = format!("{root}.{}", setting.name);
            diagnostics.push(
                Diagnostic::warning(
                    DiagnosticCode::StaticSettingLookup,
                    format!("literal setting key `{key}` has a typed member"),
                    expression.span,
                )
                .with_primary_label(format!("use `{replacement}` for this declared setting"))
                .with_secondary_label(setting.name_span, "the setting is declared here")
                .with_machine_applicable_fix(
                    format!("replace the string lookup with `{replacement}`"),
                    expression.span,
                    replacement,
                ),
            );
            continue;
        }

        let mut diagnostic = Diagnostic::warning(
            DiagnosticCode::StaticSettingLookup,
            format!("`contains({})` is always true", quote_string_literal(key)),
            argument.span,
        )
        .with_primary_label("this exact value-setting key is declared statically")
        .with_secondary_label(setting.name_span, "the setting is declared here")
        .with_machine_applicable_fix(
            "replace the known membership test with `true`",
            expression.span,
            "true",
        )
        .with_note(
            "use `contains` with a computed key when declaration membership is genuinely data-driven",
        );

        if setting.source_visible
            && let Some(root) = direct_root
        {
            let member = format!("{root}.{}", setting.name);
            diagnostic = diagnostic.with_note(format!(
                "read the declared setting's typed value as `{member}`"
            ));
            if matches!(setting.kind, ast::SettingKind::Bool { .. }) {
                diagnostic = diagnostic.with_fix(DiagnosticFix {
                    title: format!("read the enabled value with `{member}`"),
                    applicability: FixApplicability::MaybeIncorrect,
                    edits: vec![TextEdit {
                        span: expression.span,
                        replacement: member,
                    }],
                });
            }
        }
        diagnostics.push(diagnostic);
    }

    diagnostics
}

fn validate_async_recursion(
    syntax: &Program,
    hir: &TypedProgram,
    effects: &OperationAnalysis,
) -> Vec<Diagnostic> {
    #[derive(Default)]
    struct Calls {
        values: HashSet<ast::FunctionId>,
    }
    impl TypedVisitor for Calls {
        fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
            if let Some(call) = program.call(expression.id) {
                match call {
                    ResolvedCall::UserFunction { function, .. }
                    | ResolvedCall::UserMethod { function, .. } => {
                        self.values.insert(*function);
                    }
                    ResolvedCall::StandardLibrary { item, .. } => {
                        for function in program.library_functions(*item) {
                            self.values.insert(function);
                        }
                    }
                    ResolvedCall::ManagedSnapshot { .. }
                    | ResolvedCall::ManagedComponent { .. }
                    | ResolvedCall::ManagedInstances { .. } => {}
                    ResolvedCall::ResultError { .. }
                    | ResolvedCall::OptionSome { .. }
                    | ResolvedCall::IteratorItem { .. }
                    | ResolvedCall::ResultSuccess { .. } => {}
                }
            }
            hir::walk_typed_expression(self, expression, program);
        }
    }

    let mut graph = HashMap::<ast::FunctionId, HashSet<ast::FunctionId>>::new();
    for body in hir.all_function_bodies() {
        let mut calls = Calls::default();
        calls.visit_block(&body.body, hir);
        graph
            .entry(body.function.function)
            .or_default()
            .extend(calls.values);
    }

    fn reaches(
        target: ast::FunctionId,
        current: ast::FunctionId,
        graph: &HashMap<ast::FunctionId, HashSet<ast::FunctionId>>,
        visited: &mut HashSet<ast::FunctionId>,
    ) -> bool {
        graph.get(&current).is_some_and(|callees| {
            callees.contains(&target)
                || callees
                    .iter()
                    .copied()
                    .any(|callee| visited.insert(callee) && reaches(target, callee, graph, visited))
        })
    }

    syntax
        .functions
        .iter()
        .filter(|function| {
            effects.function(function.id).suspension == crate::stdlib::SuspensionKind::Suspends
                && reaches(function.id, function.id, &graph, &mut HashSet::new())
        })
        .map(|function| {
            Diagnostic::semantic(
                format!("async function `{}` cannot be recursive yet", function.name),
                function.name_span,
            )
            .with_primary_label("recursive future-frame allocation has no configured limit")
            .with_note(
                "rewrite the recursion as a loop until bounded recursive futures are specified",
            )
        })
        .collect()
}

fn validate_future_storage(
    syntax: &Program,
    semantics: &SemanticModel,
    enum_types: &[EnumDecl],
) -> Vec<Diagnostic> {
    fn contains_future(
        ty: crate::types::TypeId,
        syntax: &Program,
        semantics: &SemanticModel,
        enum_types: &[EnumDecl],
        visited: &mut HashSet<crate::types::TypeId>,
    ) -> bool {
        if !visited.insert(ty) {
            return false;
        }
        match semantics.types().kind(ty) {
            TypeKind::Error => false,
            TypeKind::Async { .. } => true,
            TypeKind::Struct(structure) => syntax
                .structs
                .iter()
                .find(|declaration| declaration.id == *structure)
                .is_some_and(|declaration| {
                    declaration.fields.iter().any(|field| {
                        semantics.struct_field_type(field.id).is_some_and(|field| {
                            contains_future(field, syntax, semantics, enum_types, visited)
                        })
                    })
                }),
            TypeKind::Enum(enumeration) => enum_types
                .iter()
                .find(|declaration| declaration.id == *enumeration)
                .is_some_and(|declaration| {
                    declaration.variants.iter().any(|variant| {
                        semantics
                            .enum_variant_payload(variant.id)
                            .is_some_and(|payload| {
                                contains_future(payload, syntax, semantics, enum_types, visited)
                            })
                    })
                }),
            TypeKind::Array { element, .. }
            | TypeKind::Set { element, .. }
            | TypeKind::Option { value: element, .. }
            | TypeKind::Result { value: element, .. }
            | TypeKind::Iterator { item: element, .. }
            | TypeKind::Range { bound: element, .. } => {
                contains_future(*element, syntax, semantics, enum_types, visited)
            }
            TypeKind::Application { arguments, .. } => arguments
                .iter()
                .any(|argument| contains_future(*argument, syntax, semantics, enum_types, visited)),
            TypeKind::Callable { .. } => false,
            TypeKind::Builtin(_)
            | TypeKind::Standard(_)
            | TypeKind::StateSnapshot
            | TypeKind::SettingsView
            | TypeKind::ManagedClass(_)
            | TypeKind::ManagedReference(_)
            | TypeKind::GenericParameter { .. } => false,
        }
    }

    syntax
        .globals
        .iter()
        .filter(|global| {
            semantics.value_type(global.id).is_some_and(|ty| {
                contains_future(ty, syntax, semantics, enum_types, &mut HashSet::new())
            })
        })
        .map(|global| {
            Diagnostic::semantic(
                format!(
                    "global `{}` cannot store an attachment-lifetime async value",
                    global.name
                ),
                global.name_span,
            )
            .with_primary_label("this value may retain a cancelled process operation")
            .with_note(
                "store the future in an onAttach local and await it before the process closes",
            )
        })
        .collect()
}

fn validate_async_function_results(
    standard_library: &StandardLibrary,
    syntax: &Program,
    hir: &TypedProgram,
    effects: &OperationAnalysis,
) -> Vec<Diagnostic> {
    let library_functions = standard_library
        .all_items()
        .iter()
        .flat_map(|item| hir.library_functions(item.id))
        .collect::<HashSet<_>>();
    let mut diagnostics = Vec::new();
    for function in &syntax.functions {
        if library_functions.contains(&function.id) {
            continue;
        }
        let inferred_async =
            effects.function(function.id).suspension == crate::stdlib::SuspensionKind::Suspends;
        match (
            function.return_annotation,
            function.return_is_async,
            inferred_async,
        ) {
            (Some(_), false, true) => {
                let annotation = function
                    .return_annotation_span
                    .expect("explicit return annotations retain their span");
                let insertion = ast::Span {
                    start: annotation.start,
                    end: annotation.start,
                };
                diagnostics.push(
                    Diagnostic::semantic(
                        format!(
                            "function `{}` suspends, so its explicit result must be marked `async`",
                            function.name
                        ),
                        annotation,
                    )
                    .with_machine_applicable_fix(
                        "mark the result as async",
                        insertion,
                        "async ",
                    ),
                );
            }
            (Some(_), true, false) => {
                let keyword = function
                    .return_async_span
                    .expect("async return annotations retain the keyword span");
                let annotation = function
                    .return_annotation_span
                    .expect("explicit return annotations retain their span");
                diagnostics.push(
                    Diagnostic::semantic(
                        format!(
                            "function `{}` is declared async but never suspends",
                            function.name
                        ),
                        keyword,
                    )
                    .with_machine_applicable_fix(
                        "remove `async`",
                        ast::Span {
                            start: keyword.start,
                            end: annotation.start,
                        },
                        "",
                    ),
                );
            }
            _ => {}
        }
    }
    diagnostics
}

fn validate_suspending_calls(
    standard_library: &StandardLibrary,
    syntax: &Program,
    hir: &TypedProgram,
    effects: &OperationAnalysis,
) -> Vec<Diagnostic> {
    #[derive(Default)]
    struct AwaitCollector {
        operands: HashSet<crate::ast::ExprId>,
    }

    impl TypedVisitor for AwaitCollector {
        fn visit_expression(&mut self, expression: &hir::TypedExpression, program: &TypedProgram) {
            if let hir::TypedExpressionKind::Suspend {
                mode: ast::SuspensionMode::Await,
                value,
                ..
            } = expression.kind
            {
                self.operands.insert(value);
            }
            hir::walk_typed_expression(self, expression, program);
        }
    }

    fn call_semantics(
        call: &ResolvedCall,
        standard_library: &StandardLibrary,
        syntax: &Program,
        hir: &TypedProgram,
        effects: &OperationAnalysis,
    ) -> Option<(crate::stdlib::SuspensionKind, String)> {
        match call {
            ResolvedCall::StandardLibrary { item, .. } => {
                let declaration = standard_library.item(*item);
                let suspension = hir
                    .library_functions(*item)
                    .map(|function| effects.function(function).suspension)
                    .max_by_key(|suspension| match suspension {
                        crate::stdlib::SuspensionKind::None => 0,
                        crate::stdlib::SuspensionKind::Suspends => 1,
                    })
                    .unwrap_or_else(|| standard_library.operation_semantics(*item).suspension);
                Some((suspension, declaration.qualified_name.to_owned()))
            }
            ResolvedCall::UserFunction { function, .. }
            | ResolvedCall::UserMethod { function, .. } => {
                let name = syntax
                    .functions
                    .iter()
                    .find(|declaration| declaration.id == *function)
                    .map(|declaration| declaration.name.clone())
                    .unwrap_or_else(|| "function".to_owned());
                Some((effects.function(*function).suspension, name))
            }
            ResolvedCall::ManagedSnapshot { .. } | ResolvedCall::ManagedComponent { .. } => None,
            ResolvedCall::ManagedInstances { .. } => Some((
                crate::stdlib::SuspensionKind::Suspends,
                "instances".to_owned(),
            )),
            ResolvedCall::ResultError { .. }
            | ResolvedCall::OptionSome { .. }
            | ResolvedCall::IteratorItem { .. }
            | ResolvedCall::ResultSuccess { .. } => None,
        }
    }

    fn has_runtime_future_storage(
        call: &ResolvedCall,
        _standard_library: &StandardLibrary,
    ) -> bool {
        match call {
            ResolvedCall::UserFunction { .. } | ResolvedCall::UserMethod { .. } => true,
            ResolvedCall::StandardLibrary { .. } => true,
            ResolvedCall::ManagedSnapshot { .. } | ResolvedCall::ManagedComponent { .. } => false,
            ResolvedCall::ManagedInstances { .. } => true,
            ResolvedCall::ResultError { .. }
            | ResolvedCall::OptionSome { .. }
            | ResolvedCall::IteratorItem { .. }
            | ResolvedCall::ResultSuccess { .. } => false,
        }
    }

    let mut awaited = AwaitCollector::default();
    for function in hir.all_function_bodies() {
        awaited.visit_block(&function.body, hir);
    }
    for action in hir.action_bodies() {
        awaited.visit_block(&action.body, hir);
    }

    let mut diagnostics = Vec::new();
    for expression in hir.expressions() {
        let Some(call) = hir.call(expression.id) else {
            continue;
        };
        let Some((suspension, name)) = call_semantics(call, standard_library, syntax, hir, effects)
        else {
            continue;
        };
        if suspension == crate::stdlib::SuspensionKind::Suspends
            && !awaited.operands.contains(&expression.id)
            && !has_runtime_future_storage(call, standard_library)
        {
            diagnostics.push(Diagnostic::semantic(
                format!("`{name}` suspends and must be awaited"),
                expression.span,
            ));
        }
    }
    for operand in awaited.operands {
        let Some(expression) = hir.expression(operand) else {
            continue;
        };
        let Some(call) = hir.call(operand) else {
            continue;
        };
        let Some((suspension, name)) = call_semantics(call, standard_library, syntax, hir, effects)
        else {
            continue;
        };
        if !suspension.is_awaitable() {
            diagnostics.push(Diagnostic::semantic(
                format!("`{name}` is synchronous and cannot be awaited"),
                expression.span,
            ));
        }
    }
    diagnostics
}

#[derive(Debug, Clone, Copy)]
enum LocalBindingKind {
    Parameter,
    Variable,
    Loop,
    Suspension,
    Pattern,
}

impl LocalBindingKind {
    fn description(self) -> &'static str {
        match self {
            Self::Parameter => "parameter",
            Self::Variable => "variable",
            Self::Loop => "loop binding",
            Self::Suspension => "suspension binding",
            Self::Pattern => "pattern binding",
        }
    }
}

#[derive(Debug)]
struct LocalBinding {
    id: ast::ValueId,
    name: String,
    name_span: ast::Span,
    kind: LocalBindingKind,
}

#[derive(Default)]
struct LocalBindingCollector {
    bindings: Vec<LocalBinding>,
}

impl LocalBindingCollector {
    fn push(&mut self, id: ast::ValueId, name: &str, name_span: ast::Span, kind: LocalBindingKind) {
        if self.bindings.iter().any(|binding| binding.id == id) {
            return;
        }
        self.bindings.push(LocalBinding {
            id,
            name: name.to_owned(),
            name_span,
            kind,
        });
    }
}

impl<'ast> SyntaxVisitor<'ast> for LocalBindingCollector {
    fn visit_parameter(&mut self, parameter: &'ast ast::Parameter) {
        parameter.binding.visit_bindings(&mut |binding| {
            self.push(
                binding.id,
                &binding.name,
                binding.name_span,
                LocalBindingKind::Parameter,
            );
        });
        visit::walk_parameter(self, parameter);
    }

    fn visit_variable(&mut self, variable: &'ast ast::VariableDecl) {
        variable.binding.visit_bindings(&mut |binding| {
            self.push(
                binding.id,
                &binding.name,
                binding.name_span,
                LocalBindingKind::Variable,
            );
        });
        visit::walk_variable(self, variable);
    }

    fn visit_suspension_binding(&mut self, binding: &'ast ast::SuspensionBinding) {
        self.push(
            binding.id,
            &binding.name,
            binding.name_span,
            LocalBindingKind::Suspension,
        );
        if let Some(annotation) = &binding.annotation {
            self.visit_type_ref(annotation);
        }
    }

    fn visit_for_binding(&mut self, binding: &'ast ast::ForBinding) {
        binding.binding.visit_bindings(&mut |binding| {
            self.push(
                binding.id,
                &binding.name,
                binding.name_span,
                LocalBindingKind::Loop,
            );
        });
    }

    fn visit_pattern(&mut self, pattern: &'ast ast::MatchPattern) {
        let binding = match pattern {
            ast::MatchPattern::Bool(_)
            | ast::MatchPattern::Char(_)
            | ast::MatchPattern::String(_)
            | ast::MatchPattern::Int { .. }
            | ast::MatchPattern::IntRange { .. }
            | ast::MatchPattern::FileVersion(_)
            | ast::MatchPattern::None
            | ast::MatchPattern::IteratorEnd
            | ast::MatchPattern::OptionSome(_)
            | ast::MatchPattern::IteratorItem(_)
            | ast::MatchPattern::ResultSuccess(_)
            | ast::MatchPattern::ResultError(_)
            | ast::MatchPattern::Struct { .. }
            | ast::MatchPattern::Enum { .. }
            | ast::MatchPattern::Wildcard
            | ast::MatchPattern::Array(_)
            | ast::MatchPattern::Alternation(_) => None,
            ast::MatchPattern::Binding(binding) => Some(binding),
        };
        if let Some(binding) = binding {
            self.push(
                binding.id,
                &binding.name,
                binding.name_span,
                LocalBindingKind::Pattern,
            );
        }
        visit::walk_pattern(self, pattern);
    }
}

fn validate_unused_bindings(
    syntax: &Program,
    hir: &TypedProgram,
    function_profiles: &HashMap<ast::FunctionId, UseProfiles>,
) -> Vec<Diagnostic> {
    let visible_functions = hir
        .function_bodies()
        .map(|body| body.function.function)
        .collect::<HashSet<_>>();
    let mut collector = LocalBindingCollector::default();
    for function in syntax
        .functions
        .iter()
        .filter(|function| visible_functions.contains(&function.id))
    {
        for (index, parameter) in function.params.iter().enumerate() {
            // Methods receive an implicit `self` parameter whose source span is
            // the receiver type. It is not a user-written binding declaration.
            if function.method_of.is_some() && index == 0 && parameter.name == "self" {
                continue;
            }
            parameter.binding.visit_bindings(&mut |binding| {
                collector.push(
                    binding.id,
                    &binding.name,
                    binding.name_span,
                    LocalBindingKind::Parameter,
                );
            });
        }
        collector.visit_block(&function.body);
    }
    for action in &syntax.actions {
        collector.visit_block(&action.body);
    }
    let occupied_names = collector
        .bindings
        .iter()
        .map(|binding| binding.name.clone())
        .chain(syntax.globals.iter().flat_map(|global| {
            let mut names = Vec::new();
            global
                .binding
                .visit_bindings(&mut |binding| names.push(binding.name.clone()));
            names
        }))
        .collect::<HashSet<_>>();

    let mut usage = LocalUsageCollector::default();
    for body in hir.function_bodies() {
        usage.active_profiles = function_profiles
            .get(&body.function.function)
            .copied()
            .unwrap_or({
                if body.debug_only {
                    UseProfiles::DEBUG
                } else {
                    // Preserve ordinary binding diagnostics inside unreachable
                    // functions without inventing a debug-only execution path.
                    UseProfiles::ALL
                }
            });
        usage.visit_block(&body.body, hir);
    }
    for body in hir.action_bodies() {
        usage.active_profiles = UseProfiles::ALL;
        usage.visit_block(&body.body, hir);
    }

    let mut diagnostics = Vec::new();
    for binding in collector
        .bindings
        .into_iter()
        .filter(|binding| !binding.name.starts_with('_'))
    {
        let read_profiles = usage
            .reads
            .get(&binding.id)
            .copied()
            .unwrap_or(UseProfiles(0));
        if read_profiles.is_empty() {
            let mut replacement = format!("_{}", binding.name);
            while occupied_names.contains(&replacement) {
                replacement.insert(0, '_');
            }
            let mut edits = vec![TextEdit {
                span: binding.name_span,
                replacement: replacement.clone(),
            }];
            edits.extend(
                usage
                    .writes
                    .get(&binding.id)
                    .into_iter()
                    .flatten()
                    .map(|(span, _)| TextEdit {
                        span: ast::Span {
                            start: span.start,
                            end: span.start + binding.name.len(),
                        },
                        replacement: replacement.clone(),
                    }),
            );
            diagnostics.push(
                Diagnostic::warning(
                    DiagnosticCode::UnusedBinding,
                    format!("unused {} `{}`", binding.kind.description(), binding.name),
                    binding.name_span,
                )
                .with_primary_label("this binding is never read")
                .with_note("prefix the name with `_` to indicate that this is intentional")
                .with_fix(DiagnosticFix {
                    title: format!("rename `{}` to `{replacement}`", binding.name),
                    applicability: FixApplicability::MachineApplicable,
                    edits,
                }),
            );
            continue;
        }

        let Some(declaration) = usage.declarations.get(&binding.id) else {
            continue;
        };
        if declaration.profiles.contains(UseProfiles::RELEASE)
            && read_profiles.contains(UseProfiles::DEBUG)
            && !read_profiles.contains(UseProfiles::RELEASE)
        {
            let release_write = usage
                .writes
                .get(&binding.id)
                .into_iter()
                .flatten()
                .any(|(_, profiles)| profiles.contains(UseProfiles::RELEASE));
            let mut diagnostic = Diagnostic::warning(
                DiagnosticCode::DebugOnlyUse,
                format!("local `{}` is only read by debug code", binding.name),
                binding.name_span,
            )
            .with_primary_label("this local and its initializer are retained in release builds")
            .with_note(
                "mark declarations used exclusively for diagnostics as `debug` so release builds can erase them",
            );
            if release_write {
                diagnostic = diagnostic.with_note(
                    "this local is also assigned by release-visible code, so the compiler cannot safely apply that change",
                );
            } else {
                diagnostic = diagnostic.with_machine_applicable_fix(
                    format!("mark `{}` as debug-only", binding.name),
                    declaration.insertion,
                    "debug ",
                );
            }
            diagnostics.push(diagnostic);
        }
    }
    diagnostics.sort_by_key(|diagnostic| (diagnostic.span.start, diagnostic.span.end));
    diagnostics
}

#[derive(Debug, Clone, Copy)]
struct ErasableLocalDeclaration {
    profiles: UseProfiles,
    insertion: ast::Span,
}

struct LocalUsageCollector {
    active_profiles: UseProfiles,
    reads: HashMap<ast::ValueId, UseProfiles>,
    writes: HashMap<ast::ValueId, Vec<(ast::Span, UseProfiles)>>,
    declarations: HashMap<ast::ValueId, ErasableLocalDeclaration>,
}

impl Default for LocalUsageCollector {
    fn default() -> Self {
        Self {
            active_profiles: UseProfiles::ALL,
            reads: HashMap::new(),
            writes: HashMap::new(),
            declarations: HashMap::new(),
        }
    }
}

impl LocalUsageCollector {
    fn record_read(&mut self, value: ast::ValueId) {
        merge_profiled(&mut self.reads, value, self.active_profiles);
    }
}

impl TypedVisitor for LocalUsageCollector {
    fn visit_statement(&mut self, statement: &hir::TypedStatement, program: &TypedProgram) {
        let inherited_profiles = self.active_profiles;
        if statement.debug_only {
            self.active_profiles = self.active_profiles.intersect(UseProfiles::DEBUG);
        }
        if self.active_profiles.is_empty() {
            self.active_profiles = inherited_profiles;
            return;
        }

        match &statement.kind {
            TypedStatementKind::Variable { binding, .. } => {
                binding.pattern.pattern.visit_bindings(&mut |binding| {
                    self.declarations.insert(
                        binding.id,
                        ErasableLocalDeclaration {
                            profiles: self.active_profiles,
                            insertion: ast::Span {
                                start: statement.span.start,
                                end: statement.span.start,
                            },
                        },
                    );
                });
            }
            TypedStatementKind::Suspend {
                binding: Some(value),
                ..
            } => {
                self.declarations.insert(
                    *value,
                    ErasableLocalDeclaration {
                        profiles: self.active_profiles,
                        insertion: ast::Span {
                            start: statement.span.start,
                            end: statement.span.start,
                        },
                    },
                );
            }
            TypedStatementKind::Assign { assignment, op, .. } => {
                self.writes
                    .entry(assignment.target)
                    .or_default()
                    .push((assignment.span, self.active_profiles));
                if op.is_some() {
                    self.record_read(assignment.target);
                }
            }
            TypedStatementKind::StateAssign { .. }
            | TypedStatementKind::IndexAssign { .. }
            | TypedStatementKind::If { .. }
            | TypedStatementKind::While { .. }
            | TypedStatementKind::For { .. }
            | TypedStatementKind::Suspend { binding: None, .. }
            | TypedStatementKind::Yield { .. }
            | TypedStatementKind::Expression(_) => {}
        }

        hir::walk_typed_statement(self, statement, program);
        self.active_profiles = inherited_profiles;
    }

    fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
        if let Some(ExpressionResolution::DynamicCall(DynamicCallCallee::Value(value))) =
            expression.resolution
        {
            self.record_read(value);
        }
        let root = program
            .value_path(expression.id)
            .and_then(|(root, _)| root)
            .or_else(|| {
                program.call(expression.id).and_then(|call| {
                    call.receiver()
                        .and_then(|receiver| receiver.path().map(|(root, _)| root))
                })
            });
        if let Some(value) = root.and_then(ResolvedValue::source_value) {
            self.record_read(value);
        }
        hir::walk_typed_expression(self, expression, program);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum DeclarationWorkItem {
    Global(ast::ValueId),
    Function(ast::FunctionId),
}

/// Build profiles in which a use is retained.
///
/// Keeping this on dependency edges lets ordinary unused analysis and
/// release-erasure guidance share one reachability graph. A debug-only edge
/// can never make the declaration behind it release-reachable, including
/// through an arbitrary chain of otherwise ordinary helper functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UseProfiles(u8);

impl UseProfiles {
    const DEBUG: Self = Self(1 << 0);
    const RELEASE: Self = Self(1 << 1);
    const ALL: Self = Self(Self::DEBUG.0 | Self::RELEASE.0);

    fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    fn is_empty(self) -> bool {
        self.0 == 0
    }
}

fn merge_profiled<K: Eq + std::hash::Hash + Copy>(
    values: &mut HashMap<K, UseProfiles>,
    key: K,
    profiles: UseProfiles,
) {
    values
        .entry(key)
        .and_modify(|existing| *existing = existing.union(profiles))
        .or_insert(profiles);
}

struct DeclarationDependencyCollector<'a> {
    semantics: &'a SemanticModel,
    active_profiles: UseProfiles,
    dependencies: HashMap<DeclarationWorkItem, UseProfiles>,
    writes: HashMap<ast::ValueId, UseProfiles>,
    types: HashSet<crate::types::TypeId>,
    observed_state_fields: HashSet<ast::ValueId>,
    observed_settings: HashSet<ast::ValueId>,
    literal_setting_keys: HashSet<String>,
    has_dynamic_setting_lookup: bool,
    observed_struct_fields: HashSet<ast::StructFieldId>,
    observed_enum_variants: HashSet<ast::EnumVariantId>,
    fully_observed_types: HashSet<crate::types::TypeId>,
    capability_calls: HashMap<(crate::types::TypeId, StdlibItemId), UseProfiles>,
}

impl<'a> DeclarationDependencyCollector<'a> {
    fn new(semantics: &'a SemanticModel) -> Self {
        Self::with_profiles(semantics, UseProfiles::ALL)
    }

    fn with_profiles(semantics: &'a SemanticModel, active_profiles: UseProfiles) -> Self {
        Self {
            semantics,
            active_profiles,
            dependencies: HashMap::new(),
            writes: HashMap::new(),
            types: HashSet::new(),
            observed_state_fields: HashSet::new(),
            observed_settings: HashSet::new(),
            literal_setting_keys: HashSet::new(),
            has_dynamic_setting_lookup: false,
            observed_struct_fields: HashSet::new(),
            observed_enum_variants: HashSet::new(),
            fully_observed_types: HashSet::new(),
            capability_calls: HashMap::new(),
        }
    }

    fn expand_capability_dependencies(
        &mut self,
        capabilities: &CapabilityAnalysis,
        semantics: &SemanticModel,
    ) {
        for ((receiver, requirement), profiles) in std::mem::take(&mut self.capability_calls) {
            let dependencies = capabilities.method_dependencies(receiver, requirement, semantics);
            for function in dependencies.source_functions {
                merge_profiled(
                    &mut self.dependencies,
                    DeclarationWorkItem::Function(function),
                    profiles,
                );
            }
            self.fully_observed_types
                .extend(dependencies.derived_aggregates);
        }
    }
}

impl TypedVisitor for DeclarationDependencyCollector<'_> {
    fn visit_statement(&mut self, statement: &hir::TypedStatement, program: &TypedProgram) {
        let inherited_profiles = self.active_profiles;
        if statement.debug_only {
            self.active_profiles = self.active_profiles.intersect(UseProfiles::DEBUG);
        }
        if self.active_profiles.is_empty() {
            self.active_profiles = inherited_profiles;
            return;
        }
        if let TypedStatementKind::Assign { assignment, .. } = &statement.kind {
            merge_profiled(&mut self.writes, assignment.target, self.active_profiles);
        }
        if let TypedStatementKind::StateAssign {
            op: None, value, ..
        } = &statement.kind
        {
            // Replacing `current.field` writes the snapshot slot but does not
            // observe the value produced by polling it. Compound assignment
            // still walks its target through the ordinary visitor because it
            // reads the previous value before writing the result.
            self.visit_expression(
                program
                    .expression(*value)
                    .expect("state assignment value belongs to typed HIR"),
                program,
            );
            self.active_profiles = inherited_profiles;
            return;
        }
        hir::walk_typed_statement(self, statement, program);
        self.active_profiles = inherited_profiles;
    }

    fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
        self.types.insert(expression.ty);
        for ty in hir::implicit_display_types(expression, program, self.semantics) {
            merge_profiled(
                &mut self.capability_calls,
                (ty, StdlibItemId::DisplayToString),
                self.active_profiles,
            );
        }
        let mut observe_members = |members: &[ResolvedMember]| {
            self.observed_state_fields
                .extend(members.iter().filter_map(|member| match member {
                    ResolvedMember::StateField(field) => Some(*field),
                    ResolvedMember::SettingField(_)
                    | ResolvedMember::StructField(_)
                    | ResolvedMember::ManagedField(_)
                    | ResolvedMember::StandardField(_) => None,
                }));
            self.observed_settings
                .extend(members.iter().filter_map(|member| match member {
                    ResolvedMember::SettingField(setting) => Some(*setting),
                    ResolvedMember::StateField(_)
                    | ResolvedMember::StructField(_)
                    | ResolvedMember::ManagedField(_)
                    | ResolvedMember::StandardField(_) => None,
                }));
            self.observed_struct_fields
                .extend(members.iter().filter_map(|member| match member {
                    ResolvedMember::StructField(field) => Some(*field),
                    ResolvedMember::StateField(_)
                    | ResolvedMember::SettingField(_)
                    | ResolvedMember::ManagedField(_)
                    | ResolvedMember::StandardField(_) => None,
                }));
        };
        match &expression.resolution {
            Some(ExpressionResolution::ValuePath { members, .. })
            | Some(ExpressionResolution::Member { members }) => observe_members(members),
            Some(ExpressionResolution::Call(call)) => {
                if let Some(receiver) = call.receiver() {
                    observe_members(receiver.members());
                }
            }
            Some(ExpressionResolution::DynamicCall(_)) => {}
            Some(ExpressionResolution::FunctionValue(function)) => {
                merge_profiled(
                    &mut self.dependencies,
                    DeclarationWorkItem::Function(function.function),
                    self.active_profiles,
                );
            }
            Some(ExpressionResolution::EnumConstructor {
                variant: ResolvedEnumVariantId::Source(variant),
            }) => {
                self.observed_enum_variants.insert(*variant);
            }
            Some(ExpressionResolution::StructLiteral { .. })
            | Some(ExpressionResolution::EnumConstructor {
                variant: ResolvedEnumVariantId::Standard(_),
            })
            | None => {}
        }
        if let TypedExpressionKind::Binary {
            op: ast::BinaryOp::Eq | ast::BinaryOp::Ne,
            left,
            ..
        } = expression.kind
        {
            self.fully_observed_types.insert(
                program
                    .expression(left)
                    .expect("binary operand belongs to typed HIR")
                    .ty,
            );
        }
        let resolved_root = program
            .value_path(expression.id)
            .and_then(|(root, _)| root)
            .or_else(|| {
                program.call(expression.id).and_then(|call| {
                    call.receiver()
                        .and_then(|receiver| receiver.path().map(|(root, _)| root))
                })
            });
        let source_value = resolved_root.and_then(ResolvedValue::source_value);
        if let Some(value) = source_value {
            merge_profiled(
                &mut self.dependencies,
                DeclarationWorkItem::Global(value),
                self.active_profiles,
            );
        }

        if let Some(
            ResolvedValue::CurrentState(field)
            | ResolvedValue::OldState(field)
            | ResolvedValue::StateCandidate(field),
        ) = resolved_root
        {
            self.observed_state_fields.insert(field);
        }

        if let Some(ResolvedValue::Setting(setting) | ResolvedValue::OldSetting(setting)) =
            resolved_root
        {
            self.observed_settings.insert(setting);
        }

        if let (
            Some(ResolvedCall::StandardLibrary { item, .. }),
            TypedExpressionKind::Call { arguments, .. },
        ) = (program.call(expression.id), &expression.kind)
            && matches!(
                *item,
                crate::stdlib::StdlibItemId::SettingsViewEnabled
                    | crate::stdlib::StdlibItemId::SettingsViewContains
            )
            && let Some(argument) = arguments.first()
        {
            match &program
                .expression(*argument)
                .expect("call argument belongs to typed HIR")
                .kind
            {
                TypedExpressionKind::String(key) => {
                    self.literal_setting_keys.insert(key.clone());
                }
                _ => self.has_dynamic_setting_lookup = true,
            }
        }

        if let Some(
            ResolvedCall::UserFunction { function, .. } | ResolvedCall::UserMethod { function, .. },
        ) = program.call(expression.id)
        {
            merge_profiled(
                &mut self.dependencies,
                DeclarationWorkItem::Function(*function),
                self.active_profiles,
            );
        }

        if let Some(ResolvedCall::StandardLibrary {
            item,
            receiver_type: Some(receiver),
            ..
        }) = program.call(expression.id)
        {
            let declaration = program.standard_library().item(*item);
            if declaration.implementation == Implementation::CapabilityRequirement
                && matches!(declaration.owner, StdlibOwner::Capability(_))
            {
                merge_profiled(
                    &mut self.capability_calls,
                    (*receiver, *item),
                    self.active_profiles,
                );
            }
        }

        hir::walk_typed_expression(self, expression, program);
    }

    fn visit_match_arm(&mut self, arm: &TypedMatchArm, program: &TypedProgram) {
        if let Some(ResolvedEnumVariantId::Source(variant)) = arm.resolution.variant {
            self.observed_enum_variants.insert(variant);
        }
        hir::walk_typed_match_arm(self, arm, program);
    }
}

/// Finds source declarations that can be reached from compiler-invoked code.
///
/// Unlike backend reachability, global initializers are not unconditional
/// roots here. An initializer becomes reachable only after its global is read,
/// which lets this user-facing analysis diagnose a dead global together with
/// helper declarations used exclusively by that initializer.
struct UnusedDeclarationValidation {
    diagnostics: Vec<Diagnostic>,
    function_profiles: HashMap<ast::FunctionId, UseProfiles>,
}

fn validate_unused_declarations(
    syntax: &Program,
    hir: &TypedProgram,
    semantics: &SemanticModel,
    capabilities: &CapabilityAnalysis,
) -> UnusedDeclarationValidation {
    let mut globals = HashMap::new();
    for global in &syntax.globals {
        global.binding.visit_bindings(&mut |binding| {
            globals.insert(binding.id, global);
        });
    }
    let initializers = hir
        .global_initializers()
        .flat_map(|initializer| {
            let mut bindings = Vec::new();
            initializer
                .binding
                .pattern
                .pattern
                .visit_bindings(&mut |binding| bindings.push(binding.id));
            bindings
                .into_iter()
                .map(move |binding| (binding, initializer.expression))
        })
        .collect::<HashMap<_, _>>();
    let functions = hir
        .function_bodies()
        .map(|body| (body.function.function, body))
        .collect::<HashMap<_, _>>();

    let mut reachable = HashMap::<DeclarationWorkItem, UseProfiles>::new();
    let mut declaration_writes = HashMap::<ast::ValueId, UseProfiles>::new();
    let mut reachable_types = HashSet::new();
    let mut observed_state_fields = HashSet::new();
    let mut observed_settings = HashSet::new();
    let mut literal_setting_keys = HashSet::new();
    let mut has_dynamic_setting_lookup = false;
    let mut observed_struct_fields = HashSet::new();
    let mut observed_enum_variants = HashSet::new();
    let mut fully_observed_types = HashSet::new();
    let mut pending = VecDeque::new();
    let mut roots = DeclarationDependencyCollector::new(semantics);
    for action in hir.action_bodies() {
        roots.visit_block(&action.body, hir);
    }
    // Provider constructors are compiler-invoked roots, even when no user
    // lifecycle block calls the function that builds the selected profile.
    for provider in syntax.state.iter().flat_map(|state| {
        state.provider.iter().chain(
            state
                .provider_alternatives
                .iter()
                .map(|alternative| &alternative.provider),
        )
    }) {
        if let Some(selector) = &provider.selector {
            for argument in &selector.arguments {
                if let Some(expression) = hir.expression(argument.id) {
                    roots.visit_expression(expression, hir);
                }
            }
        }
    }
    roots.expand_capability_dependencies(capabilities, semantics);
    for (value, profiles) in roots.writes {
        merge_profiled(&mut declaration_writes, value, profiles);
    }
    reachable_types.extend(roots.types.iter().copied());
    observed_state_fields.extend(roots.observed_state_fields.iter().copied());
    observed_settings.extend(roots.observed_settings.iter().copied());
    literal_setting_keys.extend(roots.literal_setting_keys.iter().cloned());
    has_dynamic_setting_lookup |= roots.has_dynamic_setting_lookup;
    observed_struct_fields.extend(roots.observed_struct_fields.iter().copied());
    observed_enum_variants.extend(roots.observed_enum_variants.iter().copied());
    fully_observed_types.extend(roots.fully_observed_types.iter().copied());
    pending.extend(roots.dependencies);

    // Every state source and filter executes while its physical field is
    // active, even when the resulting snapshot value is never consumed. Keep
    // calls, globals, settings, types, and other effects in declaration
    // reachability, but do not mistake evaluating a field for observing the
    // value that it produces.
    let mut state_execution_roots = DeclarationDependencyCollector::new(semantics);
    for (_, expression) in hir.state_sources() {
        state_execution_roots.visit_expression(
            hir.expression(expression)
                .expect("state source belongs to typed HIR"),
            hir,
        );
    }
    for transform in hir.state_transforms() {
        state_execution_roots.visit_expression(
            hir.expression(transform.expression)
                .expect("state transform belongs to typed HIR"),
            hir,
        );
    }
    state_execution_roots.expand_capability_dependencies(capabilities, semantics);
    for (value, profiles) in state_execution_roots.writes {
        merge_profiled(&mut declaration_writes, value, profiles);
    }
    reachable_types.extend(state_execution_roots.types.iter().copied());
    observed_settings.extend(state_execution_roots.observed_settings.iter().copied());
    literal_setting_keys.extend(state_execution_roots.literal_setting_keys.iter().cloned());
    has_dynamic_setting_lookup |= state_execution_roots.has_dynamic_setting_lookup;
    observed_struct_fields.extend(state_execution_roots.observed_struct_fields.iter().copied());
    observed_enum_variants.extend(state_execution_roots.observed_enum_variants.iter().copied());
    fully_observed_types.extend(state_execution_roots.fully_observed_types.iter().copied());
    pending.extend(state_execution_roots.dependencies);

    // Conditional schema predicates are compiler-consumed runtime reads. They
    // are not ordinary HIR expressions, so seed their globals, discriminator
    // state fields, and matched variants explicitly into the same reachability
    // graph used for user-authored reads.
    let predicates = syntax
        .state
        .iter()
        .flat_map(|state| state.all_fields())
        .filter_map(|field| semantics.state_field_shape_predicate(field.id))
        .chain(
            syntax
                .managed_class_declarations()
                .into_iter()
                .flat_map(|class| class.all_fields())
                .filter_map(|field| semantics.managed_field_shape_predicate(field.id)),
        );
    for predicate in predicates {
        for constraint in predicate.alternatives.iter().flatten() {
            observed_enum_variants.insert(constraint.variant);
            match constraint.dimension {
                crate::semantic::ResolvedShapeDimension::Global(value) => {
                    pending.push_back((DeclarationWorkItem::Global(value), UseProfiles::ALL))
                }
                crate::semantic::ResolvedShapeDimension::StateField(value) => {
                    observed_state_fields.insert(value);
                }
            }
        }
    }

    // State storage and settings declarations exist at runtime even when user
    // code never reads their values. Their complete value types therefore
    // seed nominal type reachability. This does not make the internal state
    // snapshot a host-observable interface.
    if let Some(state) = &syntax.state {
        reachable_types.extend(
            state
                .all_fields()
                .filter_map(|field| semantics.value_type(field.id)),
        );
    }
    reachable_types.extend(
        syntax
            .settings
            .iter()
            .filter_map(|setting| semantics.value_type(setting.id)),
    );
    for setting in &syntax.settings {
        if let ast::SettingKind::Choice { options, .. } = &setting.kind {
            if let Some(default) = hir.setting_choice_default(setting.id) {
                observed_enum_variants.insert(default);
            }
            observed_enum_variants.extend(
                options
                    .iter()
                    .filter_map(|option| hir.setting_choice_option(option.id)),
            );
        }
    }

    while let Some((item, incoming_profiles)) = pending.pop_front() {
        let declaration_profiles = match item {
            DeclarationWorkItem::Global(value) => {
                globals.get(&value).map_or(UseProfiles::ALL, |global| {
                    if global.debug_only {
                        UseProfiles::DEBUG
                    } else {
                        UseProfiles::ALL
                    }
                })
            }
            DeclarationWorkItem::Function(function) => {
                functions.get(&function).map_or(UseProfiles::ALL, |body| {
                    if body.debug_only {
                        UseProfiles::DEBUG
                    } else {
                        UseProfiles::ALL
                    }
                })
            }
        };
        let incoming_profiles = incoming_profiles.intersect(declaration_profiles);
        let previous_profiles = reachable.get(&item).copied().unwrap_or(UseProfiles(0));
        let new_profiles = incoming_profiles.difference(previous_profiles);
        if new_profiles.is_empty() {
            continue;
        }
        merge_profiled(&mut reachable, item, new_profiles);

        let mut collector = DeclarationDependencyCollector::with_profiles(semantics, new_profiles);
        match item {
            DeclarationWorkItem::Global(value) => {
                let Some(expression) = initializers.get(&value).copied() else {
                    // State, setting, and local values are ordinary semantic
                    // value roots but are not declaration work items.
                    continue;
                };
                collector.visit_expression(
                    hir.expression(expression)
                        .expect("global initializer belongs to typed HIR"),
                    hir,
                );
                if let Some(ty) = semantics.value_type(value) {
                    reachable_types.insert(ty);
                }
            }
            DeclarationWorkItem::Function(function) => {
                let Some(body) = functions.get(&function) else {
                    // Hidden standard-library source bodies are catalog-owned,
                    // not user declarations diagnosed by this pass.
                    continue;
                };
                collector.visit_block(&body.body, hir);
                reachable_types
                    .extend(semantics.function_parameter_types(function).iter().copied());
                if let Some(result) = semantics.function_result(function) {
                    reachable_types.insert(result);
                }
            }
        }
        collector.expand_capability_dependencies(capabilities, semantics);
        for (value, profiles) in collector.writes {
            merge_profiled(&mut declaration_writes, value, profiles);
        }
        reachable_types.extend(collector.types.iter().copied());
        observed_state_fields.extend(collector.observed_state_fields.iter().copied());
        observed_settings.extend(collector.observed_settings.iter().copied());
        literal_setting_keys.extend(collector.literal_setting_keys.iter().cloned());
        has_dynamic_setting_lookup |= collector.has_dynamic_setting_lookup;
        observed_struct_fields.extend(collector.observed_struct_fields.iter().copied());
        observed_enum_variants.extend(collector.observed_enum_variants.iter().copied());
        fully_observed_types.extend(collector.fully_observed_types.iter().copied());
        pending.extend(collector.dependencies);
    }

    let (reachable_structs, reachable_enums) =
        expand_reachable_nominal_types(&mut reachable_types, syntax, semantics);
    expand_fully_observed_types(
        fully_observed_types,
        syntax,
        semantics,
        &mut observed_state_fields,
        &mut observed_struct_fields,
        &mut observed_enum_variants,
    );
    expand_observed_state_field_dependencies(&mut observed_state_fields, syntax, semantics);

    let mut diagnostics = Vec::new();
    if let Some(state) = &syntax.state {
        for storage in semantics.state_storage_fields() {
            if observed_state_fields.contains(storage) {
                continue;
            }
            let declarations = state
                .all_fields()
                .filter(|field| semantics.state_storage_field(field.id) == Some(*storage))
                .collect::<Vec<_>>();
            let Some(field) = declarations.first().copied() else {
                continue;
            };
            if field.name.starts_with('_') {
                continue;
            }
            let mut diagnostic = Diagnostic::warning(
                DiagnosticCode::UnusedMember,
                format!("unused state field `{}`", field.name),
                state_field_name_span(field),
            )
            .with_primary_label("this field is polled, but its value is never read from reachable code")
            .with_note(
                "polling still runs; this warning does not remove process reads or other effects",
            )
            .with_note(
                "read the field through `current`, `old`, or another used state field to make its value observable",
            )
            .with_note("prefix the field name with `_` to indicate that this is intentional");
            for declaration in declarations.iter().skip(1) {
                diagnostic = diagnostic.with_secondary_label(
                    state_field_name_span(declaration),
                    "this conditional declaration shares the same snapshot field",
                );
            }
            diagnostics.push(diagnostic);
        }
    }
    if !has_dynamic_setting_lookup {
        observed_settings.extend(
            syntax
                .settings
                .iter()
                .filter(|setting| literal_setting_keys.contains(setting.runtime_key()))
                .map(|setting| setting.id),
        );
        for setting in syntax.settings.iter().filter(|setting| {
            setting.source_visible
                && !matches!(setting.kind, ast::SettingKind::Title { .. })
                && !setting.name.starts_with('_')
                && !observed_settings.contains(&setting.id)
        }) {
            let replacement = if setting.external_key.is_some() {
                format!("_{}", setting.name)
            } else {
                format!(
                    "_{} key {}",
                    setting.name,
                    quote_string_literal(&setting.name)
                )
            };
            diagnostics.push(
                Diagnostic::warning(
                    DiagnosticCode::UnusedMember,
                    format!("unused setting `{}`", setting.name),
                    setting.name_span,
                )
                .with_primary_label("this setting is never read from reachable code")
                .with_note(
                    "a declared setting cannot affect script behavior until its value is read",
                )
                .with_note("prefix the source name with `_` to indicate that this is intentional")
                .with_fix(DiagnosticFix {
                    title: format!("rename `{}` to `_{}`", setting.name, setting.name),
                    applicability: FixApplicability::MachineApplicable,
                    edits: vec![TextEdit {
                        span: setting.name_span,
                        replacement,
                    }],
                }),
            );
        }
    }
    for global in &syntax.globals {
        let mut bindings = Vec::new();
        global.binding.visit_bindings(&mut |binding| {
            bindings.push((binding.id, binding.name.clone(), binding.name_span));
        });
        for (id, name, name_span) in bindings {
            if name.starts_with('_') {
                continue;
            }
            let profiles = reachable.get(&DeclarationWorkItem::Global(id)).copied();
            if profiles.is_none() {
                diagnostics.push(
                    Diagnostic::warning(
                        DiagnosticCode::UnusedDeclaration,
                        format!("unused global `{name}`"),
                        name_span,
                    )
                    .with_primary_label("this global is never read from reachable code")
                    .with_note("prefix the name with `_` to indicate that this is intentional"),
                );
            } else if !global.debug_only
                && profiles.is_some_and(|profiles| {
                    profiles.contains(UseProfiles::DEBUG)
                        && !profiles.contains(UseProfiles::RELEASE)
                })
            {
                let release_write = declaration_writes
                    .get(&id)
                    .is_some_and(|profiles| profiles.contains(UseProfiles::RELEASE));
                let mut diagnostic = Diagnostic::warning(
                    DiagnosticCode::DebugOnlyUse,
                    format!("global `{name}` is only read by debug code"),
                    name_span,
                )
                .with_primary_label("this global is retained in release builds")
                .with_note(
                    "mark declarations used exclusively for diagnostics as `debug` so release builds can erase them",
                );
                if release_write {
                    diagnostic = diagnostic.with_note(
                        "this global is also assigned by release-visible code, so the compiler cannot safely apply that change",
                    );
                } else {
                    diagnostic = diagnostic.with_machine_applicable_fix(
                        format!("mark `{name}` as debug-only"),
                        ast::Span {
                            start: global.span.start,
                            end: global.span.start,
                        },
                        "debug ",
                    );
                }
                diagnostics.push(diagnostic);
            }
        }
    }
    for function in syntax
        .functions
        .iter()
        .filter(|function| functions.contains_key(&function.id) && !function.name.starts_with('_'))
    {
        let profiles = reachable
            .get(&DeclarationWorkItem::Function(function.id))
            .copied();
        if profiles.is_none() {
            diagnostics.push(
                Diagnostic::warning(
                    DiagnosticCode::UnusedDeclaration,
                    format!("unused function `{}`", function.name),
                    function.name_span,
                )
                .with_primary_label("this function is never called from reachable code")
                .with_note("prefix the name with `_` to indicate that this is intentional"),
            );
        } else if !function.debug_only
            && profiles.is_some_and(|profiles| {
                profiles.contains(UseProfiles::DEBUG) && !profiles.contains(UseProfiles::RELEASE)
            })
        {
            diagnostics.push(
                Diagnostic::warning(
                    DiagnosticCode::DebugOnlyUse,
                    format!("function `{}` is only used by debug code", function.name),
                    function.name_span,
                )
                .with_primary_label("this function is retained in release builds")
                .with_note(
                    "mark declarations used exclusively for diagnostics as `debug` so release builds can erase them",
                )
                .with_machine_applicable_fix(
                    format!("mark `{}` as debug-only", function.name),
                    ast::Span {
                        start: function.span.start,
                        end: function.span.start,
                    },
                    "debug ",
                ),
            );
        }
    }
    for structure in &syntax.structs {
        if !structure.name.starts_with('_') && !reachable_structs.contains(&structure.id) {
            diagnostics.push(
                Diagnostic::warning(
                    DiagnosticCode::UnusedDeclaration,
                    format!("unused struct `{}`", structure.name),
                    structure.name_span,
                )
                .with_primary_label("this struct is never used by reachable code")
                .with_note("prefix the name with `_` to indicate that this is intentional"),
            );
        }
    }
    for enumeration in &syntax.enums {
        if !enumeration.name.starts_with('_') && !reachable_enums.contains(&enumeration.id) {
            diagnostics.push(
                Diagnostic::warning(
                    DiagnosticCode::UnusedDeclaration,
                    format!("unused enum `{}`", enumeration.name),
                    enumeration.name_span,
                )
                .with_primary_label("this enum is never used by reachable code")
                .with_note("prefix the name with `_` to indicate that this is intentional"),
            );
        }
    }
    for structure in syntax
        .structs
        .iter()
        .filter(|structure| reachable_structs.contains(&structure.id))
    {
        for field in &structure.fields {
            if !field.name.starts_with('_') && !observed_struct_fields.contains(&field.id) {
                diagnostics.push(
                    Diagnostic::warning(
                        DiagnosticCode::UnusedMember,
                        format!("unused struct field `{}.{}`", structure.name, field.name),
                        field.name_span,
                    )
                    .with_primary_label("this field is never read from reachable code")
                    .with_note("constructing or deserializing a struct does not read its fields")
                    .with_note(
                        "prefix the field name with `_` to indicate that this is intentional",
                    ),
                );
            }
        }
    }
    for enumeration in syntax
        .enums
        .iter()
        .filter(|enumeration| reachable_enums.contains(&enumeration.id))
    {
        for variant in &enumeration.variants {
            if !variant.name.starts_with('_') && !observed_enum_variants.contains(&variant.id) {
                diagnostics.push(
                    Diagnostic::warning(
                        DiagnosticCode::UnusedMember,
                        format!(
                            "unused enum variant `{}.{}`",
                            enumeration.name, variant.name
                        ),
                        variant.name_span,
                    )
                    .with_primary_label(
                        "this variant is never constructed or matched by reachable code",
                    )
                    .with_note(
                        "prefix the variant name with `_` to indicate that this is intentional",
                    ),
                );
            }
        }
    }
    diagnostics.sort_by_key(|diagnostic| (diagnostic.span.start, diagnostic.span.end));
    let function_profiles = reachable
        .into_iter()
        .filter_map(|(declaration, profiles)| match declaration {
            DeclarationWorkItem::Function(function) => Some((function, profiles)),
            DeclarationWorkItem::Global(_) => None,
        })
        .collect();
    UnusedDeclarationValidation {
        diagnostics,
        function_profiles,
    }
}

fn quote_string_literal(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\'' => quoted.push_str("\\'"),
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

fn state_field_name_span(field: &ast::StateField) -> ast::Span {
    ast::Span {
        start: field.span.start,
        end: field.span.start + field.name.len(),
    }
}

/// Closes state-value observation over the physical candidate dependency
/// graph. Shared multi-provider fields map to one storage field, so reading the
/// public field keeps the dependencies of every runtime-selected physical
/// source observable without producing duplicate warnings.
fn expand_observed_state_field_dependencies(
    observed: &mut HashSet<ast::ValueId>,
    syntax: &Program,
    semantics: &SemanticModel,
) {
    let Some(state) = &syntax.state else {
        observed.clear();
        return;
    };
    let mut normalized = std::mem::take(observed)
        .into_iter()
        .map(|field| semantics.state_storage_field(field).unwrap_or(field))
        .collect::<HashSet<_>>();
    let mut pending = normalized.iter().copied().collect::<VecDeque<_>>();

    while let Some(storage) = pending.pop_front() {
        for declaration in state
            .all_fields()
            .filter(|field| semantics.state_storage_field(field.id) == Some(storage))
        {
            for dependency in semantics.state_dependencies(declaration.id) {
                let dependency = semantics
                    .state_storage_field(*dependency)
                    .unwrap_or(*dependency);
                if normalized.insert(dependency) {
                    pending.push_back(dependency);
                }
            }
        }
    }

    *observed = normalized;
}

fn expand_fully_observed_types(
    roots: HashSet<crate::types::TypeId>,
    syntax: &Program,
    semantics: &SemanticModel,
    observed_state_fields: &mut HashSet<ast::ValueId>,
    observed_struct_fields: &mut HashSet<ast::StructFieldId>,
    observed_enum_variants: &mut HashSet<ast::EnumVariantId>,
) {
    let structs = syntax
        .structs
        .iter()
        .map(|structure| (structure.id, structure))
        .collect::<HashMap<_, _>>();
    let enums = syntax
        .enums
        .iter()
        .map(|enumeration| (enumeration.id, enumeration))
        .collect::<HashMap<_, _>>();
    let mut pending = roots.into_iter().collect::<VecDeque<_>>();
    let mut expanded = HashSet::new();

    while let Some(ty) = pending.pop_front() {
        if !expanded.insert(ty) {
            continue;
        }
        match semantics.types().kind(ty) {
            TypeKind::Error => {}
            TypeKind::StateSnapshot => {
                if let Some(state) = &syntax.state {
                    for field in state.all_fields() {
                        observed_state_fields
                            .insert(semantics.state_storage_field(field.id).unwrap_or(field.id));
                        if let Some(ty) = semantics.value_type(field.id) {
                            pending.push_back(ty);
                        }
                    }
                }
            }
            TypeKind::SettingsView => pending.extend(
                syntax
                    .settings
                    .iter()
                    .filter_map(|setting| semantics.value_type(setting.id)),
            ),
            TypeKind::Struct(structure) => {
                if let Some(structure) = structs.get(structure) {
                    for field in &structure.fields {
                        observed_struct_fields.insert(field.id);
                        if let Some(ty) = semantics.struct_field_type(field.id) {
                            pending.push_back(ty);
                        }
                    }
                }
            }
            TypeKind::Enum(enumeration) => {
                if let Some(enumeration) = enums.get(enumeration) {
                    for variant in &enumeration.variants {
                        observed_enum_variants.insert(variant.id);
                        if let Some(ty) = semantics.enum_variant_payload(variant.id) {
                            pending.push_back(ty);
                        }
                    }
                }
            }
            TypeKind::Array { element, .. }
            | TypeKind::Set { element, .. }
            | TypeKind::Option { value: element, .. }
            | TypeKind::Result { value: element, .. }
            | TypeKind::Async { value: element, .. }
            | TypeKind::Iterator { item: element, .. }
            | TypeKind::Range { bound: element, .. } => pending.push_back(*element),
            TypeKind::Application { arguments, .. } => {
                pending.extend(arguments.iter().copied());
            }
            TypeKind::Callable {
                parameters, result, ..
            } => {
                pending.extend(parameters.iter().copied());
                pending.push_back(*result);
            }
            TypeKind::Builtin(_)
            | TypeKind::Standard(_)
            | TypeKind::ManagedClass(_)
            | TypeKind::ManagedReference(_)
            | TypeKind::GenericParameter { .. } => {}
        }
    }
}

fn expand_reachable_nominal_types(
    roots: &mut HashSet<crate::types::TypeId>,
    syntax: &Program,
    semantics: &SemanticModel,
) -> (HashSet<ast::StructId>, HashSet<ast::EnumId>) {
    let structs = syntax
        .structs
        .iter()
        .map(|structure| (structure.id, structure))
        .collect::<HashMap<_, _>>();
    let enums = syntax
        .enums
        .iter()
        .map(|enumeration| (enumeration.id, enumeration))
        .collect::<HashMap<_, _>>();
    let mut pending = roots.iter().copied().collect::<VecDeque<_>>();
    let mut expanded = HashSet::new();
    let mut reachable_structs = HashSet::new();
    let mut reachable_enums = HashSet::new();

    while let Some(ty) = pending.pop_front() {
        if !expanded.insert(ty) {
            continue;
        }
        match semantics.types().kind(ty) {
            TypeKind::Error => {}
            TypeKind::StateSnapshot => {
                if let Some(state) = &syntax.state {
                    pending.extend(
                        state
                            .all_fields()
                            .filter_map(|field| semantics.value_type(field.id)),
                    );
                }
            }
            TypeKind::SettingsView => pending.extend(
                syntax
                    .settings
                    .iter()
                    .filter_map(|setting| semantics.value_type(setting.id)),
            ),
            TypeKind::Struct(structure) => {
                reachable_structs.insert(*structure);
                if let Some(structure) = structs.get(structure) {
                    pending.extend(
                        structure
                            .fields
                            .iter()
                            .filter_map(|field| semantics.struct_field_type(field.id)),
                    );
                }
            }
            TypeKind::Enum(enumeration) => {
                reachable_enums.insert(*enumeration);
                if let Some(enumeration) = enums.get(enumeration) {
                    pending.extend(
                        enumeration
                            .variants
                            .iter()
                            .filter_map(|variant| semantics.enum_variant_payload(variant.id)),
                    );
                }
            }
            TypeKind::Array { element, .. }
            | TypeKind::Set { element, .. }
            | TypeKind::Option { value: element, .. }
            | TypeKind::Result { value: element, .. }
            | TypeKind::Async { value: element, .. }
            | TypeKind::Iterator { item: element, .. }
            | TypeKind::Range { bound: element, .. } => pending.push_back(*element),
            TypeKind::Application { arguments, .. } => {
                pending.extend(arguments.iter().copied());
            }
            TypeKind::Callable {
                parameters, result, ..
            } => {
                pending.extend(parameters.iter().copied());
                pending.push_back(*result);
            }
            TypeKind::Builtin(_)
            | TypeKind::Standard(_)
            | TypeKind::ManagedClass(_)
            | TypeKind::ManagedReference(_)
            | TypeKind::GenericParameter { .. } => {}
        }
    }

    roots.extend(expanded);
    (reachable_structs, reachable_enums)
}

fn validate_must_use(
    standard_library: &StandardLibrary,
    hir: &TypedProgram,
    semantics: &SemanticModel,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for body in hir.function_bodies() {
        validate_must_use_block(
            standard_library,
            hir,
            semantics,
            &body.body,
            &mut diagnostics,
        );
    }
    for body in hir.action_bodies() {
        validate_must_use_block(
            standard_library,
            hir,
            semantics,
            &body.body,
            &mut diagnostics,
        );
    }
    diagnostics
}

fn validate_must_use_block(
    standard_library: &StandardLibrary,
    hir: &TypedProgram,
    semantics: &SemanticModel,
    block: &TypedBlock,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for statement in &block.statements {
        match &statement.kind {
            TypedStatementKind::Expression(expression_id) => {
                let expression = hir
                    .expression(*expression_id)
                    .expect("typed statement expressions belong to typed HIR");
                let callable = hir.call(*expression_id).and_then(|call| {
                    let crate::semantic::ResolvedCall::StandardLibrary { item, .. } = call else {
                        return None;
                    };
                    let item = standard_library.item(*item);
                    standard_library
                        .must_use(item.id)
                        .map(|reason| (item.qualified_name, reason))
                });
                let constructed = match semantics.types().kind(expression.ty) {
                    TypeKind::Option { .. } => Some(StdlibTypeConstructorId::Option),
                    TypeKind::Result { .. } => Some(StdlibTypeConstructorId::Result),
                    _ => None,
                }
                .and_then(|constructor| {
                    let declaration = standard_library.type_constructor(constructor);
                    declaration
                        .must_use
                        .map(|reason| (declaration.name, reason))
                })
                .or_else(|| {
                    matches!(semantics.types().kind(expression.ty), TypeKind::Async { .. })
                        .then_some((
                            "async operation",
                            "Await the future or store it for later; discarding it means the operation is never polled.",
                        ))
                });
                if let Some((name, reason)) = callable.or(constructed) {
                    diagnostics.push(
                        Diagnostic::warning(
                            DiagnosticCode::MustUse,
                            format!("unused result of `{name}`"),
                            expression.span,
                        )
                        .with_primary_label("this returned value is discarded")
                        .with_note(reason),
                    );
                }
            }
            TypedStatementKind::If {
                then_block,
                else_block,
                ..
            } => {
                validate_must_use_block(standard_library, hir, semantics, then_block, diagnostics);
                if let Some(else_block) = else_block {
                    validate_must_use_block(
                        standard_library,
                        hir,
                        semantics,
                        else_block,
                        diagnostics,
                    );
                }
            }
            TypedStatementKind::While { body, .. } | TypedStatementKind::For { body, .. } => {
                validate_must_use_block(standard_library, hir, semantics, body, diagnostics);
            }
            TypedStatementKind::Variable { .. }
            | TypedStatementKind::Assign { .. }
            | TypedStatementKind::StateAssign { .. }
            | TypedStatementKind::IndexAssign { .. }
            | TypedStatementKind::Suspend { .. }
            | TypedStatementKind::Yield { .. } => {}
        }
    }
}

const MAX_FUNCTION_INSTANCES: usize = 256;
const MAX_INSTANCE_DEPTH: usize = 64;

fn validate_function_instances(
    syntax: &Program,
    hir: &TypedProgram,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
    capabilities: &CapabilityAnalysis,
) -> Vec<Diagnostic> {
    use std::collections::{BTreeSet, HashMap};

    let mut calls = HashMap::<Option<ast::FunctionId>, Vec<_>>::new();
    for expression in hir.all_expressions() {
        let Some(call) = hir.call(expression.id) else {
            continue;
        };
        let owner = syntax
            .functions
            .iter()
            .filter(|function| {
                function.body.span.start <= expression.span.start
                    && expression.span.end <= function.body.span.end
            })
            .min_by_key(|function| function.body.span.end - function.body.span.start)
            .map(|function| function.id);
        calls
            .entry(owner)
            .or_default()
            .push((call, expression.span));
    }

    let to_instance = |call: &ResolvedCall| match call {
        ResolvedCall::UserFunction {
            function,
            type_arguments,
            signature,
        }
        | ResolvedCall::UserMethod {
            function,
            type_arguments,
            signature,
            ..
        } => Some(FunctionInstance {
            function: *function,
            type_arguments: type_arguments.clone(),
            signature: signature.clone(),
        }),
        ResolvedCall::StandardLibrary { .. }
        | ResolvedCall::ManagedSnapshot { .. }
        | ResolvedCall::ManagedComponent { .. }
        | ResolvedCall::ManagedInstances { .. }
        | ResolvedCall::ResultError { .. }
        | ResolvedCall::OptionSome { .. }
        | ResolvedCall::IteratorItem { .. }
        | ResolvedCall::ResultSuccess { .. } => None,
    };

    let mut pending = calls
        .get(&None)
        .into_iter()
        .flatten()
        .filter_map(|(call, span)| {
            to_instance(call).map(|instance| {
                let depth = usize::from(!instance.type_arguments.is_empty());
                (instance, depth, *span)
            })
        })
        .collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    let mut generic_instances = 0usize;
    let mut diagnostics = Vec::new();
    while let Some((instance, depth, span)) = pending.pop() {
        if !visited.insert(instance.clone()) {
            continue;
        }
        generic_instances += usize::from(!instance.type_arguments.is_empty());
        if generic_instances > MAX_FUNCTION_INSTANCES {
            return vec![Diagnostic::semantic(
                format!(
                    "generic function expansion exceeds the limit of {MAX_FUNCTION_INSTANCES} concrete instances"
                ),
                span,
            )];
        }
        if depth > MAX_INSTANCE_DEPTH {
            return vec![Diagnostic::semantic(
                format!(
                    "generic function expansion exceeds the recursion-depth limit of {MAX_INSTANCE_DEPTH}"
                ),
                span,
            )];
        }
        for (parameter, argument) in semantics
            .function_type_parameters(instance.function)
            .iter()
            .zip(&instance.type_arguments)
        {
            for constraint in semantics.generic_parameter_constraints(*parameter) {
                if let Err(error) = capabilities.require(*argument, *constraint, semantics) {
                    let function_name = syntax
                        .functions
                        .iter()
                        .find(|function| function.id == instance.function)
                        .map_or("function", |function| function.name.as_str());
                    let capability = standard_library.capability(*constraint);
                    diagnostics.push(capability_diagnostic(
                        format!(
                            "`{function_name}` requires capability `{}` for this specialization: {error}",
                            capability.name
                        ),
                        span,
                        *argument,
                        *constraint,
                        syntax,
                        semantics,
                        standard_library,
                        capabilities,
                    ));
                }
            }
        }
        for (call, call_span) in calls.get(&Some(instance.function)).into_iter().flatten() {
            if let Some(called) = to_instance(call) {
                let called = semantics.specialize_function_instance(&instance, &called);
                let called_depth = depth + usize::from(!called.type_arguments.is_empty());
                pending.push((called, called_depth, *call_span));
            }
        }
    }
    diagnostics
}

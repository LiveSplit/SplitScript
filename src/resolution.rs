//! Declaration collection and nominal-name resolution.
//!
//! Parsing retains nominal spellings. This module validates declarations and
//! records stable source/catalog identities beside the immutable syntax tree
//! before type checking.

use std::collections::HashMap;

use crate::{
    Diagnostic, DiagnosticFix, FixApplicability, TextEdit,
    ast::{
        EnumReference, EnumVariantId, ExprId, ExprKind, MatchPattern, PatternId, Program,
        SettingKind, Span, StateProviderRef, TypeApplicationId, TypeNameId, TypeRef, ValueId,
    },
    migration::{legacy_type_diagnostic, migration_diagnostic},
    name_matching::closest_single_typo,
    stdlib::{StandardLibrary, StateProviderProcesses, StdlibStateProviderId, StdlibTypeKind},
    types::{EnumTypeId, ResolvedTypeRef},
    visit::{self, Visitor},
};

/// Catalog identities resolved from one parsed program before type checking.
///
/// These facts deliberately live beside the immutable syntax tree. Syntax
/// retains the names and spans the author wrote, while later compiler stages
/// consume stable catalog identities from this table.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProgramResolutions {
    state_provider: Option<StdlibStateProviderId>,
    state_provider_selector: Option<usize>,
    state_provider_alternatives: HashMap<EnumVariantId, ResolvedStateProviderAlternative>,
    type_names: HashMap<TypeNameId, ResolvedTypeRef>,
    type_applications: HashMap<TypeApplicationId, ResolvedTypeRef>,
    managed_references: HashMap<crate::ast::ManagedReferenceTypeId, ResolvedTypeRef>,
    expression_enums: HashMap<ExprId, EnumTypeId>,
    pattern_enums: HashMap<PatternId, EnumTypeId>,
    setting_enums: HashMap<ValueId, EnumTypeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedStateProviderAlternative {
    pub provider: StdlibStateProviderId,
    pub selector: Option<usize>,
}

impl ProgramResolutions {
    pub(crate) fn state_provider(&self) -> Option<StdlibStateProviderId> {
        self.state_provider
    }

    pub(crate) fn state_provider_selector(&self) -> Option<usize> {
        self.state_provider_selector
    }

    pub(crate) fn state_provider_alternative(
        &self,
        variant: EnumVariantId,
    ) -> Option<ResolvedStateProviderAlternative> {
        self.state_provider_alternatives.get(&variant).copied()
    }

    pub(crate) fn state_provider_alternatives(
        &self,
    ) -> impl Iterator<Item = (EnumVariantId, ResolvedStateProviderAlternative)> + '_ {
        self.state_provider_alternatives
            .iter()
            .map(|(variant, provider)| (*variant, *provider))
    }

    pub(crate) fn state_providers(&self) -> impl Iterator<Item = StdlibStateProviderId> + '_ {
        self.state_provider.into_iter().chain(
            self.state_provider_alternatives
                .values()
                .map(|alternative| alternative.provider),
        )
    }

    pub(crate) fn type_ref(&self, ty: TypeRef) -> Option<ResolvedTypeRef> {
        match ty {
            TypeRef::Named(name) => self.type_names.get(&name).copied(),
            TypeRef::Core(core) => Some(ResolvedTypeRef::Core(core)),
            TypeRef::Array(id) => Some(ResolvedTypeRef::Array(id)),
            TypeRef::Option(id) => Some(ResolvedTypeRef::Option(id)),
            TypeRef::Result(id) => Some(ResolvedTypeRef::Result(id)),
            TypeRef::Async(id) => Some(ResolvedTypeRef::Async(id)),
            TypeRef::Iterator(id) => Some(ResolvedTypeRef::Iterator(id)),
            TypeRef::Callable(id) => Some(ResolvedTypeRef::Callable(id)),
            TypeRef::Range(id) => Some(ResolvedTypeRef::Range(id)),
            TypeRef::Application(id) => self.type_applications.get(&id).copied(),
            TypeRef::ManagedReference(id) => self.managed_references.get(&id).copied(),
        }
    }

    pub(crate) fn expression_enum(&self, expression: ExprId) -> Option<EnumTypeId> {
        self.expression_enums.get(&expression).copied()
    }

    pub(crate) fn pattern_enum(&self, pattern: PatternId) -> Option<EnumTypeId> {
        self.pattern_enums.get(&pattern).copied()
    }

    pub(crate) fn setting_enum(&self, setting: ValueId) -> Option<EnumTypeId> {
        self.setting_enums.get(&setting).copied()
    }
}

/// Validates nominal declarations after syntax construction. Keeping these
/// diagnostics out of token collection is the first enforceable boundary
/// between parsing and resolution.
pub(crate) fn validate_declarations(
    program: &Program,
    standard_library: &StandardLibrary,
) -> Vec<Diagnostic> {
    let core_names = standard_library
        .core_types()
        .iter()
        .map(|ty| ty.name)
        .chain(["Address"])
        .collect::<std::collections::HashSet<_>>();
    let mut declared = HashMap::<&str, Span>::new();
    let mut diagnostics = Vec::new();
    let managed_classes = program.managed_class_declarations();

    for function in &program.functions {
        if function
            .name
            .starts_with(crate::stdlib::RESERVED_FUNCTION_PREFIX)
        {
            diagnostics.push(Diagnostic::type_error(
                "function names beginning with `__splitscript_stdlib_` are reserved",
                function.span,
            ));
        }
    }

    for (kind, name, span) in program
        .structs
        .iter()
        .map(|decl| ("struct", decl.name.as_str(), decl.name_span))
        .chain(
            program
                .enums
                .iter()
                .map(|decl| ("enum", decl.name.as_str(), decl.name_span)),
        )
        .chain(
            managed_classes
                .iter()
                .map(|decl| ("managed class", decl.name.as_str(), decl.name_span)),
        )
    {
        if let Some(first) = declared.insert(name, span) {
            diagnostics.push(
                Diagnostic::type_error(format!("duplicate named type `{name}`"), span)
                    .with_secondary_label(first, "the first declaration is here"),
            );
        }
        if core_names.contains(name) {
            diagnostics.push(Diagnostic::type_error(
                format!("`{name}` is a core type and cannot be redeclared as a {kind}"),
                span,
            ));
        }
        if let Some(ty) = standard_library.type_by_name(name) {
            let standard_kind = if ty.kind == StdlibTypeKind::Enum {
                "enum"
            } else {
                "type"
            };
            diagnostics.push(Diagnostic::type_error(
                format!(
                    "`{name}` is a standard-library {standard_kind} and cannot be redeclared as a {kind}"
                ),
                span,
            ));
        }
    }

    let applied_constructor_occurrences = program
        .type_applications
        .iter()
        .flat_map(|application| {
            application
                .occurrences
                .iter()
                .map(|occurrence| occurrence.constructor)
        })
        .collect::<std::collections::HashSet<_>>();

    for (name, span) in program.type_names.iter().zip(&program.type_name_spans) {
        if standard_library.type_by_name(name).is_none()
            && standard_library
                .named_type_constructor_by_name(name)
                .is_none()
            && !(matches!(name.as_str(), "Array" | "Option" | "Result")
                && applied_constructor_occurrences.contains(span))
            && !program
                .structs
                .iter()
                .any(|structure| structure.name == *name)
            && !program
                .enum_declarations()
                .any(|enumeration| enumeration.name == *name)
            && !managed_classes.iter().any(|class| class.name == *name)
        {
            if let Some(diagnostic) = legacy_type_migration_diagnostic(name, *span) {
                diagnostics.push(diagnostic);
            } else {
                diagnostics.push(Diagnostic::type_error(
                    format!("unknown type `{name}`"),
                    *span,
                ));
            }
        }
    }

    for (id, name, _) in program.type_names() {
        if standard_library
            .named_type_constructor_by_name(name)
            .is_none()
        {
            continue;
        }
        for span in program.type_name_occurrences(id) {
            if !applied_constructor_occurrences.contains(span) {
                diagnostics.push(Diagnostic::type_error(
                    format!("`{name}` is a generic type constructor; provide its type arguments as `{name}<...>`"),
                    *span,
                ));
            }
        }
    }

    diagnostics
}

/// Resolves syntax whose grammatical shape is independent of its nominal
/// meaning. This is intentionally run by `lower`, never by the parser.
pub(crate) fn resolve_program(
    program: &Program,
    standard_library: &StandardLibrary,
    resolutions: &mut ProgramResolutions,
) -> Vec<Diagnostic> {
    let mut provider_diagnostics = Vec::new();
    if let Some(state) = &program.state {
        if !state.provider_alternatives.is_empty() {
            let mut seen = HashMap::new();
            for alternative in &state.provider_alternatives {
                if let Some(resolved) = resolve_state_provider_reference(
                    &alternative.provider,
                    &alternative.processes,
                    standard_library,
                    &mut provider_diagnostics,
                ) {
                    if standard_library
                        .state_provider_preparation(resolved.provider, resolved.selector)
                        .is_some()
                        || !standard_library
                            .state_provider(resolved.provider)
                            .contexts
                            .is_empty()
                    {
                        provider_diagnostics.push(
                            Diagnostic::type_error(
                                format!(
                                    "state provider `{}` requires its own attachment preparation context",
                                    alternative.provider.name
                                ),
                                alternative.provider.span,
                            )
                            .with_primary_label(
                                "multi-provider state alternatives must read directly from the attached process",
                            ),
                        );
                    }
                    if let Some(previous) = seen.insert(resolved.provider, alternative.span) {
                        provider_diagnostics.push(
                            Diagnostic::type_error(
                                format!(
                                    "state provider `{}` is listed more than once",
                                    alternative.provider.name
                                ),
                                alternative.provider.span,
                            )
                            .with_secondary_label(previous, "the first alternative is here"),
                        );
                    }
                    resolutions
                        .state_provider_alternatives
                        .insert(alternative.variant, resolved);
                }
            }
        } else if let Some(reference) = &state.provider {
            if let Some(resolved) = resolve_state_provider_reference(
                reference,
                &state.processes,
                standard_library,
                &mut provider_diagnostics,
            ) {
                resolutions.state_provider = Some(resolved.provider);
                resolutions.state_provider_selector = resolved.selector;
            }
        } else {
            resolutions.state_provider = standard_library
                .default_state_provider()
                .map(|provider| provider.id);
        }
    }

    let mut enums = program
        .enum_declarations()
        .map(|enumeration| (enumeration.name.clone(), EnumTypeId::Source(enumeration.id)))
        .collect::<HashMap<_, _>>();
    for ty in standard_library.all_types() {
        if ty.kind == StdlibTypeKind::Enum {
            enums
                .entry(ty.name.to_owned())
                .or_insert(EnumTypeId::Standard(ty.id));
        }
    }

    let type_names = program
        .type_names()
        .filter_map(|(id, name, _)| {
            let resolved = if let Some(structure) =
                program.structs.iter().find(|item| item.name == *name)
            {
                ResolvedTypeRef::Struct(structure.id)
            } else if let Some(enumeration) =
                program.enum_declarations().find(|item| item.name == *name)
            {
                ResolvedTypeRef::Enum(enumeration.id)
            } else if let Some(class) = program
                .managed_class_declarations()
                .into_iter()
                .find(|item| item.name == *name)
            {
                ResolvedTypeRef::ManagedClass(class.id)
            } else {
                ResolvedTypeRef::Standard(standard_library.type_by_name_including_private(name)?.id)
            };
            Some((id, resolved))
        })
        .collect::<HashMap<_, _>>();
    resolutions.type_names = type_names;

    for reference in &program.managed_reference_types {
        let class_name = program.type_name(reference.class);
        let Some(ResolvedTypeRef::ManagedClass(class)) =
            resolutions.type_names.get(&reference.class).copied()
        else {
            provider_diagnostics.push(
                Diagnostic::type_error(
                    format!("`{class_name}.Ref` requires a managed class schema"),
                    program.type_name_span(reference.class),
                )
                .with_primary_label("this name does not resolve to a class declared in an `image`")
                .with_note("declare the class in a managed image schema before using its live reference type"),
            );
            continue;
        };
        resolutions
            .managed_references
            .insert(reference.id, ResolvedTypeRef::ManagedReference(class));
    }

    for application in &program.type_applications {
        let name = program.type_name(application.constructor);
        let Some(constructor) =
            standard_library.named_type_constructor_by_name_including_private(name)
        else {
            if let Some((canonical, opening, closing)) = match name {
                "Array" => Some(("[T]", "[", "]")),
                "Option" => Some(("T?", "", "?")),
                "Result" => Some(("T!", "", "!")),
                _ => None,
            } {
                let edits = application
                    .occurrences
                    .iter()
                    .flat_map(|occurrence| {
                        [
                            TextEdit {
                                span: Span {
                                    start: occurrence.constructor.start,
                                    end: occurrence.opening.end,
                                },
                                replacement: opening.to_owned(),
                            },
                            TextEdit {
                                span: occurrence.closing,
                                replacement: closing.to_owned(),
                            },
                        ]
                    })
                    .collect();
                provider_diagnostics.push(
                    Diagnostic::type_error(
                        format!("`{name}<T>` is not SplitScript type syntax"),
                        program.type_name_span(application.constructor),
                    )
                    .with_primary_label(format!("write `{canonical}` instead"))
                    .with_fix(DiagnosticFix {
                        title: format!("rewrite using `{canonical}` syntax"),
                        applicability: FixApplicability::MachineApplicable,
                        edits,
                    }),
                );
                continue;
            }
            let is_undeclared_legacy_type = legacy_type_diagnostic(name).is_some()
                && !program
                    .structs
                    .iter()
                    .any(|structure| structure.name == name)
                && !program
                    .enums
                    .iter()
                    .any(|enumeration| enumeration.name == name)
                && !program
                    .managed_class_declarations()
                    .iter()
                    .any(|class| class.name == name);
            if !is_undeclared_legacy_type {
                provider_diagnostics.push(Diagnostic::type_error(
                    format!("unknown generic type constructor `{name}`"),
                    program.type_name_span(application.constructor),
                ));
            }
            continue;
        };
        if application.arguments.len() != constructor.parameters.len() {
            provider_diagnostics.push(Diagnostic::type_error(
                format!(
                    "`{name}` expects {} type argument{}, found {}",
                    constructor.parameters.len(),
                    if constructor.parameters.len() == 1 {
                        ""
                    } else {
                        "s"
                    },
                    application.arguments.len(),
                ),
                program.type_name_span(application.constructor),
            ));
            continue;
        }
        let resolved = if constructor.id == crate::stdlib::StdlibTypeConstructorId::Set {
            ResolvedTypeRef::Set(application.id)
        } else {
            ResolvedTypeRef::Application(application.id)
        };
        resolutions
            .type_applications
            .insert(application.id, resolved);
    }

    let mut resolver = EnumResolver {
        enums: &enums,
        resolutions,
        diagnostics: Vec::new(),
    };
    resolver.visit_program(program);
    provider_diagnostics.extend(resolver.diagnostics);
    for selector in program
        .state
        .iter()
        .flat_map(|state| {
            state.provider.iter().chain(
                state
                    .provider_alternatives
                    .iter()
                    .map(|alternative| &alternative.provider),
            )
        })
        .filter_map(|provider| provider.selector.as_ref())
    {
        for argument in &selector.arguments {
            if !crate::constant::is_constant(argument, resolutions, program, standard_library) {
                provider_diagnostics.push(
                    Diagnostic::type_error(
                        "state-provider configuration must be a compile-time constant",
                        argument.span,
                    )
                    .with_primary_label(
                        "use constant values or a constructor function that only returns a constant expression",
                    ),
                );
            }
        }
    }
    provider_diagnostics
}

fn resolve_state_provider_reference(
    reference: &StateProviderRef,
    processes: &[String],
    standard_library: &StandardLibrary,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ResolvedStateProviderAlternative> {
    let Some(provider) = standard_library.state_provider_by_name(&reference.name) else {
        let mut provider_names = standard_library
            .state_providers()
            .iter()
            .map(|provider| provider.name)
            .collect::<Vec<_>>();
        provider_names.sort_unstable_by_key(|name| name.to_ascii_lowercase());
        let suggestion = closest_single_typo(&reference.name, provider_names.iter().copied());
        let message = suggestion.as_ref().map_or_else(
            || format!("unknown state provider `{}`", reference.name),
            |suggestion| {
                format!(
                    "unknown state provider `{}`; did you mean `{suggestion}`?",
                    reference.name
                )
            },
        );
        let valid = provider_names
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut diagnostic = Diagnostic::type_error(message, reference.span)
            .with_note(format!("valid state providers are {valid}"))
            .with_documentation_uri(crate::documentation::STATE_PROVIDER_INDEX_URI);
        if let Some(suggestion) = suggestion {
            diagnostic = diagnostic.with_machine_applicable_fix(
                format!("replace with `{suggestion}`"),
                reference.span,
                suggestion,
            );
        }
        diagnostics.push(diagnostic);
        return None;
    };

    let selector = reference.selector.as_ref().and_then(|reference| {
        let Some((index, selector)) = provider
            .selectors
            .iter()
            .enumerate()
            .find(|(_, selector)| selector.name == reference.name)
        else {
            diagnostics.push(Diagnostic::type_error(
                format!(
                    "state provider `{}` has no selector `{}`",
                    provider.name, reference.name
                ),
                reference.name_span,
            ));
            return None;
        };
        if reference.arguments.len() != selector.parameters.len() {
            diagnostics.push(Diagnostic::type_error(
                format!(
                    "state-provider selector `{}.{}` expects {} argument{}, found {}",
                    provider.name,
                    selector.name,
                    selector.parameters.len(),
                    if selector.parameters.len() == 1 {
                        ""
                    } else {
                        "s"
                    },
                    reference.arguments.len(),
                ),
                reference.span,
            ));
        }
        Some(index)
    });

    match provider.processes {
        StateProviderProcesses::SourceState if processes.is_empty() => diagnostics.push(
            Diagnostic::type_error(
                format!(
                    "state provider `{}` needs at least one process name",
                    reference.name
                ),
                reference.span,
            )
            .with_primary_label(
                "add an exact process name after the provider, such as `[\"game.exe\"]`",
            ),
        ),
        StateProviderProcesses::Declared(_) if !processes.is_empty() => diagnostics.push(
            Diagnostic::type_error(
                format!(
                    "state provider `{}` declares its supported processes",
                    reference.name
                ),
                reference.span,
            )
            .with_primary_label("remove the process name list after this provider"),
        ),
        StateProviderProcesses::SourceState | StateProviderProcesses::Declared(_) => {}
    }

    Some(ResolvedStateProviderAlternative {
        provider: provider.id,
        selector,
    })
}

fn legacy_type_migration_diagnostic(name: &str, span: Span) -> Option<Diagnostic> {
    let id = legacy_type_diagnostic(name)?;
    let metadata = migration_diagnostic(id)
        .expect("resolution migration diagnostic IDs must exist in the migration catalog");
    let mut diagnostic = Diagnostic::type_error(metadata.message, span)
        .with_primary_label(metadata.primary_label)
        .with_migration_topic(metadata.concept.as_str());
    for note in metadata.notes {
        diagnostic = diagnostic.with_note(*note);
    }
    Some(diagnostic)
}

struct EnumResolver<'a> {
    enums: &'a HashMap<String, EnumTypeId>,
    resolutions: &'a mut ProgramResolutions,
    diagnostics: Vec<Diagnostic>,
}

impl EnumResolver<'_> {
    fn resolve_reference(
        &mut self,
        reference: &EnumReference,
        source_only: bool,
    ) -> Option<EnumTypeId> {
        let EnumReference { name, span } = reference;
        let Some(enumeration) = self.enums.get(name).copied() else {
            self.diagnostics.push(
                legacy_type_migration_diagnostic(name, *span).unwrap_or_else(|| {
                    Diagnostic::type_error(format!("unknown enum `{name}`"), *span)
                }),
            );
            return None;
        };
        if source_only && matches!(enumeration, EnumTypeId::Standard(_)) {
            self.diagnostics.push(Diagnostic::type_error(
                format!("choice settings require a source enum, found `{name}`"),
                *span,
            ));
            return None;
        }
        Some(enumeration)
    }
}

impl<'ast> Visitor<'ast> for EnumResolver<'_> {
    fn visit_parameter(&mut self, parameter: &'ast crate::ast::Parameter) {
        self.resolve_pattern(
            &parameter.binding.pattern.kind,
            parameter.binding.pattern.id,
        );
        visit::walk_parameter(self, parameter);
    }

    fn visit_variable(&mut self, variable: &'ast crate::ast::VariableDecl) {
        self.resolve_pattern(&variable.binding.pattern.kind, variable.binding.pattern.id);
        visit::walk_variable(self, variable);
    }

    fn visit_for_binding(&mut self, binding: &'ast crate::ast::ForBinding) {
        self.resolve_pattern(&binding.binding.pattern.kind, binding.binding.pattern.id);
    }

    fn visit_setting(&mut self, setting: &'ast crate::ast::SettingDecl) {
        if let SettingKind::Choice { enumeration, .. } = &setting.kind
            && let Some(enumeration) = self.resolve_reference(enumeration, true)
        {
            self.resolutions
                .setting_enums
                .insert(setting.id, enumeration);
        }
    }

    fn visit_expr(&mut self, expression: &'ast crate::ast::Expr) {
        if let ExprKind::Is { pattern, .. } = &expression.kind {
            self.resolve_pattern(&pattern.kind, pattern.id);
        }
        let enumeration = match &expression.kind {
            ExprKind::Path(path) => {
                let Some((_, prefix)) = path.split_last() else {
                    visit::walk_expr(self, expression);
                    return;
                };
                self.enums.get(&prefix.join(".")).copied()
            }
            ExprKind::Call { callee, args, .. } => {
                let Some((_, prefix)) = callee.split_last() else {
                    visit::walk_expr(self, expression);
                    return;
                };
                self.enums.get(&prefix.join(".")).copied().inspect(|_| {
                    if args.len() > 1 {
                        self.diagnostics.push(Diagnostic::type_error(
                            "enum constructors accept at most one payload",
                            expression.span,
                        ));
                    }
                })
            }
            _ => None,
        };
        if let Some(enumeration) = enumeration {
            self.resolutions
                .expression_enums
                .insert(expression.id, enumeration);
        }
        visit::walk_expr(self, expression);
    }

    fn visit_match_arm(&mut self, arm: &'ast crate::ast::MatchArm) {
        self.resolve_pattern(&arm.pattern, arm.pattern_id);
        visit::walk_match_arm(self, arm);
    }
}

impl EnumResolver<'_> {
    fn resolve_pattern(&mut self, pattern: &MatchPattern, id: crate::ast::PatternId) {
        match pattern {
            MatchPattern::Struct { fields, .. } => {
                for field in fields {
                    self.resolve_pattern(&field.pattern.kind, field.pattern.id);
                }
            }
            MatchPattern::Enum { enumeration, .. } => {
                if let Some(enumeration) = self.resolve_reference(enumeration, false) {
                    self.resolutions.pattern_enums.insert(id, enumeration);
                }
                if let MatchPattern::Enum {
                    payload: Some(payload),
                    ..
                } = pattern
                {
                    self.resolve_pattern(&payload.kind, payload.id);
                }
            }
            MatchPattern::OptionSome(payload)
            | MatchPattern::IteratorItem(payload)
            | MatchPattern::ResultSuccess(payload)
            | MatchPattern::ResultError(payload) => {
                self.resolve_pattern(&payload.kind, payload.id);
            }
            MatchPattern::Array(array) => {
                for element in array.elements() {
                    self.resolve_pattern(&element.kind, element.id);
                }
            }
            MatchPattern::Alternation(elements) => {
                for element in elements {
                    self.resolve_pattern(&element.kind, element.id);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{ast::TypeRef, lower, parse};

    #[test]
    fn resolves_nominal_cast_targets_after_parsing() {
        let source = r#"
            state "game.exe" {}
            whileAttached { print(42 as String) }
        "#;
        let lowered = lower(parse(source).unwrap());
        let cast = lowered
            .syntax()
            .actions
            .iter()
            .flat_map(|action| &action.body.statements)
            .find_map(|statement| {
                let crate::ast::Stmt::Expression(expression) = statement else {
                    return None;
                };
                let crate::ast::ExprKind::Call { args, .. } = &expression.kind else {
                    return None;
                };
                let crate::ast::ExprKind::Cast { target, .. } = &args.first()?.kind else {
                    return None;
                };
                Some(*target)
            })
            .expect("the print argument should contain the cast");
        let TypeRef::Named(name) = cast else {
            panic!("lowering should preserve the source-written nominal type");
        };
        assert_eq!(lowered.syntax().type_name(name), "String");
    }
}

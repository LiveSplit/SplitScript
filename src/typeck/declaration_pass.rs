//! Source declaration and signature collection before body checking.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{Program, SettingDecl, SettingKind, Span, StateMemoryDecoder, StateSource},
    inference::{Requirements, Type},
    intrinsic_registry::{MAX_NATIVE_STRING_BYTES, MAX_NATIVE_UTF16_UNITS},
    stdlib::{CoreTypeId, StdlibCapabilityId, StdlibTypeId},
    stdlib_semantic::StandardLibrarySemanticExt,
    types::EnumTypeId,
};

use super::{
    Checker,
    control_flow::contains_value_return,
    declarations::{Binding, FunctionSignature, RuntimeSettingDeclaration, RuntimeSettingKind},
};

pub(super) fn collect(checker: &mut Checker, program: &Program) {
    collect_state_fields(checker, program);
    collect_settings(checker, program);
    collect_named_type_members(checker, program);
    collect_function_signatures(checker, program);
}

/// Collects declarations whose shape depends on checked source expressions.
///
/// This deliberately runs after global initializers have established their
/// bindings and types. Conditional schema declarations can therefore use an
/// ordinary enum global as their discriminator without introducing a second,
/// shape-specific name-resolution path.
pub(super) fn collect_conditional_fields(checker: &mut Checker, program: &Program) {
    collect_conditional_state_fields(checker, program);
    collect_conditional_managed_fields(checker, program);
}

fn collect_state_fields(checker: &mut Checker, program: &Program) {
    let Some(state) = program.state.as_ref() else {
        return;
    };
    let provider = checker
        .provider_value
        .map(|(provider, _)| checker.standard_library.state_provider(provider));
    if !state.has_provider_alternatives() {
        for field in &state.fields {
            let ty = collect_state_field_type(checker, field, provider);
            checker.semantics.resolve_value_type(field.id, ty);
            checker.declarations.state_fields_by_id.insert(field.id, ty);
            checker
                .declarations
                .state_field_spans
                .insert(field.id, field.span);
            checker
                .declarations
                .state_storage_fields
                .insert(field.id, field.id);
            if checker
                .declarations
                .state_fields
                .insert(field.name.clone(), (field.id, ty))
                .is_some()
            {
                checker.error(
                    format!("duplicate state field `{}`", field.name),
                    field.span,
                );
            }
        }
    } else {
        // First collect every provider alternative independently. Alternatives
        // may omit names or use the same name with a different type.
        for (variant, variant_fields) in state.provider_variant_fields() {
            let variant_provider = checker
                .resolutions
                .state_provider_alternative(variant)
                .map(|alternative| {
                    checker
                        .standard_library
                        .state_provider(alternative.provider)
                })
                .or(provider);
            let mut fields = HashMap::new();
            for field in variant_fields {
                let ty = collect_state_field_type(checker, field, variant_provider);
                if let Some(alternative) = checker.resolutions.state_provider_alternative(variant) {
                    checker
                        .semantics
                        .resolve_state_field_provider(field.id, alternative.provider);
                }
                checker.semantics.resolve_value_type(field.id, ty);
                checker.declarations.state_fields_by_id.insert(field.id, ty);
                checker
                    .declarations
                    .state_field_spans
                    .insert(field.id, field.span);
                if fields.insert(field.name.clone(), (field.id, ty)).is_some() {
                    checker.error(
                        format!(
                            "duplicate state field `{}` in this provider alternative",
                            field.name
                        ),
                        field.span,
                    );
                }
            }
            checker
                .declarations
                .provider_state_fields
                .insert(variant, fields);
        }

        // A name becomes part of StateSnapshot's common interface only when
        // every provider alternative declares it and explicit annotations do
        // not conflict.
        // Unannotated declarations still participate in bidirectional
        // inference by unifying with the canonical declaration.
        let first = state.canonical_provider_fields();
        for field in first {
            let declarations = state
                .provider_variant_fields()
                .map(|(_, fields)| fields.iter().find(|item| item.name == field.name))
                .collect::<Option<Vec<_>>>();
            let is_common = state.is_common_field(&field.name);
            if is_common {
                let canonical_ty = checker.declarations.state_fields_by_id[&field.id];
                checker
                    .declarations
                    .state_fields
                    .insert(field.name.clone(), (field.id, canonical_ty));
                for declaration in declarations.unwrap() {
                    let ty = checker.declarations.state_fields_by_id[&declaration.id];
                    if declaration.id != field.id {
                        let canonical_name = checker.type_name(canonical_ty);
                        checker.with_expected_type_source(
                            super::ExpectedTypeSource {
                                span: field.span,
                                label: format!(
                                    "the first provider alternative declares `{}` as `{canonical_name}`",
                                    field.name
                                ),
                            },
                            |checker| {
                                checker.unify_expected(ty, canonical_ty, declaration.span);
                            },
                        );
                    }
                    checker
                        .declarations
                        .state_storage_fields
                        .insert(declaration.id, field.id);
                }
            }
        }

        // Provider subsets may also share one physical snapshot slot. This is
        // what lets an or-pattern such as `StateProvider.Windows |
        // StateProvider.GBA` retain a field that has the same name and type in
        // both alternatives even when a third omits it. Conflicting types
        // remain independent.
        let mut compatible_storage = HashMap::<(String, Type), crate::ast::ValueId>::new();
        for (_, fields) in state.provider_variant_fields() {
            for field in fields {
                let ty = checker.declarations.state_fields_by_id[&field.id];
                let key = (field.name.clone(), ty);
                if let Some(storage) = checker
                    .declarations
                    .state_storage_fields
                    .get(&field.id)
                    .copied()
                {
                    compatible_storage.entry(key).or_insert(storage);
                } else {
                    let storage = *compatible_storage.entry(key).or_insert(field.id);
                    checker
                        .declarations
                        .state_storage_fields
                        .insert(field.id, storage);
                }
            }
        }
    }

    if let Some(provider_value) = state.provider_value {
        let ty = if let Some(provider_enum) = &state.provider_enum {
            checker.enum_type(EnumTypeId::Source(provider_enum.id))
        } else {
            unreachable!("a provider discriminant has a generated enum type")
        };
        checker.semantics.resolve_value_type(provider_value, ty);
        checker.declarations.globals.insert(
            state
                .refinement_value_name()
                .expect("a refinement value has a source name")
                .to_owned(),
            Binding {
                id: Some(provider_value),
                ty,
                mutable: false,
                debug_only: false,
                declaration_span: None,
            },
        );
    }

    if !state.has_provider_alternatives() {
        // Conditional fields are collected after globals have been checked;
        // that later phase finalizes the physical snapshot layout as well.
        return;
    }

    let storage_fields = {
        let mut fields = state
            .canonical_provider_fields()
            .iter()
            .filter(|field| {
                checker
                    .declarations
                    .state_fields
                    .get(&field.name)
                    .is_some_and(|(canonical, _)| *canonical == field.id)
            })
            .map(|field| field.id)
            .collect::<Vec<_>>();
        let common = fields.iter().copied().collect::<HashSet<_>>();
        fields.extend(state.all_fields().filter_map(|field| {
            (checker.declarations.state_storage_fields[&field.id] == field.id
                && !common.contains(&field.id))
            .then_some(field.id)
        }));
        fields
    };
    let provider_fields = state
        .provider_variant_fields()
        .map(|(variant, fields)| (variant, fields.iter().map(|field| field.id).collect()))
        .collect();
    checker.semantics.resolve_state_storage(
        storage_fields,
        checker.declarations.state_storage_fields.clone(),
        provider_fields,
    );
}

fn collect_conditional_state_fields(checker: &mut Checker, program: &Program) {
    let Some(state) = program.state.as_ref() else {
        return;
    };
    if state.has_provider_alternatives() {
        return;
    }
    let provider = checker
        .provider_value
        .map(|(provider, _)| checker.standard_library.state_provider(provider));
    let predicates = checker.shape_branch_predicates(&state.conditional_fields);
    for (group, predicate) in state.conditional_fields.iter().zip(predicates) {
        let mut names = state
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect::<HashSet<_>>();
        for field in &group.fields {
            let ty = collect_state_field_type(checker, field, provider);
            checker.semantics.resolve_value_type(field.id, ty);
            checker.declarations.state_fields_by_id.insert(field.id, ty);
            checker
                .declarations
                .state_field_spans
                .insert(field.id, field.span);
            checker
                .declarations
                .state_storage_fields
                .insert(field.id, field.id);
            checker
                .declarations
                .conditional_state_fields
                .entry(field.name.clone())
                .or_default()
                .push((field.id, ty, predicate.clone()));
            checker
                .declarations
                .conditional_state_field_predicates
                .insert(field.id, predicate.clone());
            for dependency in predicate
                .alternatives
                .iter()
                .flatten()
                .filter_map(|constraint| match constraint.dimension {
                    super::declarations::ShapeDimension::StateField(value) => Some(value),
                    super::declarations::ShapeDimension::Global(_) => None,
                })
            {
                checker
                    .semantics
                    .resolve_state_dependency(field.id, dependency);
            }
            checker
                .semantics
                .resolve_conditional_state_field(field.id, resolved_shape_predicate(&predicate));
            if !names.insert(field.name.clone()) {
                checker.error(
                    format!("duplicate conditional state field `{}`", field.name),
                    field.span,
                );
            }
        }
    }
    for (name, declarations) in checker.declarations.conditional_state_fields.clone() {
        if declarations.is_empty()
            || !checker
                .shape_predicates_cover_all(declarations.iter().map(|(_, _, predicate)| predicate))
        {
            continue;
        }
        let (canonical, canonical_ty, _) = declarations[0].clone();
        let compatible = declarations
            .iter()
            .skip(1)
            .all(|(_, ty, _)| checker.inference.unify(*ty, canonical_ty).is_ok());
        if !compatible {
            continue;
        }
        checker
            .declarations
            .state_fields
            .insert(name, (canonical, canonical_ty));
        for (field, _, _) in declarations {
            checker
                .declarations
                .state_storage_fields
                .insert(field, canonical);
        }
    }
    let storage_fields = state
        .all_fields()
        .filter_map(|field| {
            (checker.declarations.state_storage_fields[&field.id] == field.id).then_some(field.id)
        })
        .collect();
    checker.semantics.resolve_state_storage(
        storage_fields,
        checker.declarations.state_storage_fields.clone(),
        HashMap::new(),
    );
}

fn collect_state_field_type(
    checker: &mut Checker,
    field: &crate::ast::StateField,
    provider: Option<&crate::stdlib::StdlibStateProvider>,
) -> Type {
    let ty = if let Some(annotation) = field.annotation {
        checker.syntax_type(annotation)
    } else {
        checker.fresh_inference(Requirements::none(), None)
    };
    if let Some(standard) = checker.standard_type_id(ty) {
        let declaration = checker.standard_library.type_decl(standard);
        if !declaration.value_usage.state_field {
            checker.error(
                format!("{} cannot be stored in a state field", declaration.name),
                field.span,
            );
        }
    }
    if let StateSource::Pointer(path) = &field.source {
        // An explicitly optional pointer field observes read failure as
        // `None`. The pointer still reads the contained representation; the
        // outer Option belongs to snapshot semantics rather than process
        // memory layout.
        let memory_ty = if matches!(field.annotation, Some(crate::ast::TypeRef::Option(_))) {
            match ty {
                Type::Option(option) => checker.inference.option_value(option),
                Type::Known(ty) => match checker.inference.type_store().kind(ty) {
                    crate::types::TypeKind::Option { value, .. } => Type::Known(*value),
                    _ => Type::Known(ty),
                },
                _ => ty,
            }
        } else {
            ty
        };
        if let Some(decoder) = path.decoder {
            let string = checker.standard_type(StdlibTypeId::String);
            checker.unify(memory_ty, string, field.span);
            match decoder {
                StateMemoryDecoder::Utf8 { max_bytes, span } => {
                    if max_bytes == 0 {
                        checker.error("a UTF-8 state read must allow at least one byte", span);
                    } else if max_bytes > MAX_NATIVE_STRING_BYTES {
                        checker.error(
                            format!(
                                "a UTF-8 state read is limited to {MAX_NATIVE_STRING_BYTES} bytes"
                            ),
                            span,
                        );
                    }
                }
                StateMemoryDecoder::Utf16Le { max_units, span } => {
                    if max_units == 0 {
                        checker.error(
                            "a UTF-16LE state read must allow at least one code unit",
                            span,
                        );
                    } else if max_units > MAX_NATIVE_UTF16_UNITS {
                        checker.error(
                            format!(
                                "a UTF-16LE state read is limited to {MAX_NATIVE_UTF16_UNITS} code units"
                            ),
                            span,
                        );
                    }
                }
            }
        } else {
            checker.require(
                memory_ty,
                Requirements::capability(StdlibCapabilityId::MemoryReadable),
                field.span,
            );
        }
        if let Some(provider) = provider
            && checker
                .standard_library
                .item(provider.direct_read)
                .signature
                .parameters[0]
                .ty
                == crate::stdlib::TypeRef::Core(CoreTypeId::U32)
        {
            if path.decoder.is_some() {
                checker.error(
                    format!(
                        "`state {}` does not yet support decoded string fields",
                        provider.name
                    ),
                    field.span,
                );
            }
            if matches!(path.base, crate::ast::PointerPathBase::Module { .. }) {
                checker.error(
                    format!(
                        "`state {}` direct reads use hardware addresses and cannot name a module",
                        provider.name
                    ),
                    field.span,
                );
            }
            if matches!(path.base, crate::ast::PointerPathBase::Absolute(address) if address > u32::MAX.into())
            {
                checker.error(
                    format!("`state {}` addresses must fit in `u32`", provider.name),
                    field.span,
                );
            }
            if path
                .offsets
                .iter()
                .any(|offset| *offset < i32::MIN.into() || *offset > u32::MAX.into())
            {
                checker.error(
                    format!(
                        "`state {}` pointer offsets must fit in 32 bits",
                        provider.name
                    ),
                    field.span,
                );
            }
        }
    }
    ty
}

fn collect_settings(checker: &mut Checker, program: &Program) {
    let mut runtime_keys = HashMap::<String, Span>::new();
    for family in &program.setting_families {
        checker
            .semantics
            .resolve_value_type(family.binding_id, checker.core_type(CoreTypeId::U32));
    }
    for setting in &program.settings {
        let runtime_key = setting.runtime_key();
        let key_span = setting
            .external_key
            .as_ref()
            .map_or(setting.span, crate::ast::SettingExternalKey::span);
        if runtime_key.is_empty() {
            checker.error("a setting key cannot be empty", key_span);
        } else if let Some(first_span) = runtime_keys.insert(runtime_key.to_owned(), key_span) {
            checker.errors.push(
                crate::Diagnostic::type_error(
                    format!("duplicate runtime setting key `{runtime_key}`"),
                    key_span,
                )
                .with_primary_label("this key is declared again here")
                .with_secondary_label(first_span, "the first declaration is here"),
            );
        } else {
            let kind = match setting.kind {
                SettingKind::Bool { .. } => RuntimeSettingKind::Bool,
                SettingKind::Text { .. } => RuntimeSettingKind::Text,
                SettingKind::Choice { .. } => RuntimeSettingKind::Choice,
                SettingKind::File { .. } => RuntimeSettingKind::File,
                SettingKind::Title { .. } => RuntimeSettingKind::Title,
            };
            checker.declarations.settings_by_runtime_key.insert(
                runtime_key.to_owned(),
                RuntimeSettingDeclaration {
                    source_name: (setting.source_visible && kind != RuntimeSettingKind::Title)
                        .then(|| setting.name.clone()),
                    kind,
                    span: key_span,
                },
            );
        }
        if let Some(ty) = setting_value_type(checker, setting) {
            checker.semantics.resolve_value_type(setting.id, ty);
            if setting.source_visible
                && checker
                    .declarations
                    .settings
                    .insert(setting.name.clone(), (setting.id, ty))
                    .is_some()
            {
                checker.error(
                    format!("duplicate setting `{}`", setting.name),
                    setting.span,
                );
            }
        }
        let SettingKind::Choice {
            default_variant,
            options,
            ..
        } = &setting.kind
        else {
            continue;
        };
        let Some(EnumTypeId::Source(enumeration)) = checker.resolutions.setting_enum(setting.id)
        else {
            checker.error("unresolved enum used by choice setting", setting.span);
            continue;
        };
        let declaration = checker
            .declarations
            .enums
            .iter()
            .find(|item| item.id == enumeration)
            .cloned();
        let Some(declaration) = declaration else {
            checker.error("unknown enum used by choice setting", setting.span);
            continue;
        };
        let mut seen = HashSet::new();
        for option in options {
            let Some(variant) = declaration
                .variants
                .iter()
                .find(|variant| variant.name == option.variant)
            else {
                checker.error(
                    format!(
                        "enum `{}` has no variant `{}`",
                        declaration.name, option.variant
                    ),
                    option.span,
                );
                continue;
            };
            if variant.payload.is_some() {
                checker.error("choice variants cannot have payloads", option.span);
            }
            checker
                .semantics
                .resolve_setting_choice_option(option.id, variant.id);
            if !seen.insert(option.variant.clone()) {
                checker.error(
                    format!("duplicate choice option `{}`", option.variant),
                    option.span,
                );
            }
        }
        if !seen.contains(default_variant) {
            checker.error(
                "the default choice must be one of its options",
                setting.span,
            );
        } else if let Some(variant) = declaration
            .variants
            .iter()
            .find(|variant| variant.name == *default_variant)
        {
            checker
                .semantics
                .resolve_setting_choice_default(setting.id, variant.id);
        }
    }
}

fn setting_value_type(checker: &Checker, setting: &SettingDecl) -> Option<Type> {
    match &setting.kind {
        SettingKind::Bool { .. } => Some(checker.core_type(CoreTypeId::Bool)),
        SettingKind::Choice { .. } => {
            Some(checker.enum_type(checker.resolutions.setting_enum(setting.id)?))
        }
        SettingKind::Text { .. } | SettingKind::File { .. } => {
            Some(checker.standard_type(StdlibTypeId::String))
        }
        SettingKind::Title { .. } => None,
    }
}

fn collect_named_type_members(checker: &mut Checker, program: &Program) {
    let mut struct_names = HashSet::new();
    for structure in &program.structs {
        if !struct_names.insert(structure.name.clone()) {
            checker.error(
                format!("duplicate struct `{}`", structure.name),
                structure.span,
            );
        }
        let mut fields = HashSet::new();
        for field in &structure.fields {
            let field_ty = checker.syntax_type(field.ty);
            checker
                .semantics
                .resolve_struct_field_type(field.id, field_ty);
            if !fields.insert(field.name.clone()) {
                checker.error(
                    format!(
                        "duplicate field `{}` in struct `{}`",
                        field.name, structure.name
                    ),
                    field.span,
                );
            }
            if let Some(standard) = checker.standard_type_id(field_ty) {
                let declaration = checker.standard_library.type_decl(standard);
                if !declaration.value_usage.struct_field {
                    checker.error(
                        format!("{} cannot be stored in a struct field", declaration.name),
                        field.span,
                    );
                }
            }
        }
    }

    for class in program.managed_class_declarations() {
        let mut common_fields = HashSet::new();
        let mut common_metadata_names = HashMap::new();
        for field in &class.fields {
            collect_managed_field(
                checker,
                class.name.as_str(),
                field,
                &mut common_fields,
                &mut common_metadata_names,
            );
        }
    }

    let mut enum_names = HashSet::new();
    let enum_declarations = checker.declarations.enums.clone();
    for enumeration in &enum_declarations {
        if !enum_names.insert(enumeration.name.clone()) || struct_names.contains(&enumeration.name)
        {
            checker.error(
                format!("duplicate named type `{}`", enumeration.name),
                enumeration.span,
            );
        }
        if let Some(representation) = enumeration.representation {
            let representation = checker.syntax_type(representation);
            checker
                .semantics
                .resolve_enum_representation(enumeration.id, representation);
            let valid = representation
                .try_to_ref(checker.inference.type_store())
                .is_some_and(|representation| {
                    matches!(
                        representation,
                        crate::types::ResolvedTypeRef::Core(
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
                });
            if !valid {
                checker.error(
                    "an enum process-memory representation must be one of `i8`, `u8`, `i16`, `u16`, `i32`, `u32`, `i64`, or `u64`",
                    enumeration
                        .representation_span
                        .unwrap_or(enumeration.name_span),
                );
            }
        }
        let mut variants = HashSet::new();
        for variant in &enumeration.variants {
            let payload = variant.payload.map(|ty| checker.syntax_type(ty));
            checker
                .semantics
                .resolve_enum_variant_payload(variant.id, payload);
            if !variants.insert(variant.name.clone()) {
                checker.error(
                    format!(
                        "duplicate variant `{}` in enum `{}`",
                        variant.name, enumeration.name
                    ),
                    variant.span,
                );
            }
            if enumeration.representation.is_some() && payload.is_some() {
                checker.error(
                    format!(
                        "process-readable enum variant `{}.{}` cannot carry a payload",
                        enumeration.name, variant.name
                    ),
                    variant.span,
                );
            }
            if let Some(discriminant) = variant
                .discriminant
                .filter(|_| enumeration.representation.is_none())
            {
                checker.error(
                    "an explicit discriminant requires an integer representation on the enum",
                    discriminant.span,
                );
            }
            if let Some(standard) = payload.and_then(|ty| checker.standard_type_id(ty))
                && !checker
                    .standard_library
                    .type_decl(standard)
                    .value_usage
                    .enum_payload
            {
                checker.error(
                    "enum payloads cannot store this standard-library type",
                    variant.span,
                );
            }
        }
        if enumeration.variants.is_empty() {
            checker.error("an enum needs at least one variant", enumeration.span);
        }
    }
}

fn collect_conditional_managed_fields(checker: &mut Checker, program: &Program) {
    for class in program.managed_class_declarations() {
        let common_fields: HashSet<String> = class
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect();
        let common_metadata_names: HashMap<String, (String, Span)> = class
            .fields
            .iter()
            .flat_map(|field| {
                field
                    .binding_name_candidates()
                    .into_iter()
                    .map(|(name, span, _)| (name, (field.name.clone(), span)))
            })
            .collect();
        let predicates = checker.shape_branch_predicates(&class.conditional_fields);
        for (group, predicate) in class.conditional_fields.iter().zip(predicates) {
            if predicate.alternatives.iter().flatten().any(|constraint| {
                matches!(
                    constraint.dimension,
                    super::declarations::ShapeDimension::StateField(_)
                )
            }) {
                checker.error(
                    "managed field conditions cannot depend on dynamically polled state fields",
                    group.keyword_span,
                );
            }
            let mut fields = common_fields.clone();
            let mut metadata_names = common_metadata_names.clone();
            for field in &group.fields {
                collect_managed_field(
                    checker,
                    class.name.as_str(),
                    field,
                    &mut fields,
                    &mut metadata_names,
                );
                checker
                    .declarations
                    .conditional_managed_fields
                    .insert(field.id, predicate.clone());
                checker.semantics.resolve_conditional_managed_field(
                    field.id,
                    resolved_shape_predicate(&predicate),
                );
            }
        }
    }
}

fn resolved_shape_predicate(
    predicate: &super::declarations::ShapePredicate,
) -> crate::semantic::ResolvedShapePredicate {
    crate::semantic::ResolvedShapePredicate {
        alternatives: predicate
            .alternatives
            .iter()
            .map(|alternative| {
                alternative
                    .iter()
                    .map(|constraint| crate::semantic::ResolvedShapeConstraint {
                        dimension: match constraint.dimension {
                            super::declarations::ShapeDimension::Global(value) => {
                                crate::semantic::ResolvedShapeDimension::Global(value)
                            }
                            super::declarations::ShapeDimension::StateField(value) => {
                                crate::semantic::ResolvedShapeDimension::StateField(value)
                            }
                        },
                        variant: constraint.variant,
                    })
                    .collect()
            })
            .collect(),
    }
}

fn collect_managed_field(
    checker: &mut Checker,
    class_name: &str,
    field: &crate::ast::ManagedFieldDecl,
    fields: &mut HashSet<String>,
    metadata_names: &mut HashMap<String, (String, Span)>,
) {
    let field_ty = checker.syntax_type(field.ty);
    // Materialize nullable live-reference projections even for declarations
    // that are not read, so semantic binding plans remain read-only queries.
    checker.managed_read_value_type(field.ty);
    let managed_string = managed_string_kind(checker, field_ty);
    match (managed_string, field.max_length) {
        (Some(_), None) => {
            let insertion = Span {
                start: field.span.end.saturating_sub(1),
                end: field.span.end.saturating_sub(1),
            };
            checker.errors.push(
                crate::Diagnostic::type_error(
                    "a managed `String` field needs an explicit maximum length",
                    field.name_span,
                )
                .with_primary_label("the compiler must bound the remote UTF-16 read")
                .with_fix(crate::DiagnosticFix {
                    title: "add an example maximum length".into(),
                    applicability: crate::FixApplicability::MaybeIncorrect,
                    edits: vec![crate::TextEdit {
                        span: insertion,
                        replacement: " maxLength 64".into(),
                    }],
                })
                .with_note(
                    "choose a bound that is valid for this field in every supported game version",
                ),
            );
        }
        (None, Some(max_length)) => checker.error(
            "`maxLength` is only valid on managed `String` or `String?` fields",
            max_length.span,
        ),
        (Some(_), Some(max_length)) if max_length.value == 0 => checker.error(
            "a managed string maximum length must be greater than zero",
            max_length.value_span,
        ),
        (Some(_), Some(max_length)) if max_length.value > MAX_NATIVE_UTF16_UNITS => checker.error(
            format!(
                "a managed string read is limited to {MAX_NATIVE_UTF16_UNITS} UTF-16 code units"
            ),
            max_length.value_span,
        ),
        _ => {}
    }
    checker
        .semantics
        .resolve_managed_field_type(field.id, field_ty);
    if !fields.insert(field.name.clone()) {
        checker.error(
            format!("duplicate field `{}` in class `{class_name}`", field.name),
            field.span,
        );
    }

    let mut candidate_names_seen = HashSet::new();
    for metadata_name in &field.metadata_names.values {
        if !candidate_names_seen.insert(metadata_name.value.clone()) {
            checker.error(
                format!(
                    "duplicate metadata name `{}` for field `{}`",
                    metadata_name.value, field.name
                ),
                metadata_name.span,
            );
        }
    }

    for (candidate, span, _) in field.binding_name_candidates() {
        match metadata_names.entry(candidate.clone()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert((field.name.clone(), span));
            }
            std::collections::hash_map::Entry::Occupied(entry) if entry.get().0 != field.name => {
                let (first_field, first_span) = entry.get();
                checker.errors.push(
                    crate::Diagnostic::type_error(
                        format!(
                            "managed metadata name `{candidate}` is claimed by both `{first_field}` and `{}` in class `{class_name}`",
                            field.name
                        ),
                        span,
                    )
                    .with_primary_label("this metadata name is ambiguous")
                    .with_secondary_label(*first_span, "the first field claims the same name"),
                );
            }
            std::collections::hash_map::Entry::Occupied(_) => {}
        }
    }
}

fn managed_string_kind(checker: &mut Checker, ty: Type) -> Option<bool> {
    let ty = checker.inference.shallow(ty);
    if checker.standard_type_id(ty) == Some(StdlibTypeId::String) {
        return Some(false);
    }
    let value = match ty {
        Type::Option(option) => checker.inference.option_value(option),
        Type::Known(id) => match checker.inference.type_store().kind(id) {
            crate::types::TypeKind::Option { value, .. } => Type::Known(*value),
            _ => return None,
        },
        _ => return None,
    };
    let value = checker.inference.shallow(value);
    (checker.standard_type_id(value) == Some(StdlibTypeId::String)).then_some(true)
}

fn collect_function_signatures(checker: &mut Checker, program: &Program) {
    for function in &program.functions {
        let params = function
            .params
            .iter()
            .map(|parameter| {
                let ty = if let Some(annotation) = parameter.annotation {
                    let ty = checker.syntax_type(annotation);
                    checker.inference.freshen_omitted_array_shapes(ty)
                } else {
                    checker.fresh_inference(Requirements::none(), None)
                };
                checker.semantics.resolve_value_type(parameter.id, ty);
                ty
            })
            .collect::<Vec<_>>();
        let annotated = function.return_annotation.map(|annotation| {
            let ty = checker.syntax_type(annotation);
            checker.inference.freshen_omitted_array_shapes(ty)
        });
        let is_generator = crate::typeck::control_flow::contains_yield(&function.body);
        // Generic catalog bodies omit their source-level result annotation and
        // are seeded from the privileged catalog during body checking. Keep
        // generator classification independent from that rendering detail.
        let catalog_generator = is_generator
            && checker
                .standard_library
                .source_body_item_by_function_name(&function.name)
                .is_some_and(|item| {
                    matches!(item.signature.result, crate::stdlib::TypeRef::Iterator(_))
                });
        let inferred_generator_result = catalog_generator.then(|| {
            let item = checker.fresh_inference(Requirements::none(), None);
            Type::Iterator(checker.inference.iterator_type(item))
        });
        let completion = if let Some(Type::Async(future)) = annotated {
            checker.inference.async_value(future)
        } else if (is_generator && matches!(annotated, Some(Type::Iterator(_))))
            || catalog_generator
        {
            checker.core_type(crate::stdlib::CoreTypeId::None)
        } else if let Some(annotation) = annotated {
            annotation
        } else if contains_value_return(&function.body) {
            checker.fresh_inference(Requirements::none(), None)
        } else {
            checker.core_type(crate::stdlib::CoreTypeId::None)
        };
        let is_async = function.return_is_async
            || crate::typeck::control_flow::contains_suspension(&function.body);
        let result = if function.return_is_iterator {
            annotated.expect("an explicit iterator result has an annotation")
        } else if let Some(result) = inferred_generator_result {
            result
        } else if is_async {
            match annotated {
                Some(result @ Type::Async(_)) => result,
                _ => Type::Async(checker.inference.async_type(completion)),
            }
        } else if let Some(annotation) = annotated {
            annotation
        } else {
            completion
        };
        checker
            .semantics
            .resolve_function_result(function.id, result);
        checker
            .semantics
            .resolve_function_completion(function.id, completion);
        let signature = FunctionSignature {
            id: function.id,
            params,
            parameter_declarations: function
                .params
                .iter()
                .map(
                    |parameter| super::declarations::FunctionParameterDeclaration {
                        name: parameter.name.clone(),
                        span: parameter.span,
                    },
                )
                .collect(),
            result,
            completion,
            generalized: Vec::new(),
            generalized_array_shapes: Vec::new(),
            associated_projections: Vec::new(),
        };
        checker
            .declarations
            .function_signatures
            .insert(function.id, signature.clone());
        if let Some(receiver) = function.method_of {
            let key = (checker.syntax_type(receiver), function.name.clone());
            if checker
                .declarations
                .methods
                .insert(key, signature)
                .is_some()
            {
                checker.error(
                    format!("duplicate method `{}` for `{receiver}`", function.name),
                    function.span,
                );
            }
            continue;
        }
        if function.name == "Err"
            || !checker
                .standard_library
                .function_candidates(std::slice::from_ref(&function.name))
                .is_empty()
            || checker.declarations.functions.contains_key(&function.name)
        {
            checker.error(
                format!("duplicate or reserved function name `{}`", function.name),
                function.span,
            );
            continue;
        }
        checker
            .declarations
            .functions
            .insert(function.name.clone(), signature);
    }
}

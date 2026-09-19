//! Expression constraints, wrapper flow, pattern checking, and operators.

use std::collections::{HashMap, HashSet};

use crate::{
    Diagnostic, DiagnosticFix, FixApplicability, TextEdit,
    ast::{
        BinaryOp, Expr, ExprId, ExprKind, InterpolatedPart, MatchPattern, Span, SuspensionMode,
        UnaryOp,
    },
    inference::{Requirements, Type},
    migration::{ASL_SETTINGS_LOOKUP_DIAGNOSTIC, migration_diagnostic},
    semantic::{
        PendingFunctionValue, ResolvedEnumVariantId, ResolvedStructFieldId, ResolvedStructId,
        ResolvedWrapperPattern,
    },
    signature::parse_signature,
    stdlib::{RuntimeRepresentation, StdlibCapabilityId, StdlibTypeId},
    types::{EnumTypeId, TypeKind},
};

use super::{
    Checker, context::NonePolicy, declarations::Binding, pattern_usefulness::PatternCoverage,
};

pub(super) struct CheckedPattern {
    pub(super) coverage: PatternCoverage,
    pub(super) shape_variants: Option<HashSet<crate::ast::EnumVariantId>>,
}

impl CheckedPattern {
    fn new(coverage: PatternCoverage) -> Self {
        Self {
            coverage,
            shape_variants: None,
        }
    }
}

/// Bindings that are definitely initialized for each outcome of a boolean
/// expression. `None` denotes an unreachable outcome; a present map contains
/// only declarations introduced by the expression itself and guaranteed on
/// every path reaching that outcome.
#[derive(Clone)]
pub(super) struct ConditionFlow {
    pub(super) when_true: Option<HashMap<String, Binding>>,
    pub(super) when_false: Option<HashMap<String, Binding>>,
}

impl ConditionFlow {
    fn unknown() -> Self {
        Self {
            when_true: Some(HashMap::new()),
            when_false: Some(HashMap::new()),
        }
    }

    fn literal(value: bool) -> Self {
        Self {
            when_true: value.then(HashMap::new),
            when_false: (!value).then(HashMap::new),
        }
    }

    fn negated(mut self) -> Self {
        std::mem::swap(&mut self.when_true, &mut self.when_false);
        self
    }
}

fn sequence_condition_paths(
    first: &Option<HashMap<String, Binding>>,
    second: &Option<HashMap<String, Binding>>,
) -> Option<HashMap<String, Binding>> {
    let mut combined = first.as_ref()?.clone();
    combined.extend(
        second
            .as_ref()?
            .iter()
            .map(|(name, binding)| (name.clone(), *binding)),
    );
    Some(combined)
}

fn join_condition_paths(
    left: &Option<HashMap<String, Binding>>,
    right: &Option<HashMap<String, Binding>>,
) -> Option<HashMap<String, Binding>> {
    match (left, right) {
        (None, path) | (path, None) => path.clone(),
        (Some(left), Some(right)) => Some(
            left.iter()
                .filter_map(|(name, binding)| {
                    right
                        .get(name)
                        .filter(|other| other.id == binding.id)
                        .map(|_| (name.clone(), *binding))
                })
                .collect(),
        ),
    }
}

impl Checker {
    pub(super) fn check_condition(&mut self, expression: &Expr) -> ConditionFlow {
        self.expr(
            expression,
            Some(self.core_type(crate::stdlib::CoreTypeId::Bool)),
        );
        self.condition_flows
            .get(&expression.id)
            .cloned()
            .unwrap_or_else(ConditionFlow::unknown)
    }

    pub(super) fn with_condition_path<T>(
        &mut self,
        path: Option<&HashMap<String, Binding>>,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let bindings = path.cloned().unwrap_or_default();
        let names = bindings.keys().cloned().collect();
        self.scopes.push(bindings);
        self.active_condition_bindings.push(names);
        let output = operation(self);
        self.active_condition_bindings.pop();
        self.scopes.pop();
        output
    }

    pub(super) fn expr(&mut self, expr: &Expr, expected: Option<Type>) -> Option<Type> {
        let ty = match &expr.kind {
            ExprKind::Error => {
                self.error("cannot type-check a recovered expression", expr.span);
                return None;
            }
            ExprKind::None => {
                let none = self.core_type(crate::stdlib::CoreTypeId::None);
                match expected.map(|ty| self.shallow_type(ty)) {
                    Some(expected) if self.expected_accepts_none(expected) => {
                        self.semantics.resolve_value_conversion(
                            expr.id,
                            crate::semantic::ValueConversionKind::NoneToOptional,
                            none,
                            expected,
                        );
                        expected
                    }
                    Some(expected) if self.none_policy == NonePolicy::DomainNullable => {
                        self.semantics.resolve_value_conversion(
                            expr.id,
                            crate::semantic::ValueConversionKind::NoneToDomainNullable,
                            none,
                            expected,
                        );
                        expected
                    }
                    Some(Type::Result(result))
                        if self.shallow_type(self.inference.result_value(result)) != none =>
                    {
                        let value = self.inference.result_value(result);
                        self.expect_expression(expr.id, none, Some(value), expr.span)?
                    }
                    Some(expected) => {
                        self.expect_expression(expr.id, none, Some(expected), expr.span)?
                    }
                    None => none,
                }
            }
            ExprKind::IteratorEnd => {
                let item = self.fresh_inference(Requirements::none(), None);
                let step = Type::Application(self.inference.application_type(
                    crate::stdlib::StdlibTypeConstructorId::IteratorStep,
                    vec![item],
                ));
                self.expect_expression(expr.id, step, expected, expr.span)?
            }
            ExprKind::Bool(_) => self.expect_expression(
                expr.id,
                self.core_type(crate::stdlib::CoreTypeId::Bool),
                expected,
                expr.span,
            )?,
            ExprKind::Int {
                value,
                negative,
                suffix,
            } => {
                let ty = if let Some(suffix) = suffix {
                    let suffix = self.syntax_type(*suffix);
                    let fits = if *negative {
                        self.inference.fits_negative_literal(*value, suffix)
                    } else {
                        self.inference.fits_unsigned_literal(*value, suffix)
                    };
                    if !fits {
                        self.error(
                            format!("integer literal does not fit in `{suffix}`"),
                            expr.span,
                        );
                        return None;
                    }
                    suffix
                } else {
                    if *negative {
                        self.inference.fresh_negative_integer_literal(
                            Requirements::capability(StdlibCapabilityId::Numeric),
                            *value,
                        )
                    } else {
                        self.fresh_inference(
                            Requirements::capability(StdlibCapabilityId::Numeric),
                            Some(*value),
                        )
                    }
                };
                self.expect_expression(expr.id, ty, expected, expr.span)?
            }
            ExprKind::Float(_) => {
                let ty = self.inference.fresh_float_literal();
                self.expect_expression(expr.id, ty, expected, expr.span)?
            }
            ExprKind::Char(_) => self.expect_expression(
                expr.id,
                self.core_type(crate::stdlib::CoreTypeId::Char),
                expected,
                expr.span,
            )?,
            ExprKind::String(_) => self.expect_expression(
                expr.id,
                self.standard_type(StdlibTypeId::String),
                expected,
                expr.span,
            )?,
            ExprKind::InterpolatedString(parts) => {
                self.array_type_id(self.standard_type(StdlibTypeId::String));
                for part in parts {
                    if let InterpolatedPart::Expr(value) = part {
                        let value_type = self.expr(value, None)?;
                        self.require(
                            value_type,
                            Requirements::capability(StdlibCapabilityId::Display),
                            value.span,
                        );
                    }
                }
                self.expect_expression(
                    expr.id,
                    self.standard_type(StdlibTypeId::String),
                    expected,
                    expr.span,
                )?
            }
            ExprKind::Signature(signature) => {
                if let Err(message) = parse_signature(signature) {
                    self.error(message, expr.span);
                }
                self.expect_expression(
                    expr.id,
                    self.standard_type(StdlibTypeId::Signature),
                    expected,
                    expr.span,
                )?
            }
            ExprKind::Array(elements) => {
                let value_expected = expected.map(|ty| self.expected_value_type(ty));
                let hinted = value_expected.and_then(|ty| match ty {
                    Type::Array(id) => self
                        .inference
                        .arrays()
                        .iter()
                        .find(|array| array.id == id)
                        .map(|array| (id, array.element)),
                    _ => None,
                });
                let (id, element_type) = if let Some((expected_id, element)) = hinted {
                    let id = if matches!(
                        self.inference.array_shape(expected_id),
                        crate::inference::ArrayShape::Variable(_)
                    ) {
                        // A literal creates growable storage unless an exact
                        // contextual length says otherwise. Do not let an
                        // omitted annotation borrow the shape variable as if
                        // construction itself were shape-neutral.
                        self.array_type_id(element)
                    } else {
                        expected_id
                    };
                    (id, element)
                } else {
                    let element = self.fresh_inference(Requirements::none(), None);
                    let id = self.array_type_id(element);
                    (id, element)
                };
                if elements.is_empty() {
                    self.inferred_empty_collections.push((
                        element_type,
                        expr.span,
                        "element",
                        "array",
                    ));
                }
                if let Some(expected_length) = self.inference.array_length(id)
                    && elements.len() != expected_length as usize
                {
                    self.error(
                        format!(
                            "expected {expected_length} array elements, found {}",
                            elements.len()
                        ),
                        expr.span,
                    );
                }
                for element in elements {
                    self.expr(element, Some(element_type));
                }
                self.expect_expression(expr.id, Type::Array(id), expected, expr.span)?
            }
            ExprKind::Range {
                start, end, kind, ..
            } => {
                let hinted = expected
                    .map(|ty| self.expected_value_type(ty))
                    .and_then(|ty| match self.shallow_type(ty) {
                        Type::Range(range) if self.inference.range_kind(range) == *kind => {
                            Some((range, self.inference.range_bound(range)))
                        }
                        _ => None,
                    });
                let (range, bound) = hinted.unwrap_or_else(|| {
                    let bound = self.fresh_inference(
                        Requirements::capability(StdlibCapabilityId::Integer),
                        None,
                    );
                    (self.inference.range_type(bound, *kind), bound)
                });
                self.require(
                    bound,
                    Requirements::capability(StdlibCapabilityId::Integer),
                    expr.span,
                );
                self.expr(start, Some(bound));
                self.expr(end, Some(bound));
                self.expect_expression(expr.id, Type::Range(range), expected, expr.span)?
            }
            ExprKind::Block(block) => {
                self.scopes.push(HashMap::new());
                let tail = block
                    .statements
                    .last()
                    .and_then(|statement| match statement {
                        crate::ast::Stmt::Expression(expression) => Some(expression),
                        _ => None,
                    });
                let prefix_len = block.statements.len() - usize::from(tail.is_some());
                for statement in &block.statements[..prefix_len] {
                    self.statement(statement);
                }
                let prefix_is_terminal = block.statements[..prefix_len]
                    .iter()
                    .any(|statement| super::body_pass::statement_is_terminal(self, statement));
                let ty = if prefix_is_terminal {
                    if let Some(tail) = tail {
                        self.expr(tail, None);
                    }
                    Some(self.core_type(crate::stdlib::CoreTypeId::Never))
                } else if let Some(tail) = tail {
                    self.expr(tail, expected)
                } else if super::body_pass::block_is_terminal(self, block) {
                    Some(self.core_type(crate::stdlib::CoreTypeId::Never))
                } else {
                    let none = self.core_type(crate::stdlib::CoreTypeId::None);
                    match expected.map(|expected| self.shallow_type(expected)) {
                        Some(expected) if self.expected_accepts_none(expected) => {
                            self.semantics.resolve_value_conversion(
                                expr.id,
                                crate::semantic::ValueConversionKind::NoneToOptional,
                                none,
                                expected,
                            );
                            Some(expected)
                        }
                        Some(expected) if self.none_policy == NonePolicy::DomainNullable => {
                            self.semantics.resolve_value_conversion(
                                expr.id,
                                crate::semantic::ValueConversionKind::NoneToDomainNullable,
                                none,
                                expected,
                            );
                            Some(expected)
                        }
                        Some(expected)
                            if !matches!(expected, Type::Variable(_)) && expected != none =>
                        {
                            let expected_name = self.type_name(expected);
                            let closing = Span {
                                start: block.span.end.saturating_sub(1),
                                end: block.span.end,
                            };
                            self.errors.push(
                                Diagnostic::type_error(
                                    format!(
                                        "this value block needs a final `{expected_name}` expression"
                                    ),
                                    closing,
                                )
                                .with_primary_label("the block reaches its end without a value")
                                .with_note(
                                    "write the value as the block's final expression; `return` exits the enclosing function instead",
                                ),
                            );
                            None
                        }
                        expected => self.expect_expression(expr.id, none, expected, expr.span),
                    }
                };
                self.scopes.pop();
                ty?
            }
            ExprKind::Loop(block) => {
                let inferred_result = expected.is_none();
                let result =
                    expected.unwrap_or_else(|| self.fresh_inference(Requirements::none(), None));
                self.scopes.push(HashMap::new());
                let (_, has_break) =
                    self.with_value_loop(result, |checker| checker.block(block, true));
                self.scopes.pop();
                if has_break {
                    self.expect_expression(expr.id, result, expected, expr.span)?
                } else {
                    let never = self.core_type(crate::stdlib::CoreTypeId::Never);
                    if inferred_result {
                        self.unify(result, never, expr.span);
                    }
                    self.expect_expression(expr.id, never, expected, expr.span)?
                }
            }
            ExprKind::Struct { name, fields, .. } => {
                let declaration = self
                    .declarations
                    .structs
                    .iter()
                    .find(|declaration| declaration.name == *name)
                    .cloned();
                if let Some(declaration) = declaration {
                    self.semantics
                        .resolve_struct_literal(expr.id, ResolvedStructId::Source(declaration.id));
                    let mut seen = HashSet::new();
                    let mut resolved_fields = Vec::with_capacity(fields.len());
                    for literal_field in fields {
                        let name = &literal_field.name;
                        let value = &literal_field.value;
                        if !seen.insert(name.clone()) {
                            self.error(format!("duplicate struct field `{name}`"), value.span);
                            continue;
                        }
                        if let Some(field) =
                            declaration.fields.iter().find(|field| field.name == *name)
                        {
                            let field_type = self.syntax_type(field.ty);
                            let field_type_name = self.type_name(field_type);
                            self.with_expected_type_source(
                                super::ExpectedTypeSource {
                                    span: field.span,
                                    label: format!(
                                        "struct field `{}.{}` is declared as `{field_type_name}`",
                                        declaration.name, field.name,
                                    ),
                                },
                                |checker| checker.expr(value, Some(field_type)),
                            );
                            resolved_fields.push(ResolvedStructFieldId::Source(field.id));
                        } else {
                            self.expr(value, None);
                            self.error(
                                format!("struct `{}` has no field `{name}`", declaration.name),
                                value.span,
                            );
                        }
                    }
                    self.semantics
                        .resolve_struct_literal_fields(expr.id, resolved_fields);
                    for field in &declaration.fields {
                        if !seen.contains(&field.name) {
                            self.error(
                                format!(
                                    "struct `{}` initializer is missing field `{}`",
                                    declaration.name, field.name
                                ),
                                expr.span,
                            );
                        }
                    }
                    self.expect_expression(
                        expr.id,
                        self.struct_type(declaration.id),
                        expected,
                        expr.span,
                    )?
                } else if let Some(declaration) = (if self.is_library_function() {
                    self.standard_library.type_by_name_including_private(name)
                } else {
                    self.standard_library.type_by_name(name)
                })
                .copied()
                {
                    let privileged_library_body = self.is_library_function();
                    if (!privileged_library_body && !declaration.public_construction)
                        || !matches!(
                            declaration.representation,
                            RuntimeRepresentation::GcStruct { .. }
                        )
                    {
                        self.error(
                            format!(
                                "standard-library type `{name}` can only be constructed by standard-library source"
                            ),
                            expr.span,
                        );
                        return None;
                    }
                    self.semantics.resolve_struct_literal(
                        expr.id,
                        ResolvedStructId::Standard(declaration.id),
                    );
                    let declared_fields = self
                        .standard_library
                        .fields_of(declaration.id)
                        .copied()
                        .collect::<Vec<_>>();
                    let mut seen = HashSet::new();
                    let mut resolved_fields = Vec::with_capacity(fields.len());
                    for literal_field in fields {
                        let name = &literal_field.name;
                        let value = &literal_field.value;
                        if !seen.insert(name.clone()) {
                            self.error(format!("duplicate struct field `{name}`"), value.span);
                            continue;
                        }
                        if let Some(field) =
                            declared_fields.iter().find(|field| field.name == *name)
                        {
                            self.expr(value, Some(self.standard_field_type(field.id)));
                            resolved_fields.push(ResolvedStructFieldId::Standard(field.id));
                        } else {
                            self.expr(value, None);
                            self.error(
                                format!("struct `{}` has no field `{name}`", declaration.name),
                                value.span,
                            );
                        }
                    }
                    self.semantics
                        .resolve_struct_literal_fields(expr.id, resolved_fields);
                    for field in &declared_fields {
                        if !seen.contains(field.name) {
                            self.error(
                                format!(
                                    "struct `{}` initializer is missing field `{}`",
                                    declaration.name, field.name
                                ),
                                expr.span,
                            );
                        }
                    }
                    self.expect_expression(
                        expr.id,
                        self.standard_type(declaration.id),
                        expected,
                        expr.span,
                    )?
                } else if self.is_library_function()
                    && let Some(declaration) = self
                        .standard_library
                        .named_type_constructor_by_name_including_private(name)
                        .copied()
                {
                    let variables = declaration
                        .parameters
                        .iter()
                        .map(|parameter| {
                            let requirements = parameter.constraints.iter().fold(
                                Requirements::none(),
                                |requirements, constraint| {
                                    requirements | Requirements::capability(*constraint)
                                },
                            );
                            (parameter.name, self.fresh_inference(requirements, None))
                        })
                        .collect::<HashMap<_, _>>();
                    let arguments = declaration
                        .parameters
                        .iter()
                        .map(|parameter| variables[parameter.name])
                        .collect::<Vec<_>>();
                    let struct_type = self.catalog_application_type(declaration.id, arguments);
                    let Type::Application(application) = struct_type else {
                        unreachable!("named runtime struct constructors use application layouts")
                    };
                    self.semantics.resolve_struct_literal(
                        expr.id,
                        ResolvedStructId::StandardConstructor(application),
                    );
                    let declared_fields = self
                        .standard_library
                        .fields_of_constructor(declaration.id)
                        .copied()
                        .collect::<Vec<_>>();
                    let mut seen = HashSet::new();
                    let mut resolved_fields = Vec::with_capacity(fields.len());
                    for literal_field in fields {
                        let name = &literal_field.name;
                        let value = &literal_field.value;
                        if !seen.insert(name.clone()) {
                            self.error(format!("duplicate struct field `{name}`"), value.span);
                            continue;
                        }
                        if let Some(field) =
                            declared_fields.iter().find(|field| field.name == *name)
                        {
                            let field_type = self.catalog_type(field.ty, &variables);
                            self.expr(value, Some(field_type));
                            resolved_fields.push(ResolvedStructFieldId::Standard(field.id));
                        } else {
                            self.expr(value, None);
                            self.error(
                                format!("struct `{}` has no field `{name}`", declaration.name),
                                value.span,
                            );
                        }
                    }
                    self.semantics
                        .resolve_struct_literal_fields(expr.id, resolved_fields);
                    for field in &declared_fields {
                        if !seen.contains(field.name) {
                            self.error(
                                format!(
                                    "struct `{}` initializer is missing field `{}`",
                                    declaration.name, field.name
                                ),
                                expr.span,
                            );
                        }
                    }
                    self.expect_expression(expr.id, struct_type, expected, expr.span)?
                } else {
                    self.error(format!("unknown struct type `{name}`"), expr.span);
                    return None;
                }
            }
            ExprKind::Is { value, pattern, .. } => {
                let value_type = self.expr(value, None)?;
                self.scopes.push(HashMap::new());
                let checked =
                    self.check_pattern(&pattern.kind, pattern.id, value_type, pattern.span);
                let bindings = self.scopes.pop().expect("an `is` pattern owns a scope");
                pattern.kind.visit_bindings(&mut |binding| {
                    if !self
                        .conditional_binding_declarations
                        .iter()
                        .any(|(_, span)| *span == binding.name_span)
                    {
                        self.conditional_binding_declarations
                            .push((binding.name.clone(), binding.name_span));
                    }
                });
                for name in bindings.keys() {
                    if self
                        .active_condition_bindings
                        .iter()
                        .any(|active| active.contains(name))
                    {
                        self.error(
                            format!("conditional pattern binding `{name}` is already in scope"),
                            pattern.span,
                        );
                    }
                }
                self.condition_flows.insert(
                    expr.id,
                    ConditionFlow {
                        when_true: Some(bindings),
                        when_false: (!checked.coverage.is_irrefutable()).then(HashMap::new),
                    },
                );
                self.expect_expression(
                    expr.id,
                    self.core_type(crate::stdlib::CoreTypeId::Bool),
                    expected,
                    expr.span,
                )?
            }
            ExprKind::Match { value, arms } => {
                let value_type = self.expr(value, None)?;
                let refines_state_provider = matches!(
                    &value.kind,
                    ExprKind::Path(path)
                        if matches!(path.as_slice(), [name] if name == "provider")
                            && self.state_refinement_value.is_some()
                );
                let mut unguarded_patterns = Vec::<PatternCoverage>::new();
                let mut result_type = expected;
                let mut never_arm_type = None;
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    let checked =
                        self.check_pattern(&arm.pattern, arm.pattern_id, value_type, arm.span);
                    let provider_variants = if refines_state_provider {
                        checked.shape_variants.clone()
                    } else {
                        None
                    };
                    let shape_constraints = self.shape_match_constraints(value, &arm.pattern);
                    let arm_type = self.with_provider_variants(provider_variants, |checker| {
                        checker.with_shape_constraints(shape_constraints.as_deref(), |checker| {
                            if let Some(guard) = &arm.guard {
                                let flow = checker.check_condition(guard);
                                checker.with_condition_path(flow.when_true.as_ref(), |checker| {
                                    checker.expr(&arm.value, result_type)
                                })
                            } else {
                                checker.expr(&arm.value, result_type)
                            }
                        })
                    });
                    self.scopes.pop();
                    if expected.is_none()
                        && let Some(arm_type) = arm_type
                    {
                        if self.is_never_type(arm_type) {
                            never_arm_type.get_or_insert(arm_type);
                        } else if result_type.is_none()
                            || result_type.is_some_and(|ty| self.is_never_type(ty))
                        {
                            result_type = Some(arm_type);
                        }
                    }

                    if arm.guard.is_none() {
                        if !self.pattern_is_useful(
                            &unguarded_patterns,
                            &checked.coverage,
                            value_type,
                        ) {
                            let message = if unguarded_patterns.contains(&checked.coverage) {
                                format!("duplicate match arm `{}`", checked.coverage.display())
                            } else {
                                format!("unreachable match arm `{}`", checked.coverage.display())
                            };
                            self.error(message, arm.span);
                        }
                        unguarded_patterns.push(checked.coverage);
                    } else if !self.pattern_is_useful(
                        &unguarded_patterns,
                        &checked.coverage,
                        value_type,
                    ) {
                        self.error("unreachable guarded match arm", arm.span);
                    }
                }

                let matched_type = self.shallow_type(value_type);
                let is_iterator_step = matches!(matched_type, Type::Application(step)
                    if self.inference.application_constructor(step)
                        == crate::stdlib::StdlibTypeConstructorId::IteratorStep);
                let is_source_struct = self.source_struct_id(matched_type).is_some();
                let is_enum = self.enum_info_for_type(matched_type).is_some();
                let missing_patterns = self.missing_patterns(&unguarded_patterns, matched_type);
                let is_matchable = matched_type == self.core_type(crate::stdlib::CoreTypeId::Bool)
                    || matched_type == self.core_type(crate::stdlib::CoreTypeId::Char)
                    || matched_type == self.standard_type(StdlibTypeId::String)
                    || matched_type == self.standard_type(StdlibTypeId::FileVersion)
                    || self.inference.is_integer(matched_type)
                    || matches!(
                        matched_type,
                        Type::Option(_) | Type::Result(_) | Type::Array(_)
                    )
                    || is_iterator_step
                    || is_source_struct
                    || is_enum
                    || self.is_never_type(matched_type);
                if missing_patterns.is_empty() {
                    // A catch-all pattern is valid even when matching itself
                    // imposes no concrete type constraint on the value.
                } else if matches!(matched_type, Type::Variable(_)) {
                    self.error(
                        "match patterns do not determine the matched value's type",
                        value.span,
                    );
                } else if !is_matchable {
                    let ty = self.type_name(matched_type);
                    self.error(format!("type `{ty}` cannot be matched"), value.span);
                } else {
                    for missing in missing_patterns {
                        let message = if self.inference.is_integer(matched_type) {
                            format!(
                                "non-exhaustive integer match: missing `{}`; add a `_` arm",
                                missing.display()
                            )
                        } else if matched_type == self.core_type(crate::stdlib::CoreTypeId::Char) {
                            "non-exhaustive character match: add a `_` arm".to_owned()
                        } else if matched_type == self.standard_type(StdlibTypeId::String) {
                            "non-exhaustive string match: add a `_` arm".to_owned()
                        } else if matched_type == self.standard_type(StdlibTypeId::FileVersion) {
                            "non-exhaustive file-version match: add a `_` arm".to_owned()
                        } else if matches!(matched_type, Type::Array(_)) {
                            "non-exhaustive array match: add a `_` arm".to_owned()
                        } else if is_source_struct {
                            "non-exhaustive struct match: add a `_` arm".to_owned()
                        } else {
                            format!("non-exhaustive match: missing `{}`", missing.display())
                        };
                        self.error(message, expr.span);
                    }
                }
                let Some(result_type) = result_type.or(never_arm_type) else {
                    self.error("a match needs at least one arm", expr.span);
                    return None;
                };
                self.expect_expression(expr.id, result_type, expected, expr.span)?
            }
            ExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                let flow = self.check_condition(condition);
                let shape_constraints = self.shape_constraints(condition);
                let inverse_shape_constraints = self.inverse_shape_constraints(condition);
                let result_type =
                    expected.unwrap_or_else(|| self.fresh_inference(Requirements::none(), None));
                // `None` can either remain the zero-sized unit value or lift into
                // any optional type. When the conditional has no surrounding
                // expected type, let the value-bearing branch establish the
                // result before checking a bare `None`. This keeps inference
                // independent of which branch happens to be written first.
                let (then_type, else_type) = if expected.is_none()
                    && expression_is_bare_none(then_expr)
                    && !expression_is_bare_none(else_expr)
                {
                    let else_type = self.with_condition_path(flow.when_false.as_ref(), |checker| {
                        checker.with_shape_constraints(
                            inverse_shape_constraints.as_deref(),
                            |checker| checker.expr(else_expr, Some(result_type)),
                        )
                    });
                    let then_type = self.with_condition_path(flow.when_true.as_ref(), |checker| {
                        checker.with_shape_constraints(shape_constraints.as_deref(), |checker| {
                            checker.expr(then_expr, Some(result_type))
                        })
                    });
                    (then_type, else_type)
                } else {
                    let then_type = self.with_condition_path(flow.when_true.as_ref(), |checker| {
                        checker.with_shape_constraints(shape_constraints.as_deref(), |checker| {
                            checker.expr(then_expr, Some(result_type))
                        })
                    });
                    let else_type = self.with_condition_path(flow.when_false.as_ref(), |checker| {
                        checker.with_shape_constraints(
                            inverse_shape_constraints.as_deref(),
                            |checker| checker.expr(else_expr, Some(result_type)),
                        )
                    });
                    (then_type, else_type)
                };
                if expected.is_none()
                    && then_type.is_some_and(|ty| self.is_never_type(ty))
                    && else_type.is_some_and(|ty| self.is_never_type(ty))
                {
                    let never = self.core_type(crate::stdlib::CoreTypeId::Never);
                    self.unify(result_type, never, expr.span);
                    never
                } else {
                    self.expect_expression(expr.id, result_type, expected, expr.span)?
                }
            }
            ExprKind::Fallback { value, fallback } => {
                let wrapper = self.expr(value, None)?;
                let wrapper = self.shallow_type(wrapper);
                let value_type = match wrapper {
                    Type::Option(option) => self.inference.option_value(option),
                    Type::Result(result) => self.inference.result_value(result),
                    ty => {
                        let ty = self.type_name(ty);
                        self.error(
                            format!("`else` can only unwrap `T?` or `T!`, found `{ty}`"),
                            value.span,
                        );
                        return None;
                    }
                };
                let direct_optional_return = if self.expression_mode
                    == super::context::ExpressionMode::DirectReturn
                    && matches!(wrapper, Type::Result(_))
                    && matches!(fallback.kind, ExprKind::None)
                {
                    expected.is_some_and(|expected| {
                        matches!(self.shallow_type(expected), Type::Option(_))
                    })
                } else {
                    false
                };
                if direct_optional_return {
                    self.expr(fallback, None);
                    self.errors.push(
                        Diagnostic::type_error(
                            "`else None` supplies the unwrapped fallback value, not the optional function return",
                            fallback.span,
                        )
                        .with_primary_label("return the absent optional value from this branch")
                        .with_note(
                            "write `else return None` when failure should make the enclosing function return `None`",
                        )
                        .with_machine_applicable_fix(
                            "return `None` from the fallback branch",
                            Span {
                                start: fallback.span.start,
                                end: fallback.span.start,
                            },
                            "return ",
                        ),
                    );
                } else {
                    self.expr(fallback, Some(value_type));
                }
                self.expect_expression(expr.id, value_type, expected, expr.span)?
            }
            ExprKind::Break(value) => {
                match self.loops.break_target() {
                    None => {
                        if let Some(value) = value {
                            self.expr(value, None);
                        }
                        self.error("`break` is only available inside a loop", expr.span);
                    }
                    Some(super::context::BreakTarget::Statement) => {
                        if let Some(value) = value {
                            self.expr(value, None);
                            self.error(
                                "only a `loop` expression can break with a value",
                                value.span,
                            );
                        }
                        self.loops.record_break();
                    }
                    Some(super::context::BreakTarget::Value(result)) => {
                        if let Some(value) = value {
                            self.expr(value, Some(result));
                        } else {
                            let none = self.core_type(crate::stdlib::CoreTypeId::None);
                            self.unify(result, none, expr.span);
                        }
                        self.loops.record_break();
                    }
                }
                let never = self.core_type(crate::stdlib::CoreTypeId::Never);
                self.expect_expression(expr.id, never, expected, expr.span)?
            }
            ExprKind::Continue => {
                if !self.loops.is_inside() {
                    self.error("`continue` is only available inside a loop", expr.span);
                }
                let never = self.core_type(crate::stdlib::CoreTypeId::Never);
                self.expect_expression(expr.id, never, expected, expr.span)?
            }
            ExprKind::Return(value) => {
                self.check_return(value.as_deref(), expr.span);
                let never = self.core_type(crate::stdlib::CoreTypeId::Never);
                self.expect_expression(expr.id, never, expected, expr.span)?
            }
            ExprKind::Throw(error) => {
                let boundary = self.failure.propagate();
                if let Some(boundary) = boundary {
                    self.semantics.resolve_propagation_target(
                        expr.id,
                        boundary,
                        self.failure.retry_expression(),
                    );
                } else {
                    self.error(
                        "`throw` needs `onAttach`, a fallible function, `selectProcess`, or an explicit catch boundary",
                        expr.span,
                    );
                }
                self.expr(error, Some(self.standard_type(StdlibTypeId::String)));
                let never = self.core_type(crate::stdlib::CoreTypeId::Never);
                self.expect_expression(expr.id, never, expected, expr.span)?
            }
            ExprKind::Suspend {
                mode,
                destination,
                value,
            } => {
                if self.expression_mode == super::context::ExpressionMode::SuspensionOperand {
                    let keyword = match mode {
                        SuspensionMode::Await => "await",
                        SuspensionMode::Retry => "retry",
                    };
                    self.error(
                        format!(
                            "`{keyword}` cannot be evaluated inside an `await` or `retry` operand"
                        ),
                        expr.span,
                    );
                }
                if self.generator_item.is_some() {
                    let keyword = match mode {
                        SuspensionMode::Await => "await",
                        SuspensionMode::Retry => "retry",
                    };
                    self.error(
                        format!("`{keyword}` is not available inside a synchronous generator"),
                        expr.span,
                    );
                } else if !self.callable.can_suspend() {
                    let keyword = match mode {
                        SuspensionMode::Await => "await",
                        SuspensionMode::Retry => "retry",
                    };
                    self.error(
                        format!("`{keyword}` is not available in this synchronous body"),
                        expr.span,
                    );
                }
                let (operand, retry_boundary) = match mode {
                    SuspensionMode::Await => (
                        self.with_expression_mode(
                            super::context::ExpressionMode::SuspensionOperand,
                            |checker| checker.expr(value, None),
                        )?,
                        None,
                    ),
                    SuspensionMode::Retry => {
                        let completion = expected
                            .unwrap_or_else(|| self.fresh_inference(Requirements::none(), None));
                        let boundary = Type::Result(self.inference.result_type(completion));
                        // `value` is checked through the normal expression
                        // entry point. In particular, a block operand is the
                        // same generic value block used everywhere else; only
                        // its surrounding failure boundary is retry-specific.
                        let (operand, failure) = self.with_expression_mode(
                            super::context::ExpressionMode::SuspensionOperand,
                            |checker| {
                                checker.with_failure_context(
                                    super::context::FailureContext::retry(boundary, expr.id),
                                    |checker| checker.expr(value, Some(boundary)),
                                )
                            },
                        );
                        if !failure.propagated() && !failure.observed_result() {
                            self.error(
                                "`retry` expects a result value (`T!`) or synchronous fallible work using `?` or `throw`",
                                value.span,
                            );
                            return None;
                        }
                        (operand?, Some((boundary, completion)))
                    }
                };
                let result = match mode {
                    SuspensionMode::Await => match self.shallow_type(operand) {
                        Type::Async(future) => self.inference.async_value(future),
                        operand => {
                            let operand = self.type_name(operand);
                            self.error(
                                format!("`await` expects an async value, found `{operand}`"),
                                value.span,
                            );
                            return None;
                        }
                    },
                    SuspensionMode::Retry => match self.shallow_type(operand) {
                        Type::Result(result) => {
                            let value = self.inference.result_value(result);
                            let (_, completion) = retry_boundary
                                .expect("retry operands create a local failure boundary");
                            self.unify(value, completion, expr.span)?;
                            completion
                        }
                        operand => {
                            let operand = self.type_name(operand);
                            self.error(
                                format!("`retry` expects a result value (`T!`), found `{operand}`"),
                                value.span,
                            );
                            return None;
                        }
                    },
                };
                self.semantics.resolve_value_type(*destination, result);
                self.expect_expression(expr.id, result, expected, expr.span)?
            }
            ExprKind::Propagate(value) => {
                let input = self.expr(value, None)?;
                let Type::Result(input_result) = self.shallow_type(input) else {
                    self.error("`?` requires a result value (`T!`)", value.span);
                    return None;
                };
                let Some(boundary) = self.failure.propagate() else {
                    self.error(
                        "`?` needs `onAttach`, a state-field boundary, `selectProcess`, or a function returning `T!`",
                        expr.span,
                    );
                    return None;
                };
                let Type::Result(_) = self.shallow_type(boundary) else {
                    unreachable!("failure boundaries are result types")
                };
                self.semantics.resolve_propagation_target(
                    expr.id,
                    boundary,
                    self.failure.retry_expression(),
                );
                let value_type = self.inference.result_value(input_result);
                self.expect_expression(expr.id, value_type, expected, expr.span)?
            }
            ExprKind::Path(path) => {
                if let Some(enumeration) = self.resolutions.expression_enum(expr.id) {
                    let variant = path
                        .last()
                        .expect("resolved enum paths retain a variant segment");
                    self.enum_constructor(expr.id, enumeration, variant, &[], expected, expr.span)?
                } else if let [name] = path.as_slice()
                    && self.binding(name).is_none()
                    && let Some(signature) = self.declarations.functions.get(name).cloned()
                {
                    let signature = if self.active_function_component.contains(&signature.id) {
                        signature.monomorphic_call()
                    } else {
                        signature.instantiate(&mut self.inference)
                    };
                    let callable = Type::Callable(
                        self.inference
                            .callable_type(signature.params.clone(), signature.result),
                    );
                    self.semantics.resolve_function_value(
                        expr.id,
                        PendingFunctionValue {
                            function: signature.id,
                            type_arguments: signature.type_arguments,
                            signature: signature
                                .params
                                .iter()
                                .copied()
                                .chain([signature.result])
                                .collect(),
                        },
                    );
                    self.expect_expression(expr.id, callable, expected, expr.span)?
                } else {
                    let resolution = self.path(path, expr.span, Some(expr.id))?;
                    if let Some(value) = resolution.value {
                        self.semantics.resolve_value(expr.id, value);
                    }
                    if let Some(members) = resolution.members {
                        self.semantics.resolve_path_members(expr.id, members);
                    }
                    self.expect_expression(expr.id, resolution.ty, expected, expr.span)?
                }
            }
            ExprKind::Member {
                receiver,
                name,
                name_span,
            } => {
                let receiver_ty = self.expr(receiver, None)?;
                let (ty, members) = self.resolve_members_or_defer(
                    receiver_ty,
                    std::slice::from_ref(name),
                    *name_span,
                    Some(expr.id),
                )?;
                if let Some(members) = members {
                    self.semantics.resolve_path_members(expr.id, members);
                }
                self.expect_expression(expr.id, ty, expected, expr.span)?
            }
            ExprKind::Index {
                receiver,
                index,
                bracket_span,
            } => {
                let element = self.indexed_element_type(receiver, index, expr.id, *bracket_span)?;
                self.expect_expression(expr.id, element, expected, expr.span)?
            }
            ExprKind::Unary { op, expr: inner } => match op {
                UnaryOp::Not => {
                    let operand_hint = expected.map(|ty| self.expected_value_type(ty));
                    let inner_ty = self.expr(inner, operand_hint)?;
                    let result = self
                        .resolve_unary_operator(*op, inner_ty, expr.id, inner.id, expr.span)
                        .unwrap_or_else(|| self.error_type());
                    self.expect_expression(expr.id, result, expected, expr.span)?
                }
                UnaryOp::Neg => {
                    let operand_hint = expected.map(|ty| self.expected_value_type(ty));
                    let inner_ty = self.expr(inner, operand_hint)?;
                    if let Some(result) =
                        self.resolve_unary_operator(*op, inner_ty, expr.id, inner.id, expr.span)
                    {
                        self.expect_expression(expr.id, result, expected, expr.span)?
                    } else {
                        self.require(
                            inner_ty,
                            Requirements::capabilities([
                                StdlibCapabilityId::Numeric,
                                StdlibCapabilityId::Signed,
                            ]),
                            expr.span,
                        )?;
                        self.expect_expression(expr.id, inner_ty, expected, expr.span)?
                    }
                }
            },
            ExprKind::Cast {
                expr: inner,
                target,
            } => {
                let source = self.expr(inner, None)?;
                let target = self.syntax_type(*target);
                if target == self.core_type(crate::stdlib::CoreTypeId::U32)
                    && source == self.core_type(crate::stdlib::CoreTypeId::Char)
                {
                    // Unicode scalar values convert losslessly to their code point.
                } else if self.inference.is_numeric(target) {
                    self.require(
                        source,
                        Requirements::capability(StdlibCapabilityId::Numeric),
                        expr.span,
                    )?;
                } else if target == self.standard_type(StdlibTypeId::String) {
                    self.require(
                        source,
                        Requirements::capability(StdlibCapabilityId::Display),
                        expr.span,
                    )?;
                } else {
                    let target_name = self.type_name(target);
                    self.error(
                        format!("`as` cannot convert a value to `{target_name}`"),
                        expr.span,
                    );
                    return None;
                }
                self.expect_expression(expr.id, target, expected, expr.span)?
            }
            ExprKind::Binary { op, left, right } => {
                self.binary(*op, left, right, expected, expr.id, expr.span)?
            }
            ExprKind::Call {
                callee,
                name_span,
                receiver,
                type_arguments,
                args,
                ..
            } => {
                if receiver.is_none()
                    && type_arguments.is_empty()
                    && let [name] = callee.as_slice()
                    && self.binding(name).is_some()
                {
                    let binding = self.binding_for_use(name, *name_span)?;
                    let callable = match self.shallow_type(binding.ty) {
                        Type::Callable(callable) => callable,
                        Type::Variable(_) => {
                            // Calling an otherwise unconstrained parameter is itself
                            // enough information to infer a callable type. The
                            // parameter and result variables become part of the
                            // surrounding function's generalized signature.
                            let parameters = (0..args.len())
                                .map(|_| self.fresh_inference(Requirements::none(), None))
                                .collect::<Vec<_>>();
                            let result = expected.unwrap_or_else(|| {
                                self.fresh_inference(Requirements::none(), None)
                            });
                            let callable = self.inference.callable_type(parameters, result);
                            self.unify(binding.ty, Type::Callable(callable), *name_span)?;
                            callable
                        }
                        _ => {
                            let actual = self.type_name(binding.ty);
                            self.error(
                                format!("`{name}` is not callable; found `{actual}`"),
                                *name_span,
                            );
                            return None;
                        }
                    };
                    self.semantics.resolve_dynamic_call(
                        expr.id,
                        crate::semantic::DynamicCallCallee::Value(
                            binding.id.expect("local callable bindings have identities"),
                        ),
                    );
                    self.invoke_callable_type(callable, args, expected, expr.id, expr.span)?
                } else if let Some(enumeration) = self.resolutions.expression_enum(expr.id) {
                    debug_assert!(receiver.is_none());
                    if !type_arguments.is_empty() {
                        self.error("enum variants do not accept type arguments", expr.span);
                        return None;
                    }
                    let variant = callee
                        .last()
                        .expect("resolved enum constructors retain a variant segment");
                    self.enum_constructor(expr.id, enumeration, variant, args, expected, expr.span)?
                } else {
                    self.call(
                        super::CallSyntax {
                            callee,
                            name_span: *name_span,
                            postfix_receiver: receiver.as_deref(),
                            type_arguments,
                        },
                        args,
                        expected,
                        expr.id,
                        expr.span,
                    )?
                }
            }
            ExprKind::Invoke { callee, args } => {
                self.invoke_expression(callee, args, expected, expr.id, expr.span)?
            }
            ExprKind::Closure {
                params,
                return_annotation,
                return_annotation_span,
                body,
                ..
            } => {
                let hinted = expected
                    .map(|ty| self.expected_value_type(ty))
                    .map(|ty| self.shallow_type(ty))
                    .and_then(|ty| match ty {
                        Type::Callable(callable) => Some((
                            self.inference.callable_parameters(callable).to_vec(),
                            self.inference.callable_result(callable),
                        )),
                        _ => None,
                    });
                if let Some((parameters, _)) = &hinted
                    && parameters.len() != params.len()
                {
                    self.error(
                        format!(
                            "closure expects {} parameters from context, but declares {}",
                            parameters.len(),
                            params.len()
                        ),
                        expr.span,
                    );
                    return None;
                }
                let parameter_types = params
                    .iter()
                    .enumerate()
                    .map(|(index, parameter)| {
                        let annotation = parameter.annotation.map(|ty| {
                            let ty = self.syntax_type(ty);
                            self.inference.freshen_omitted_array_shapes(ty)
                        });
                        let contextual = hinted.as_ref().map(|(parameters, _)| parameters[index]);
                        match (annotation, contextual) {
                            (Some(annotation), Some(contextual)) => self
                                .unify(annotation, contextual, parameter.span)
                                .unwrap_or(annotation),
                            (Some(annotation), None) => annotation,
                            (None, Some(contextual)) => contextual,
                            (None, None) => self.fresh_inference(Requirements::none(), None),
                        }
                    })
                    .collect::<Vec<_>>();
                let annotated_result = return_annotation.map(|ty| {
                    let ty = self.syntax_type(ty);
                    self.inference.freshen_omitted_array_shapes(ty)
                });
                let is_generator = crate::typeck::control_flow::expression_contains_yield(body);
                let generator_item = is_generator
                    .then(|| {
                        annotated_result.and_then(|result| match self.shallow_type(result) {
                            Type::Iterator(iterator) => {
                                Some(self.inference.iterator_item(iterator))
                            }
                            Type::Known(result) => match self.inference.type_store().kind(result) {
                                TypeKind::Iterator { item, .. } => Some(Type::Known(*item)),
                                _ => None,
                            },
                            _ => None,
                        })
                    })
                    .flatten();
                let annotated_completion =
                    annotated_result.map(|result| match self.shallow_type(result) {
                        Type::Async(future) => self.inference.async_value(future),
                        Type::Iterator(_) if is_generator => {
                            self.core_type(crate::stdlib::CoreTypeId::None)
                        }
                        result => result,
                    });
                let completion = annotated_completion
                    .or_else(|| {
                        hinted
                            .as_ref()
                            .map(|(_, result)| match self.shallow_type(*result) {
                                Type::Async(future) => self.inference.async_value(future),
                                result => result,
                            })
                    })
                    .unwrap_or_else(|| self.fresh_inference(Requirements::none(), None));
                let is_async = generator_item.is_none()
                    && (annotated_result
                        .is_some_and(|result| matches!(self.shallow_type(result), Type::Async(_)))
                        || crate::typeck::control_flow::expression_contains_suspension(body));
                let result = if generator_item.is_some() {
                    annotated_result.expect("generator closures have explicit iterator results")
                } else if is_async {
                    match annotated_result {
                        Some(result @ Type::Async(_)) => result,
                        _ => Type::Async(self.inference.async_type(completion)),
                    }
                } else {
                    completion
                };
                if let Some(annotation) = annotated_result {
                    self.unify(
                        result,
                        annotation,
                        return_annotation_span.unwrap_or(expr.span),
                    );
                }
                if let Some((_, expected_result)) = hinted {
                    self.unify(result, expected_result, expr.span);
                }

                self.scopes.push(HashMap::new());
                for (parameter, ty) in params.iter().zip(parameter_types.iter().copied()) {
                    self.bind_irrefutable_parameter(
                        &parameter.binding,
                        ty,
                        self.debug_context.is_debug(),
                        "closure parameter",
                        "closure",
                    );
                }
                let failure = match self.shallow_type(completion) {
                    result @ Type::Result(_) => super::context::FailureContext::boundary(result),
                    _ => super::context::FailureContext::None,
                };
                let return_type_source = return_annotation_span.map(|span| {
                    let result = self.type_name(completion);
                    super::ExpectedTypeSource {
                        span,
                        label: format!("closure is declared to return `{result}`"),
                    }
                });
                // A closure lexically inside an injected library body keeps
                // access to its private implementation operations. Ordinary
                // user closures still enter the unprivileged closure context.
                let closure_context = if self.is_library_function() {
                    super::context::CallableContext::CompilerGenerated
                } else {
                    super::context::CallableContext::Closure
                };
                self.with_return_type_source(return_type_source.clone(), |checker| {
                    checker.with_callable_context(
                        closure_context,
                        completion,
                        failure,
                        |checker| {
                            checker.with_generator_item(generator_item, |checker| {
                                if let Some(source) = return_type_source {
                                    checker.with_expected_type_source(source, |checker| {
                                        checker.expr(body, Some(completion));
                                    });
                                } else {
                                    checker.expr(body, Some(completion));
                                }
                            });
                        },
                    );
                });
                self.scopes.pop();
                let callable =
                    Type::Callable(self.inference.callable_type(parameter_types, result));
                self.expect_expression(expr.id, callable, expected, expr.span)?
            }
        };
        match &expr.kind {
            ExprKind::Bool(value) => {
                self.condition_flows
                    .insert(expr.id, ConditionFlow::literal(*value));
            }
            ExprKind::Unary {
                op: UnaryOp::Not,
                expr: inner,
            } => {
                let flow = self
                    .condition_flows
                    .get(&inner.id)
                    .cloned()
                    .unwrap_or_else(ConditionFlow::unknown)
                    .negated();
                self.condition_flows.insert(expr.id, flow);
            }
            _ => {}
        }
        self.semantics.resolve_expression_type(expr.id, ty);
        Some(ty)
    }

    /// Whether a contextual wrapper type has an optional success value to
    /// which a bare `None` can flow.
    ///
    /// Result layers are transparent here because an ordinary value is lifted
    /// into their successful branch. The first option layer consumes `None` as
    /// absence. This lets implicit failure boundaries contextualize `None`
    /// without inventing a special state-field conversion.
    fn expected_accepts_none(&mut self, expected: Type) -> bool {
        match self.shallow_type(expected) {
            Type::Option(_) => true,
            Type::Result(result) => {
                let value = self.inference.result_value(result);
                self.expected_accepts_none(value)
            }
            _ => false,
        }
    }

    fn invoke_expression(
        &mut self,
        callee: &Expr,
        arguments: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        let parameters = (0..arguments.len())
            .map(|_| self.fresh_inference(Requirements::none(), None))
            .collect::<Vec<_>>();
        let result = expected.unwrap_or_else(|| self.fresh_inference(Requirements::none(), None));
        let callable = Type::Callable(self.inference.callable_type(parameters.clone(), result));
        let callee_type = self.expr(callee, Some(callable))?;
        self.unify(callee_type, callable, callee.span)?;
        for (argument, parameter) in arguments.iter().zip(parameters) {
            self.expr(argument, Some(parameter));
        }
        // The backend resolves this expression as a dynamic call after capture
        // analysis has selected its closure representation.
        self.semantics.resolve_dynamic_call(
            expression,
            crate::semantic::DynamicCallCallee::Expression(callee.id),
        );
        self.expect_expression(expression, result, expected, span)
    }

    fn invoke_callable_type(
        &mut self,
        callable: crate::ast::CallableTypeId,
        arguments: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        let parameters = self.inference.callable_parameters(callable).to_vec();
        if parameters.len() != arguments.len() {
            self.error(
                format!(
                    "callable expects {} arguments, found {}",
                    parameters.len(),
                    arguments.len()
                ),
                span,
            );
            return None;
        }
        for (argument, parameter) in arguments.iter().zip(parameters) {
            self.expr(argument, Some(parameter));
        }
        let result = self.inference.callable_result(callable);
        self.expect_expression(expression, result, expected, span)
    }

    fn enum_constructor(
        &mut self,
        expression: ExprId,
        enumeration: EnumTypeId,
        variant: &str,
        arguments: &[Expr],
        expected: Option<Type>,
        span: Span,
    ) -> Option<Type> {
        let Some(declaration) = self.enum_info(enumeration) else {
            self.error("unknown enum type", span);
            return None;
        };
        let Some(declared_variant) = declaration
            .variants
            .iter()
            .find(|declared| declared.name == variant)
        else {
            self.error(
                format!("enum `{}` has no variant `{variant}`", declaration.name),
                span,
            );
            return None;
        };
        self.semantics
            .resolve_enum_variant(expression, declared_variant.id);
        match (declared_variant.payload, arguments.first()) {
            (Some(payload_type), Some(payload)) => {
                if let Some(source_span) = declared_variant.source_span {
                    let payload_type_name = self.type_name(payload_type);
                    self.with_expected_type_source(
                        super::ExpectedTypeSource {
                            span: source_span,
                            label: format!(
                                "variant `{}.{variant}` declares a payload of type `{payload_type_name}`",
                                declaration.name,
                            ),
                        },
                        |checker| checker.expr(payload, Some(payload_type)),
                    );
                } else {
                    self.expr(payload, Some(payload_type));
                }
            }
            (Some(_), None) => self.error(format!("variant `{variant}` requires a payload"), span),
            (None, Some(payload)) => {
                self.expr(payload, None);
                self.error(
                    format!("variant `{variant}` does not accept a payload"),
                    span,
                );
            }
            (None, None) => {}
        }
        for extra in arguments.iter().skip(1) {
            self.expr(extra, None);
        }
        self.expect_expression(expression, self.enum_type(enumeration), expected, span)
    }

    fn infer_option_pattern(
        &mut self,
        value_type: Type,
        span: Span,
        requirement: &str,
    ) -> Option<crate::ast::OptionTypeId> {
        match self.shallow_type(value_type) {
            Type::Option(option) => Some(option),
            Type::Variable(_) => {
                let value = self.fresh_inference(Requirements::none(), None);
                let option = self.inference.option_type(value);
                self.unify(value_type, Type::Option(option), span)?;
                Some(option)
            }
            ty => {
                let ty = self.type_name(ty);
                self.error(format!("{requirement}, found `{ty}`"), span);
                None
            }
        }
    }

    fn infer_result_pattern(
        &mut self,
        value_type: Type,
        span: Span,
        requirement: &str,
    ) -> Option<crate::ast::ResultTypeId> {
        match self.shallow_type(value_type) {
            Type::Result(result) => Some(result),
            Type::Variable(_) => {
                let value = self.fresh_inference(Requirements::none(), None);
                let result = self.inference.result_type(value);
                self.unify(value_type, Type::Result(result), span)?;
                Some(result)
            }
            ty => {
                let ty = self.type_name(ty);
                self.error(format!("{requirement}, found `{ty}`"), span);
                None
            }
        }
    }

    fn infer_iterator_step_pattern(
        &mut self,
        value_type: Type,
        span: Span,
        requirement: &str,
    ) -> Option<(crate::ast::TypeApplicationId, Type)> {
        match self.shallow_type(value_type) {
            Type::Application(step)
                if self.inference.application_constructor(step)
                    == crate::stdlib::StdlibTypeConstructorId::IteratorStep =>
            {
                Some((step, self.inference.application_arguments(step)[0]))
            }
            Type::Variable(_) => {
                let item = self.fresh_inference(Requirements::none(), None);
                let step = self.inference.application_type(
                    crate::stdlib::StdlibTypeConstructorId::IteratorStep,
                    vec![item],
                );
                self.unify(value_type, Type::Application(step), span)?;
                Some((step, item))
            }
            ty => {
                let ty = self.type_name(ty);
                self.error(format!("{requirement}, found `{ty}`"), span);
                None
            }
        }
    }

    fn check_array_pattern(
        &mut self,
        pattern: &crate::ast::ArrayPattern,
        pattern_id: crate::ast::PatternId,
        value_type: Type,
        span: Span,
    ) -> CheckedPattern {
        let Type::Array(array) = self.shallow_type(value_type) else {
            let ty = self.type_name(value_type);
            self.error(
                format!("an array pattern requires an array value, found `{ty}`"),
                span,
            );
            return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
        };
        let element_type = self.inference.array_element(array);
        let fixed_length = self.inference.array_length(array);
        let explicit_length = pattern.prefix.len() + pattern.suffix.len();
        if let Some(length) = fixed_length
            && if pattern.rest.is_some() {
                explicit_length > length as usize
            } else {
                explicit_length != length as usize
            }
        {
            let array_name = self.type_name(value_type);
            let message = if pattern.rest.is_some() {
                format!(
                    "array rest pattern requires at least {explicit_length} elements, but `{array_name}` has exactly {length}"
                )
            } else {
                format!(
                    "array pattern has {explicit_length} elements, but `{array_name}` requires exactly {length}"
                )
            };
            self.error(message, span);
        }
        let mut prefix = Vec::with_capacity(pattern.prefix.len());
        for element in &pattern.prefix {
            let checked = self.check_pattern(&element.kind, element.id, element_type, element.span);
            prefix.push(checked.coverage);
        }
        let mut suffix = Vec::with_capacity(pattern.suffix.len());
        for element in &pattern.suffix {
            let checked = self.check_pattern(&element.kind, element.id, element_type, element.span);
            suffix.push(checked.coverage);
        }
        CheckedPattern::new(PatternCoverage::Array {
            prefix,
            rest: pattern.rest.is_some(),
            suffix,
        })
    }

    fn check_alternation_pattern(
        &mut self,
        alternatives: &[crate::ast::PatternNode],
        value_type: Type,
    ) -> CheckedPattern {
        let base_scope = self
            .scopes
            .last()
            .expect("patterns are checked inside an arm scope")
            .clone();
        let mut expected_names = None::<HashSet<String>>;
        let mut arm_bindings = HashMap::<String, Binding>::new();
        let mut coverage = Vec::with_capacity(alternatives.len());
        let mut shape_variants = Some(HashSet::new());

        for alternative in alternatives {
            *self.scopes.last_mut().unwrap() = base_scope.clone();
            let checked = self.check_pattern(
                &alternative.kind,
                alternative.id,
                value_type,
                alternative.span,
            );
            if !self.pattern_is_useful(&coverage, &checked.coverage, value_type) {
                self.error(
                    format!("unreachable alternative `{}`", checked.coverage.display()),
                    alternative.span,
                );
            }

            let mut names = HashSet::new();
            alternative.kind.visit_bindings(&mut |binding| {
                names.insert(binding.name.clone());
            });
            if let Some(expected) = &expected_names {
                for missing in expected.difference(&names) {
                    self.error(
                        format!("this alternative does not bind `{missing}`"),
                        alternative.span,
                    );
                }
                for extra in names.difference(expected) {
                    self.error(
                        format!(
                            "this alternative binds `{extra}`, but the other alternatives do not"
                        ),
                        alternative.span,
                    );
                }
            } else {
                expected_names = Some(names.clone());
                for name in &names {
                    if let Some(binding) = self.scopes.last().unwrap().get(name).copied() {
                        arm_bindings.insert(name.clone(), binding);
                    }
                }
            }

            match (&mut shape_variants, checked.shape_variants) {
                (Some(all), Some(current)) => all.extend(current),
                (slot, None) => *slot = None,
                (None, Some(_)) => {}
            }
            coverage.push(checked.coverage);
        }

        *self.scopes.last_mut().unwrap() = base_scope;
        self.scopes.last_mut().unwrap().extend(arm_bindings);

        let coverage = if coverage.iter().any(PatternCoverage::is_irrefutable) {
            PatternCoverage::Irrefutable
        } else {
            PatternCoverage::Alternation(coverage)
        };
        CheckedPattern {
            coverage,
            shape_variants,
        }
    }

    fn integer_pattern_bound_type(
        &mut self,
        value: u64,
        negative: bool,
        suffix: Option<crate::ast::TypeRef>,
        span: Span,
    ) -> Type {
        if let Some(suffix) = suffix {
            if !suffix.is_integer() {
                self.error("integer match patterns require an integer type", span);
            } else if !(if negative {
                self.inference
                    .fits_negative_literal(value, self.syntax_type(suffix))
            } else {
                self.inference
                    .fits_unsigned_literal(value, self.syntax_type(suffix))
            }) {
                self.error(format!("integer literal does not fit in `{suffix}`"), span);
            }
            self.syntax_type(suffix)
        } else if negative {
            self.inference.fresh_negative_integer_literal(
                Requirements::capability(StdlibCapabilityId::Integer),
                value,
            )
        } else {
            self.fresh_inference(
                Requirements::capability(StdlibCapabilityId::Integer),
                Some(value),
            )
        }
    }

    pub(super) fn check_pattern(
        &mut self,
        pattern: &MatchPattern,
        pattern_id: crate::ast::PatternId,
        value_type: Type,
        span: Span,
    ) -> CheckedPattern {
        match pattern {
            MatchPattern::Struct { name, fields, .. } => {
                let contextual = self.shallow_type(value_type);
                let application = match contextual {
                    Type::Application(application) => Some(application),
                    Type::Known(id) => match self.inference.type_store().kind(id) {
                        TypeKind::Application { layout, .. } => Some(*layout),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(application) = application {
                    let constructor = self.inference.application_constructor(application);
                    let declaration = self.standard_library.type_constructor(constructor);
                    if name
                        .as_ref()
                        .is_some_and(|name| name.as_str() != declaration.name)
                    {
                        // A contextual generic struct only applies when an
                        // explicit name agrees with that context. Continue
                        // into ordinary source-struct diagnostics otherwise.
                    } else {
                        let declared_fields = self
                            .standard_library
                            .fields_of_constructor(constructor)
                            .filter(|field| {
                                field.visibility == crate::stdlib::FieldVisibility::Public
                            })
                            .copied()
                            .collect::<Vec<_>>();
                        if !declared_fields.is_empty() {
                            let arguments =
                                self.inference.application_arguments(application).to_vec();
                            let variables = declaration
                                .parameters
                                .iter()
                                .zip(arguments)
                                .map(|(parameter, argument)| (parameter.name, argument))
                                .collect::<HashMap<_, _>>();
                            self.semantics.resolve_struct_pattern(
                                pattern_id,
                                ResolvedStructId::StandardConstructor(application),
                            );
                            let mut seen = HashSet::new();
                            let mut resolved_fields = Vec::with_capacity(fields.len());
                            let mut coverage = Vec::with_capacity(fields.len());
                            for pattern_field in fields {
                                if !seen.insert(pattern_field.name.clone()) {
                                    self.error(
                                        format!(
                                            "duplicate struct pattern field `{}`",
                                            pattern_field.name
                                        ),
                                        pattern_field.name_span,
                                    );
                                    continue;
                                }
                                let Some(field) = declared_fields
                                    .iter()
                                    .find(|field| field.name == pattern_field.name)
                                else {
                                    self.error(
                                        format!(
                                            "struct `{}` has no field `{}`",
                                            declaration.name, pattern_field.name
                                        ),
                                        pattern_field.name_span,
                                    );
                                    continue;
                                };
                                let field_type = self.catalog_type(field.ty, &variables);
                                let checked = self.check_pattern(
                                    &pattern_field.pattern.kind,
                                    pattern_field.pattern.id,
                                    field_type,
                                    pattern_field.pattern.span,
                                );
                                resolved_fields.push(ResolvedStructFieldId::Standard(field.id));
                                coverage.push((
                                    ResolvedStructFieldId::Standard(field.id),
                                    field.name.to_owned(),
                                    checked.coverage,
                                ));
                            }
                            self.semantics
                                .resolve_struct_pattern_fields(pattern_id, resolved_fields);
                            coverage.sort_by_key(|(field, _, _)| {
                                declared_fields
                                    .iter()
                                    .position(|declared| {
                                        *field == ResolvedStructFieldId::Standard(declared.id)
                                    })
                                    .unwrap_or(usize::MAX)
                            });
                            return CheckedPattern::new(PatternCoverage::Struct {
                                structure: ResolvedStructId::StandardConstructor(application),
                                name: declaration.name.to_owned(),
                                fields: coverage,
                            });
                        }
                    }
                }
                let declaration = if let Some(name) = name {
                    let Some(declaration) = self
                        .declarations
                        .structs
                        .iter()
                        .find(|declaration| declaration.name == *name)
                        .cloned()
                    else {
                        self.error(format!("unknown struct type `{name}`"), span);
                        self.check_invalid_struct_pattern_fields(fields);
                        return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                    };
                    self.unify(value_type, self.struct_type(declaration.id), span);
                    declaration
                } else {
                    let contextual = self.shallow_type(value_type);
                    let structure = match contextual {
                        Type::Known(id) => match self.inference.type_store().kind(id) {
                            TypeKind::Struct(structure) => Some(*structure),
                            TypeKind::Error => None,
                            _ => {
                                let actual = self.type_name(contextual);
                                self.errors.push(
                                    Diagnostic::type_error(
                                        format!(
                                            "cannot destructure `{actual}` with an anonymous struct pattern"
                                        ),
                                        span,
                                    )
                                    .with_primary_label("this context does not contain a struct value")
                                    .with_note(
                                        "anonymous `{ field }` patterns remain nominal and require one concrete struct type",
                                    ),
                                );
                                None
                            }
                        },
                        _ => {
                            let field_names = fields
                                .iter()
                                .map(|field| field.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ");
                            self.errors.push(
                                Diagnostic::type_error(
                                    "anonymous struct pattern needs a known struct type",
                                    span,
                                )
                                .with_primary_label(
                                    "the surrounding context does not identify a concrete struct",
                                )
                                .with_note(format!(
                                    "add an annotation such as `{{ {field_names} }}: Pos`, or write `Pos {{ {field_names} }}`"
                                ))
                                .with_note(
                                    "anonymous struct patterns are nominal; field names alone never infer a struct type",
                                ),
                            );
                            None
                        }
                    };
                    let Some(structure) = structure else {
                        self.check_invalid_struct_pattern_fields(fields);
                        return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                    };
                    self.declarations
                        .structs
                        .iter()
                        .find(|declaration| declaration.id == structure)
                        .cloned()
                        .expect("contextual source struct types have declarations")
                };
                self.semantics
                    .resolve_struct_pattern(pattern_id, ResolvedStructId::Source(declaration.id));
                let mut seen = HashSet::new();
                let mut resolved_fields = Vec::with_capacity(fields.len());
                let mut coverage = Vec::with_capacity(fields.len());
                for pattern_field in fields {
                    if !seen.insert(pattern_field.name.clone()) {
                        self.error(
                            format!("duplicate struct pattern field `{}`", pattern_field.name),
                            pattern_field.name_span,
                        );
                        continue;
                    }
                    let Some(field) = declaration
                        .fields
                        .iter()
                        .find(|field| field.name == pattern_field.name)
                    else {
                        self.error(
                            format!(
                                "struct `{}` has no field `{}`",
                                declaration.name, pattern_field.name
                            ),
                            pattern_field.name_span,
                        );
                        continue;
                    };
                    let checked = self.check_pattern(
                        &pattern_field.pattern.kind,
                        pattern_field.pattern.id,
                        self.syntax_type(field.ty),
                        pattern_field.pattern.span,
                    );
                    resolved_fields.push(ResolvedStructFieldId::Source(field.id));
                    coverage.push((field.id, field.name.clone(), checked.coverage));
                }
                self.semantics
                    .resolve_struct_pattern_fields(pattern_id, resolved_fields);
                // Field order is not semantically significant. Canonicalize
                // coverage so duplicate and unreachable-arm diagnostics do
                // not depend on the order chosen in source.
                coverage.sort_by_key(|(field, _, _)| *field);
                CheckedPattern::new(PatternCoverage::Struct {
                    structure: ResolvedStructId::Source(declaration.id),
                    name: declaration.name,
                    fields: coverage
                        .into_iter()
                        .map(|(field, name, coverage)| {
                            (ResolvedStructFieldId::Source(field), name, coverage)
                        })
                        .collect(),
                })
            }
            MatchPattern::Enum {
                variant, payload, ..
            } => {
                let Some(enumeration) = self.resolutions.pattern_enum(pattern_id) else {
                    self.error("unresolved enum type", span);
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.unify(value_type, self.enum_type(enumeration), span);
                let mut resolved_variant = None;
                let mut declared_payload = None;
                if let Some(declaration) = self.enum_info(enumeration) {
                    if let Some(declared_variant) = declaration
                        .variants
                        .iter()
                        .find(|declared| declared.name == *variant)
                    {
                        resolved_variant = Some(declared_variant.id);
                        declared_payload = declared_variant.payload;
                    } else {
                        self.error(
                            format!("enum `{}` has no variant `{variant}`", declaration.name),
                            span,
                        );
                    }
                } else {
                    self.error("unknown enum type", span);
                }
                let Some(resolved_variant) = resolved_variant else {
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.semantics
                    .resolve_pattern_variant(pattern_id, resolved_variant);
                let payload_coverage = match (declared_payload, payload) {
                    (Some(payload_type), Some(payload)) => {
                        self.check_pattern(&payload.kind, payload.id, payload_type, payload.span)
                            .coverage
                    }
                    (None, Some(payload)) => {
                        self.error(
                            format!("variant `{variant}` has no payload to match"),
                            payload.span,
                        );
                        PatternCoverage::Invalid(payload.id)
                    }
                    _ => PatternCoverage::Irrefutable,
                };
                CheckedPattern {
                    coverage: PatternCoverage::Enum {
                        variant: resolved_variant,
                        name: variant.clone(),
                        payload: Box::new(payload_coverage),
                    },
                    shape_variants: match resolved_variant {
                        ResolvedEnumVariantId::Source(variant) => Some(HashSet::from([variant])),
                        ResolvedEnumVariantId::Standard(_) => None,
                    },
                }
            }
            MatchPattern::Bool(value) => {
                self.unify(
                    value_type,
                    self.core_type(crate::stdlib::CoreTypeId::Bool),
                    span,
                );
                CheckedPattern::new(PatternCoverage::Bool(*value))
            }
            MatchPattern::Char(value) => {
                self.unify(
                    value_type,
                    self.core_type(crate::stdlib::CoreTypeId::Char),
                    span,
                );
                CheckedPattern::new(PatternCoverage::Char(*value))
            }
            MatchPattern::String(value) => {
                self.unify(value_type, self.standard_type(StdlibTypeId::String), span);
                CheckedPattern::new(PatternCoverage::String(value.clone()))
            }
            MatchPattern::Int {
                value,
                negative,
                suffix,
            } => {
                let pattern_type =
                    self.integer_pattern_bound_type(*value, *negative, *suffix, span);
                self.unify(value_type, pattern_type, span);
                CheckedPattern::new(PatternCoverage::Int {
                    value: *value,
                    negative: *negative,
                })
            }
            MatchPattern::IntRange {
                start,
                start_negative,
                start_suffix,
                start_span,
                end,
                end_negative,
                end_suffix,
                end_span,
                kind,
                ..
            } => {
                let start_type = self.integer_pattern_bound_type(
                    *start,
                    *start_negative,
                    *start_suffix,
                    *start_span,
                );
                let end_type =
                    self.integer_pattern_bound_type(*end, *end_negative, *end_suffix, *end_span);
                self.unify(value_type, start_type, *start_span);
                self.unify(value_type, end_type, *end_span);
                let start = signed_pattern_integer(*start, *start_negative);
                let end = signed_pattern_integer(*end, *end_negative);
                let empty = match kind {
                    crate::ast::RangeKind::Exclusive => start >= end,
                    crate::ast::RangeKind::Inclusive => start > end,
                };
                if empty {
                    self.error("integer range pattern matches no values", span);
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                }
                CheckedPattern::new(PatternCoverage::IntRange {
                    start,
                    end,
                    kind: *kind,
                })
            }
            MatchPattern::FileVersion(components) => {
                self.unify(
                    value_type,
                    self.standard_type(StdlibTypeId::FileVersion),
                    span,
                );
                CheckedPattern::new(PatternCoverage::FileVersion(*components))
            }
            MatchPattern::None => {
                let Some(option) = self.infer_option_pattern(
                    value_type,
                    span,
                    "a `None` pattern requires an optional value",
                ) else {
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.semantics.resolve_wrapper_pattern(
                    pattern_id,
                    ResolvedWrapperPattern::OptionNone(option),
                );
                CheckedPattern::new(PatternCoverage::OptionNone)
            }
            MatchPattern::OptionSome(payload) => {
                let Some(option) = self.infer_option_pattern(
                    value_type,
                    span,
                    "a `Some(value)` pattern requires an optional value",
                ) else {
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.semantics.resolve_wrapper_pattern(
                    pattern_id,
                    ResolvedWrapperPattern::OptionSome(option),
                );
                let checked = self.check_pattern(
                    &payload.kind,
                    payload.id,
                    self.inference.option_value(option),
                    payload.span,
                );
                CheckedPattern::new(PatternCoverage::OptionSome(Box::new(checked.coverage)))
            }
            MatchPattern::IteratorEnd => {
                let Some((step, _)) = self.infer_iterator_step_pattern(
                    value_type,
                    span,
                    "an `End` pattern requires an iterator step",
                ) else {
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.semantics
                    .resolve_wrapper_pattern(pattern_id, ResolvedWrapperPattern::IteratorEnd(step));
                CheckedPattern::new(PatternCoverage::IteratorEnd)
            }
            MatchPattern::IteratorItem(payload) => {
                let Some((step, item)) = self.infer_iterator_step_pattern(
                    value_type,
                    span,
                    "an `Item(value)` pattern requires an iterator step",
                ) else {
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.semantics.resolve_wrapper_pattern(
                    pattern_id,
                    ResolvedWrapperPattern::IteratorItem(step),
                );
                let checked = self.check_pattern(&payload.kind, payload.id, item, payload.span);
                CheckedPattern::new(PatternCoverage::IteratorItem(Box::new(checked.coverage)))
            }
            MatchPattern::ResultSuccess(payload) => {
                let Some(result) = self.infer_result_pattern(
                    value_type,
                    span,
                    "an `Ok(value)` pattern requires a result value",
                ) else {
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.semantics.resolve_wrapper_pattern(
                    pattern_id,
                    ResolvedWrapperPattern::ResultSuccess(result),
                );
                let checked = self.check_pattern(
                    &payload.kind,
                    payload.id,
                    self.inference.result_value(result),
                    payload.span,
                );
                CheckedPattern::new(PatternCoverage::ResultSuccess(Box::new(checked.coverage)))
            }
            MatchPattern::ResultError(payload) => {
                let Some(result) = self.infer_result_pattern(
                    value_type,
                    span,
                    "an `Err(error)` pattern requires a result value",
                ) else {
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.semantics.resolve_wrapper_pattern(
                    pattern_id,
                    ResolvedWrapperPattern::ResultError(result),
                );
                let checked = self.check_pattern(
                    &payload.kind,
                    payload.id,
                    self.standard_type(StdlibTypeId::String),
                    payload.span,
                );
                CheckedPattern::new(PatternCoverage::ResultError(Box::new(checked.coverage)))
            }
            MatchPattern::Array(array) => {
                self.check_array_pattern(array, pattern_id, value_type, span)
            }
            MatchPattern::Alternation(alternatives) => {
                self.check_alternation_pattern(alternatives, value_type)
            }
            MatchPattern::Binding(binding) => {
                self.bind_pattern_value(binding, value_type, span);
                CheckedPattern::new(PatternCoverage::Irrefutable)
            }
            MatchPattern::Wildcard => CheckedPattern::new(PatternCoverage::Irrefutable),
        }
    }

    fn check_invalid_struct_pattern_fields(&mut self, fields: &[crate::ast::StructPatternField]) {
        let error = self.error_type();
        for field in fields {
            self.check_pattern(
                &field.pattern.kind,
                field.pattern.id,
                error,
                field.pattern.span,
            );
        }
    }

    pub(super) fn binary(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        if matches!(op, BinaryOp::Or | BinaryOp::And) {
            let bool_type = self.core_type(crate::stdlib::CoreTypeId::Bool);
            self.expr(left, Some(bool_type));
            let left_flow = self
                .condition_flows
                .get(&left.id)
                .cloned()
                .unwrap_or_else(ConditionFlow::unknown);
            let (bindings, constraints) = match op {
                BinaryOp::And => (
                    left_flow.when_true.as_ref(),
                    self.truthy_shape_constraints(left),
                ),
                BinaryOp::Or => (
                    left_flow.when_false.as_ref(),
                    self.falsy_shape_constraints(left),
                ),
                _ => unreachable!("logical operators were matched above"),
            };
            self.with_condition_path(bindings, |checker| {
                checker.with_shape_constraints(Some(&constraints), |checker| {
                    checker.expr(right, Some(bool_type));
                });
            });
            let right_flow = self
                .condition_flows
                .get(&right.id)
                .cloned()
                .unwrap_or_else(ConditionFlow::unknown);
            let flow = match op {
                BinaryOp::And => ConditionFlow {
                    when_true: sequence_condition_paths(
                        &left_flow.when_true,
                        &right_flow.when_true,
                    ),
                    when_false: join_condition_paths(
                        &left_flow.when_false,
                        &sequence_condition_paths(&left_flow.when_true, &right_flow.when_false),
                    ),
                },
                BinaryOp::Or => ConditionFlow {
                    when_true: join_condition_paths(
                        &left_flow.when_true,
                        &sequence_condition_paths(&left_flow.when_false, &right_flow.when_true),
                    ),
                    when_false: sequence_condition_paths(
                        &left_flow.when_false,
                        &right_flow.when_false,
                    ),
                },
                _ => unreachable!("logical operators were matched above"),
            };
            self.condition_flows.insert(expression, flow);
            return self.expect_expression(expression, bool_type, expected, span);
        }

        let result_is_bool = matches!(
            op,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        );
        let operand_hint = if result_is_bool {
            None
        } else {
            expected.map(|ty| self.expected_value_type(ty))
        };
        // `None` has no payload from which to infer its `T?` type. Equality,
        // however, supplies exactly that context through the opposite operand.
        // Check the informative side first without otherwise making binary
        // expression inference depend on operand order.
        let (left_ty, right_ty) = match (&left.kind, &right.kind) {
            (ExprKind::None, ExprKind::None) => {
                let left_ty = self.expr(left, operand_hint)?;
                let right_ty = self.expr(right, operand_hint)?;
                (left_ty, right_ty)
            }
            (ExprKind::None, _) if matches!(op, BinaryOp::Eq | BinaryOp::Ne) => {
                let right_ty = self.expr(right, operand_hint)?;
                let left_ty = self.expr(left, Some(right_ty))?;
                (left_ty, right_ty)
            }
            (_, ExprKind::None) if matches!(op, BinaryOp::Eq | BinaryOp::Ne) => {
                let left_ty = self.expr(left, operand_hint)?;
                let right_ty = self.expr(right, Some(left_ty))?;
                (left_ty, right_ty)
            }
            (ExprKind::Array(_), _) if matches!(op, BinaryOp::Eq | BinaryOp::Ne) => {
                let right_ty = self.expr(right, operand_hint)?;
                let left_ty = self.expr(left, Some(right_ty))?;
                (left_ty, right_ty)
            }
            (_, ExprKind::Array(_)) if matches!(op, BinaryOp::Eq | BinaryOp::Ne) => {
                let left_ty = self.expr(left, operand_hint)?;
                let right_ty = self.expr(right, Some(left_ty))?;
                (left_ty, right_ty)
            }
            _ => {
                let left_ty = self.expr(left, operand_hint)?;
                // A catalog-defined operator may accept a different right-hand
                // type, such as `Instant + Duration`. The expected result can
                // guide the receiver; the operator declaration guides this
                // expression once a candidate has been selected.
                let right_ty = self.expr(right, None)?;
                (left_ty, right_ty)
            }
        };

        if self.is_error_type(left_ty) || self.is_error_type(right_ty) {
            let result = if result_is_bool {
                self.core_type(crate::stdlib::CoreTypeId::Bool)
            } else {
                self.error_type()
            };
            return self.expect_expression(expression, result, expected, span);
        }

        if self.diagnose_string_addition(op, left_ty, right_ty, span) {
            let result = self.error_type();
            return self.expect_expression(expression, result, expected, span);
        }

        if let Some(result) =
            self.resolve_binary_operator(op, left_ty, right_ty, expression, left.id, span)
        {
            return self.expect_expression(expression, result, expected, span);
        }

        let operand_ty = self.unify(left_ty, right_ty, span)?;

        self.require_binary_operand(op, operand_ty, span)?;

        let result = if result_is_bool {
            self.core_type(crate::stdlib::CoreTypeId::Bool)
        } else {
            operand_ty
        };
        self.expect_expression(expression, result, expected, span)
    }

    pub(super) fn diagnose_string_addition(
        &mut self,
        op: BinaryOp,
        left: Type,
        right: Type,
        span: Span,
    ) -> bool {
        if op != BinaryOp::Add {
            return false;
        }
        let left = self.shallow_type(left);
        let right = self.shallow_type(right);
        if self.standard_type_id(left) != Some(StdlibTypeId::String)
            && self.standard_type_id(right) != Some(StdlibTypeId::String)
        {
            return false;
        }

        self.errors.push(
            Diagnostic::type_error("`+` does not concatenate strings in SplitScript", span)
                .with_primary_label("construct the string explicitly")
                .with_note(
                    "use a template literal such as `{left}{right}` when joining values; interpolation accepts any `Display` value",
                )
                .with_note(
                    "use `String.concat(values)` when the inputs are already stored in a `[String]`",
                )
                .with_note(
                    "no automatic rewrite is offered because the intended text, separators, and evaluation grouping belong to the author",
                ),
        );
        true
    }

    pub(super) fn diagnose_string_compound_assignment(
        &mut self,
        op: BinaryOp,
        left: Type,
        right: Type,
        span: Span,
    ) -> bool {
        let left = self.shallow_type(left);
        self.standard_type_id(left) == Some(StdlibTypeId::String)
            && self.diagnose_string_addition(op, left, right, span)
    }

    pub(super) fn require_binary_operand(
        &mut self,
        op: BinaryOp,
        operand_ty: Type,
        span: Span,
    ) -> Option<()> {
        match op {
            BinaryOp::Eq | BinaryOp::Ne => self.require(
                operand_ty,
                Requirements::capability(StdlibCapabilityId::Equatable),
                span,
            ),
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => self.require(
                operand_ty,
                Requirements::capability(StdlibCapabilityId::Numeric),
                span,
            ),
            BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::BitAnd
            | BinaryOp::Shl
            | BinaryOp::Shr => self.require(
                operand_ty,
                Requirements::capability(StdlibCapabilityId::Integer),
                span,
            ),
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => self
                .require(
                    operand_ty,
                    if op == BinaryOp::Rem {
                        Requirements::capability(StdlibCapabilityId::Integer)
                    } else {
                        Requirements::capability(StdlibCapabilityId::Numeric)
                    },
                    span,
                ),
            BinaryOp::Or | BinaryOp::And => {
                unreachable!("logical operators are checked separately")
            }
        }
    }

    pub(super) fn indexed_element_type(
        &mut self,
        receiver: &Expr,
        index: &Expr,
        expression: ExprId,
        bracket_span: Span,
    ) -> Option<Type> {
        let receiver_ty = self.expr(receiver, None)?;
        if self.is_error_type(receiver_ty) {
            self.expr(index, None);
            return Some(receiver_ty);
        }
        if self.diagnose_unhandled_result(receiver_ty, "indexing it", bracket_span) {
            self.expr(index, None);
            return None;
        }
        let receiver_ty = self.shallow_type(receiver_ty);
        if matches!(
            receiver_ty,
            Type::Known(id)
                if matches!(self.inference.type_store().kind(id), TypeKind::SettingsView)
        ) {
            self.expr(index, Some(self.standard_type(StdlibTypeId::String)));
            let metadata = migration_diagnostic(ASL_SETTINGS_LOOKUP_DIAGNOSTIC)
                .expect("type checker migration diagnostic IDs must exist");
            let opening = Span {
                start: bracket_span.start,
                end: bracket_span.start + 1,
            };
            let closing = Span {
                start: bracket_span.end - 1,
                end: bracket_span.end,
            };
            let mut diagnostic = Diagnostic::type_error(metadata.message, bracket_span)
                .with_primary_label(metadata.primary_label)
                .with_migration_topic(metadata.concept.as_str())
                .with_fix(DiagnosticFix {
                    title: "replace indexed lookup with `enabled`".to_owned(),
                    applicability: FixApplicability::MachineApplicable,
                    edits: vec![
                        TextEdit {
                            span: opening,
                            replacement: ".enabled(".to_owned(),
                        },
                        TextEdit {
                            span: closing,
                            replacement: ")".to_owned(),
                        },
                    ],
                });
            for note in metadata.notes {
                diagnostic = diagnostic.with_note(*note);
            }
            self.errors.push(diagnostic);
            return None;
        }
        let element = match receiver_ty {
            Type::Array(array) => self.inference.array_element(array),
            Type::Known(id) => match self.inference.type_store().kind(id) {
                crate::types::TypeKind::Array { element, .. } => Type::Known(*element),
                _ => {
                    return self.resolve_index_getter(
                        receiver_ty,
                        receiver.id,
                        index,
                        expression,
                        bracket_span,
                    );
                }
            },
            _ => {
                return self.resolve_index_getter(
                    receiver_ty,
                    receiver.id,
                    index,
                    expression,
                    bracket_span,
                );
            }
        };
        let u32_type = self.core_type(crate::stdlib::CoreTypeId::U32);
        self.expr(index, Some(u32_type));
        Some(element)
    }
}

fn signed_pattern_integer(value: u64, negative: bool) -> i128 {
    if negative && value != 0 {
        -i128::from(value)
    } else {
        i128::from(value)
    }
}

pub(super) fn expression_is_bare_none(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::None => true,
        ExprKind::Block(block) => block
            .statements
            .last()
            .is_none_or(|statement| match statement {
                crate::ast::Stmt::Expression(value) => expression_is_bare_none(value),
                _ => true,
            }),
        _ => false,
    }
}

//! Inference recovery, normalization, and semantic-product publication.

use std::collections::HashMap;

use crate::{
    Diagnostic,
    ast::{Expr, ExprKind, FunctionId, Program, Span, StateSource, Stmt, ValueId},
    inference::Type,
    semantic::{ResolvedValue, SemanticBuilder},
    stdlib::CoreTypeId,
    types::{
        ResolvedApplicationType, ResolvedArrayType, ResolvedAsyncType, ResolvedCallableType,
        ResolvedConstructedTypes, ResolvedOptionType, ResolvedRangeType, ResolvedResultType,
        ResolvedSetType, TypeKind,
    },
    visit::{Visitor, walk_expr, walk_stmt},
};

use super::{CheckOutput, Checker, RecoveringCheckOutput};

pub(super) fn finish(mut checker: Checker, program: &Program) -> RecoveringCheckOutput {
    checker.resolve_deferred_member_paths();
    checker.diagnose_ambiguous_process_reads();
    if checker.errors.is_empty() {
        checker.inference.finish_associated_projections();
    }
    let (function_type_parameters, generic_parameter_constraints, function_associated_projections) =
        if checker.errors.is_empty() {
            bind_function_generics(&mut checker, program)
        } else {
            (HashMap::new(), HashMap::new(), HashMap::new())
        };
    if checker.errors.is_empty() {
        checker.diagnose_ambiguous_empty_collections();
    }
    if checker.errors.is_empty() {
        checker.default_inference_variables(program);
    }
    if !checker.errors.is_empty() {
        checker.inference.recover_unbound();
    }
    for field in program
        .state
        .as_ref()
        .into_iter()
        .flat_map(|state| state.all_fields())
    {
        if matches!(field.source, StateSource::Pointer(_)) {
            let Some(field_type) = checker
                .declarations
                .state_fields_by_id
                .get(&field.id)
                .copied()
            else {
                continue;
            };
            let poll_result = Type::Result(checker.inference.result_type(field_type));
            checker
                .semantics
                .resolve_state_poll_result(field.id, poll_result);
        }
    }
    checker.inference.finalize_constructed_types();
    checker.inference.intern_resolved_constructed_types();
    let source_associated_types = checker
        .inference
        .source_associated_types()
        .collect::<Vec<_>>();
    let array_types = checker
        .inference
        .arrays()
        .iter()
        .map(|array| ResolvedArrayType {
            id: array.id,
            element: array.element.to_ref(checker.inference.type_store()),
            length: match array.shape {
                crate::inference::ArrayShape::Exact(length) => Some(length),
                crate::inference::ArrayShape::Growable => None,
                crate::inference::ArrayShape::Variable(_) => {
                    unreachable!("array shapes are concrete after finalization")
                }
            },
        })
        .collect::<Vec<_>>();
    let option_types = checker
        .inference
        .options()
        .iter()
        .map(|option| ResolvedOptionType {
            id: option.id,
            value: option.value.to_ref(checker.inference.type_store()),
        })
        .collect::<Vec<_>>();
    let result_types = checker
        .inference
        .results()
        .iter()
        .map(|result| ResolvedResultType {
            id: result.id,
            value: result.value.to_ref(checker.inference.type_store()),
        })
        .collect::<Vec<_>>();
    let async_types = checker
        .inference
        .asyncs()
        .iter()
        .map(|future| ResolvedAsyncType {
            id: future.id,
            value: future.value.to_ref(checker.inference.type_store()),
        })
        .collect::<Vec<_>>();
    let iterator_types = checker
        .inference
        .iterators()
        .iter()
        .map(|iterator| crate::types::ResolvedIteratorType {
            id: iterator.id,
            item: iterator.item.to_ref(checker.inference.type_store()),
        })
        .collect::<Vec<_>>();
    let callable_types = checker
        .inference
        .callables()
        .iter()
        .map(|callable| ResolvedCallableType {
            id: callable.id,
            parameters: callable
                .parameters
                .iter()
                .map(|parameter| parameter.to_ref(checker.inference.type_store()))
                .collect(),
            result: callable.result.to_ref(checker.inference.type_store()),
        })
        .collect::<Vec<_>>();
    let range_types = checker
        .inference
        .ranges()
        .iter()
        .map(|range| ResolvedRangeType {
            id: range.id,
            bound: range.lower.to_ref(checker.inference.type_store()),
            kind: range.kind,
        })
        .collect::<Vec<_>>();
    let set_types = checker
        .inference
        .sets()
        .iter()
        .map(|set| ResolvedSetType {
            id: set.id,
            element: set.element.to_ref(checker.inference.type_store()),
            backing: set
                .backing
                .expect("set backing arrays are assigned during inference initialization"),
        })
        .collect::<Vec<_>>();
    let application_types = checker
        .inference
        .applications()
        .iter()
        .map(|application| ResolvedApplicationType {
            id: application.id,
            constructor: application.constructor,
            arguments: application
                .arguments
                .iter()
                .map(|argument| argument.to_ref(checker.inference.type_store()))
                .collect(),
        })
        .collect::<Vec<_>>();
    for array in &array_types {
        let element = checker.resolved_type_ref(array.element);
        checker
            .semantics
            .resolve_array_element_type(array.id, element);
    }
    let semantics = std::mem::take(&mut checker.semantics);
    let semantic_types = checker.inference.type_store().clone();
    let enum_types = checker.declarations.enums.clone();
    let mut diagnostics = std::mem::take(&mut checker.errors);
    let mut semantics = semantics.finish(
        semantic_types,
        ResolvedConstructedTypes {
            arrays: &array_types,
            options: &option_types,
            results: &result_types,
            asyncs: &async_types,
            iterators: &iterator_types,
            callables: &callable_types,
            sets: &set_types,
            applications: &application_types,
        },
        |ty| checker.resolved_type(ty),
    );
    semantics.set_function_parameter_types(
        program
            .functions
            .iter()
            .map(|function| {
                (
                    function.id,
                    function
                        .params
                        .iter()
                        .map(|parameter| {
                            semantics
                                .value_type(parameter.id)
                                .expect("checked parameters have semantic types")
                        })
                        .collect(),
                )
            })
            .collect(),
    );
    semantics.set_function_type_parameters(
        function_type_parameters,
        generic_parameter_constraints,
        function_associated_projections,
    );
    semantics.set_source_associated_types(
        source_associated_types
            .into_iter()
            .map(|associated| {
                let Type::Known(value) = checker.resolved_type(associated.value) else {
                    unreachable!("resolved associated types have semantic identities")
                };
                (
                    (associated.receiver, associated.capability, associated.name),
                    value,
                )
            })
            .collect(),
    );
    diagnose_float_literal_ranges(program, &semantics, &mut diagnostics);
    RecoveringCheckOutput {
        output: CheckOutput {
            semantics,
            enum_types,
            array_types,
            option_types,
            result_types,
            async_types,
            iterator_types,
            callable_types,
            range_types,
            set_types,
            application_types,
        },
        diagnostics,
    }
}

fn diagnose_float_literal_ranges(
    program: &Program,
    semantics: &crate::semantic::SemanticModel,
    diagnostics: &mut Vec<Diagnostic>,
) {
    struct FloatLiteralValidator<'a> {
        semantics: &'a crate::semantic::SemanticModel,
        diagnostics: &'a mut Vec<Diagnostic>,
    }

    impl<'ast> Visitor<'ast> for FloatLiteralValidator<'_> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let ExprKind::Float(literal) = &expression.kind
                && self
                    .semantics
                    .expression_type(expression.id)
                    .is_some_and(|ty| {
                        matches!(
                            self.semantics.types().kind(ty),
                            TypeKind::Builtin(CoreTypeId::F32)
                        )
                    })
            {
                let narrowed = literal
                    .normalized
                    .parse::<f32>()
                    .expect("the lexer and parser validated the decimal float");
                let message = if narrowed.is_infinite() {
                    Some("floating-point literal overflows the finite `f32` range")
                } else if literal.value != 0.0 && narrowed == 0.0 {
                    Some("floating-point literal underflows `f32` to zero")
                } else {
                    None
                };
                if let Some(message) = message {
                    self.diagnostics
                        .push(Diagnostic::type_error(message, expression.span));
                }
            }
            walk_expr(self, expression);
        }
    }

    FloatLiteralValidator {
        semantics,
        diagnostics,
    }
    .visit_program(program);
}

type FunctionGenericBindings = (
    HashMap<FunctionId, Vec<crate::types::TypeId>>,
    HashMap<crate::types::TypeId, Vec<crate::stdlib::StdlibCapabilityId>>,
    HashMap<FunctionId, Vec<crate::semantic::FunctionAssociatedProjection>>,
);

fn bind_function_generics(checker: &mut Checker, program: &Program) -> FunctionGenericBindings {
    let mut roots = HashMap::new();
    let mut parameters = HashMap::new();
    let mut constraints = HashMap::new();
    let mut associated_projections = HashMap::new();
    for function in &program.functions {
        let generalized = checker.declarations.function_signatures[&function.id]
            .generalized
            .clone();
        let mut function_parameters = Vec::with_capacity(generalized.len());
        for (index, variable) in generalized.into_iter().enumerate() {
            let parameter = if let Some(parameter) = roots.get(&variable) {
                *parameter
            } else {
                let parameter = checker
                    .inference
                    .intern_generic_parameter(function.id, index as u32);
                constraints.insert(
                    parameter,
                    checker
                        .inference
                        .variable_requirements(variable)
                        .as_slice()
                        .to_vec(),
                );
                roots.insert(variable, parameter);
                parameter
            };
            function_parameters.push(parameter);
        }
        if !function_parameters.is_empty() {
            parameters.insert(function.id, function_parameters);
        }
        let resolved_projections = checker.declarations.function_signatures[&function.id]
            .associated_projections
            .iter()
            .filter_map(|projection| {
                Some(crate::semantic::FunctionAssociatedProjection {
                    receiver: *roots.get(&projection.receiver)?,
                    output: *roots.get(&projection.output)?,
                    capability: projection.capability,
                    name: projection.name,
                })
            })
            .collect::<Vec<_>>();
        if !resolved_projections.is_empty() {
            associated_projections.insert(function.id, resolved_projections);
        }
    }
    for (variable, parameter) in roots {
        checker
            .inference
            .bind_generic_parameter(variable, parameter);
    }
    (parameters, constraints, associated_projections)
}

impl Checker {
    pub(super) fn diagnose_ambiguous_empty_collections(&mut self) {
        let collections = self.inferred_empty_collections.clone();
        for (ty, span, part, collection) in collections {
            if self.inference.is_unbound_without_default(ty) {
                self.error(
                    format!(
                        "cannot infer the {part} type of this empty {collection}; use it where the type is known or add explicit type arguments"
                    ),
                    span,
                );
                self.inference.recover_unbound_type(ty);
            }
        }
    }

    pub(super) fn default_inference_variables(&mut self, program: &Program) {
        self.diagnose_ambiguous_globals(program);
        for field in program
            .state
            .as_ref()
            .into_iter()
            .flat_map(|state| state.all_fields())
            .filter(|field| {
                field.annotation.is_none()
                    && matches!(field.source, StateSource::Pointer(_))
                    && !matches!(
                        field.source,
                        StateSource::Pointer(crate::ast::PointerPath {
                            decoder: Some(_),
                            ..
                        })
                    )
            })
        {
            let ty = self.declarations.state_fields_by_id[&field.id];
            if self.inference.is_unbound_without_default(ty) {
                self.error(
                    format!(
                        "cannot infer the memory type of state field `{}`; add an explicit type such as `{}: i32 at ...`",
                        field.name, field.name
                    ),
                    field.span,
                );
                self.inference.recover_unbound_type(ty);
            }
        }
        for error in self.inference.default_unbound() {
            let message = self.inference_error_message(error);
            self.error(message, Span::default());
        }
    }

    fn diagnose_ambiguous_globals(&mut self, program: &Program) {
        for global in &program.globals {
            if global.annotation.is_some() {
                continue;
            }
            let mut declared = Vec::new();
            global.binding.visit_bindings(&mut |binding| {
                declared.push((binding.id, binding.name.clone(), binding.name_span));
            });
            for (id, name, name_span) in declared {
                let ty = self.declarations.globals[&name].ty;
                if !self.inference.is_unbound_without_default(ty) {
                    continue;
                }

                let qualifier = if global.value.is_none() { "bare " } else { "" };
                let mut diagnostic = Diagnostic::type_error(
                format!(
                    "cannot infer the type of {qualifier}global `{}`",
                    name
                ),
                name_span,
            )
            .with_primary_label("this declaration needs one concrete type")
            .with_note(
                "add an explicit type annotation or constrain the value with a concrete assignment or use",
            );
                let (assignments, uses) = ambiguous_global_sites(program, &self.semantics, id);
                for span in assignments.into_iter().take(3) {
                    diagnostic = diagnostic.with_secondary_label(
                        span,
                        "this assignment still does not determine one concrete type",
                    );
                }
                for span in uses.into_iter().take(3) {
                    diagnostic = diagnostic.with_secondary_label(
                        span,
                        "this use still does not determine one concrete type",
                    );
                }
                self.errors.push(diagnostic);
                self.inference.recover_unbound_type(ty);
            }
        }
    }

    pub(super) fn diagnose_ambiguous_process_reads(&mut self) {
        let reads = self.inferred_process_reads.clone();
        let generalized = self
            .declarations
            .function_signatures
            .values()
            .flat_map(|signature| signature.generalized.iter().copied())
            .collect::<Vec<_>>();
        for (ty, span) in reads {
            let belongs_to_scheme = self
                .inference
                .unbound_variables_in([ty])
                .iter()
                .any(|variable| generalized.contains(variable));
            if !belongs_to_scheme && self.inference.is_unbound_without_default(ty) {
                self.error(
                    "cannot infer the memory type read by `process.read`; add a result annotation such as `let value: i32! = process.read(address)`, or use `process.read<i32>(address)`",
                    span,
                );
            }
        }
    }

    pub(super) fn resolved_type(&mut self, ty: Type) -> Type {
        self.inference.resolve(ty)
    }
}

fn ambiguous_global_sites(
    program: &Program,
    semantics: &SemanticBuilder,
    target: ValueId,
) -> (Vec<Span>, Vec<Span>) {
    struct Collector<'a> {
        semantics: &'a SemanticBuilder,
        target: ValueId,
        assignments: Vec<Span>,
        uses: Vec<Span>,
    }

    impl<'ast> Visitor<'ast> for Collector<'_> {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if let Stmt::Assign { id, span, .. } = statement
                && self.semantics.resolved_assignment(*id) == Some(self.target)
            {
                self.assignments.push(*span);
            }
            walk_stmt(self, statement);
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            if self.semantics.resolved_value(expression.id)
                == Some(ResolvedValue::Variable(self.target))
            {
                self.uses.push(expression.span);
            }
            walk_expr(self, expression);
        }
    }

    let mut collector = Collector {
        semantics,
        target,
        assignments: Vec::new(),
        uses: Vec::new(),
    };
    collector.visit_program(program);
    collector
        .assignments
        .sort_by_key(|span| (span.start, span.end));
    collector.assignments.dedup();
    collector.uses.sort_by_key(|span| (span.start, span.end));
    collector.uses.dedup();
    (collector.assignments, collector.uses)
}

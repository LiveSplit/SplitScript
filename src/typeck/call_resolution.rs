//! Call, path, catalog-overload, and member resolution during type checking.

use std::collections::{HashMap, HashSet};

use crate::{
    Diagnostic,
    ast::{ActionKind, ArrayTypeId, Expr, ExprId, ExprKind, Span},
    inference::{ArrayShape, InferenceError, Requirements, Type, type_may_have_capability},
    intrinsic_registry::FailureChannelPolicy,
    migration::{
        ASL_SETTINGS_ADD_DIAGNOSTIC, ASL_SETTINGS_LOOKUP_DIAGNOSTIC, ForeignSpellingContext,
        foreign_spelling, legacy_array_field_diagnostic, legacy_managed_method_diagnostic,
        legacy_set_field_diagnostic, legacy_static_call_diagnostic, legacy_string_field_diagnostic,
        legacy_string_method_diagnostic, legacy_value_path_diagnostic, migration_diagnostic,
    },
    name_matching::closest_name,
    semantic::{PendingResolvedCall, ResolvedMember, ResolvedValue},
    signature::parse_signature,
    stdlib::{
        Availability, CapabilityBehavior, DeclaredTypeRef, ItemKind, ParameterRule,
        StandardBinaryOperator, StandardUnaryOperator, StdlibItem, StdlibItemId, StdlibOwner,
        StdlibTypeConstructorId, StdlibTypeId, TypeRef as CatalogTypeRef,
    },
    stdlib_semantic::{CallCandidate, StandardLibrarySemanticExt},
    types::TypeKind,
};

use super::{
    CallSyntax, Checker, DeferredMemberPath, ExpectedTypeSource, MethodReceiver, PathResolution,
    ResolvedReceiver, catalog_type_argument,
    context::{CallableContext, ExpressionMode},
    declarations::{FunctionParameterDeclaration, RuntimeSettingDeclaration, RuntimeSettingKind},
};

impl Checker {
    /// Resolves compiler-owned indexing syntax through the `Index.at`
    /// capability requirement. Concrete arrays retain their direct lowering;
    /// every other receiver follows ordinary structural capability dispatch.
    pub(super) fn resolve_index_getter(
        &mut self,
        receiver_type: Type,
        receiver: ExprId,
        index: &Expr,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        let requirement = Requirements::capability(crate::stdlib::StdlibCapabilityId::Index);
        if self
            .inference
            .require(receiver_type, requirement.clone())
            .is_err()
        {
            let message = self.capability_failure_message(receiver_type, &requirement);
            self.error(message, span);
            return None;
        }
        let standard_library = self.standard_library.clone();
        let candidate = standard_library
            .method_candidates_including_private("at")
            .into_iter()
            .find(|candidate| candidate.item.id == StdlibItemId::IndexAt)
            .expect("the bundled Index capability declares its indexing requirement");
        self.catalog_call(
            &candidate,
            Some(MethodReceiver {
                ty: receiver_type,
                value: ResolvedReceiver::Expression {
                    expression: receiver,
                    members: Vec::new(),
                },
            }),
            &[],
            std::slice::from_ref(index),
            None,
            expression,
            span,
        )
    }

    /// Records the `IndexAssign.set` capability operation used by compound
    /// bracket assignment. Its inherited key and value projections are owned
    /// by `Index`, so read and write syntax cannot disagree about their shape.
    pub(super) fn resolve_index_setter(
        &mut self,
        assignment: crate::ast::AssignmentId,
        receiver_type: Type,
        receiver: ExprId,
        span: Span,
    ) {
        let receiver_type = self.shallow_type(receiver_type);
        if matches!(receiver_type, Type::Array(_))
            || matches!(
                receiver_type,
                Type::Known(id)
                    if matches!(self.inference.type_store().kind(id), TypeKind::Array { .. })
            )
        {
            return;
        }
        if self
            .require(
                receiver_type,
                Requirements::capability(crate::stdlib::StdlibCapabilityId::IndexAssign),
                span,
            )
            .is_none()
        {
            return;
        }
        let key = self.inference.associated_type(
            receiver_type,
            crate::stdlib::StdlibCapabilityId::Index,
            "Key",
        );
        let value = self.inference.associated_type(
            receiver_type,
            crate::stdlib::StdlibCapabilityId::Index,
            "Value",
        );
        let none = self.core_type(crate::stdlib::CoreTypeId::None);
        self.semantics.resolve_index_assignment_setter(
            assignment,
            PendingResolvedCall::StandardLibrary {
                item: StdlibItemId::IndexAssignSet,
                type_arguments: vec![receiver_type],
                signature: vec![receiver_type, key, value, none],
                receiver: Some(ResolvedReceiver::Expression {
                    expression: receiver,
                    members: Vec::new(),
                }),
                receiver_type: Some(receiver_type),
            },
        );
    }

    /// Resolves unary syntax through a catalog-declared zero-argument method.
    pub(super) fn resolve_unary_operator(
        &mut self,
        op: crate::ast::UnaryOp,
        operand_type: Type,
        expression: ExprId,
        operand: ExprId,
        span: Span,
    ) -> Option<Type> {
        let operator = match op {
            crate::ast::UnaryOp::Not => StandardUnaryOperator::Not,
            crate::ast::UnaryOp::Neg => StandardUnaryOperator::Negate,
        };
        let candidates = self
            .standard_library
            .unary_operator_candidates(operator)
            .into_iter()
            .collect();
        let error_count = self.errors.len();
        let resolved = self.operator_call(
            candidates,
            operand_type,
            None,
            ResolvedReceiver::Expression {
                expression: operand,
                members: Vec::new(),
            },
            span,
            false,
        );
        let Some((result, call)) = resolved else {
            if self.errors.len() == error_count {
                let operand = self.type_name(operand_type);
                let requirement = match op {
                    crate::ast::UnaryOp::Not => "`bool` or an integer",
                    crate::ast::UnaryOp::Neg => "a signed number",
                };
                self.error(
                    format!("operator cannot be applied to `{operand}`; expected {requirement}"),
                    span,
                );
            }
            return None;
        };
        self.semantics.resolve_call(expression, call);
        Some(result)
    }

    /// Resolves binary syntax through a catalog-declared method. Inferred
    /// operands select capability bindings; concrete operands may select a
    /// more specific implementation owned by their exact type.
    pub(super) fn resolve_binary_operator(
        &mut self,
        op: crate::ast::BinaryOp,
        left_type: Type,
        right_type: Type,
        expression: ExprId,
        left: ExprId,
        span: Span,
    ) -> Option<Type> {
        let (result, call) = self.binary_operator_call(
            op,
            left_type,
            right_type,
            ResolvedReceiver::Expression {
                expression: left,
                members: Vec::new(),
            },
            span,
        )?;
        self.semantics.resolve_call(expression, call);
        Some(result)
    }

    pub(super) fn resolve_assignment_operator(
        &mut self,
        assignment: crate::ast::AssignmentId,
        op: crate::ast::BinaryOp,
        left_type: Type,
        right_type: Type,
        target: crate::ast::ValueId,
        span: Span,
    ) -> Option<Type> {
        let (result, call) = self.binary_operator_call(
            op,
            left_type,
            right_type,
            ResolvedReceiver::Path {
                root: ResolvedValue::Variable(target),
                members: Vec::new(),
            },
            span,
        )?;
        self.semantics.resolve_assignment_call(assignment, call);
        Some(result)
    }

    pub(super) fn resolve_state_assignment_operator(
        &mut self,
        assignment: crate::ast::AssignmentId,
        op: crate::ast::BinaryOp,
        left_type: Type,
        right_type: Type,
        target: crate::ast::ValueId,
        span: Span,
    ) -> Option<Type> {
        let (result, call) = self.binary_operator_call(
            op,
            left_type,
            right_type,
            ResolvedReceiver::Path {
                root: ResolvedValue::CurrentState(target),
                members: Vec::new(),
            },
            span,
        )?;
        self.semantics.resolve_assignment_call(assignment, call);
        Some(result)
    }

    pub(super) fn resolve_index_assignment_operator(
        &mut self,
        assignment: crate::ast::AssignmentId,
        op: crate::ast::BinaryOp,
        left_type: Type,
        right_type: Type,
        receiver: crate::ast::ExprId,
        span: Span,
    ) -> Option<Type> {
        let (result, call) = self.binary_operator_call(
            op,
            left_type,
            right_type,
            ResolvedReceiver::Expression {
                expression: receiver,
                members: Vec::new(),
            },
            span,
        )?;
        self.semantics.resolve_assignment_call(assignment, call);
        Some(result)
    }

    fn binary_operator_call(
        &mut self,
        op: crate::ast::BinaryOp,
        left_type: Type,
        right_type: Type,
        receiver: ResolvedReceiver,
        span: Span,
    ) -> Option<(Type, PendingResolvedCall)> {
        let operator = match op {
            crate::ast::BinaryOp::Add => StandardBinaryOperator::Add,
            crate::ast::BinaryOp::Sub => StandardBinaryOperator::Subtract,
            crate::ast::BinaryOp::Mul => StandardBinaryOperator::Multiply,
            crate::ast::BinaryOp::Div => StandardBinaryOperator::Divide,
            crate::ast::BinaryOp::Rem => StandardBinaryOperator::Remainder,
            crate::ast::BinaryOp::BitOr => StandardBinaryOperator::BitOr,
            crate::ast::BinaryOp::BitXor => StandardBinaryOperator::BitXor,
            crate::ast::BinaryOp::BitAnd => StandardBinaryOperator::BitAnd,
            crate::ast::BinaryOp::Shl => StandardBinaryOperator::ShiftLeft,
            crate::ast::BinaryOp::Shr => StandardBinaryOperator::ShiftRight,
            crate::ast::BinaryOp::Eq => StandardBinaryOperator::Equal,
            crate::ast::BinaryOp::Ne => StandardBinaryOperator::NotEqual,
            crate::ast::BinaryOp::Lt => StandardBinaryOperator::LessThan,
            crate::ast::BinaryOp::Le => StandardBinaryOperator::LessThanOrEqual,
            crate::ast::BinaryOp::Gt => StandardBinaryOperator::GreaterThan,
            crate::ast::BinaryOp::Ge => StandardBinaryOperator::GreaterThanOrEqual,
            _ => return None,
        };
        let candidates = self
            .standard_library
            .binary_operator_candidates(operator)
            .into_iter()
            .collect();
        self.operator_call(
            candidates,
            left_type,
            Some(right_type),
            receiver,
            span,
            true,
        )
    }

    fn operator_call(
        &mut self,
        candidates: Vec<CallCandidate>,
        receiver_operand: Type,
        argument_operand: Option<Type>,
        receiver: ResolvedReceiver,
        span: Span,
        generic_only_when_inferred: bool,
    ) -> Option<(Type, PendingResolvedCall)> {
        let inferred_receiver = matches!(self.shallow_type(receiver_operand), Type::Variable(_));
        let candidates = candidates
            .into_iter()
            .filter(|candidate| {
                (!generic_only_when_inferred
                    || !inferred_receiver
                    || matches!(candidate.receiver(), Some(CatalogTypeRef::Parameter(_))))
                    && self.inferred_receiver_may_select(candidate, receiver_operand)
                    && self.catalog_candidate_may_apply(candidate, receiver_operand)
            })
            .collect::<Vec<_>>();
        let [candidate] = candidates.as_slice() else {
            if candidates.len() > 1 {
                self.error(
                    format!(
                        "operator is ambiguous between {}",
                        candidates
                            .iter()
                            .map(|candidate| candidate.item.qualified_name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    span,
                );
            }
            return None;
        };
        let item = candidate.item;
        let mut variables = HashMap::new();
        for parameter in item.signature.type_parameters {
            let requirements = parameter
                .constraints
                .iter()
                .fold(Requirements::none(), |requirements, constraint| {
                    requirements | Requirements::capability(*constraint)
                });
            let ty = self.fresh_inference(requirements.clone(), None);
            if !requirements.is_empty() {
                self.require(ty, requirements, span)?;
            }
            variables.insert(parameter.name, ty);
        }
        let receiver_type = self.catalog_type(
            candidate
                .receiver()
                .expect("operator bindings are validated methods"),
            &variables,
        );
        self.unify(receiver_operand, receiver_type, span)?;
        let mut signature = vec![receiver_type];
        match (argument_operand, item.signature.parameters) {
            (Some(argument_operand), [parameter]) => {
                let parameter_type = self.catalog_type(parameter.ty, &variables);
                self.unify(argument_operand, parameter_type, span)?;
                signature.push(parameter_type);
            }
            (None, []) => {}
            _ => unreachable!("operator bindings have a validated arity"),
        };
        let result = self.catalog_type(item.signature.result, &variables);
        signature.push(result);
        let type_arguments = item
            .signature
            .type_parameters
            .iter()
            .map(|parameter| variables[parameter.name])
            .collect();
        Some((
            result,
            PendingResolvedCall::StandardLibrary {
                item: item.id,
                type_arguments,
                signature,
                receiver: Some(receiver),
                receiver_type: Some(receiver_type),
            },
        ))
    }

    fn inferred_receiver_may_select(&mut self, candidate: &CallCandidate, receiver: Type) -> bool {
        let Type::Variable(variable) = self.shallow_type(receiver) else {
            return true;
        };
        let requirements = self.inference.variable_requirements(variable);
        let concrete = match candidate.receiver() {
            Some(CatalogTypeRef::Core(core)) => {
                Some(self.declared_type(DeclaredTypeRef::Core(core)))
            }
            Some(CatalogTypeRef::Standard(standard)) => Some(self.standard_type(standard)),
            _ => None,
        };
        concrete.is_none_or(|concrete| {
            requirements.as_slice().iter().all(|capability| {
                type_may_have_capability(
                    &self.standard_library,
                    self.inference.type_store(),
                    concrete,
                    *capability,
                )
            })
        })
    }

    pub(super) fn call(
        &mut self,
        call: CallSyntax<'_>,
        args: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        let CallSyntax {
            callee,
            name_span,
            postfix_receiver,
            type_arguments,
        } = call;
        if !type_arguments.is_empty()
            && postfix_receiver.is_none()
            && matches!(callee, [name] if matches!(name.as_str(), "Some" | "Ok" | "Err" | "Item"))
        {
            self.error(
                format!("`{}` does not accept explicit type arguments", callee[0]),
                span,
            );
            return None;
        }
        if postfix_receiver.is_none() && callee == ["Some"] {
            if args.len() != 1 {
                self.error("`Some` expects one value", span);
                return None;
            }
            let expected_option = expected.and_then(|ty| match self.shallow_type(ty) {
                Type::Option(option) => Some(option),
                _ => None,
            });
            if expected.is_some()
                && expected_option.is_none()
                && !matches!(
                    expected.map(|ty| self.shallow_type(ty)),
                    Some(Type::Variable(_))
                )
            {
                let other = expected.map(|ty| self.shallow_type(ty)).unwrap();
                self.error(
                    format!("`Some` constructs an optional value, but `{other}` was expected"),
                    span,
                );
                return None;
            }
            let value_hint = expected_option.map(|option| self.inference.option_value(option));
            let value = self.expr(&args[0], value_hint)?;
            let option = expected_option.unwrap_or_else(|| self.inference.option_type(value));
            let ty = Type::Option(option);
            self.semantics
                .resolve_call(expression, PendingResolvedCall::OptionSome { option });
            return self.expect_expression(expression, ty, expected, span);
        }

        if postfix_receiver.is_none() && callee == ["Item"] {
            if args.len() != 1 {
                self.error("`Item` expects one value", span);
                return None;
            }
            let expected_step = expected.and_then(|ty| match self.shallow_type(ty) {
                Type::Application(application)
                    if self.inference.application_constructor(application)
                        == StdlibTypeConstructorId::IteratorStep =>
                {
                    Some(application)
                }
                _ => None,
            });
            if expected.is_some()
                && expected_step.is_none()
                && !matches!(
                    expected.map(|ty| self.shallow_type(ty)),
                    Some(Type::Variable(_))
                )
            {
                let other = expected.map(|ty| self.shallow_type(ty)).unwrap();
                self.error(
                    format!("`Item` constructs an iterator step, but `{other}` was expected"),
                    span,
                );
                return None;
            }
            let value_hint =
                expected_step.map(|step| self.inference.application_arguments(step)[0]);
            let value = self.expr(&args[0], value_hint)?;
            let step = expected_step.unwrap_or_else(|| {
                self.inference
                    .application_type(StdlibTypeConstructorId::IteratorStep, vec![value])
            });
            let ty = Type::Application(step);
            self.semantics
                .resolve_call(expression, PendingResolvedCall::IteratorItem { step });
            return self.expect_expression(expression, ty, expected, span);
        }

        if postfix_receiver.is_none() && callee == ["Ok"] {
            if args.len() != 1 {
                self.error("`Ok` expects one value", span);
                return None;
            }
            let expected_result = expected.and_then(|ty| match self.shallow_type(ty) {
                Type::Result(result) => Some(result),
                _ => None,
            });
            if expected.is_some()
                && expected_result.is_none()
                && !matches!(
                    expected.map(|ty| self.shallow_type(ty)),
                    Some(Type::Variable(_))
                )
            {
                let other = expected.map(|ty| self.shallow_type(ty)).unwrap();
                self.error(
                    format!("`Ok` constructs a result, but `{other}` was expected"),
                    span,
                );
                return None;
            }
            let value_hint = expected_result.map(|result| self.inference.result_value(result));
            let value = self.expr(&args[0], value_hint)?;
            let result = expected_result.unwrap_or_else(|| self.inference.result_type(value));
            let ty = Type::Result(result);
            self.semantics
                .resolve_call(expression, PendingResolvedCall::ResultSuccess { result });
            return self.expect_expression(expression, ty, expected, span);
        }

        if postfix_receiver.is_none() && callee == ["Err"] {
            if args.len() != 1 {
                self.error("`Err` expects one error message", span);
                return None;
            }
            self.expr(&args[0], Some(self.standard_type(StdlibTypeId::String)));
            let result = match expected.map(|ty| self.shallow_type(ty)) {
                Some(result @ Type::Result(_)) => result,
                Some(Type::Variable(_)) | None => {
                    self.error(
                        "cannot infer the success type of `Err`; add a `T!` annotation",
                        span,
                    );
                    return None;
                }
                Some(other) => {
                    self.error(
                        format!("`Err` constructs a result, but `{other}` was expected"),
                        span,
                    );
                    return None;
                }
            };
            let Type::Result(result_id) = result else {
                unreachable!()
            };
            self.semantics.resolve_call(
                expression,
                PendingResolvedCall::ResultError { result: result_id },
            );
            if let Some(expected) = expected {
                self.failure.observe_result(expected, result);
            }
            return Some(result);
        }

        if let Some((active_provider, _)) = self.provider_value
            && !matches!(
                self.callable,
                CallableContext::Action(ActionKind::SelectProcess)
            )
            && !matches!(
                self.callable,
                CallableContext::LibraryFunction(_) | CallableContext::CompilerGenerated
            )
            && let Some(native_provider) = self.standard_library.default_state_provider()
            && self
                .standard_library
                .state_provider(active_provider)
                .value_name
                != native_provider.value_name
            && callee
                .first()
                .is_some_and(|root| root == native_provider.value_name)
        {
            let provider = self.standard_library.state_provider(active_provider);
            self.error(
                format!(
                    "`{}` is unavailable under `state {}`; use `{}` instead",
                    native_provider.value_name, provider.name, provider.value_name
                ),
                span,
            );
            return None;
        }

        if let Some(receiver) = postfix_receiver {
            return self.postfix_method_call(
                receiver,
                callee,
                name_span,
                type_arguments,
                args,
                expected,
                expression,
                span,
            );
        }

        if let [class_name, method] = callee
            && method == "instances"
            && let Some(class) = self
                .declarations
                .managed_classes
                .iter()
                .find(|class| class.name == *class_name)
                .cloned()
        {
            return self.managed_instances_call(
                class.id,
                type_arguments,
                args,
                expected,
                expression,
                span,
            );
        }

        let standard_library = self.standard_library.clone();
        let mut function_candidates = if self.is_library_function() {
            standard_library.function_candidates_including_private(callee)
        } else {
            standard_library.function_candidates(callee)
        };
        if function_candidates.len() > 1 {
            self.ambiguous_catalog_call(callee, &function_candidates, span);
            return None;
        }
        if let Some(candidate) = function_candidates.pop() {
            return self.catalog_call(
                &candidate,
                None,
                type_arguments,
                args,
                expected,
                expression,
                span,
            );
        }
        if callee
            .first()
            .is_some_and(|receiver| self.binding(receiver).is_none())
            && let Some(id) = legacy_static_call_diagnostic(callee, args.len())
        {
            let metadata =
                migration_diagnostic(id).expect("type checker migration diagnostic IDs must exist");
            let mut diagnostic = Diagnostic::type_error(metadata.message, name_span)
                .with_primary_label(metadata.primary_label)
                .with_migration_topic(metadata.concept.as_str());
            for note in metadata.notes {
                diagnostic = diagnostic.with_note(*note);
            }
            self.errors.push(diagnostic);
            return None;
        }
        let (
            display_name,
            signature_id,
            signature_result,
            parameters,
            parameter_declarations,
            resolved_call,
        ) = if let [name] = callee {
            if !type_arguments.is_empty() {
                self.error(
                    "explicit type arguments are currently supported on standard-library calls",
                    span,
                );
                return None;
            }
            let Some(signature) = self.declarations.functions.get(name).cloned() else {
                let suggestion = self.function_name_suggestion(callee);
                self.unknown_function(callee, name_span, span, suggestion.as_deref());
                return None;
            };
            let signature = if self.active_function_component.contains(&signature.id) {
                signature.monomorphic_call()
            } else {
                signature.instantiate(&mut self.inference)
            };
            let concrete_signature = signature
                .params
                .iter()
                .copied()
                .chain([signature.result])
                .collect();
            (
                name.clone(),
                signature.id,
                signature.result,
                signature.params,
                signature.parameter_declarations,
                PendingResolvedCall::UserFunction {
                    function: signature.id,
                    type_arguments: signature.type_arguments,
                    signature: concrete_signature,
                },
            )
        } else {
            if let Some(suggestion) = self.function_name_suggestion(callee) {
                self.unknown_function(callee, name_span, span, Some(&suggestion));
                return None;
            }
            let (method, receiver_path) = callee.split_last().unwrap();
            let receiver = self.path(receiver_path, span, None)?;
            if self.is_error_type(receiver.ty) {
                for argument in args {
                    self.expr(argument, None);
                }
                return self.expect_expression(expression, receiver.ty, expected, span);
            }
            let receiver_value = receiver
                .value
                .expect("method receiver paths resolve to a declaration or snapshot value");
            let receiver_members = receiver
                .members
                .expect("method receiver types must be known while resolving a call");
            let receiver_type = self.shallow_type(receiver.ty);
            if method == "snapshot"
                && let Type::Known(receiver_id) = receiver_type
                && let TypeKind::ManagedReference(class) =
                    self.inference.type_store().kind(receiver_id)
            {
                return self.managed_snapshot_call(
                    *class,
                    MethodReceiver {
                        ty: receiver_type,
                        value: ResolvedReceiver::Path {
                            root: receiver_value,
                            members: receiver_members,
                        },
                    },
                    type_arguments,
                    args,
                    expected,
                    expression,
                    span,
                );
            }
            if method == "component"
                && self.standard_type_id(receiver_type) == Some(StdlibTypeId::UnityGameObject)
            {
                return self.managed_component_call(
                    MethodReceiver {
                        ty: receiver_type,
                        value: ResolvedReceiver::Path {
                            root: receiver_value,
                            members: receiver_members,
                        },
                    },
                    type_arguments,
                    args,
                    expected,
                    expression,
                    span,
                );
            }
            let method_candidates =
                if self.is_library_function() || name_span.start == name_span.end {
                    standard_library.method_candidates_including_private(method)
                } else {
                    standard_library.method_candidates(method)
                };
            let mut candidates = method_candidates
                .into_iter()
                .filter(|candidate| self.catalog_candidate_may_apply(candidate, receiver_type))
                .collect::<Vec<_>>();
            Self::prefer_matching_catalog_call_shape(
                &mut candidates,
                type_arguments.len(),
                args.len(),
            );
            self.prefer_specific_catalog_candidates(&mut candidates, receiver_type);
            if candidates.len() > 1 {
                self.ambiguous_catalog_call(callee, &candidates, span);
                return None;
            }
            if let Some(candidate) = candidates.pop() {
                return self.catalog_call(
                    &candidate,
                    Some(MethodReceiver {
                        ty: receiver_type,
                        value: ResolvedReceiver::Path {
                            root: receiver_value,
                            members: receiver_members,
                        },
                    }),
                    type_arguments,
                    args,
                    expected,
                    expression,
                    span,
                );
            }
            if let Some(id) = legacy_managed_method_diagnostic(method) {
                self.migration_member_error(id, name_span, None);
                return None;
            }
            if !type_arguments.is_empty() {
                self.error(
                    "explicit type arguments are currently supported on standard-library calls",
                    span,
                );
                return None;
            }
            let Some(signature) = self
                .declarations
                .methods
                .get(&(receiver_type, method.clone()))
                .cloned()
            else {
                let suggestion = self.method_name_suggestion(receiver_type, method);
                self.unknown_method(
                    receiver_type,
                    method,
                    name_span,
                    span,
                    suggestion.as_deref(),
                );
                return None;
            };
            let signature = if self.active_function_component.contains(&signature.id) {
                signature.monomorphic_call()
            } else {
                signature.instantiate(&mut self.inference)
            };
            let receiver_name = self.type_name(receiver_type);
            let concrete_signature = signature
                .params
                .iter()
                .copied()
                .chain([signature.result])
                .collect();
            (
                format!("{receiver_name}.{method}"),
                signature.id,
                signature.result,
                signature.params.into_iter().skip(1).collect(),
                signature
                    .parameter_declarations
                    .into_iter()
                    .skip(1)
                    .collect(),
                PendingResolvedCall::UserMethod {
                    function: signature.id,
                    type_arguments: signature.type_arguments,
                    signature: concrete_signature,
                    receiver: ResolvedReceiver::Path {
                        root: receiver_value,
                        members: receiver_members,
                    },
                    receiver_type,
                },
            )
        };
        if self.declarations.debug_functions.contains(&signature_id)
            && !self.debug_context.is_debug()
        {
            self.error(
                format!("debug-only function `{display_name}` can only be called from debug code"),
                span,
            );
        }
        if args.len() != parameters.len() {
            self.error(
                format!(
                    "`{display_name}` expects {} arguments, found {}",
                    parameters.len(),
                    args.len()
                ),
                span,
            );
            return None;
        }
        self.check_user_call_arguments(args, &parameters, &parameter_declarations);
        let result = self.expect_expression(expression, signature_result, expected, span)?;
        self.semantics.resolve_call(expression, resolved_call);
        Some(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn postfix_method_call(
        &mut self,
        written_receiver: &Expr,
        callee: &[String],
        name_span: Span,
        type_arguments: &[crate::ast::TypeRef],
        args: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        let standard_library = self.standard_library.clone();
        let base_type = self.expr(written_receiver, None)?;
        let base_type = self.shallow_type(base_type);
        if self.is_error_type(base_type) {
            for argument in args {
                self.expr(argument, None);
            }
            return self.expect_expression(expression, base_type, expected, span);
        }
        let method = callee.last().expect("postfix calls name a method");
        let (receiver_type, receiver_members) =
            self.resolve_members(base_type, &callee[..callee.len() - 1], span)?;
        let method_receiver = MethodReceiver {
            ty: receiver_type,
            value: ResolvedReceiver::Expression {
                expression: written_receiver.id,
                members: receiver_members,
            },
        };
        if method == "snapshot"
            && let Type::Known(receiver_id) = receiver_type
            && let TypeKind::ManagedReference(class) = self.inference.type_store().kind(receiver_id)
        {
            return self.managed_snapshot_call(
                *class,
                method_receiver,
                type_arguments,
                args,
                expected,
                expression,
                span,
            );
        }
        if method == "component"
            && self.standard_type_id(receiver_type) == Some(StdlibTypeId::UnityGameObject)
        {
            return self.managed_component_call(
                method_receiver,
                type_arguments,
                args,
                expected,
                expression,
                span,
            );
        }
        let method_candidates = if self.is_library_function() || name_span.start == name_span.end {
            standard_library.method_candidates_including_private(method)
        } else {
            standard_library.method_candidates(method)
        };
        let mut candidates = method_candidates
            .into_iter()
            .filter(|candidate| self.catalog_candidate_may_apply(candidate, receiver_type))
            .collect::<Vec<_>>();
        Self::prefer_matching_catalog_call_shape(&mut candidates, type_arguments.len(), args.len());
        self.prefer_specific_catalog_candidates(&mut candidates, receiver_type);
        if candidates.len() > 1 {
            self.ambiguous_catalog_call(std::slice::from_ref(method), &candidates, span);
            return None;
        }
        if let Some(candidate) = candidates.pop() {
            return self.catalog_call(
                &candidate,
                Some(method_receiver),
                type_arguments,
                args,
                expected,
                expression,
                span,
            );
        }
        if let Some(id) = legacy_managed_method_diagnostic(method) {
            self.migration_member_error(id, name_span, None);
            return None;
        }
        if !type_arguments.is_empty() {
            self.error(
                "explicit type arguments are currently supported on standard-library calls",
                span,
            );
            return None;
        }
        let Some(signature) = self
            .declarations
            .methods
            .get(&(receiver_type, method.to_owned()))
            .cloned()
        else {
            let suggestion = self.method_name_suggestion(receiver_type, method);
            self.unknown_method(
                receiver_type,
                method,
                name_span,
                span,
                suggestion.as_deref(),
            );
            return None;
        };
        let signature = if self.active_function_component.contains(&signature.id) {
            signature.monomorphic_call()
        } else {
            signature.instantiate(&mut self.inference)
        };
        if self.declarations.debug_functions.contains(&signature.id)
            && !self.debug_context.is_debug()
        {
            self.error(
                format!("debug-only method `{method}` can only be called from debug code"),
                span,
            );
        }
        let parameters = signature.params.iter().copied().skip(1).collect::<Vec<_>>();
        let parameter_declarations = signature
            .parameter_declarations
            .iter()
            .skip(1)
            .cloned()
            .collect::<Vec<_>>();
        if args.len() != parameters.len() {
            self.error(
                format!(
                    "`{method}` expects {} arguments, found {}",
                    parameters.len(),
                    args.len()
                ),
                span,
            );
            return None;
        }
        self.check_user_call_arguments(args, &parameters, &parameter_declarations);
        let concrete_signature = signature
            .params
            .iter()
            .copied()
            .chain([signature.result])
            .collect();
        let result = self.expect_expression(expression, signature.result, expected, span)?;
        self.semantics.resolve_call(
            expression,
            PendingResolvedCall::UserMethod {
                function: signature.id,
                type_arguments: signature.type_arguments,
                signature: concrete_signature,
                receiver: method_receiver.value,
                receiver_type,
            },
        );
        Some(result)
    }

    fn prepare_managed_collection_read(&mut self, value: Type) {
        let mut inner = self.inference.shallow(value);
        loop {
            match inner {
                Type::Option(option) => {
                    inner = self.inference.shallow(self.inference.option_value(option))
                }
                Type::Known(id) => match self.inference.type_store().kind(id) {
                    TypeKind::Option { value, .. } => inner = Type::Known(*value),
                    TypeKind::Array { length: None, .. } => break,
                    TypeKind::Application { constructor, .. }
                        if matches!(
                            *constructor,
                            StdlibTypeConstructorId::List | StdlibTypeConstructorId::Map
                        ) =>
                    {
                        break;
                    }
                    _ => return,
                },
                Type::Array(array) if self.inference.array_length(array).is_none() => break,
                Type::Application(application)
                    if matches!(
                        self.inference.application_constructor(application),
                        StdlibTypeConstructorId::List | StdlibTypeConstructorId::Map
                    ) =>
                {
                    break;
                }
                _ => return,
            }
        }
        self.prepare_managed_read_type(value);
    }

    fn prepare_managed_read_type(&mut self, value: Type) {
        let mut pending = vec![value];
        let mut visited = std::collections::HashSet::new();
        while let Some(value) = pending.pop() {
            let value = self.inference.shallow(value);
            if !visited.insert(value) {
                continue;
            }
            let owned = self.managed_owned_type(value);
            self.inference.result_type(owned);
            match value {
                Type::Application(application)
                    if matches!(
                        self.inference.application_constructor(application),
                        StdlibTypeConstructorId::List | StdlibTypeConstructorId::Map
                    ) =>
                {
                    pending.extend(self.inference.application_arguments(application));
                }
                Type::Array(array) => pending.push(self.inference.array_element(array)),
                Type::Option(option) => pending.push(self.inference.option_value(option)),
                Type::Known(id) => match self.inference.type_store().kind(id).clone() {
                    TypeKind::Application {
                        constructor: StdlibTypeConstructorId::List | StdlibTypeConstructorId::Map,
                        arguments,
                        ..
                    } => pending.extend(arguments.into_iter().map(Type::Known)),
                    TypeKind::Array { element, .. } => pending.push(Type::Known(element)),
                    TypeKind::Option { value, .. } => pending.push(Type::Known(value)),
                    TypeKind::ManagedClass(class) => {
                        let declaration = self
                            .declarations
                            .managed_classes
                            .iter()
                            .find(|candidate| candidate.id == class)
                            .cloned()
                            .unwrap();
                        for field in declaration.all_fields().filter(|field| !field.is_static) {
                            let live = self.managed_read_value_type(field.ty);
                            self.inference.result_type(live);
                            pending.push(self.syntax_type(field.ty));
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn managed_snapshot_call(
        &mut self,
        class: crate::ast::ManagedClassId,
        receiver: MethodReceiver,
        type_arguments: &[crate::ast::TypeRef],
        args: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        if !type_arguments.is_empty() {
            self.error("`snapshot` does not accept type arguments", span);
            return None;
        }
        if !args.is_empty() {
            self.error(
                format!("`snapshot` expects 0 arguments, found {}", args.len()),
                span,
            );
            for argument in args {
                self.expr(argument, None);
            }
            return None;
        }
        let mut pending = vec![class];
        let mut visited = std::collections::HashSet::new();
        while let Some(current) = pending.pop() {
            if !visited.insert(current) {
                continue;
            }
            let declaration = self
                .declarations
                .managed_classes
                .iter()
                .find(|candidate| candidate.id == current)
                .cloned()
                .expect("managed snapshot receivers have class declarations");
            let snapshot = Type::Known(self.inference.type_store().id_for_managed_class(current));
            self.inference.result_type(snapshot);
            for field in declaration.all_fields().filter(|field| !field.is_static) {
                let live = self.managed_read_value_type(field.ty);
                self.inference.result_type(live);
                let storage = self.syntax_type(field.ty);
                let owned = self.managed_owned_type(storage);
                self.inference.result_type(owned);
                self.prepare_managed_collection_read(storage);
                let child = match self.inference.shallow(owned) {
                    Type::Option(option) => self.inference.option_value(option),
                    Type::Known(id) => match self.inference.type_store().kind(id) {
                        TypeKind::Option { value, .. } => Type::Known(*value),
                        _ => owned,
                    },
                    _ => owned,
                };
                if let Type::Known(id) = child
                    && let TypeKind::ManagedClass(child) = self.inference.type_store().kind(id)
                {
                    pending.push(*child);
                }
            }
        }
        let snapshot = Type::Known(self.inference.type_store().id_for_managed_class(class));
        let result = Type::Result(self.inference.result_type(snapshot));
        let result = self.expect_expression(expression, result, expected, span)?;
        let Type::Result(result_id) = result else {
            unreachable!("managed snapshot calls produce Result values")
        };
        self.semantics.resolve_call(
            expression,
            PendingResolvedCall::ManagedSnapshot {
                class,
                result: result_id,
                receiver: receiver.value,
                receiver_type: receiver.ty,
            },
        );
        Some(result)
    }

    fn managed_instances_call(
        &mut self,
        class: crate::ast::ManagedClassId,
        type_arguments: &[crate::ast::TypeRef],
        args: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        if !self.managed_provider_available() {
            self.errors.push(
                Diagnostic::type_error(
                    "managed instance discovery requires a Unity state provider",
                    span,
                )
                .with_primary_label("declare the state as `state Unity [\"game.exe\"] { ... }`"),
            );
            return None;
        }
        if !type_arguments.is_empty() {
            self.error("`instances` does not accept type arguments", span);
            return None;
        }
        if !args.is_empty() {
            self.error(
                format!("`instances` expects 0 arguments, found {}", args.len()),
                span,
            );
            for argument in args {
                self.expr(argument, None);
            }
            return None;
        }

        let reference = Type::Known(self.inference.type_store().id_for_managed_reference(class));
        let array = Type::Array(self.inference.array_type(reference));
        let future = Type::Async(self.inference.async_type(array));
        let result = self.expect_expression(expression, future, expected, span)?;
        self.semantics
            .resolve_call(expression, PendingResolvedCall::ManagedInstances { class });
        Some(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn managed_component_call(
        &mut self,
        receiver: MethodReceiver,
        type_arguments: &[crate::ast::TypeRef],
        args: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        if !self.managed_provider_available() {
            self.errors.push(
                Diagnostic::type_error(
                    "Unity component lookup requires a Unity state provider",
                    span,
                )
                .with_primary_label("declare the state as `state Unity [\"game.exe\"] { ... }`"),
            );
            return None;
        }
        let [type_argument] = type_arguments else {
            self.error(
                format!(
                    "`component` expects 1 managed class type argument, found {}",
                    type_arguments.len()
                ),
                span,
            );
            return None;
        };
        if !args.is_empty() {
            self.error(
                format!("`component` expects 0 arguments, found {}", args.len()),
                span,
            );
            for argument in args {
                self.expr(argument, None);
            }
            return None;
        }
        let selected = self.syntax_type(*type_argument);
        let Type::Known(selected) = self.shallow_type(selected) else {
            self.error(
                "`component<T>` requires a declared managed class for `T`",
                span,
            );
            return None;
        };
        let TypeKind::ManagedClass(class) = self.inference.type_store().kind(selected) else {
            self.error(
                "`component<T>` requires a declared managed class for `T`",
                span,
            );
            return None;
        };
        let class = *class;
        let reference = Type::Known(self.inference.type_store().id_for_managed_reference(class));
        let result = Type::Result(self.inference.result_type(reference));
        let result = self.expect_expression(expression, result, expected, span)?;
        let Type::Result(result_id) = result else {
            unreachable!("managed component calls produce Result values")
        };
        self.semantics.resolve_call(
            expression,
            PendingResolvedCall::ManagedComponent {
                class,
                result: result_id,
                receiver: receiver.value,
                receiver_type: receiver.ty,
            },
        );
        Some(result)
    }

    fn check_user_call_arguments(
        &mut self,
        arguments: &[Expr],
        parameters: &[Type],
        declarations: &[FunctionParameterDeclaration],
    ) {
        debug_assert_eq!(arguments.len(), parameters.len());
        debug_assert_eq!(parameters.len(), declarations.len());
        for ((argument, parameter), declaration) in arguments
            .iter()
            .zip(parameters.iter().copied())
            .zip(declarations)
        {
            let expected = self.shallow_type(parameter);
            let label = if let Type::Variable(variable) = expected {
                let requirements = self.inference.variable_requirements(variable);
                let capabilities = self
                    .standard_library
                    .minimal_capabilities(requirements.as_slice())
                    .into_iter()
                    .map(|capability| self.standard_library.capability(capability).name)
                    .collect::<Vec<_>>();
                if capabilities.is_empty() {
                    format!("parameter `{}` is declared here", declaration.name)
                } else {
                    format!(
                        "parameter `{}` requires `{}`",
                        declaration.name,
                        capabilities.join("` + `")
                    )
                }
            } else if let Type::Array(array) = expected
                && self.inference.array_shape(array) == ArrayShape::Growable
            {
                let expected_name = self.type_name(expected);
                format!(
                    "parameter `{}` requires growable `{expected_name}`",
                    declaration.name
                )
            } else {
                let expected_name = self.type_name(expected);
                format!(
                    "parameter `{}` requires `{expected_name}`",
                    declaration.name
                )
            };
            let source = ExpectedTypeSource {
                span: declaration.span,
                label,
            };
            self.with_expected_type_source(source, |checker| {
                checker.expr(argument, Some(parameter));
            });
        }
    }

    pub(super) fn function_name_suggestion(&self, callee: &[String]) -> Option<String> {
        let (name, prefix) = callee.split_last()?;
        let prefix = prefix.iter().map(String::as_str).collect::<Vec<_>>();
        let mut candidates = self
            .standard_library
            .items_with_path_prefix(&prefix)
            .map(|item| item.name)
            .collect::<Vec<_>>();
        if prefix.is_empty() {
            candidates.extend(self.declarations.functions.keys().map(String::as_str));
            candidates.extend(["Some", "Ok", "Err"]);
        }
        closest_name(name, candidates.into_iter())
    }

    pub(super) fn method_name_suggestion(
        &mut self,
        receiver: Type,
        method: &str,
    ) -> Option<String> {
        let standard_library = self.standard_library.clone();
        let mut candidates = Vec::new();
        for item in standard_library.items() {
            let ItemKind::Method { .. } = item.kind else {
                continue;
            };
            let candidate = CallCandidate { item };
            if self.catalog_candidate_may_apply(&candidate, receiver) {
                candidates.push(item.name.to_owned());
            }
        }
        candidates.extend(
            self.declarations
                .methods
                .keys()
                .filter(|(candidate_receiver, _)| *candidate_receiver == receiver)
                .map(|(_, name)| name.clone()),
        );
        if let Some(migration) = foreign_spelling(method, ForeignSpellingContext::Method)
            && candidates
                .iter()
                .any(|candidate| candidate == migration.replacement.text())
        {
            return Some(migration.replacement.text().to_owned());
        }
        closest_name(method, candidates.iter().map(String::as_str))
    }

    pub(super) fn unknown_function(
        &mut self,
        callee: &[String],
        name_span: Span,
        span: Span,
        suggestion: Option<&str>,
    ) {
        let name = callee.join(".");
        let Some(suggestion) = suggestion else {
            self.error(format!("unknown function `{name}`"), span);
            return;
        };
        let suggested_name = callee[..callee.len() - 1]
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(suggestion))
            .collect::<Vec<_>>()
            .join(".");
        self.errors.push(
            Diagnostic::type_error(
                format!("unknown function `{name}`; did you mean `{suggested_name}`?"),
                name_span,
            )
            .with_primary_label("this name is not defined")
            .with_machine_applicable_fix(
                format!("replace `{}` with `{suggestion}`", callee.last().unwrap()),
                name_span,
                suggestion,
            ),
        );
    }

    pub(super) fn unknown_method(
        &mut self,
        receiver: Type,
        method: &str,
        name_span: Span,
        span: Span,
        suggestion: Option<&str>,
    ) {
        if let Some(id) = legacy_managed_method_diagnostic(method) {
            self.migration_member_error(id, name_span, None);
            return;
        }
        if matches!(
            receiver,
            Type::Known(id)
                if matches!(
                    self.inference.type_store().kind(id),
                    TypeKind::Standard(StdlibTypeId::String)
                )
        ) && let Some(id) = legacy_string_method_diagnostic(method)
        {
            let metadata =
                migration_diagnostic(id).expect("type checker migration diagnostic IDs must exist");
            let mut diagnostic = Diagnostic::type_error(metadata.message, name_span)
                .with_primary_label(metadata.primary_label)
                .with_migration_topic(metadata.concept.as_str());
            for note in metadata.notes {
                diagnostic = diagnostic.with_note(*note);
            }
            self.errors.push(diagnostic);
            return;
        }
        if method == "Add"
            && matches!(
                receiver,
                Type::Known(id)
                    if matches!(self.inference.type_store().kind(id), TypeKind::SettingsView)
            )
        {
            let metadata = migration_diagnostic(ASL_SETTINGS_ADD_DIAGNOSTIC)
                .expect("type checker migration diagnostic IDs must exist");
            let mut diagnostic = Diagnostic::type_error(metadata.message, name_span)
                .with_primary_label(metadata.primary_label)
                .with_migration_topic(metadata.concept.as_str());
            for note in metadata.notes {
                diagnostic = diagnostic.with_note(*note);
            }
            self.errors.push(diagnostic);
            return;
        }
        if method == "ContainsKey"
            && matches!(
                receiver,
                Type::Known(id)
                    if matches!(self.inference.type_store().kind(id), TypeKind::SettingsView)
            )
        {
            self.migration_member_error(
                ASL_SETTINGS_LOOKUP_DIAGNOSTIC,
                name_span,
                Some(("replace `ContainsKey` with `contains`", "contains")),
            );
            return;
        }
        let receiver = self.type_name(receiver);
        let Some(suggestion) = suggestion else {
            self.error(format!("type `{receiver}` has no method `{method}`"), span);
            return;
        };
        self.errors.push(
            Diagnostic::type_error(
                format!("type `{receiver}` has no method `{method}`; did you mean `{suggestion}`?"),
                name_span,
            )
            .with_primary_label("this method is not defined for the receiver type")
            .with_machine_applicable_fix(
                format!("replace `{method}` with `{suggestion}`"),
                name_span,
                suggestion,
            ),
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn catalog_call(
        &mut self,
        candidate: &CallCandidate,
        receiver: Option<MethodReceiver>,
        explicit_type_arguments: &[crate::ast::TypeRef],
        args: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        let item = candidate.item;
        if !explicit_type_arguments.is_empty()
            && explicit_type_arguments.len() != item.signature.explicit_type_parameters
        {
            self.error(
                format!(
                    "`{}` expects {} type arguments, found {}",
                    item.qualified_name,
                    item.signature.explicit_type_parameters,
                    explicit_type_arguments.len()
                ),
                span,
            );
            return None;
        }
        let mut variables = HashMap::new();
        let inherited_type_parameters = item
            .signature
            .type_parameters
            .len()
            .saturating_sub(item.signature.explicit_type_parameters);
        for (index, parameter) in item.signature.type_parameters.iter().enumerate() {
            let requirements = parameter
                .constraints
                .iter()
                .fold(Requirements::none(), |requirements, constraint| {
                    requirements | Requirements::capability(*constraint)
                });
            let explicit_index = index.checked_sub(inherited_type_parameters);
            let explicitly_selected = explicit_index
                .and_then(|index| explicit_type_arguments.get(index))
                .is_some();
            let ty = explicit_index
                .and_then(|index| explicit_type_arguments.get(index))
                .map(|ty| self.syntax_type(*ty))
                .unwrap_or_else(|| self.fresh_inference(requirements.clone(), None));
            if !requirements.is_empty() {
                if explicitly_selected {
                    if let Err(error) = self.inference.require(ty, requirements) {
                        let message = self.inference_error_message(error);
                        self.error(message, span);
                        return None;
                    }
                } else {
                    self.require(ty, requirements, span)?;
                }
            }
            variables.insert(parameter.name, ty);
        }
        if item.id == StdlibItemId::ProcessRead && explicit_type_arguments.is_empty() {
            self.inferred_process_reads.push((variables["T"], span));
        }
        if item.id == StdlibItemId::SetNew && explicit_type_arguments.is_empty() {
            self.inferred_empty_collections
                .push((variables["U"], span, "element", "set"));
        }
        if item.id == StdlibItemId::MapNew && explicit_type_arguments.is_empty() {
            self.inferred_empty_collections
                .push((variables["A"], span, "key", "map"));
            self.inferred_empty_collections
                .push((variables["B"], span, "value", "map"));
        }
        let mut concrete_signature = Vec::new();
        if let Some(receiver) = &receiver {
            let declared_receiver = self.catalog_type(
                candidate
                    .receiver()
                    .expect("method candidates declare a receiver"),
                &variables,
            );
            self.unify(receiver.ty, declared_receiver, span)?;
            let receiver_type = self.shallow_type(receiver.ty);
            if let Some((constructor, arguments)) = self.constructed_field_receiver(receiver_type) {
                let declaration = self.standard_library.type_constructor(constructor);
                let constructor_variables = declaration
                    .parameters
                    .iter()
                    .zip(arguments)
                    .map(|(parameter, argument)| (parameter.name, argument))
                    .collect::<HashMap<_, _>>();
                let definitions = declaration.associated_types.to_vec();
                for definition in definitions {
                    let value = self.catalog_type(definition.value, &constructor_variables);
                    variables.insert(definition.name, value);
                }
            }
            if matches!(
                item.id,
                StdlibItemId::ArrayPush
                    | StdlibItemId::ArrayExtend
                    | StdlibItemId::ArrayRemoveAt
                    | StdlibItemId::ArrayRemove
                    | StdlibItemId::ArrayPop
                    | StdlibItemId::ArrayClear
            ) && let Type::Array(array) = self.shallow_type(receiver.ty)
                && self.inference.require_growable_array(array).is_err()
            {
                let fixed = self.inference.array_length(array).map_or_else(
                    || "fixed array".to_owned(),
                    |length| format!("fixed array `[T; {length}]`"),
                );
                let method = match item.id {
                    StdlibItemId::ArrayPush => "push",
                    StdlibItemId::ArrayExtend => "extend",
                    StdlibItemId::ArrayRemoveAt => "removeAt",
                    StdlibItemId::ArrayRemove => "remove",
                    StdlibItemId::ArrayPop => "pop",
                    StdlibItemId::ArrayClear => "clear",
                    _ => unreachable!(),
                };
                self.error(
                    format!("cannot change the length of {fixed}; `{method}` is only available on growable `[T]`"),
                    span,
                );
                return None;
            }
            concrete_signature.push(declared_receiver);
        }
        if let StdlibOwner::Capability(capability) = item.owner {
            let receiver = receiver
                .as_ref()
                .expect("capability members are receiver methods")
                .ty;
            for (owner, associated) in self
                .standard_library
                .capability_associated_types(capability)
            {
                let value = self
                    .inference
                    .associated_type(receiver, owner, associated.name);
                variables.insert(associated.name, value);
            }
        }
        let operation = self.standard_library.operation_semantics(item.id);
        if args.len() != item.signature.parameters.len() {
            self.error(
                format!(
                    "`{}` expects {} arguments, found {}",
                    item.qualified_name,
                    item.signature.parameters.len(),
                    args.len()
                ),
                span,
            );
            return None;
        }
        let failure_channel = match item.implementation {
            crate::stdlib::Implementation::Intrinsic(intrinsic) => {
                crate::intrinsic_registry::contract(intrinsic).failure_channel
            }
            _ => FailureChannelPolicy::Declared,
        };
        let (result_type, result) =
            if let FailureChannelPolicy::ReuseAsyncArgument(index) = failure_channel {
                // Failure-augmenting operations use the language's one ordinary
                // error channel. Let every argument constrain the signature first,
                // then reuse an async operand's existing `U!` completion rather
                // than manufacturing an inferred `U!!`.
                let mut argument_types = Vec::with_capacity(args.len());
                for (argument, parameter) in args.iter().zip(item.signature.parameters) {
                    let parameter_type = self.catalog_type(parameter.ty, &variables);
                    let argument_type = self
                        .expr(argument, Some(parameter_type))
                        .unwrap_or(parameter_type);
                    argument_types.push(argument_type);
                    self.validate_catalog_argument(argument, parameter.rule, item);
                    concrete_signature.push(parameter_type);
                }
                let argument_type = self.shallow_type(argument_types[usize::from(index)]);
                let Type::Async(future) = argument_type else {
                    unreachable!("failure-channel metadata must select an async argument")
                };
                let value = self.inference.async_value(future);
                let value = self.shallow_type(value);
                let completion_type = match value {
                    Type::Result(result) => Type::Result(result),
                    value => Type::Result(self.inference.result_type(value)),
                };
                let result_type = Type::Async(self.inference.async_type(completion_type));
                let result = self.expect_expression(expression, result_type, expected, span)?;
                (result_type, result)
            } else {
                let expected_result = expected.map(|ty| self.shallow_type(ty));
                let expected_completion = expected_result.map(|expected| match expected {
                    Type::Async(future) => self.inference.async_value(future),
                    expected => expected,
                });
                let completion_type = if let (Some(value), Some(Type::Result(result))) = (
                    catalog_type_argument(item.signature.result, StdlibTypeConstructorId::Result),
                    expected_completion,
                ) {
                    let declared_value = self.catalog_type(value, &variables);
                    let expected_value = self.inference.result_value(result);
                    self.unify(declared_value, expected_value, span)?;
                    Type::Result(result)
                } else {
                    self.catalog_type(item.signature.result, &variables)
                };
                let result_type = if item.signature.result_is_async {
                    Type::Async(self.inference.async_type(completion_type))
                } else {
                    completion_type
                };
                let result = self.expect_expression(expression, result_type, expected, span)?;
                for (argument, parameter) in args.iter().zip(item.signature.parameters) {
                    let parameter_type = self.catalog_type(parameter.ty, &variables);
                    self.expr(argument, Some(parameter_type));
                    self.validate_catalog_argument(argument, parameter.rule, item);
                    concrete_signature.push(parameter_type);
                }
                (result_type, result)
            };
        if matches!(
            item.id,
            StdlibItemId::SettingsViewEnabled | StdlibItemId::SettingsViewContains
        ) && let Some(argument) = args.first()
        {
            self.validate_setting_key_argument(item.id, argument);
        }
        concrete_signature.push(result_type);
        if operation.availability == Availability::OnAttach && !self.callable.can_suspend() {
            self.error(
                format!(
                    "`{}` must be awaited in `onAttach`, `whileAttached`, or an inferred async function",
                    item.qualified_name
                ),
                span,
            );
        }
        let type_arguments = item
            .signature
            .type_parameters
            .iter()
            .map(|parameter| variables[parameter.name])
            .collect();
        self.semantics.resolve_call(
            expression,
            PendingResolvedCall::StandardLibrary {
                item: item.id,
                type_arguments,
                signature: concrete_signature,
                receiver: receiver.as_ref().map(|receiver| receiver.value.clone()),
                receiver_type: receiver.as_ref().map(|receiver| receiver.ty),
            },
        );
        Some(result)
    }

    pub(super) fn catalog_candidate_may_apply(
        &mut self,
        candidate: &CallCandidate,
        receiver: Type,
    ) -> bool {
        let receiver = self.shallow_type(receiver);
        let declared = candidate
            .receiver()
            .expect("only method candidates are matched against receivers");
        match declared {
            CatalogTypeRef::Core(expected) => {
                let expected = self.declared_type(DeclaredTypeRef::Core(expected));
                matches!(receiver, Type::Variable(_)) || receiver == expected
            }
            CatalogTypeRef::Standard(standard) => {
                matches!(receiver, Type::Variable(_)) || receiver == self.standard_type(standard)
            }
            CatalogTypeRef::Application { constructor, .. } => {
                matches!(receiver, Type::Variable(_))
                    || (constructor == StdlibTypeConstructorId::Array
                        && matches!(receiver, Type::Array(_)))
                    || (constructor == StdlibTypeConstructorId::Option
                        && matches!(receiver, Type::Option(_)))
                    || (constructor == StdlibTypeConstructorId::Result
                        && matches!(receiver, Type::Result(_)))
                    || (constructor == StdlibTypeConstructorId::Set
                        && matches!(receiver, Type::Set(_)))
                    || matches!(receiver, Type::Application(application)
                        if self.inference.application_constructor(application) == constructor)
                    || (constructor == StdlibTypeConstructorId::ExclusiveRange
                        && matches!(receiver, Type::Range(range) if self.inference.range_kind(range) == crate::ast::RangeKind::Exclusive))
                    || (constructor == StdlibTypeConstructorId::InclusiveRange
                        && matches!(receiver, Type::Range(range) if self.inference.range_kind(range) == crate::ast::RangeKind::Inclusive))
            }
            CatalogTypeRef::FixedArray { length, .. } => {
                matches!(receiver, Type::Variable(_))
                    || matches!(receiver, Type::Array(array) if self.inference.array_length(array) == Some(length))
            }
            CatalogTypeRef::Parameter(name) => candidate
                .item
                .signature
                .type_parameters
                .iter()
                .find(|parameter| parameter.name == name)
                .is_none_or(|parameter| {
                    parameter.constraints.iter().all(|constraint| {
                        matches!(receiver, Type::Variable(_))
                            || type_may_have_capability(
                                &self.standard_library,
                                self.inference.type_store(),
                                receiver,
                                *constraint,
                            )
                    })
                }),
            CatalogTypeRef::Associated(_) => false,
            CatalogTypeRef::Async(_) => matches!(receiver, Type::Async(_)),
            CatalogTypeRef::Iterator(_) => matches!(receiver, Type::Iterator(_)),
            CatalogTypeRef::Callable { .. } => matches!(receiver, Type::Callable(_)),
        }
    }

    fn prefer_specific_catalog_candidates(
        &mut self,
        candidates: &mut Vec<CallCandidate>,
        receiver: Type,
    ) {
        if candidates.len() < 2 {
            return;
        }
        let receiver = self.shallow_type(receiver);
        let receiver_is_inferred = matches!(receiver, Type::Variable(_));
        if let Type::Variable(variable) = receiver {
            let requirements = self.inference.variable_requirements(variable);
            let capability_candidates = candidates
                .iter()
                .filter(|candidate| {
                    matches!(
                        candidate.item.owner,
                        StdlibOwner::Capability(capability)
                            if requirements.as_slice().contains(&capability)
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            if !capability_candidates.is_empty() {
                *candidates = capability_candidates;
                return;
            }
        }
        if receiver_is_inferred && let CallableContext::LibraryFunction(item) = self.callable {
            let owner = self.standard_library.item(item).owner;
            if candidates
                .iter()
                .any(|candidate| candidate.item.owner == owner)
            {
                candidates.retain(|candidate| candidate.item.owner == owner);
            }
        }
        let specificity = |candidate: &CallCandidate| match candidate.receiver() {
            Some(CatalogTypeRef::Parameter(_)) => 1,
            Some(_) if receiver_is_inferred => 0,
            Some(_) => 2,
            None => 0,
        };
        let strongest = candidates.iter().map(specificity).max().unwrap_or_default();
        candidates.retain(|candidate| specificity(candidate) == strongest);
    }

    /// Resolve overloads by the syntax written at the call site before using
    /// receiver specificity. Keeping this independent of type unification
    /// means methods inherited through capabilities can safely share a name
    /// with a concrete overload of another arity, such as `Display.toString()`
    /// and `Integer.toString(radix)`.
    fn prefer_matching_catalog_call_shape(
        candidates: &mut Vec<CallCandidate>,
        explicit_type_arguments: usize,
        arguments: usize,
    ) {
        let matches = |candidate: &&CallCandidate| {
            candidate.item.signature.parameters.len() == arguments
                && (explicit_type_arguments == 0
                    || candidate.item.signature.explicit_type_parameters == explicit_type_arguments)
        };
        if candidates.iter().filter(matches).count() > 0 {
            candidates.retain(|candidate| matches(&candidate));
        }
    }

    pub(super) fn ambiguous_catalog_call(
        &mut self,
        callee: &[String],
        candidates: &[CallCandidate],
        span: Span,
    ) {
        let names = candidates
            .iter()
            .map(|candidate| candidate.item.qualified_name)
            .collect::<Vec<_>>()
            .join(", ");
        self.error(
            format!(
                "call to `{}` is ambiguous between {names}",
                callee.join(".")
            ),
            span,
        );
    }

    pub(super) fn catalog_type(
        &mut self,
        ty: CatalogTypeRef,
        variables: &HashMap<&'static str, Type>,
    ) -> Type {
        match ty {
            CatalogTypeRef::Core(core) => self.declared_type(DeclaredTypeRef::Core(core)),
            CatalogTypeRef::Standard(standard) => self.standard_type(standard),
            CatalogTypeRef::Parameter(name) | CatalogTypeRef::Associated(name) => variables[name],
            CatalogTypeRef::Async(value) => {
                let value = self.catalog_type(*value, variables);
                Type::Async(self.inference.async_type(value))
            }
            CatalogTypeRef::Iterator(item) => {
                let item = self.catalog_type(*item, variables);
                Type::Iterator(self.inference.iterator_type(item))
            }
            CatalogTypeRef::FixedArray { element, length } => {
                let element = self.catalog_type(*element, variables);
                Type::Array(self.inference.array_type_with_length(element, Some(length)))
            }
            CatalogTypeRef::Callable { parameters, result } => {
                let parameters = parameters
                    .iter()
                    .map(|parameter| self.catalog_type(*parameter, variables))
                    .collect();
                let result = self.catalog_type(*result, variables);
                Type::Callable(self.inference.callable_type(parameters, result))
            }
            CatalogTypeRef::Application {
                constructor,
                arguments,
            } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| self.catalog_type(*argument, variables))
                    .collect();
                self.catalog_application_type(constructor, arguments)
            }
        }
    }

    pub(super) fn catalog_application_type(
        &mut self,
        constructor: StdlibTypeConstructorId,
        arguments: Vec<Type>,
    ) -> Type {
        let [value] = arguments.as_slice() else {
            return Type::Application(self.inference.application_type(constructor, arguments));
        };
        let value = *value;
        if constructor == StdlibTypeConstructorId::Array {
            Type::Array(self.inference.inferred_array_type(value))
        } else if constructor == StdlibTypeConstructorId::Option {
            Type::Option(self.inference.option_type(value))
        } else if constructor == StdlibTypeConstructorId::Result {
            Type::Result(self.inference.result_type(value))
        } else if constructor == StdlibTypeConstructorId::Set {
            Type::Set(self.inference.set_type(value))
        } else if constructor == StdlibTypeConstructorId::ExclusiveRange {
            Type::Range(
                self.inference
                    .range_type(value, crate::ast::RangeKind::Exclusive),
            )
        } else if constructor == StdlibTypeConstructorId::InclusiveRange {
            Type::Range(
                self.inference
                    .range_type(value, crate::ast::RangeKind::Inclusive),
            )
        } else {
            Type::Application(self.inference.application_type(constructor, arguments))
        }
    }

    pub(super) fn validate_catalog_argument(
        &mut self,
        argument: &Expr,
        rule: ParameterRule,
        item: &StdlibItem,
    ) {
        match rule {
            ParameterRule::Value => {}
            ParameterRule::StringLiteral if !matches!(argument.kind, ExprKind::String(_)) => {
                self.error(
                    format!("`{}` expects a string literal", item.qualified_name),
                    argument.span,
                );
            }
            ParameterRule::SignatureLiteral => match &argument.kind {
                ExprKind::Signature(signature) => {
                    if let Err(message) = parse_signature(signature) {
                        self.error(message, argument.span);
                    }
                }
                _ => self.error(
                    format!("`{}` expects a `sig\"...\"` literal", item.qualified_name),
                    argument.span,
                ),
            },
            ParameterRule::StringLiteral => {}
        }
    }

    fn validate_setting_key_argument(&mut self, item: StdlibItemId, argument: &Expr) {
        let ExprKind::String(key) = &argument.kind else {
            // Data-driven keys are deliberately supported. Only literals can
            // be proven invalid against this program's declarations.
            return;
        };
        let accepts = |declaration: &RuntimeSettingDeclaration| match item {
            StdlibItemId::SettingsViewEnabled => declaration.kind == RuntimeSettingKind::Bool,
            StdlibItemId::SettingsViewContains => declaration.kind != RuntimeSettingKind::Title,
            _ => unreachable!(),
        };
        if let Some(declaration) = self.declarations.settings_by_runtime_key.get(key).cloned() {
            if accepts(&declaration) {
                return;
            }
            let (actual, expected) = match declaration.kind {
                RuntimeSettingKind::Text => ("a text setting", "a boolean setting"),
                RuntimeSettingKind::Choice => ("a choice setting", "a boolean setting"),
                RuntimeSettingKind::File => ("a file setting", "a boolean setting"),
                RuntimeSettingKind::Title => ("a settings heading", "a value setting"),
                RuntimeSettingKind::Bool => unreachable!(),
            };
            let mut diagnostic = Diagnostic::type_error(
                format!("setting key `{key}` names {actual}, not {expected}"),
                argument.span,
            )
            .with_primary_label(format!(
                "`{}` cannot read this declaration",
                self.standard_library.item(item).qualified_name
            ))
            .with_secondary_label(declaration.span, "the setting is declared here");
            if let Some(name) = declaration.source_name {
                diagnostic = diagnostic.with_note(format!(
                    "read this statically declared setting as `settings.{name}` or `oldSettings.{name}`"
                ));
            }
            self.errors.push(diagnostic);
            return;
        }

        let suggestion = closest_name(
            key,
            self.declarations
                .settings_by_runtime_key
                .iter()
                .filter(|(_, declaration)| accepts(declaration))
                .map(|(key, _)| key.as_str()),
        );
        let qualified_name = self.standard_library.item(item).qualified_name;
        let dynamic_note = if item == StdlibItemId::SettingsViewEnabled {
            "computed string keys remain valid for data-driven lookup; use `settings.contains(key)` when unknown and disabled settings must be distinguished"
        } else {
            "computed string keys remain valid for data-driven lookup and return false when no value setting declares the runtime key"
        };
        let mut diagnostic = Diagnostic::type_error(
            format!("unknown setting key `{key}` passed to `{qualified_name}`"),
            argument.span,
        )
        .with_primary_label("no compatible setting declares this exact host key")
        .with_note(dynamic_note);
        if let Some(suggestion) = suggestion
            && let Some(declaration) = self.declarations.settings_by_runtime_key.get(&suggestion)
        {
            diagnostic = diagnostic
                .with_secondary_label(declaration.span, "the closest declared key is here")
                .with_machine_applicable_fix(
                    format!("replace `{key}` with `{suggestion}`"),
                    argument.span,
                    quote_string_literal(&suggestion),
                );
        }
        self.errors.push(diagnostic);
    }

    pub(super) fn array_type_id(&mut self, element: Type) -> ArrayTypeId {
        self.inference.array_type(element)
    }

    pub(super) fn path(
        &mut self,
        path: &[String],
        span: Span,
        expression: Option<ExprId>,
    ) -> Option<PathResolution> {
        let qualified = path.join(".");
        let constant = if self.is_library_function() {
            self.standard_library
                .item_by_name_including_private(&qualified)
        } else {
            self.standard_library.item_by_name(&qualified)
        }
        .filter(|item| item.kind == ItemKind::Constant);
        if let Some(constant) = constant {
            debug_assert!(constant.signature.type_parameters.is_empty());
            debug_assert!(constant.signature.parameters.is_empty());
            debug_assert!(!constant.signature.result_is_async);
            let ty = self.catalog_type(constant.signature.result, &HashMap::new());
            return Some(PathResolution {
                ty,
                value: Some(ResolvedValue::StandardLibraryConstant(constant.id)),
                members: Some(Vec::new()),
            });
        }
        if let [class_name, field_name] = path
            && let Some(class) = self
                .declarations
                .managed_classes
                .iter()
                .find(|class| class.name == *class_name)
                .cloned()
            && let Some(field) = self.visible_managed_field(class.id, true, field_name)
        {
            if !self.managed_provider_available() {
                self.errors.push(
                    Diagnostic::type_error(
                        "live managed fields require a Unity state provider",
                        span,
                    )
                    .with_primary_label(
                        "declare the state as `state Unity [\"game.exe\"] { ... }`",
                    ),
                );
                return None;
            }
            let value = self.managed_read_value_type(field.ty);
            let storage = self.syntax_type(field.ty);
            self.prepare_managed_collection_read(storage);
            return Some(PathResolution {
                ty: Type::Result(self.inference.result_type(value)),
                value: Some(ResolvedValue::ManagedStatic {
                    class: class.id,
                    field: field.id,
                }),
                members: Some(Vec::new()),
            });
        }
        if let [class_name, field_name] = path
            && let Some(class) = self
                .declarations
                .managed_classes
                .iter()
                .find(|class| class.name == *class_name)
            && class.conditional_fields.iter().any(|group| {
                group
                    .fields
                    .iter()
                    .any(|field| field.is_static && field.name == *field_name)
            })
        {
            self.error(
                format!(
                    "managed field `{class_name}.{field_name}` is conditional; access it only where its shape condition is established"
                ),
                span,
            );
            return None;
        }
        if let [class_name, field_name, next, ..] = path
            && self
                .declarations
                .managed_classes
                .iter()
                .find(|class| class.name == *class_name)
                .is_some_and(|class| {
                    class
                        .fields
                        .iter()
                        .any(|field| field.is_static && field.name == *field_name)
                })
        {
            self.errors.push(
                Diagnostic::type_error(
                    format!(
                        "managed field `{class_name}.{field_name}` is fallible; use `?` before accessing `{next}`"
                    ),
                    span,
                )
                .with_primary_label(format!(
                    "write `{class_name}.{field_name}?.{next}` to propagate a failed remote read"
                )),
            );
            return None;
        }
        match path {
            [name, fields @ ..]
                if matches!(
                    name.as_str(),
                    "current" | "old" | "settings" | "oldSettings"
                ) && self.binding(name).is_some() =>
            {
                let binding = self
                    .binding_for_use(name, span)
                    .expect("the shadowing binding was found above");
                let (ty, members) =
                    self.resolve_members_or_defer(binding.ty, fields, span, expression)?;
                Some(PathResolution {
                    ty,
                    value: binding.id.map(ResolvedValue::Variable),
                    members,
                })
            }
            [root] if root == "current" || root == "old" => {
                self.require_state_snapshot(span)?;
                Some(PathResolution {
                    ty: Type::Known(self.inference.type_store().id_for_state_snapshot()),
                    value: Some(if root == "current" {
                        ResolvedValue::CurrentSnapshot
                    } else {
                        ResolvedValue::OldSnapshot
                    }),
                    members: Some(Vec::new()),
                })
            }
            [root, field, fields @ ..] if root == "current" || root == "old" => {
                self.require_state_snapshot(span)?;
                let Some((id, ty)) = self.visible_state_field(field) else {
                    self.unknown_state_field(field, span);
                    return None;
                };
                let (ty, members) = self.resolve_members_or_defer(ty, fields, span, expression)?;
                let value = if root == "current" {
                    ResolvedValue::CurrentState(id)
                } else {
                    ResolvedValue::OldState(id)
                };
                Some(PathResolution {
                    ty,
                    value: Some(value),
                    members,
                })
            }
            [root, field, fields @ ..] if root == "settings" || root == "oldSettings" => {
                let Some((id, ty)) = self.declarations.settings.get(field).copied() else {
                    self.error(format!("unknown setting `{field}`"), span);
                    return None;
                };
                let (ty, members) = self.resolve_members_or_defer(ty, fields, span, expression)?;
                let value = if root == "settings" {
                    ResolvedValue::Setting(id)
                } else {
                    ResolvedValue::OldSetting(id)
                };
                Some(PathResolution {
                    ty,
                    value: Some(value),
                    members,
                })
            }
            [root] if root == "settings" || root == "oldSettings" => Some(PathResolution {
                ty: Type::Known(self.inference.type_store().id_for_settings_view()),
                value: Some(if root == "settings" {
                    ResolvedValue::SettingsView
                } else {
                    ResolvedValue::OldSettingsView
                }),
                members: Some(Vec::new()),
            }),
            [name, fields @ ..]
                if matches!(
                    self.callable,
                    CallableContext::Action(ActionKind::SelectProcess)
                ) && self
                    .standard_library
                    .default_state_provider()
                    .is_some_and(|provider| provider.value_name == name) =>
            {
                let provider = self
                    .standard_library
                    .default_state_provider()
                    .expect("the selector root matched the native provider");
                let provider_type = self.standard_type(provider.process_type);
                let (ty, members) =
                    self.resolve_members_or_defer(provider_type, fields, span, expression)?;
                Some(PathResolution {
                    ty,
                    value: Some(ResolvedValue::ProviderValue(provider.id)),
                    members,
                })
            }
            [name, fields @ ..]
                if !matches!(
                    self.callable,
                    CallableContext::Action(ActionKind::SelectProcess)
                ) && self.provider_value_by_name(name).is_some() =>
            {
                let (provider, provider_type) = self
                    .provider_value_by_name(name)
                    .expect("provider name was discovered above");
                if !matches!(
                    self.callable,
                    CallableContext::LibraryFunction(_) | CallableContext::CompilerGenerated
                ) && self.active_state_provider() != Some(provider)
                {
                    self.error(
                        format!(
                            "`{name}` is available only when the `{}` state provider is selected",
                            self.standard_library.state_provider(provider).name
                        ),
                        span,
                    );
                    return None;
                }
                let (ty, members) =
                    self.resolve_members_or_defer(provider_type, fields, span, expression)?;
                Some(PathResolution {
                    ty,
                    value: Some(ResolvedValue::ProviderValue(provider)),
                    members,
                })
            }
            [name, fields @ ..]
                if !matches!(
                    self.callable,
                    CallableContext::Action(ActionKind::SelectProcess)
                ) && self.provider_context_by_name(name).is_some() =>
            {
                let (provider, context, context_type) = self
                    .provider_context_by_name(name)
                    .expect("provider context name was discovered above");
                if !matches!(
                    self.callable,
                    CallableContext::LibraryFunction(_) | CallableContext::CompilerGenerated
                ) && self.active_state_provider() != Some(provider)
                {
                    self.error(
                        format!(
                            "`{name}` is available only when the `{}` state provider is selected",
                            self.standard_library.state_provider(provider).name
                        ),
                        span,
                    );
                    return None;
                }
                let (ty, members) =
                    self.resolve_members_or_defer(context_type, fields, span, expression)?;
                Some(PathResolution {
                    ty,
                    value: Some(ResolvedValue::ProviderContext {
                        provider,
                        context: context as u32,
                    }),
                    members,
                })
            }
            [name, fields @ ..]
                if matches!(
                    self.callable,
                    CallableContext::LibraryFunction(_) | CallableContext::CompilerGenerated
                ) && self
                    .standard_library
                    .state_providers()
                    .iter()
                    .any(|provider| provider.value_name == name) =>
            {
                let provider = self
                    .standard_library
                    .state_providers()
                    .iter()
                    .find(|provider| provider.value_name == name)
                    .expect("provider value name was discovered above");
                let provider_type = self.standard_type(provider.process_type);
                let (ty, members) =
                    self.resolve_members_or_defer(provider_type, fields, span, expression)?;
                Some(PathResolution {
                    ty,
                    value: Some(ResolvedValue::ProviderValue(provider.id)),
                    members,
                })
            }
            [name, fields @ ..]
                if self.expression_mode == ExpressionMode::StateSource
                    && self
                        .scopes
                        .iter()
                        .rev()
                        .all(|scope| !scope.contains_key(name))
                    && self.visible_state_source_field(name).is_some() =>
            {
                let (field, field_type) = self
                    .visible_state_source_field(name)
                    .expect("the sibling state field was found above");
                let owner = self
                    .active_state_field
                    .expect("state-source expressions have an active field");
                self.semantics.resolve_state_dependency(owner, field);
                let (ty, members) =
                    self.resolve_members_or_defer(field_type, fields, span, expression)?;
                Some(PathResolution {
                    ty,
                    value: Some(ResolvedValue::StateCandidate(field)),
                    members,
                })
            }
            [name, fields @ ..] => {
                let Some(binding) = self.binding_for_use(name, span) else {
                    let spelling = path.join(".");
                    if let Some(id) = legacy_value_path_diagnostic(&spelling) {
                        let metadata = migration_diagnostic(id)
                            .expect("type checker migration diagnostic IDs must exist");
                        let mut diagnostic = Diagnostic::type_error(metadata.message, span)
                            .with_primary_label(metadata.primary_label)
                            .with_migration_topic(metadata.concept.as_str());
                        for note in metadata.notes {
                            diagnostic = diagnostic.with_note(*note);
                        }
                        self.errors.push(diagnostic);
                        return None;
                    }
                    let ordinary_rule =
                        foreign_spelling(&spelling, ForeignSpellingContext::ValuePath);
                    let attached_process_rule = foreign_spelling(
                        &spelling,
                        ForeignSpellingContext::AttachedProcessValuePath,
                    );
                    if let Some((rule, requires_attached_process)) = ordinary_rule
                        .map(|rule| (rule, false))
                        .or_else(|| attached_process_rule.map(|rule| (rule, true)))
                    {
                        let mut diagnostic = Diagnostic::type_error(rule.message, span)
                            .with_primary_label(rule.primary_label);
                        let process_context_is_available = self.expression_mode
                            == ExpressionMode::StateSource
                            || matches!(
                                self.callable,
                                CallableContext::LibraryFunction(_)
                                    | CallableContext::CompilerGenerated
                                    | CallableContext::Action(
                                        ActionKind::OnAttach
                                            | ActionKind::OnStateReady
                                            | ActionKind::WhileAttached
                                            | ActionKind::Start
                                            | ActionKind::Split
                                            | ActionKind::Reset
                                            | ActionKind::IsLoading
                                            | ActionKind::GameTime
                                    )
                            );
                        let process_is_available = process_context_is_available
                            && self.provider_value.is_some_and(|(provider, _)| {
                                self.standard_library.state_provider(provider).value_name
                                    == "process"
                            });
                        if !requires_attached_process || process_is_available {
                            diagnostic = diagnostic.with_machine_applicable_fix(
                                rule.fix_title,
                                span,
                                rule.replacement.text(),
                            );
                        } else {
                            diagnostic = diagnostic.with_note(
                                "the `process` value is available only in native-process attachment and attached lifecycle contexts; pass the name into an ordinary function explicitly",
                            );
                        }
                        self.errors.push(diagnostic);
                        return None;
                    }
                    self.unknown_variable(name, span);
                    return None;
                };
                let (ty, members) =
                    self.resolve_members_or_defer(binding.ty, fields, span, expression)?;
                Some(PathResolution {
                    ty,
                    value: binding.id.map(ResolvedValue::Variable),
                    members,
                })
            }
            _ => {
                self.error(format!("unknown value `{}`", path.join(".")), span);
                None
            }
        }
    }

    fn require_state_snapshot(&mut self, span: Span) -> Option<()> {
        if self.expression_mode == ExpressionMode::StateSource {
            self.error(
                "a state field cannot read from its own `current` or `old` snapshot",
                span,
            );
            return None;
        }
        if matches!(self.callable, CallableContext::Function) {
            return Some(());
        }
        if !matches!(self.callable, CallableContext::Action(_)) {
            self.error(
                "state snapshots are only available in lifecycle actions",
                span,
            );
            return None;
        }
        if let CallableContext::Action(action) = self.callable
            && !crate::effects::action_has_state_snapshots(action)
        {
            let message = match action {
                ActionKind::Setup => "state snapshots are not available during `setup`",
                ActionKind::SelectProcess => {
                    "state snapshots are unavailable while selecting a process candidate"
                }
                ActionKind::OnAttach => {
                    "state snapshots are not available until `onAttach` completes"
                }
                ActionKind::OnDetach => "state snapshots are not guaranteed to exist in `onDetach`",
                ActionKind::OnStart => {
                    "state snapshots are unavailable in the timer-global `onStart` action"
                }
                ActionKind::OnReset => {
                    "state snapshots are unavailable in the timer-global `onReset` action"
                }
                _ => unreachable!("the remaining actions have committed snapshots"),
            };
            self.errors.push(
                crate::Diagnostic::type_error(message, span)
                    .with_migration_topic("asl.state.helper-snapshots"),
            );
            return None;
        }
        Some(())
    }

    pub(super) fn resolve_members_or_defer(
        &mut self,
        ty: Type,
        fields: &[String],
        span: Span,
        expression: Option<ExprId>,
    ) -> Option<(Type, Option<Vec<ResolvedMember>>)> {
        if fields.is_empty() {
            return Some((ty, Some(Vec::new())));
        }
        if self.is_error_type(ty) {
            return Some((ty, None));
        }
        if matches!(self.shallow_type(ty), Type::Variable(_))
            && let Some(expression) = expression
        {
            let result = self.fresh_inference(Requirements::none(), None);
            self.deferred_member_paths.push(DeferredMemberPath {
                expression,
                receiver: ty,
                fields: fields.to_vec(),
                result,
                span,
                library_item: match self.callable {
                    CallableContext::LibraryFunction(item) => Some(item),
                    _ => None,
                },
            });
            return Some((result, None));
        }
        let (ty, members) = self.resolve_members(ty, fields, span)?;
        Some((ty, Some(members)))
    }

    pub(super) fn resolve_members(
        &mut self,
        mut ty: Type,
        fields: &[String],
        span: Span,
    ) -> Option<(Type, Vec<ResolvedMember>)> {
        let mut members = Vec::with_capacity(fields.len());
        for (index, field) in fields.iter().enumerate() {
            if index != 0
                && matches!(self.shallow_type(ty), Type::Result(_) | Type::Known(_))
                && members
                    .last()
                    .is_some_and(|member| matches!(member, ResolvedMember::ManagedField(_)))
            {
                let is_result = match self.shallow_type(ty) {
                    Type::Result(_) => true,
                    Type::Known(id) => matches!(
                        self.inference.type_store().kind(id),
                        TypeKind::Result { .. }
                    ),
                    _ => false,
                };
                if is_result {
                    self.errors.push(
                        Diagnostic::type_error(
                            format!(
                                "managed field access is fallible; use `?` before accessing `{field}`"
                            ),
                            span,
                        )
                        .with_primary_label(
                            "propagate the preceding remote read before selecting another field",
                        ),
                    );
                    return None;
                }
            }
            let shallow_type = self.shallow_type(ty);
            let (next, member) = self.resolve_member(shallow_type, field, span)?;
            ty = next;
            members.push(member);
        }
        Some((ty, members))
    }

    pub(super) fn resolve_member(
        &mut self,
        ty: Type,
        field: &str,
        span: Span,
    ) -> Option<(Type, ResolvedMember)> {
        if let Type::Known(id) = ty
            && matches!(
                self.inference.type_store().kind(id),
                TypeKind::ManagedReference(_)
            )
            && !self.managed_provider_available()
        {
            self.errors.push(
                Diagnostic::type_error("live managed fields require a Unity state provider", span)
                    .with_primary_label(
                        "declare the state as `state Unity [\"game.exe\"] { ... }`",
                    ),
            );
            return None;
        }
        if let Some(resolved) = self.lookup_member(ty, field) {
            return Some(resolved);
        }
        if self.standard_type_id(ty) == Some(StdlibTypeId::String)
            && let Some(id) = legacy_string_field_diagnostic(field)
        {
            let name_span = member_name_span(span, field);
            self.migration_member_error(id, name_span, None);
            return None;
        }
        if matches!(ty, Type::Array(_))
            && let Some(id) = legacy_array_field_diagnostic(field)
        {
            let name_span = member_name_span(span, field);
            self.migration_member_error(
                id,
                name_span,
                Some((
                    if field == "Length" {
                        "replace `Length` with `length()`"
                    } else {
                        "replace `Count` with `length()`"
                    },
                    "length()",
                )),
            );
            return None;
        }
        if matches!(ty, Type::Set(_))
            && let Some(id) = legacy_set_field_diagnostic(field)
        {
            let name_span = member_name_span(span, field);
            self.migration_member_error(
                id,
                name_span,
                Some(("replace `Count` with `length()`", "length()")),
            );
            return None;
        }
        match ty {
            Type::Known(id)
                if matches!(
                    self.inference.type_store().kind(id),
                    TypeKind::StateSnapshot
                ) =>
            {
                self.unknown_state_field(field, span)
            }
            Type::Known(_) if self.source_struct_id(ty).is_some() => {
                self.error(format!("unknown struct field `{field}`"), span)
            }
            Type::Known(id)
                if let TypeKind::ManagedClass(class) | TypeKind::ManagedReference(class) =
                    self.inference.type_store().kind(id)
                    && self
                        .declarations
                        .managed_classes
                        .iter()
                        .find(|candidate| candidate.id == *class)
                        .is_some_and(|class| {
                            class
                                .conditional_fields
                                .iter()
                                .any(|group| group.fields.iter().any(|item| item.name == field))
                        }) =>
            {
                let class = self
                    .declarations
                    .managed_classes
                    .iter()
                    .find(|candidate| candidate.id == *class)
                    .expect("the conditional field belongs to a managed class");
                self.error(
                    format!(
                        "managed field `{}.{field}` is conditional; access it only where its shape condition is established",
                        class.name
                    ),
                    span,
                )
            }
            Type::Known(id)
                if matches!(
                    self.inference.type_store().kind(id),
                    TypeKind::ManagedClass(_)
                ) =>
            {
                self.error(format!("unknown managed snapshot field `{field}`"), span)
            }
            Type::Known(id) => {
                let name = self.type_name(Type::Known(id));
                self.error(format!("{name} has no field `{field}`"), span)
            }
            _ => {
                let ty = self.type_name(ty);
                self.error(format!("`{field}` cannot be accessed on `{ty}`"), span);
            }
        }
        None
    }

    fn migration_member_error(
        &mut self,
        id: crate::migration::MigrationDiagnosticId,
        span: Span,
        fix: Option<(&str, &str)>,
    ) {
        let metadata =
            migration_diagnostic(id).expect("type checker migration diagnostic IDs must exist");
        let mut diagnostic = Diagnostic::type_error(metadata.message, span)
            .with_primary_label(metadata.primary_label)
            .with_migration_topic(metadata.concept.as_str());
        for note in metadata.notes {
            diagnostic = diagnostic.with_note(*note);
        }
        if let Some((title, replacement)) = fix {
            diagnostic = diagnostic.with_machine_applicable_fix(title, span, replacement);
        }
        self.errors.push(diagnostic);
    }

    pub(super) fn lookup_member(
        &mut self,
        ty: Type,
        field: &str,
    ) -> Option<(Type, ResolvedMember)> {
        if matches!(
            ty,
            Type::Known(id)
                if matches!(self.inference.type_store().kind(id), TypeKind::StateSnapshot)
        ) {
            return self
                .visible_state_field(field)
                .map(|(field, ty)| (ty, ResolvedMember::StateField(field)));
        }
        if matches!(
            ty,
            Type::Known(id)
                if matches!(self.inference.type_store().kind(id), TypeKind::SettingsView)
        ) {
            return self
                .declarations
                .settings
                .get(field)
                .map(|(setting, ty)| (*ty, ResolvedMember::SettingField(*setting)));
        }
        if let Some(owner) = self.standard_type_id(ty)
            && let Some(field) = self.visible_standard_field(owner, field)
        {
            return Some((
                self.standard_field_type(field.id),
                ResolvedMember::StandardField(field.id),
            ));
        }
        if let Some((constructor, arguments)) = self.constructed_field_receiver(ty)
            && let Some(field) = self.visible_constructor_field(constructor, field)
        {
            let variables = self
                .standard_library
                .type_constructor(constructor)
                .parameters
                .iter()
                .zip(arguments)
                .map(|(parameter, argument)| (parameter.name, argument))
                .collect();
            let field_type = self.catalog_type(field.ty, &variables);
            return Some((field_type, ResolvedMember::StandardField(field.id)));
        }
        if let Type::Known(id) = ty
            && let TypeKind::ManagedClass(class_id) = self.inference.type_store().kind(id)
            && let Some(field) = self.visible_managed_field(*class_id, false, field)
        {
            let declared = self.syntax_type(field.ty);
            let owned = self.managed_owned_type(declared);
            return Some((owned, ResolvedMember::ManagedField(field.id)));
        }
        if let Type::Known(id) = ty
            && let TypeKind::ManagedReference(class_id) = self.inference.type_store().kind(id)
            && let Some(field) = self.visible_managed_field(*class_id, false, field)
        {
            let value = self.managed_read_value_type(field.ty);
            let storage = self.syntax_type(field.ty);
            self.prepare_managed_collection_read(storage);
            return Some((
                Type::Result(self.inference.result_type(value)),
                ResolvedMember::ManagedField(field.id),
            ));
        }
        match self.source_struct_id(ty) {
            Some(struct_id) => self
                .declarations
                .structs
                .iter()
                .find(|structure| structure.id == struct_id)
                .and_then(|structure| structure.fields.iter().find(|item| item.name == field))
                .map(|field| {
                    (
                        self.syntax_type(field.ty),
                        ResolvedMember::StructField(field.id),
                    )
                }),
            None => None,
        }
    }

    pub(super) fn managed_read_value_type(&mut self, declared: crate::ast::TypeRef) -> Type {
        let declared = self.syntax_type(declared);
        let optional_child = match self.inference.shallow(declared) {
            Type::Option(option) => Some(self.inference.option_value(option)),
            Type::Known(id) => match self.inference.type_store().kind(id) {
                TypeKind::Option { value, .. } => Some(Type::Known(*value)),
                _ => None,
            },
            _ => None,
        };
        if let Some(Type::Known(id)) = optional_child
            && let TypeKind::ManagedClass(class) = self.inference.type_store().kind(id)
        {
            let live = Type::Known(self.inference.type_store().id_for_managed_reference(*class));
            return Type::Option(self.inference.option_type(live));
        }
        match declared {
            Type::Known(id)
                if matches!(
                    self.inference.type_store().kind(id),
                    TypeKind::ManagedClass(_)
                ) =>
            {
                let TypeKind::ManagedClass(class) = self.inference.type_store().kind(id) else {
                    unreachable!()
                };
                Type::Known(self.inference.type_store().id_for_managed_reference(*class))
            }
            _ => self.managed_owned_type(declared),
        }
    }

    pub(super) fn managed_owned_type(&mut self, storage: Type) -> Type {
        let map_arguments = match self.inference.shallow(storage) {
            Type::Application(application)
                if self.inference.application_constructor(application)
                    == StdlibTypeConstructorId::Map =>
            {
                Some(self.inference.application_arguments(application).to_vec())
            }
            Type::Known(id) => match self.inference.type_store().kind(id) {
                TypeKind::Application {
                    constructor,
                    arguments,
                    ..
                } if *constructor == StdlibTypeConstructorId::Map => {
                    Some(arguments.iter().copied().map(Type::Known).collect())
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(arguments) = map_arguments {
            let owned = arguments
                .iter()
                .map(|arg| self.managed_owned_type(*arg))
                .collect::<Vec<_>>();
            // Map construction is emitted directly; materialize its private
            // entry/backing layouts even when no local Map constructor is called.
            let entry = self
                .inference
                .application_type(StdlibTypeConstructorId::MapEntry, owned.clone());
            self.inference.array_type(Type::Application(entry));
            return if owned == arguments {
                storage
            } else {
                Type::Application(
                    self.inference
                        .application_type(StdlibTypeConstructorId::Map, owned),
                )
            };
        }
        let shape = match self.inference.shallow(storage) {
            Type::Application(application)
                if self.inference.application_constructor(application)
                    == StdlibTypeConstructorId::List =>
            {
                Some((
                    self.inference.application_arguments(application)[0],
                    None,
                    true,
                ))
            }
            Type::Array(array) => Some((
                self.inference.array_element(array),
                self.inference.array_length(array),
                false,
            )),
            Type::Option(option) => {
                let value = self.inference.option_value(option);
                let owned = self.managed_owned_type(value);
                return if owned == value {
                    storage
                } else {
                    Type::Option(self.inference.option_type(owned))
                };
            }
            Type::Known(id) => match self.inference.type_store().kind(id).clone() {
                TypeKind::Application {
                    constructor,
                    arguments,
                    ..
                } if constructor == StdlibTypeConstructorId::List => {
                    Some((Type::Known(arguments[0]), None, true))
                }
                TypeKind::Array {
                    element, length, ..
                } => Some((Type::Known(element), length, false)),
                TypeKind::Option { value, .. } => {
                    let value = Type::Known(value);
                    let owned = self.managed_owned_type(value);
                    return if owned == value {
                        storage
                    } else {
                        Type::Option(self.inference.option_type(owned))
                    };
                }
                _ => None,
            },
            _ => None,
        };
        let Some((element, length, list)) = shape else {
            return storage;
        };
        let owned = self.managed_owned_type(element);
        if list || owned != element {
            Type::Array(self.inference.array_type_with_length(owned, length))
        } else {
            storage
        }
    }

    fn visible_managed_field(
        &self,
        class_id: crate::ast::ManagedClassId,
        is_static: bool,
        name: &str,
    ) -> Option<crate::ast::ManagedFieldDecl> {
        let class = self
            .declarations
            .managed_classes
            .iter()
            .find(|class| class.id == class_id)?;
        class
            .fields
            .iter()
            .find(|field| field.is_static == is_static && field.name == name)
            .or_else(|| {
                class
                    .conditional_fields
                    .iter()
                    .flat_map(|group| &group.fields)
                    .find(|field| {
                        field.is_static == is_static
                            && field.name == name
                            && self
                                .declarations
                                .conditional_managed_fields
                                .get(&field.id)
                                .is_some_and(|predicate| self.shape_predicate_satisfied(predicate))
                    })
            })
            .cloned()
    }

    fn managed_provider_available(&self) -> bool {
        self.provider_value.is_some_and(|(provider, _)| {
            self.standard_library.state_provider(provider).name == "Unity"
        })
    }

    pub(super) fn visible_state_field(&self, name: &str) -> Option<(crate::ast::ValueId, Type)> {
        self.declarations
            .state_fields
            .get(name)
            .copied()
            .or_else(|| {
                self.declarations
                    .conditional_state_fields
                    .get(name)
                    .and_then(|candidates| {
                        candidates
                            .iter()
                            .find(|(_, _, predicate)| self.shape_predicate_satisfied(predicate))
                            .map(|(field, ty, _)| (*field, *ty))
                    })
            })
            .or_else(|| {
                let variants = self.active_provider_variants.as_ref()?;
                let mut candidates = variants.iter().map(|variant| {
                    self.declarations
                        .provider_state_fields
                        .get(variant)
                        .and_then(|fields| fields.get(name))
                        .copied()
                });
                let first = candidates.next()??;
                let first_storage = self.declarations.state_storage_fields.get(&first.0)?;
                candidates
                    .all(|candidate| {
                        candidate.is_some_and(|candidate| {
                            candidate.1 == first.1
                                && self.declarations.state_storage_fields.get(&candidate.0)
                                    == Some(first_storage)
                        })
                    })
                    .then_some(first)
            })
    }

    /// Resolves a physical field while checking a state source. Provider
    /// alternatives must select their own declaration even when the field is
    /// also part of their canonical shared snapshot interface.
    fn visible_state_source_field(&self, name: &str) -> Option<(crate::ast::ValueId, Type)> {
        self.active_provider_variants
            .as_ref()
            .and_then(|variants| {
                if variants.len() != 1 {
                    return None;
                }
                let variant = variants.iter().next().unwrap();
                self.declarations
                    .provider_state_fields
                    .get(variant)
                    .and_then(|fields| fields.get(name))
                    .copied()
            })
            .or_else(|| {
                self.declarations
                    .conditional_state_fields
                    .get(name)
                    .and_then(|candidates| {
                        candidates
                            .iter()
                            .find(|(_, _, predicate)| self.shape_predicate_satisfied(predicate))
                            .map(|(field, ty, _)| (*field, *ty))
                    })
            })
            .or_else(|| self.declarations.state_fields.get(name).copied())
    }

    fn unknown_state_field(&mut self, name: &str, span: Span) {
        let layouts = self
            .declarations
            .provider_state_fields
            .values()
            .filter(|fields| fields.contains_key(name))
            .count();
        let conditional = self
            .declarations
            .conditional_state_fields
            .get(name)
            .map_or(0, Vec::len);
        if conditional != 0 {
            self.error(
                format!(
                    "state field `{name}` is conditional; access it only where its shape condition is established"
                ),
                span,
            );
        } else if layouts != 0 {
            self.error(
                format!(
                    "state field `{name}` is shape-specific; test or match its enum discriminator before accessing it"
                ),
                span,
            );
        } else {
            self.error(format!("unknown state field `{name}`"), span);
        }
    }

    fn visible_standard_field(
        &self,
        owner: StdlibTypeId,
        name: &str,
    ) -> Option<&'static crate::stdlib::StdlibField> {
        self.standard_library.public_field(owner, name).or_else(|| {
            // Injected standard-library bodies form one trusted implementation
            // unit. They may compose types through runtime-private fields even
            // when the body belongs to a different catalog type. Ordinary user
            // functions never enter this callable context, so private fields
            // remain absent from user lookup, completion, and hover.
            let (CallableContext::LibraryFunction(_) | CallableContext::CompilerGenerated) =
                self.callable
            else {
                return None;
            };
            self.standard_library
                .fields_of(owner)
                .find(|field| field.name == name)
        })
    }

    fn visible_constructor_field(
        &self,
        owner: StdlibTypeConstructorId,
        name: &str,
    ) -> Option<&'static crate::stdlib::StdlibField> {
        self.standard_library
            .public_constructor_field(owner, name)
            .or_else(|| {
                let (CallableContext::LibraryFunction(_) | CallableContext::CompilerGenerated) =
                    self.callable
                else {
                    return None;
                };
                self.standard_library
                    .fields_of_constructor(owner)
                    .find(|field| field.name == name)
            })
    }

    pub(super) fn constructed_field_receiver(
        &self,
        ty: Type,
    ) -> Option<(StdlibTypeConstructorId, Vec<Type>)> {
        match ty {
            Type::Array(array) => Some((
                StdlibTypeConstructorId::Array,
                vec![self.inference.array_element(array)],
            )),
            Type::Option(option) => Some((
                StdlibTypeConstructorId::Option,
                vec![self.inference.option_value(option)],
            )),
            Type::Result(result) => Some((
                StdlibTypeConstructorId::Result,
                vec![self.inference.result_value(result)],
            )),
            Type::Set(set) => Some((
                StdlibTypeConstructorId::Set,
                vec![self.inference.set_element(set)],
            )),
            Type::Application(application) => Some((
                self.inference.application_constructor(application),
                self.inference.application_arguments(application).to_vec(),
            )),
            Type::Range(range) => Some((
                match self.inference.range_kind(range) {
                    crate::ast::RangeKind::Exclusive => StdlibTypeConstructorId::ExclusiveRange,
                    crate::ast::RangeKind::Inclusive => StdlibTypeConstructorId::InclusiveRange,
                },
                vec![self.inference.range_bound(range)],
            )),
            _ => None,
        }
    }

    pub(super) fn resolve_deferred_member_paths(&mut self) {
        let mut pending = std::mem::take(&mut self.deferred_member_paths);
        loop {
            let mut unresolved = Vec::new();
            let mut made_progress = false;
            for deferred in pending {
                let receiver = self.shallow_type(deferred.receiver);
                if matches!(receiver, Type::Variable(_)) {
                    unresolved.push(deferred);
                    continue;
                }
                self.finish_deferred_member_path(deferred, receiver);
                made_progress = true;
            }
            pending = unresolved;
            if pending.is_empty() {
                return;
            }

            let mut variables = Vec::new();
            for deferred in &pending {
                if let Type::Variable(variable) = self.shallow_type(deferred.receiver)
                    && !variables.contains(&variable)
                {
                    variables.push(variable);
                }
            }
            for variable in variables {
                let constraints = pending
                    .iter()
                    .filter(|deferred| {
                        self.shallow_type(deferred.receiver) == Type::Variable(variable)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let contextual_item = constraints
                    .iter()
                    .find_map(|constraint| constraint.library_item);
                let mut candidates = self.member_receiver_types(contextual_item);
                candidates.retain(|candidate| {
                    constraints.iter().all(|constraint| {
                        let Some((result, _)) = self.lookup_members(*candidate, &constraint.fields)
                        else {
                            return false;
                        };
                        match self.shallow_type(constraint.result) {
                            Type::Variable(_) => true,
                            expected => result == expected,
                        }
                    })
                });
                if let [candidate] = candidates.as_slice() {
                    self.unify(Type::Variable(variable), *candidate, constraints[0].span);
                    made_progress = true;
                }
            }
            if !made_progress {
                break;
            }
        }

        let mut diagnosed = HashSet::new();
        for deferred in &pending {
            let Type::Variable(variable) = self.shallow_type(deferred.receiver) else {
                continue;
            };
            if !diagnosed.insert(variable) {
                continue;
            }
            let constraints = pending
                .iter()
                .filter(|candidate| {
                    self.shallow_type(candidate.receiver) == Type::Variable(variable)
                })
                .collect::<Vec<_>>();
            let contextual_item = constraints
                .iter()
                .find_map(|constraint| constraint.library_item);
            let mut candidates = self.member_receiver_types(contextual_item);
            candidates.retain(|candidate| {
                constraints.iter().all(|constraint| {
                    self.lookup_members(*candidate, &constraint.fields)
                        .is_some()
                })
            });
            let fields = constraints
                .iter()
                .flat_map(|constraint| constraint.fields.iter())
                .map(|field| format!("`{field}`"))
                .collect::<Vec<_>>()
                .join(", ");
            let message = if candidates.is_empty() {
                format!("cannot infer a type that provides the accessed fields {fields}")
            } else {
                let candidates = candidates
                    .into_iter()
                    .map(|candidate| self.type_name(candidate))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "member access does not uniquely determine its receiver type; fields {fields} match {candidates}"
                )
            };
            self.error(message, constraints[0].span);
        }
    }

    pub(super) fn finish_deferred_member_path(
        &mut self,
        deferred: DeferredMemberPath,
        receiver: Type,
    ) {
        let Some((result, members)) =
            self.resolve_members(receiver, &deferred.fields, deferred.span)
        else {
            return;
        };
        if self.unify(result, deferred.result, deferred.span).is_some() {
            self.semantics
                .resolve_path_members(deferred.expression, members);
        }
    }

    pub(super) fn lookup_members(
        &mut self,
        mut ty: Type,
        fields: &[String],
    ) -> Option<(Type, Vec<ResolvedMember>)> {
        let mut members = Vec::with_capacity(fields.len());
        for field in fields {
            let (next, member) = self.lookup_member(ty, field)?;
            ty = next;
            members.push(member);
        }
        Some((ty, members))
    }

    pub(super) fn member_receiver_types(
        &mut self,
        contextual_item: Option<StdlibItemId>,
    ) -> Vec<Type> {
        let mut receivers = self
            .standard_library
            .types()
            .filter(|ty| self.standard_library.public_fields(ty.id).next().is_some())
            .map(|ty| self.standard_type(ty.id))
            .chain([Type::Known(
                self.inference.type_store().id_for_state_snapshot(),
            )])
            .chain([Type::Known(
                self.inference.type_store().id_for_settings_view(),
            )])
            .chain(
                self.declarations
                    .structs
                    .iter()
                    .map(|structure| self.struct_type(structure.id)),
            )
            .collect::<Vec<_>>();

        // Generic source-defined standard-library bodies deliberately infer
        // their source parameters. When such a body accesses a structural
        // field through `self`, its catalog declaration is nevertheless an
        // exact, authoritative receiver constraint. Instantiate that receiver
        // here so ordinary deferred member inference can resolve the field and
        // relate its type parameter to the rest of the body.
        if let Some(item_id) = contextual_item {
            let item = *self.standard_library.item(item_id);
            if let ItemKind::Method { receiver } = item.kind
                && let StdlibOwner::TypeConstructor(owner) = item.owner
                && self
                    .standard_library
                    .fields_of_constructor(owner)
                    .next()
                    .is_some()
            {
                let variables = item
                    .signature
                    .type_parameters
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
                let receiver = self.catalog_type(receiver, &variables);
                if !receivers.contains(&receiver) {
                    receivers.push(receiver);
                }
            }
        }

        receivers
    }

    pub(super) fn type_name(&mut self, ty: Type) -> String {
        let ty = self.shallow_type(ty);
        match ty {
            Type::Array(array) => {
                let element = self.inference.array_element(array);
                let element = self.type_name(element);
                match self.inference.array_length(array) {
                    Some(length) => format!("[{element}; {length}]"),
                    None => format!("[{element}]"),
                }
            }
            Type::Option(option) => {
                let value = self.inference.option_value(option);
                format!("{}?", self.type_name(value))
            }
            Type::Result(result) => {
                let value = self.inference.result_value(result);
                format!("{}!", self.type_name(value))
            }
            Type::Async(future) => {
                let value = self.inference.async_value(future);
                format!("async {}", self.type_name(value))
            }
            Type::Iterator(iterator) => {
                let item = self.inference.iterator_item(iterator);
                format!("iterator {}", self.type_name(item))
            }
            Type::Callable(callable) => {
                let parameters = self
                    .inference
                    .callable_parameters(callable)
                    .to_vec()
                    .into_iter()
                    .map(|parameter| self.type_name(parameter))
                    .collect::<Vec<_>>()
                    .join(", ");
                let result = self.type_name(self.inference.callable_result(callable));
                format!("({parameters}) -> {result}")
            }
            Type::Set(set) => {
                let element = self.inference.set_element(set);
                format!("Set<{}>", self.type_name(element))
            }
            Type::Application(application) => {
                let constructor = self.inference.application_constructor(application);
                let name = self.standard_library.type_constructor(constructor).name;
                let arguments = self
                    .inference
                    .application_arguments(application)
                    .to_vec()
                    .into_iter()
                    .map(|argument| self.type_name(argument))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}<{arguments}>")
            }
            Type::Range(range) => {
                let bound = self.type_name(self.inference.range_bound(range));
                format!(
                    "{bound}{}{bound}",
                    self.inference.range_kind(range).operator()
                )
            }
            Type::Variable(_) => "an inferred type".to_owned(),
            Type::Known(id) => match self.inference.type_store().kind(id) {
                TypeKind::Struct(structure) => self
                    .declarations
                    .structs
                    .iter()
                    .find(|candidate| candidate.id == *structure)
                    .map_or_else(|| ty.to_string(), |structure| structure.name.clone()),
                TypeKind::Enum(enumeration) => self
                    .declarations
                    .enums
                    .iter()
                    .find(|candidate| candidate.id == *enumeration)
                    .map_or_else(|| ty.to_string(), |enumeration| enumeration.name.clone()),
                TypeKind::StateSnapshot => "StateSnapshot".to_owned(),
                TypeKind::SettingsView => "SettingsView".to_owned(),
                _ => self.inference.known_type_name(id),
            },
        }
    }

    pub(super) fn inference_error_message(&mut self, error: InferenceError) -> String {
        match error {
            InferenceError::Message(message) => message,
            InferenceError::TypeMismatch { left, right } => format!(
                "types do not match: `{}` and `{}`",
                self.type_name(left),
                self.type_name(right)
            ),
            InferenceError::UnsupportedOperation { ty, requirements }
            | InferenceError::UnsatisfiedConstraints { ty, requirements } => {
                self.capability_failure_message(ty, &requirements)
            }
            InferenceError::IntegerLiteralOutOfRange(ty) => {
                format!("integer literal does not fit in `{}`", self.type_name(ty))
            }
        }
    }

    fn capability_failure_message(&mut self, ty: Type, requirements: &Requirements) -> String {
        let ty_name = self.type_name(ty);
        let requirements = self
            .standard_library
            .minimal_capabilities(requirements.as_slice());
        let names = requirements
            .iter()
            .map(|capability| self.standard_library.capability(*capability).name)
            .collect::<Vec<_>>();
        let requirement = match names.as_slice() {
            [] => "the required capabilities".to_owned(),
            [name] => format!("the required `{name}` capability"),
            _ => format!("the required capabilities `{}`", names.join("` + `")),
        };
        let mut message = format!("type `{ty_name}` does not satisfy {requirement}");

        let finite = requirements.iter().all(|capability| {
            self.standard_library.capability(*capability).behavior == CapabilityBehavior::Declared
        });
        if finite {
            let store = self.inference.type_store();
            let mut accepted = self
                .standard_library
                .core_types()
                .iter()
                .filter(|candidate| {
                    let ty = Type::Known(store.id_for_core(candidate.id));
                    requirements.iter().all(|capability| {
                        type_may_have_capability(&self.standard_library, store, ty, *capability)
                    })
                })
                .map(|candidate| candidate.name)
                .chain(
                    self.standard_library
                        .types()
                        .filter(|candidate| {
                            let ty = Type::Known(store.id_for_standard(candidate.id));
                            requirements.iter().all(|capability| {
                                type_may_have_capability(
                                    &self.standard_library,
                                    store,
                                    ty,
                                    *capability,
                                )
                            })
                        })
                        .map(|candidate| candidate.name),
                )
                .collect::<Vec<_>>();
            accepted.sort_unstable();
            if !accepted.is_empty() {
                let accepted = match accepted.as_slice() {
                    [only] => (*only).to_owned(),
                    [head @ .., last] => format!("{} or {last}", head.join(", ")),
                    [] => unreachable!(),
                };
                message.push_str(&format!("; accepted concrete types are {accepted}"));
            }
        }
        message
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

fn member_name_span(span: Span, name: &str) -> Span {
    Span {
        start: span.end.saturating_sub(name.len()),
        end: span.end,
    }
}

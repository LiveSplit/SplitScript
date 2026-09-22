//! Statement, lexical-scope, and callable-control-flow checking.

use std::collections::HashMap;

use crate::{
    ast::{
        ActionKind, BindingPattern, Block, Expr, ExprKind, Span, Stmt, SuspensionMode, VariableDecl,
    },
    inference::{Requirements, Type},
};

use super::{
    Checker,
    context::{CallableContext, DebugContext, ExpressionMode, NonePolicy},
    declarations::Binding,
    pattern_usefulness::PatternCoverage,
};

impl Checker {
    pub(super) fn bind_irrefutable_pattern(
        &mut self,
        declaration: &BindingPattern,
        ty: Type,
        mutable: bool,
        debug_only: bool,
        site: &str,
    ) {
        let bindings = self.check_irrefutable_pattern(declaration, ty, mutable, debug_only, site);
        for (name, binding) in bindings {
            let duplicate = self
                .scopes
                .iter()
                .rev()
                .any(|scope| scope.contains_key(&name))
                || (!self.is_library_function() && self.declarations.globals.contains_key(&name))
                || self.is_provider_value_name(&name);
            if duplicate {
                self.error(
                    format!("variable `{name}` is already declared"),
                    binding.declaration_span.unwrap_or(declaration.pattern.span),
                );
            } else {
                self.scopes.last_mut().unwrap().insert(name, binding);
            }
        }
    }

    pub(super) fn bind_irrefutable_parameter(
        &mut self,
        declaration: &BindingPattern,
        ty: Type,
        debug_only: bool,
        site: &str,
        duplicate_kind: &str,
    ) {
        let bindings = self.check_irrefutable_pattern(declaration, ty, true, debug_only, site);
        for (name, binding) in bindings {
            if self.is_provider_value_name(&name) {
                self.error(
                    format!("`{name}` is reserved by the state provider"),
                    binding.declaration_span.unwrap_or(declaration.pattern.span),
                );
                continue;
            }
            if self
                .scopes
                .last_mut()
                .unwrap()
                .insert(name.clone(), binding)
                .is_some()
            {
                self.error(
                    format!("duplicate {duplicate_kind} parameter `{name}`"),
                    binding.declaration_span.unwrap_or(declaration.pattern.span),
                );
            }
        }
    }

    pub(super) fn check_irrefutable_pattern(
        &mut self,
        declaration: &BindingPattern,
        ty: Type,
        mutable: bool,
        debug_only: bool,
        site: &str,
    ) -> HashMap<String, Binding> {
        if let Some(previous) = self.semantics.pending_value_type(declaration.id) {
            self.unify(previous, ty, declaration.pattern.span);
        } else {
            self.semantics.resolve_value_type(declaration.id, ty);
        }
        self.scopes.push(HashMap::new());
        let checked = self.check_pattern(
            &declaration.pattern.kind,
            declaration.pattern.id,
            ty,
            declaration.pattern.span,
        );
        let mut bindings = self.scopes.pop().expect("a binding pattern owns a scope");

        let missing = if matches!(checked.coverage, PatternCoverage::Invalid(_)) {
            Vec::new()
        } else {
            self.missing_patterns(std::slice::from_ref(&checked.coverage), ty)
        };
        if !missing.is_empty() {
            let witnesses = missing
                .iter()
                .map(|pattern| format!("`{}`", pattern.display()))
                .collect::<Vec<_>>()
                .join(", ");
            self.errors.push(
                crate::Diagnostic::type_error(
                    format!("refutable pattern in {site}"),
                    declaration.pattern.span,
                )
                .with_primary_label(format!("this pattern does not match {witnesses}"))
                .with_note("use `is` or `match` when the value may have another shape"),
            );
        }

        for binding in bindings.values_mut() {
            binding.mutable = mutable;
            binding.debug_only = debug_only;
        }
        bindings
    }

    pub(super) fn block(&mut self, block: &Block, nested: bool) {
        if nested {
            self.scopes.push(HashMap::new());
        }
        for statement in &block.statements {
            self.statement(statement);
        }
        if nested {
            self.scopes.pop();
        }
    }

    pub(super) fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Debug {
                statement: inner,
                span,
            } => {
                if contains_control_flow_expression(inner) {
                    self.error(
                        "`debug` currently supports bindings, expression statements, assignments, `if`, `while`, `for`, and `await`/`retry` statements",
                        *span,
                    );
                }
                self.with_debug_context(DebugContext::DebugOnly, |checker| {
                    checker.statement(inner);
                });
            }
            Stmt::Variable(variable) => self.variable(variable),
            Stmt::Assign {
                id,
                name,
                op,
                value,
                span,
            } => {
                let binding = self.binding_for_use(name, *span);
                match binding {
                    Some(binding) if !binding.mutable => {
                        if let Some(target) = binding.id {
                            self.semantics.resolve_assignment(*id, target);
                        }
                        self.error(format!("cannot assign to constant `{name}`"), *span)
                    }
                    Some(binding) => {
                        if let Some(target) = binding.id {
                            self.semantics.resolve_assignment(*id, target);
                            if self.is_attachment_shape_global(target)
                                && !matches!(
                                    self.callable,
                                    CallableContext::Action(ActionKind::OnAttach)
                                )
                            {
                                self.error(
                                    format!(
                                        "attachment-shape global `{name}` can only be assigned in `onAttach`"
                                    ),
                                    *span,
                                );
                            }
                        }
                        if let Some(op) = op {
                            if let Some(right_type) = self.expr(value, None)
                                && !self.diagnose_string_compound_assignment(
                                    *op, binding.ty, right_type, *span,
                                )
                            {
                                let resolved = binding.id.and_then(|target| {
                                    self.resolve_assignment_operator(
                                        *id, *op, binding.ty, right_type, target, *span,
                                    )
                                });
                                if let Some(result) = resolved {
                                    if let Some(declaration_span) = binding.declaration_span {
                                        let binding_type = self.type_name(binding.ty);
                                        self.with_expected_type_source(
                                            super::ExpectedTypeSource {
                                                span: declaration_span,
                                                label: format!(
                                                    "variable `{name}` has type `{binding_type}`"
                                                ),
                                            },
                                            |checker| {
                                                checker.unify_expected(result, binding.ty, *span);
                                            },
                                        );
                                    } else {
                                        self.unify_expected(result, binding.ty, *span);
                                    }
                                } else if let Some(operand_type) =
                                    self.unify(binding.ty, right_type, *span)
                                {
                                    self.require_binary_operand(*op, operand_type, *span);
                                }
                            }
                        } else {
                            if let Some(declaration_span) = binding.declaration_span {
                                let binding_type = self.type_name(binding.ty);
                                self.with_expected_type_source(
                                    super::ExpectedTypeSource {
                                        span: declaration_span,
                                        label: format!(
                                            "variable `{name}` has type `{binding_type}`"
                                        ),
                                    },
                                    |checker| checker.expr(value, Some(binding.ty)),
                                );
                            } else {
                                self.expr(value, Some(binding.ty));
                            }
                        }
                    }
                    None => self.unknown_variable(name, *span),
                }
            }
            Stmt::StateAssign {
                id,
                target,
                op,
                value,
                span,
            } => {
                let target_type = self.expr(target, None);
                let field = match &target.kind {
                    crate::ast::ExprKind::Path(path) => path.get(1),
                    _ => None,
                }
                .and_then(|name| self.visible_state_field(name));
                let Some((field, field_type)) = field else {
                    self.expr(value, None);
                    return;
                };
                self.semantics.resolve_assignment(*id, field);
                let declaration_span = self.declarations.state_field_spans[&field];
                if let Some(op) = op {
                    let Some(right_type) = self.expr(value, None) else {
                        return;
                    };
                    if self.diagnose_string_compound_assignment(*op, field_type, right_type, *span)
                    {
                        return;
                    }
                    let resolved = self.resolve_state_assignment_operator(
                        *id, *op, field_type, right_type, field, *span,
                    );
                    if let Some(result) = resolved {
                        let field_type_name = self.type_name(field_type);
                        self.with_expected_type_source(
                            super::ExpectedTypeSource {
                                span: declaration_span,
                                label: format!("state field has type `{field_type_name}`"),
                            },
                            |checker| checker.unify_expected(result, field_type, *span),
                        );
                    } else if let Some(operand_type) = self.unify(field_type, right_type, *span) {
                        self.require_binary_operand(*op, operand_type, *span);
                    }
                } else {
                    let field_type_name = self.type_name(field_type);
                    self.with_expected_type_source(
                        super::ExpectedTypeSource {
                            span: declaration_span,
                            label: format!("state field has type `{field_type_name}`"),
                        },
                        |checker| {
                            checker.expr(value, target_type.or(Some(field_type)));
                        },
                    );
                }
            }
            Stmt::IndexAssign {
                id,
                target,
                op,
                value,
                span,
            } => {
                let Some(element_type) = self.expr(target, None) else {
                    self.expr(value, None);
                    return;
                };
                let Some(right_type) = self.expr(value, None) else {
                    return;
                };
                if self.diagnose_string_compound_assignment(*op, element_type, right_type, *span) {
                    return;
                }
                if let Some(result) = self.resolve_index_assignment_operator(
                    *id,
                    *op,
                    element_type,
                    right_type,
                    target.id,
                    *span,
                ) {
                    self.unify(result, element_type, *span);
                } else if let Some(operand_type) = self.unify(element_type, right_type, *span) {
                    self.require_binary_operand(*op, operand_type, *span);
                }
                if let crate::ast::ExprKind::Index { receiver, .. } = &target.kind
                    && let Some(receiver_type) =
                        self.semantics.inferred_expression_type(receiver.id)
                {
                    self.resolve_index_setter(*id, receiver_type, receiver.id, receiver.span);
                }
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                let flow = self.check_condition(condition);
                let constraints = self.shape_constraints(condition);
                let inverse_constraints = self.inverse_shape_constraints(condition);
                self.with_condition_path(flow.when_true.as_ref(), |checker| {
                    checker.with_shape_constraints(constraints.as_deref(), |checker| {
                        checker.block(then_block, true);
                    });
                });
                if let Some(else_block) = else_block {
                    self.with_condition_path(flow.when_false.as_ref(), |checker| {
                        checker.with_shape_constraints(inverse_constraints.as_deref(), |checker| {
                            checker.block(else_block, true);
                        });
                    });
                }
            }
            Stmt::While {
                condition, body, ..
            } => {
                let flow = self.check_condition(condition);
                self.with_condition_path(flow.when_true.as_ref(), |checker| {
                    checker.with_loop(|checker| checker.block(body, true));
                });
            }
            Stmt::For {
                binding,
                iterable_value,
                index_value,
                version_value,
                iterable,
                body,
                ..
            } => {
                // Empty literals have no elements from which to infer `T`, but
                // the loop body can still constrain its binding. Seed only
                // that otherwise-ambiguous shape; non-empty and named arrays
                // retain their exact source type (including `[T; N]`).
                let empty_array_hint = matches!(&iterable.kind, crate::ast::ExprKind::Array(values) if values.is_empty())
                    .then(|| {
                        let element = self.fresh_inference(Requirements::none(), None);
                        (Type::Array(self.array_type_id(element)), element)
                    });
                let iterable_ty = self.expr(iterable, empty_array_hint.map(|(array, _)| array));
                let iterable_ty = iterable_ty.unwrap_or_else(|| {
                    empty_array_hint.map_or_else(
                        || {
                            let element = self.fresh_inference(Requirements::none(), None);
                            Type::Array(self.array_type_id(element))
                        },
                        |(array, _)| array,
                    )
                });
                let mut iterable_ty = self.shallow_type(iterable_ty);
                // Source-defined methods intentionally omit catalog-generic
                // annotations at the parser boundary and let ordinary
                // inference recover them. When the iterable is that method's
                // still-unbound receiver, its catalog owner supplies the
                // concrete `Iterable` constructor. This is the same
                // contextual receiver constraint used by deferred member
                // resolution, applied before projecting `Iterable.Iterator`.
                if matches!(iterable_ty, Type::Variable(_))
                    && let crate::typeck::CallableContext::LibraryFunction(item) = self.callable
                    && let crate::stdlib::StdlibOwner::TypeConstructor(constructor) =
                        self.standard_library.item(item).owner
                    && self.standard_library.type_constructor_has_capability(
                        constructor,
                        crate::stdlib::StdlibCapabilityId::Iterable,
                    )
                {
                    let declaration = self.standard_library.type_constructor(constructor);
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
                        .collect::<std::collections::HashMap<_, _>>();
                    let receiver = self.catalog_application_type(
                        constructor,
                        vec![variables[declaration.parameters[0].name]],
                    );
                    self.unify(iterable_ty, receiver, iterable.span);
                    iterable_ty = self.shallow_type(receiver);
                }
                let iterable_capability = crate::stdlib::StdlibCapabilityId::Iterable;
                let iterator_capability = crate::stdlib::StdlibCapabilityId::Iterator;
                let mut consumes_iterator = false;
                let mut converts_iterable = false;
                let (element_ty, projected_iterator_ty) = if matches!(iterable_ty, Type::Variable(_)) {
                    // An associated cursor such as `T.Iterator` already carries
                    // the `Iterator` constraint declared by `Iterable`. Preserve
                    // that stronger protocol instead of adding an unrelated
                    // `Iterable` requirement merely because its concrete type is
                    // not known until a generic call is specialized.
                    let capability = match iterable_ty {
                        Type::Variable(variable)
                            if self.standard_library.capabilities_satisfy(
                                self.inference.variable_requirements(variable).as_slice(),
                                iterator_capability,
                            ) =>
                        {
                            iterator_capability
                        }
                        _ => iterable_capability,
                    };
                    self.inference
                        .require(
                            iterable_ty,
                            Requirements::capability(capability),
                        )
                        .ok()
                        .map(|()| {
                            if capability == iterator_capability {
                                consumes_iterator = true;
                                (
                                    self.inference.associated_type(
                                        iterable_ty,
                                        iterator_capability,
                                        "Item",
                                    ),
                                    None,
                                )
                            } else {
                                converts_iterable = true;
                                let iterator_ty = self.inference.associated_type(
                                    iterable_ty,
                                    iterable_capability,
                                    "Iterator",
                                );
                                (
                                    self.inference.associated_type(
                                        iterator_ty,
                                        iterator_capability,
                                        "Item",
                                    ),
                                    Some(iterator_ty),
                                )
                            }
                        })
                } else if let Type::Iterator(iterator) = iterable_ty {
                    consumes_iterator = true;
                    Some((self.inference.iterator_item(iterator), None))
                } else if let Type::Known(receiver) = iterable_ty
                    && let crate::types::TypeKind::Iterator { item, .. } =
                        self.inference.type_store().kind(receiver)
                {
                    consumes_iterator = true;
                    Some((Type::Known(*item), None))
                } else if let Type::Known(receiver) = iterable_ty
                    && matches!(
                        self.inference.type_store().kind(receiver),
                        crate::types::TypeKind::Struct(_) | crate::types::TypeKind::Enum(_)
                    )
                {
                    let has_method = |name: &str| {
                        self.declarations
                            .methods
                            .contains_key(&(iterable_ty, name.to_owned()))
                    };
                    if has_method("next") {
                        consumes_iterator = true;
                        Some((
                            self.inference.associated_type(
                                iterable_ty,
                                iterator_capability,
                                "Item",
                            ),
                            None,
                        ))
                    } else if has_method("iterator") {
                        converts_iterable = true;
                        let iterator_ty = self.inference.associated_type(
                            iterable_ty,
                            iterable_capability,
                            "Iterator",
                        );
                        Some((
                            self.inference.associated_type(
                                iterator_ty,
                                iterator_capability,
                                "Item",
                            ),
                            Some(iterator_ty),
                        ))
                    } else {
                        None
                    }
                } else {
                    self.constructed_field_receiver(iterable_ty).and_then(
                        |(constructor, arguments)| {
                        let declaration = self.standard_library.type_constructor(constructor);
                        if self.standard_library.type_constructor_has_capability(
                            constructor, iterator_capability,
                        ) {
                            consumes_iterator = true;
                        } else if self.standard_library.type_constructor_has_capability(
                            constructor, iterable_capability,
                        ) {
                            // Arrays, sets, and ranges have dedicated loop
                            // lowering. Ordinary constructed `Iterable` values
                            // use their declared iterator protocol.
                            converts_iterable = matches!(iterable_ty, Type::Application(_));
                        } else {
                            return None;
                        }
                        let variables = declaration
                            .parameters
                            .iter()
                            .zip(arguments)
                            .map(|(parameter, argument)| (parameter.name, argument))
                            .collect();
                        if consumes_iterator {
                            declaration
                                .associated_types
                                .iter()
                                .find(|associated| associated.name == "Item")
                                .map(|associated| {
                                    (self.catalog_type(associated.value, &variables), None)
                                })
                        } else {
                            declaration
                                .associated_types
                                .iter()
                                .find(|associated| associated.name == "Iterator")
                                .map(|associated| {
                                    let iterator_ty =
                                        self.catalog_type(associated.value, &variables);
                                    let item_ty = self.inference.associated_type(
                                        iterator_ty,
                                        iterator_capability,
                                        "Item",
                                    );
                                    (item_ty, Some(iterator_ty))
                                })
                        }
                    },
                    )
                }
                .unwrap_or_else(|| {
                        let actual = self.type_name(iterable_ty);
                        self.error(
                            format!(
                                "`for ... in` requires an `Iterable` or `Iterator` value, but this expression has type `{actual}`"
                            ),
                            iterable.span,
                        );
                        (self.fresh_inference(Requirements::none(), None), None)
                    });
                // A literal range keeps its upper bound directly in the
                // compiler-owned iterable slot. This lets the backend lower a
                // direct range loop without allocating the first-class range
                // object that is needed when a range escapes into a value.
                let iterable_storage_ty = if converts_iterable {
                    projected_iterator_ty.unwrap_or_else(|| {
                        self.inference
                            .associated_type(iterable_ty, iterable_capability, "Iterator")
                    })
                } else if !consumes_iterator
                    && matches!(iterable.kind, crate::ast::ExprKind::Range { .. })
                {
                    element_ty
                } else {
                    iterable_ty
                };
                self.semantics
                    .resolve_value_type(*iterable_value, iterable_storage_ty);
                let is_range = match self.shallow_type(iterable_ty) {
                    Type::Range(_) => true,
                    Type::Known(id) => matches!(
                        self.inference.type_store().kind(id),
                        crate::types::TypeKind::Range { .. }
                    ),
                    _ => false,
                };
                let index_ty = if consumes_iterator || converts_iterable {
                    self.catalog_application_type(
                        crate::stdlib::StdlibTypeConstructorId::IteratorStep,
                        vec![element_ty],
                    )
                } else if is_range {
                    element_ty
                } else {
                    self.core_type(crate::stdlib::CoreTypeId::U32)
                };
                self.semantics.resolve_value_type(*index_value, index_ty);
                self.semantics.resolve_value_type(
                    *version_value,
                    self.core_type(crate::stdlib::CoreTypeId::U32),
                );
                self.scopes.push(HashMap::new());
                self.bind_irrefutable_pattern(
                    &binding.binding,
                    element_ty,
                    false,
                    self.debug_context.is_debug(),
                    "`for` binding",
                );
                self.with_loop(|checker| checker.block(body, false));
                self.scopes.pop();
            }
            Stmt::Suspend {
                mode,
                binding,
                returns,
                value,
                span,
            } => {
                if self.generator_item.is_some() {
                    let keyword = match mode {
                        SuspensionMode::Await => "await",
                        SuspensionMode::Retry => "retry",
                    };
                    self.error(
                        format!("`{keyword}` is not available inside a synchronous generator"),
                        *span,
                    );
                } else if !self.callable.can_suspend() {
                    let keyword = match mode {
                        SuspensionMode::Await => "await",
                        SuspensionMode::Retry => "retry",
                    };
                    self.error(
                        format!(
                            "`{keyword}` is only available inside `onAttach`, `whileAttached`, or an inferred async function"
                        ),
                        *span,
                    );
                }
                let result = self
                    .with_expression_mode(ExpressionMode::SuspensionOperand, |checker| {
                        checker.expr(value, None)
                    });
                let result = result.and_then(|result| {
                    let result = match mode {
                        SuspensionMode::Await => {
                            let supported = self
                                .semantics
                                .call_is_provisionally_awaitable(value.id, &self.standard_library);
                            if !supported {
                                self.error("this operation is not awaitable", value.span);
                                return None;
                            }
                            match self.shallow_type(result) {
                                Type::Async(future) => self.inference.async_value(future),
                                Type::Result(result) => self.inference.result_value(result),
                                result => result,
                            }
                        }
                        SuspensionMode::Retry => match self.shallow_type(result) {
                            Type::Result(result) => self.inference.result_value(result),
                            _ => {
                                self.error(
                                    "`retry` expects an expression of type `T!`",
                                    value.span,
                                );
                                return None;
                            }
                        },
                    };
                    let expected = if *returns {
                        Some(self.return_ty)
                    } else {
                        binding
                            .as_ref()
                            .and_then(|binding| binding.annotation)
                            .map(|ty| self.syntax_type(ty))
                    };
                    expected.map_or(Some(result), |expected| {
                        let source = if *returns {
                            self.return_type_source.clone()
                        } else {
                            binding.as_ref().and_then(|binding| {
                                binding.annotation.map(|_| {
                                    let expected_name = self.type_name(expected);
                                    super::ExpectedTypeSource {
                                        span: binding.span,
                                        label: format!(
                                            "variable `{}` is declared as `{expected_name}`",
                                            binding.name
                                        ),
                                    }
                                })
                            })
                        };
                        if let Some(source) = source {
                            self.with_expected_type_source(source, |checker| {
                                checker.unify_expected(result, expected, value.span)
                            })
                        } else {
                            self.unify_expected(result, expected, value.span)
                        }
                    })
                });
                if let Some(binding) = binding {
                    let ty = result.unwrap_or_else(|| self.error_type());
                    let duplicate = self
                        .scopes
                        .iter()
                        .rev()
                        .any(|scope| scope.contains_key(&binding.name))
                        || (!self.is_library_function()
                            && self.declarations.globals.contains_key(&binding.name))
                        || self.is_provider_value_name(&binding.name);
                    if duplicate {
                        self.error(
                            format!("variable `{}` is already declared", binding.name),
                            binding.span,
                        );
                    }
                    self.semantics.resolve_value_type(binding.id, ty);
                    self.scopes.last_mut().unwrap().insert(
                        binding.name.clone(),
                        Binding {
                            id: Some(binding.id),
                            ty,
                            mutable: true,
                            debug_only: self.debug_context.is_debug(),
                            declaration_span: Some(binding.span),
                        },
                    );
                }
            }
            Stmt::Yield { value, span } => {
                let Some(item) = self.generator_item else {
                    self.expr(value, None);
                    self.error(
                        "`yield` is only available inside a generator function declared `-> iterator T`",
                        *span,
                    );
                    return;
                };
                self.expr(value, Some(item));
            }
            Stmt::Expression(expr) => {
                if statement.is_implicitly_debug_only() {
                    if contains_control_flow_expression(statement) {
                        self.error(
                            "`inspect(...)` used as a statement cannot contain `throw`, `return`, `break`, or `continue` because it is erased from release builds",
                            expr.span,
                        );
                    }
                    self.with_debug_context(DebugContext::DebugOnly, |checker| {
                        checker.expr(expr, None);
                    });
                } else {
                    self.expr(expr, None);
                }
            }
        }
    }

    pub(super) fn check_return(&mut self, value: Option<&Expr>, span: Span) {
        if self.generator_item.is_some() {
            if let Some(value) = value {
                self.expr(value, None);
                self.error(
                    "a generator cannot return a value; produce values with `yield`",
                    span,
                );
            }
            return;
        }
        let returns_none = self.return_ty == self.core_type(crate::stdlib::CoreTypeId::None);
        match (returns_none, self.return_ty, value) {
            (true, _, None) => {}
            (true, _, Some(value)) if !self.callable.is_function() => {
                self.expr(value, None);
                self.error("this lifecycle block cannot return a value", span);
            }
            (_, expected, Some(value)) => {
                let policy = if !self.callable.is_function()
                    && matches!(
                        self.callable.action(),
                        Some(ActionKind::IsLoading | ActionKind::GameTime)
                    ) {
                    NonePolicy::DomainNullable
                } else {
                    NonePolicy::OptionalOnly
                };
                self.with_none_policy(policy, |checker| {
                    checker.with_expression_mode(ExpressionMode::DirectReturn, |checker| {
                        if let Some(source) = checker.return_type_source.clone() {
                            checker.with_expected_type_source(source, |checker| {
                                checker.expr(value, Some(expected));
                            });
                        } else {
                            checker.expr(value, Some(expected));
                        }
                    });
                });
            }
            (false, _, None)
                if !self.callable.is_function()
                    && matches!(
                        self.callable.action(),
                        Some(
                            ActionKind::Start
                                | ActionKind::SelectProcess
                                | ActionKind::WhileAttached
                                | ActionKind::Split
                                | ActionKind::Reset
                                | ActionKind::IsLoading
                                | ActionKind::GameTime
                        )
                    ) => {}
            (false, expected, None) => {
                let expected = self.type_name(expected);
                let mut diagnostic = crate::Diagnostic::type_error(
                    format!("expected a return value of type `{expected}`"),
                    span,
                )
                .with_primary_label("this return has no value");
                if let Some(source) = self.return_type_source.clone() {
                    diagnostic = diagnostic.with_secondary_label(source.span, source.label);
                }
                self.errors.push(diagnostic);
            }
        }
    }

    fn variable(&mut self, variable: &VariableDecl) {
        let value = variable
            .value
            .as_ref()
            .expect("local variables have initializers");
        let expected = variable.annotation.map(|ty| {
            let ty = self.syntax_type(ty);
            self.inference.freshen_omitted_array_shapes(ty)
        });
        let inferred = if let Some(expected) = expected {
            let expected_name = self.type_name(expected);
            let label = variable.binding.simple_binding().map_or_else(
                || format!("binding pattern is declared as `{expected_name}`"),
                |binding| {
                    format!(
                        "variable `{}` is declared as `{expected_name}`",
                        binding.name
                    )
                },
            );
            self.with_expected_type_source(
                super::ExpectedTypeSource {
                    span: variable.name_span,
                    label,
                },
                |checker| checker.expr(value, Some(expected)),
            )
        } else {
            self.expr(value, None)
        };
        let mut ty = inferred.unwrap_or_else(|| self.error_type());
        let unsupported_standard = self.standard_type_id(ty).is_some_and(|standard| {
            !self
                .standard_library
                .type_decl(standard)
                .value_usage
                .local_variable
        });
        if unsupported_standard {
            let name = self.type_name(ty);
            self.error(
                format!("local variables cannot currently store `{name}`"),
                variable.span,
            );
            ty = self.error_type();
        }
        self.bind_irrefutable_pattern(
            &variable.binding,
            ty,
            variable.mutable,
            self.debug_context.is_debug() || variable.debug_only,
            "variable declaration",
        );
    }

    pub(super) fn binding(&self, name: &str) -> Option<Binding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
            .or_else(|| self.declarations.globals.get(name).copied())
    }

    pub(super) fn binding_for_use(&mut self, name: &str, span: Span) -> Option<Binding> {
        let binding = self.binding(name)?;
        if binding.debug_only && !self.debug_context.is_debug() {
            self.error(
                format!("debug-only binding `{name}` can only be used from debug code"),
                span,
            );
        }
        Some(binding)
    }

    pub(super) fn unknown_variable(&mut self, name: &str, span: Span) {
        if let Some((_, declaration)) = self
            .conditional_binding_declarations
            .iter()
            .rev()
            .find(|(candidate, _)| candidate == name)
        {
            self.errors.push(
                crate::Diagnostic::type_error(
                    format!(
                        "conditional pattern binding `{name}` is not available on this control-flow path"
                    ),
                    span,
                )
                .with_primary_label("the pattern is not proven to match here")
                .with_secondary_label(*declaration, "the binding is introduced by this pattern")
                .with_note(
                    "use the binding only on an `is` outcome that proves the pattern matched",
                ),
            );
        } else {
            self.error(format!("unknown variable `{name}`"), span);
        }
    }

    pub(super) fn bind_pattern_value(
        &mut self,
        binding: &crate::ast::PatternBinding,
        ty: Type,
        span: Span,
    ) {
        if self.is_provider_value_name(&binding.name) {
            self.error(
                format!("`{}` is reserved by the state provider", binding.name),
                span,
            );
        }
        if self.scopes.last().unwrap().contains_key(&binding.name) {
            self.error(
                format!("pattern binds `{}` more than once", binding.name),
                binding.name_span,
            );
            return;
        }
        if let Some(previous) = self.semantics.pending_value_type(binding.id) {
            self.unify(previous, ty, binding.name_span);
        } else {
            self.semantics.resolve_value_type(binding.id, ty);
        }
        self.scopes.last_mut().unwrap().insert(
            binding.name.clone(),
            Binding {
                id: Some(binding.id),
                ty,
                mutable: false,
                debug_only: self.debug_context.is_debug(),
                declaration_span: Some(binding.name_span),
            },
        );
    }
}

fn contains_control_flow_expression(statement: &Stmt) -> bool {
    #[derive(Default)]
    struct Finder(bool);

    impl<'ast> crate::visit::Visitor<'ast> for Finder {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if matches!(
                expression.kind,
                ExprKind::Break(_) | ExprKind::Continue | ExprKind::Return(_) | ExprKind::Throw(_)
            ) {
                self.0 = true;
            } else {
                crate::visit::walk_expr(self, expression);
            }
        }
    }

    let mut finder = Finder::default();
    crate::visit::Visitor::visit_stmt(&mut finder, statement);
    finder.0
}
